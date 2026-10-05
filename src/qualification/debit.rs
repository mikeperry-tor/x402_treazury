//! Fair, bounded receipt verification during managed reconciliation, outside the payer gate.
use super::*;
use crate::rotation::{
    base::{BaseRpc, TransferExpectation, TransferProof},
    store::StoreHandle,
};
use rusqlite::OptionalExtension;

pub async fn reconcile_debit(base: &BaseRpc, store: &StoreHandle, pool: &str) -> Result<()> {
    let Some(guard) = ACTIVE.get().filter(|g| g.managed).cloned() else {
        return Ok(());
    };
    let binding = guard.binding.clone();
    let pool = pool.to_owned();
    let candidate = tokio::task::spawn_blocking(move || candidate(&binding, &pool))
        .await?
        .inspect_err(|_| observation_failed("receipt_selection"))?;
    let Some(candidate) = candidate else {
        return Ok(());
    };
    let snapshot = store.call(|s| s.qualification_state()).await?;
    let expected = receipt_expectation(&candidate.payment, &candidate.receipt, &snapshot)
        .inspect_err(|_| observation_failed("journal_validation"))?;
    let result = base.verify_transfer(&expected).await;
    let binding = guard.binding.clone();
    tokio::task::spawn_blocking(move || record(&binding, &candidate, result)).await??;
    Ok(())
}
fn observation_failed(stage: &'static str) {
    tracing::warn!(
        category = "qualification_debit_observation_failed",
        stage,
        "qualification receipt observation failed before RPC verification; payment evidence remains incomplete, no paid retry is permitted"
    );
}
struct Candidate {
    payment: Value,
    receipt: Value,
}
fn candidate(binding: &Binding, pool: &str) -> Result<Option<Candidate>> {
    let db = connection(binding)?;
    // One receipt per reconciliation pass. Oldest checked goes last, so a missing
    // receipt cannot indefinitely starve later payments in the same pool.
    // Registry events mix JSON observations and plain-text preparation markers.
    // SQLite can reorder predicates and simplify kind-based CASE expressions.
    // Guard extraction with json_valid itself, but explicitly refuse malformed
    // observations of relevant kinds so the guard cannot silently discard them.
    let malformed: i64 = db.query_row(
        "SELECT COUNT(*) FROM events WHERE run=?1 AND kind IN
        ('application_payment','application_receipt','application_debit_verified','application_debit_check')
        AND NOT json_valid(detail)", [&binding.run], |r| r.get(0))?;
    ensure!(
        malformed == 0,
        "malformed structured payment observation; debit verification refused"
    );
    let query = "SELECT p.detail,r.detail FROM events p JOIN events r ON r.run=p.run
        AND r.kind='application_receipt'
        AND json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.case')=json_extract(CASE WHEN json_valid(p.detail) THEN p.detail END,'$.case')
        AND json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.attempt_id')=json_extract(CASE WHEN json_valid(p.detail) THEN p.detail END,'$.attempt_id')
        WHERE p.run=?1 AND p.kind='application_payment' AND json_extract(CASE WHEN json_valid(p.detail) THEN p.detail END,'$.pool')=?2
        AND json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.receipt.classification')='seller_success'
        AND NOT EXISTS(SELECT 1 FROM events v WHERE v.run=p.run AND v.kind='application_debit_verified'
          AND json_extract(CASE WHEN json_valid(v.detail) THEN v.detail END,'$.attempt_id')=json_extract(CASE WHEN json_valid(p.detail) THEN p.detail END,'$.attempt_id'))";
    let count: i64 = db.query_row(
        &format!("SELECT COUNT(*) FROM ({query})"),
        params![binding.run, pool],
        |r| r.get(0),
    )?;
    ensure!(
        count <= 10000,
        "debit verification queue exceeds 10000 receipts"
    );
    if count > 1 {
        tracing::info!(
            pending_receipts = count,
            checked_per_pass = 1,
            "qualification debit verification backlog; remaining receipts stay queued for later reconciliation passes"
        );
    }
    let row: Option<(String,String)> = db.query_row(
        &format!("{query} ORDER BY COALESCE((SELECT MAX(c.seq) FROM events c WHERE c.run=p.run
          AND c.kind='application_debit_check' AND json_extract(CASE WHEN json_valid(c.detail) THEN c.detail END,'$.attempt_id')=json_extract(CASE WHEN json_valid(p.detail) THEN p.detail END,'$.attempt_id')),0),p.seq LIMIT 1"),
        params![binding.run,pool], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    row.map(|(payment, receipt)| {
        Ok(Candidate {
            payment: bounded(payment)?,
            receipt: bounded(receipt)?,
        })
    })
    .transpose()
}
/// Construct a read-only chain query from correlated admission, seller receipt,
/// and public durable journal evidence. This grants no execution authority and
/// neither opens keys nor changes payment state. The caller must authenticate
/// the run/treasury provenance before accepting a returned chain proof.
pub fn receipt_expectation(
    payment: &Value,
    receipt: &Value,
    snapshot: &Value,
) -> Result<TransferExpectation> {
    for field in ["case", "attempt_id", "session"] {
        ensure!(
            payment[field].as_str().is_some_and(|s| !s.is_empty())
                && payment[field] == receipt[field],
            "receipt correlation missing or mismatched: {field}"
        );
    }
    ensure!(
        receipt["receipt"]["classification"] == "seller_success"
            && receipt["receipt"]["network"] == "eip155:8453",
        "receipt is not a successful Base seller claim"
    );
    let mut attempts = snapshot["payment_attempts"]
        .as_array()
        .context("payment snapshot missing")?
        .iter()
        .filter(|a| a["id"] == payment["attempt_id"]);
    let attempt = attempts.next().context("receipt payment attempt missing")?;
    ensure!(
        attempts.next().is_none(),
        "duplicate receipt payment attempt"
    );
    for (field, recorded) in [
        ("pool", "pool"),
        ("wallet", "wallet"),
        ("generation", "generation"),
        ("amount", "amount"),
        ("requirements_hash", "requirements_hash"),
    ] {
        ensure!(
            !attempt[field].is_null() && attempt[field] == payment[recorded],
            "receipt admission differs from journal: {field}"
        );
    }
    ensure!(
        matches!(
            attempt["state"].as_str(),
            Some("POSSIBLY_SUBMITTED" | "RESOLVED")
        ),
        "receipt has no journaled authorization"
    );
    let payer = attempt["payer"]
        .as_str()
        .context("payer missing")?
        .parse()?;
    let admitted: alloy_primitives::Address = payment["address"]
        .as_str()
        .context("admitted address missing")?
        .parse()?;
    ensure!(payer == admitted, "receipt payer differs from admission");
    if let Some(claimed) = receipt["receipt"].get("payer") {
        let claimed: alloy_primitives::Address = claimed
            .as_str()
            .context("receipt payer malformed")?
            .parse()?;
        ensure!(payer == claimed, "seller receipt names a different payer");
    }
    let amount = attempt["amount"]
        .as_str()
        .context("amount missing")?
        .parse::<alloy_primitives::U256>()?;
    ensure!(
        amount > alloy_primitives::U256::ZERO,
        "zero expected payment amount"
    );
    Ok(TransferExpectation {
        transaction: receipt["receipt"]["transaction"]
            .as_str()
            .context("receipt transaction missing")?
            .parse()?,
        payer,
        payee: attempt["payee"]
            .as_str()
            .context("payee missing")?
            .parse()?,
        amount,
        nonce: attempt["nonce"]
            .as_str()
            .context("nonce missing")?
            .parse()?,
    })
}
fn record(
    binding: &Binding,
    candidate: &Candidate,
    result: Result<Option<TransferProof>>,
) -> Result<()> {
    let mut db = connection(binding)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut event = json!({"case":candidate.payment["case"],"session":candidate.payment["session"],
        "attempt_id":candidate.payment["attempt_id"]});
    let proof = match result {
        Ok(Some(proof)) => {
            event["status"] = json!("verified");
            Some(proof)
        }
        Ok(None) => {
            event["status"] = json!("pending");
            None
        }
        Err(error) => {
            event["status"] = json!("unavailable");
            event["reason"] = json!(crate::rotation::base::safe_diagnostic(&error));
            None
        }
    };
    tx.execute(
        "INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_debit_check',?2,?3)",
        params![binding.run, event.to_string(), now()?],
    )?;
    if let Some(proof) = proof {
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_debit_verified' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.attempt_id')=?2)",
            params![binding.run,event["attempt_id"].as_str()], |r|r.get(0))?;
        if !exists {
            event["proof"] = serde_json::to_value(proof)?;
            tx.execute("INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_debit_verified',?2,?3)",
                params![binding.run,event.to_string(),now()?])?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn add(db: &Connection, id: &str, session: &str) -> Value {
        let payment = json!({"case":id,"attempt_id":id,"session":session,"pool":"pool","wallet":"w","generation":0,
            "amount":"7","address":format!("0x{:040x}",1),"requirements_hash":"requirements"});
        let receipt = json!({"case":id,"attempt_id":id,"session":session,
            "receipt":{"classification":"seller_success","network":"eip155:8453","transaction":format!("0x{:064x}",2)}});
        for (kind, event) in [
            ("application_payment", payment.clone()),
            ("application_receipt", receipt),
        ] {
            db.execute(
                "INSERT INTO events(run,kind,detail,at) VALUES('run',?1,?2,1)",
                params![kind, event.to_string()],
            )
            .unwrap();
        }
        json!({"id":id,"pool":"pool","wallet":"w","generation":0,"amount":"7","requirements_hash":"requirements",
            "state":"POSSIBLY_SUBMITTED","payer":format!("0x{:040x}",1),"payee":format!("0x{:040x}",3),"nonce":format!("0x{:064x}",4)})
    }
    #[test]
    fn detached_receipt_observation_requires_complete_unique_correlation() {
        let (_dir, guard, db) = super::super::tests::fixture();
        let attempt = add(&db, "first", &guard.binding.session);
        let selected = candidate(&guard.binding, "pool").unwrap().unwrap();
        let snapshot = json!({"payment_attempts":[attempt.clone()]});
        let check = |payment: &Value, receipt: &Value, state: &Value| {
            receipt_expectation(payment, receipt, state)
        };
        check(&selected.payment, &selected.receipt, &snapshot).unwrap();
        for field in ["case", "attempt_id", "session"] {
            let mut receipt = selected.receipt.clone();
            receipt[field] = json!("another");
            assert!(
                check(&selected.payment, &receipt, &snapshot).is_err(),
                "{field}"
            );
            let mut payment = selected.payment.clone();
            payment.as_object_mut().unwrap().remove(field);
            receipt.as_object_mut().unwrap().remove(field);
            assert!(
                check(&payment, &receipt, &snapshot).is_err(),
                "missing {field}"
            );
        }
        for (field, value) in [
            ("classification", json!("seller_failure")),
            ("network", json!("eip155:1")),
            ("network", Value::Null),
            ("payer", json!(format!("0x{:040x}", 9))),
            ("transaction", json!("malformed")),
        ] {
            let mut receipt = selected.receipt.clone();
            receipt["receipt"][field] = value;
            assert!(
                check(&selected.payment, &receipt, &snapshot).is_err(),
                "{field}"
            );
        }
        let duplicate = json!({"payment_attempts":[attempt.clone(),attempt]});
        assert!(check(&selected.payment, &selected.receipt, &duplicate).is_err());
        for field in [
            "pool",
            "wallet",
            "generation",
            "amount",
            "requirements_hash",
        ] {
            let mut state = snapshot.clone();
            let mut payment = selected.payment.clone();
            state["payment_attempts"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            payment.as_object_mut().unwrap().remove(field);
            assert!(
                check(&payment, &selected.receipt, &state).is_err(),
                "missing {field}"
            );
        }
        let mut state = snapshot.clone();
        state["payment_attempts"][0]["state"] = json!("ADMITTED");
        assert!(check(&selected.payment, &selected.receipt, &state).is_err());
        state["payment_attempts"][0]["state"] = json!("RESOLVED");
        check(&selected.payment, &selected.receipt, &state).unwrap();
        let mut payment = selected.payment.clone();
        payment["amount"] = json!("0");
        state["payment_attempts"][0]["amount"] = json!("0");
        assert!(check(&payment, &selected.receipt, &state).is_err());
    }
    #[test]
    fn pending_receipts_do_not_starve_others_and_verified_records_are_idempotent() {
        let (_dir, guard, db) = super::super::tests::fixture();
        // Production `prepared` markers contain raw digests, not JSON. Include
        // unrelated malformed prose and real indexes so query planning cannot
        // accidentally hide a join-order dependency in the regression.
        db.execute_batch(
            "INSERT INTO events(run,kind,detail,at) VALUES('run','prepared','not-json-digest',1);
            INSERT INTO events(run,kind,detail,at) VALUES('run','unrelated','also not JSON',1);
            CREATE INDEX receipt_lookup_fixture ON events(run,kind);
            CREATE UNIQUE INDEX receipt_expression_fixture ON events(run,json_extract(detail,'$.attempt_id')) WHERE kind='application_payment';",
        )
        .unwrap();
        let first = add(&db, "first", &guard.binding.session);
        let second = add(&db, "second", &guard.binding.session);
        assert!(candidate(&guard.binding, "other_pool").unwrap().is_none());
        let selected = candidate(&guard.binding, "pool").unwrap().unwrap();
        assert_eq!(selected.payment["case"], "first");
        let snapshot = json!({"payment_attempts":[first,second]});
        let expected =
            receipt_expectation(&selected.payment, &selected.receipt, &snapshot).unwrap();
        assert_eq!(expected.amount, alloy_primitives::U256::from(7));
        let mut wrong = snapshot.clone();
        wrong["payment_attempts"][0]["amount"] = json!("8");
        assert!(receipt_expectation(&selected.payment, &selected.receipt, &wrong).is_err());
        record(&guard.binding, &selected, Ok(None)).unwrap();
        let selected = candidate(&guard.binding, "pool").unwrap().unwrap();
        assert_eq!(selected.payment["case"], "second");
        let expected =
            receipt_expectation(&selected.payment, &selected.receipt, &snapshot).unwrap();
        let proof = TransferProof {
            transaction: expected.transaction.to_string(),
            block_height: 100,
            block_hash: format!("0x{:064x}", 5),
            payer: expected.payer.to_string(),
            payee: expected.payee.to_string(),
            amount_atomic: "7".into(),
            nonce: expected.nonce.to_string(),
        };
        record(&guard.binding, &selected, Ok(Some(proof.clone()))).unwrap();
        record(&guard.binding, &selected, Ok(Some(proof))).unwrap();
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE kind='application_debit_verified'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            candidate(&guard.binding, "pool").unwrap().unwrap().payment["case"],
            "first"
        );
        db.execute(
            "UPDATE events SET detail='malformed receipt' WHERE kind='application_receipt'",
            [],
        )
        .unwrap();
        assert!(candidate(&guard.binding, "pool").is_err());
    }
}
