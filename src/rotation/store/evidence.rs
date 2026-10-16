//! Exact supporting values live with the journal; runtime credentials never survive reopen.
pub(crate) mod repair;
pub(crate) mod replacement;
use super::*;
use crate::rotation::base::{
    Anchor,
    evidence::{BalanceObservation, EvidenceFailure},
};

#[cfg(test)]
pub(super) fn initialize_in_transaction(tx: &Connection) -> Result<()> {
    tx.execute_batch("
ALTER TABLE wallets ADD COLUMN financial_height INTEGER;
ALTER TABLE wallets ADD COLUMN financial_hash TEXT;
ALTER TABLE wallets ADD COLUMN debit_height INTEGER;
ALTER TABLE wallets ADD COLUMN debit_hash TEXT;
ALTER TABLE wallets ADD COLUMN evidence_repair TEXT;
ALTER TABLE wallets ADD COLUMN unresolved_exposure TEXT NOT NULL DEFAULT '0';
ALTER TABLE payment_attempts ADD COLUMN reconciliation_sequence INTEGER;
WITH ordered AS (SELECT id,ROW_NUMBER() OVER (ORDER BY id) AS sequence FROM payment_attempts)
UPDATE payment_attempts SET reconciliation_sequence=(SELECT sequence FROM ordered WHERE ordered.id=payment_attempts.id);
CREATE UNIQUE INDEX payment_reconciliation_sequence ON payment_attempts(reconciliation_sequence);
CREATE INDEX payment_pending_sequence ON payment_attempts(pool_id,state,reconciliation_sequence);
CREATE INDEX payment_wallet_state ON payment_attempts(wallet_id,state);
ALTER TABLE payment_admission_evidence ADD COLUMN support TEXT;
ALTER TABLE payment_admission_evidence ADD COLUMN latest_height INTEGER;
ALTER TABLE payment_admission_evidence ADD COLUMN latest_hash TEXT;
ALTER TABLE payment_admission_evidence ADD COLUMN endpoint TEXT;
CREATE INDEX admission_confirmed_support ON payment_admission_evidence(block_hash,attempt_id);
CREATE INDEX admission_latest_support ON payment_admission_evidence(latest_hash,attempt_id);
ALTER TABLE payment_resolutions ADD COLUMN support TEXT;
CREATE INDEX resolution_anchor_support ON payment_resolutions(hash,attempt_id);
CREATE INDEX IF NOT EXISTS refund_operation_outputs ON refund_outputs(operation_id);
PRAGMA user_version=14;")?;
    // Decimal text is never summed by SQLite's floating point coercion. This is
    // a one-time journal derivation, not an ordinary admission/history sweep.
    {
        let mut rows = tx.prepare("SELECT wallet_id,amount FROM payment_attempts WHERE state!='RESOLVED' ORDER BY wallet_id")?;
        let mut rows = rows.query([])?;
        let mut wallet = String::new();
        let mut total = U256::ZERO;
        while let Some(row) = rows.next()? {
            let next: String = row.get(0)?;
            if next != wallet {
                if !wallet.is_empty() {
                    tx.execute(
                        "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
                        params![wallet, total.to_string()],
                    )?;
                }
                wallet = next;
                total = U256::ZERO;
            }
            total = total
                .checked_add(amount(&row.get::<_, String>(1)?)?)
                .context("reservation overflow")?;
        }
        if !wallet.is_empty() {
            tx.execute(
                "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
                params![wallet, total.to_string()],
            )?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceSupport {
    pub observation: BalanceObservation,
    pub checkpoint: Option<Anchor>,
}

const EXPOSURE_SQL: &str = "SELECT pool_id,amount,1 FROM payment_attempts WHERE wallet_id=?1 AND state='POSSIBLY_SUBMITTED' UNION ALL SELECT a.pool_id,a.amount,EXISTS(SELECT 1 FROM payment_resolutions r WHERE r.attempt_id=a.id) FROM payment_attempts a JOIN wallets w ON w.id=a.wallet_id WHERE a.wallet_id=?1 AND a.state='RESOLVED' AND w.payment_resolutions_disputed=1";

pub(super) fn exposure(db: &Connection, wallet: &str) -> Result<U256> {
    exposure_from_rows(db, wallet, EXPOSURE_SQL)
}

// Historical read-only reporting retains its original row-level dispute contract.
// Runtime ownership only opens the current schema and never calls this decoder.
pub(super) fn historical_exposure(db: &Connection, wallet: &str) -> Result<U256> {
    exposure_from_rows(
        db,
        wallet,
        "SELECT pool_id,amount,resolution_disputed=0 FROM payment_attempts WHERE wallet_id=?1 AND state='POSSIBLY_SUBMITTED' UNION ALL SELECT a.pool_id,a.amount,a.state='RESOLVED' AND EXISTS(SELECT 1 FROM payment_resolutions r WHERE r.attempt_id=a.id) FROM payment_attempts a WHERE a.wallet_id=?1 AND a.resolution_disputed=1",
    )
}
fn exposure_from_rows(db: &Connection, wallet: &str, sql: &str) -> Result<U256> {
    let pool: String =
        db.query_row("SELECT pool_id FROM wallets WHERE id=?1", [wallet], |row| {
            row.get(0)
        })?;
    // Disjoint, wallet-indexed subsets: pending submissions and previously
    // released authorizations whose canonical support is disputed.
    let mut statement = db.prepare(sql)?;
    let mut rows = statement.query([wallet])?;
    let mut total = U256::ZERO;
    while let Some(row) = rows.next()? {
        let owner: String = row.get(0).map_err(|_| EvidenceFailure::MalformedExposure)?;
        let raw: String = row.get(1).map_err(|_| EvidenceFailure::MalformedExposure)?;
        let value = amount(&raw).map_err(|_| EvidenceFailure::MalformedExposure)?;
        ensure!(
            owner == pool
                && value > U256::ZERO
                && row
                    .get::<_, bool>(2)
                    .map_err(|_| EvidenceFailure::MalformedExposure)?,
            EvidenceFailure::MalformedExposure
        );
        total = total
            .checked_add(value)
            .context(EvidenceFailure::MalformedExposure)?;
    }
    Ok(total)
}

#[cfg(test)]
fn projected_exposure(db: &Connection, wallet: &str) -> Result<U256> {
    let value: String = db.query_row(
        "SELECT unresolved_exposure FROM wallets WHERE id=?1",
        [wallet],
        |row| row.get(0),
    )?;
    amount(&value)
}
#[cfg(test)]
pub(super) fn add_exposure(db: &Connection, wallet: &str, value: U256) -> Result<()> {
    let total = projected_exposure(db, wallet)?
        .checked_add(value)
        .context("reservation overflow")?;
    ensure!(
        db.execute(
            "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
            params![wallet, total.to_string()]
        )? == 1,
        "missing liability wallet"
    );
    Ok(())
}
#[cfg(test)]
pub(super) fn subtract_exposure(db: &Connection, wallet: &str, value: U256) -> Result<()> {
    let total = projected_exposure(db, wallet)?
        .checked_sub(value)
        .context("reservation underflow")?;
    ensure!(
        db.execute(
            "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
            params![wallet, total.to_string()]
        )? == 1,
        "missing liability wallet"
    );
    Ok(())
}

pub(super) fn next_sequence(db: &Connection) -> Result<i64> {
    let previous: i64 = db.query_row(
        "SELECT COALESCE(MAX(reconciliation_sequence),0) FROM payment_attempts",
        [],
        |row| row.get(0),
    )?;
    previous
        .checked_add(1)
        .context("reconciliation sequence exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_history_derivation_streams_exact_amounts_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let identity = store.id().to_owned();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "history",
            "1",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        let wallets = store.status().unwrap().pools.remove(0).addresses;
        let cost = U256::from(10).pow(U256::from(30));
        let mut expected = [U256::ZERO; 2];
        let tx = store.db.transaction().unwrap();
        for number in 0..2048 {
            let owner = number % 2;
            let state = if number % 3 == 0 {
                "RESOLVED"
            } else {
                "POSSIBLY_SUBMITTED"
            };
            tx.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES(?1,?2,?3,0,?4,'fixture',?5)",params![format!("attempt-{number:08}"),pool,wallets[owner].id,cost.to_string(),state]).unwrap();
            if state != "RESOLVED" {
                expected[owner] += cost;
            }
        }
        tx.commit().unwrap();
        store.db.execute_batch("DROP VIEW effective_payment_resolutions; DROP TABLE payment_resolution_replacements; DROP TABLE payment_resolution_repairs;").unwrap();
        store
            .db
            .execute_batch(include_str!(
                "../../../tests/fixtures/state/remove_evidence_v14.sql"
            ))
            .unwrap();
        // Exercise the conversion primitive explicitly. Runtime open must not
        // perform an automatic upgrade of this populated synthetic history.
        let version: i64 = store
            .db
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        store.db.execute_batch("PRAGMA user_version=13;").unwrap();
        let tx = store.db.transaction().unwrap();
        initialize_in_transaction(&tx).unwrap();
        tx.execute_batch(include_str!("evidence/replacement_schema.sql"))
            .unwrap();
        tx.pragma_update(None, "user_version", version).unwrap();
        tx.commit().unwrap();
        drop(store);
        let store = Store::open(&state, &key, &identity).unwrap();
        for (wallet, expected) in wallets.iter().zip(expected) {
            assert_eq!(exposure(&store.db, &wallet.id).unwrap(), expected);
            let checkpoint: Option<i64> = store
                .db
                .query_row(
                    "SELECT financial_height FROM wallets WHERE id=?1",
                    [&wallet.id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                checkpoint, None,
                "migration cannot invent financial support"
            );
        }
        assert_eq!(next_sequence(&store.db).unwrap(), 2049);
        let earliest: String = store
            .db
            .query_row(
                "SELECT id FROM payment_attempts ORDER BY reconciliation_sequence LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(earliest, "attempt-00000000");
        drop(store);
        let store = Store::open(&state, &key, &identity).unwrap();
        assert_eq!(next_sequence(&store.db).unwrap(), 2049);
    }

    #[tokio::test]
    async fn real_certificates_preserve_age_recheck_lineage_and_reject_predebit_or_conflicting_balances()
     {
        use crate::rotation::base::{BaseRpc, now};
        use axum::{Json, Router, routing::post};
        use serde_json::{Value, json};
        let state = std::sync::Arc::new(std::sync::Mutex::new((100u64, 100u64)));
        let rpc_state = state.clone();
        let app = Router::new().route("/",post(move |Json(request):Json<Value>| {
            let state = rpc_state.clone();
            async move {
                let (head,amount) = *state.lock().unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let height = if tag == "latest" {head} else {u64::from_str_radix(&tag[2..],16).unwrap()};
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
                    }
                    "eth_call" => { assert_eq!(request["params"][1]["requireCanonical"],true);
                        let value = if request["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231") {amount} else {1};
                        json!(format!("0x{value:064x}")) },
                    _ => panic!("unexpected RPC mutation"),
                };
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc = BaseRpc::new(
            &format!("http://{}/", listener.local_addr().unwrap()),
            12,
            120,
        )
        .unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "first",
            "1",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        let wallet = store
            .status()
            .unwrap()
            .pools
            .remove(0)
            .addresses
            .remove(0)
            .id;
        // Canonical current balances can cover fully reserved historical
        // releases without verifying their now-disputed block hashes.
        store.db.execute("UPDATE wallets SET evidence_repair='chain_repair_required',financial_height=80,financial_hash='disputed',debit_height=80,debit_hash='disputed' WHERE id=?1", [&wallet]).unwrap();
        store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES('capacity-old',?1,?2,0,'1','fixture','RESOLVED')", params![pool,wallet]).unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET payment_resolutions_disputed=1 WHERE id=?1",
                [&wallet],
            )
            .unwrap();
        store.db.execute("INSERT INTO payment_resolutions(attempt_id,outcome,height,hash,block_time,support) VALUES('capacity-old','USED',80,'disputed',1,'immutable')", []).unwrap();
        let mut query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let capacity_capture = store.capture_payment_evidence(&wallet).unwrap();
        assert!(capacity_capture.checkpoint.is_none());
        assert!(capacity_capture.previous.is_none());
        query.captures.insert(wallet.clone(), capacity_capture);
        let capacity_view = rpc.view(query).await.unwrap();
        let certificate = &capacity_view.balance_certificates[&wallet];
        assert!(check_payment_balance(&store.db, store.evidence_namespace, certificate).is_ok());
        assert!(check_balance(&store.db, store.evidence_namespace, certificate).is_err());
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(1));
        for update in [
            "financial_hash='changed'",
            "debit_hash='changed'",
            "debit_height=99",
            "evidence_repair='evidence_repair_required'",
        ] {
            store
                .db
                .execute(
                    &format!("UPDATE wallets SET {update} WHERE id=?1"),
                    [&wallet],
                )
                .unwrap();
            assert!(
                check_payment_balance(&store.db, store.evidence_namespace, certificate).is_err()
            );
            store.db.execute("UPDATE wallets SET evidence_repair='chain_repair_required',financial_height=80,financial_hash='disputed',debit_height=80,debit_hash='disputed' WHERE id=?1", [&wallet]).unwrap();
        }
        store
            .db
            .execute(
                "UPDATE wallets SET payment_resolutions_disputed=0 WHERE id=(SELECT wallet_id FROM payment_attempts WHERE id='capacity-old')",
                [],
            )
            .unwrap();
        assert!(check_payment_balance(&store.db, store.evidence_namespace, certificate).is_err());
        assert!(store.capture_payment_evidence(&wallet).is_err());
        store
            .db
            .execute(
                "UPDATE wallets SET payment_resolutions_disputed=1 WHERE id=(SELECT wallet_id FROM payment_attempts WHERE id='capacity-old')",
                [],
            )
            .unwrap();
        assert!(check_payment_balance(&store.db, store.evidence_namespace, certificate).is_ok());
        assert!(store.prepare_evidence_view(capacity_view.clone()).is_err());
        assert!(
            store
                .prepare_payment_evidence_view(capacity_view.clone())
                .is_ok()
        );
        // A fresh capture above the confirmed debit floor still cannot admit.
        store
            .db
            .execute("UPDATE wallets SET debit_height=99 WHERE id=?1", [&wallet])
            .unwrap();
        let mut query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        query.captures.insert(
            wallet.clone(),
            store.capture_payment_evidence(&wallet).unwrap(),
        );
        let before_debit = rpc.view(query).await.unwrap();
        assert!(
            check_payment_balance(
                &store.db,
                store.evidence_namespace,
                &before_debit.balance_certificates[&wallet]
            )
            .is_err()
        );
        store
            .db
            .execute("UPDATE wallets SET debit_height=80 WHERE id=?1", [&wallet])
            .unwrap();
        bump_generation(&store.db, "wallet", &wallet).unwrap();
        assert!(check_payment_balance(&store.db, store.evidence_namespace, certificate).is_err());
        let plan: Vec<String> = store
            .db
            .prepare(&format!("EXPLAIN QUERY PLAN {UNDISPUTED_SQL}"))
            .unwrap()
            .query_map([&wallet], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            plan.iter()
                .any(|line| line.contains("payment_wallet_state"))
        );
        store.evidence_cache.clear();
        store
            .db
            .execute(
                "DELETE FROM payment_resolutions WHERE attempt_id='capacity-old'",
                [],
            )
            .unwrap();
        assert!(store.capture_payment_evidence(&wallet).is_err());
        store
            .db
            .execute("DELETE FROM payment_attempts WHERE id='capacity-old'", [])
            .unwrap();
        store.db.execute("UPDATE wallets SET payment_resolutions_disputed=0,evidence_repair=NULL,financial_height=NULL,financial_hash=NULL,debit_height=NULL,debit_hash=NULL WHERE id=?1", [&wallet]).unwrap();
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let first = rpc.view(query).await.unwrap();
        store.publish_scoped_evidence(&pool, first).unwrap();
        let saved = store.evidence_cache[&wallet].1.observation().clone();
        let runtime =
            crate::rotation::store::reporting::runtime_status(&store, saved.valid_until).unwrap();
        let evidence = runtime.pools[0].addresses[0]
            .runtime_balance_evidence
            .as_ref()
            .unwrap();
        assert!(evidence.fresh && evidence.compatible);
        assert_eq!(
            evidence.confirmed.amount_atomic,
            saved.confirmed.amount.to_string()
        );
        assert_eq!(
            evidence.latest.amount_atomic,
            saved.latest.amount.to_string()
        );
        assert_eq!(evidence.confirmed.hash, saved.confirmed.anchor.hash);
        assert_eq!(evidence.latest.hash, saved.latest.anchor.hash);
        assert!(evidence.confirmed.block_timestamp.is_none());
        assert_eq!(evidence.valid_until, saved.valid_until);
        let expired =
            crate::rotation::store::reporting::runtime_status(&store, saved.valid_until + 1)
                .unwrap();
        let evidence = expired.pools[0].addresses[0]
            .runtime_balance_evidence
            .as_ref()
            .unwrap();
        assert!(!evidence.fresh && evidence.compatible);
        assert_eq!(
            evidence.unavailable_reason.as_deref(),
            Some("evidence_expired")
        );
        assert_eq!(evidence.valid_until, saved.valid_until);
        // A corrected wall clock cannot revive the cached certificate or a raw
        // view clone acquired before another clone merges with that cache.
        assert!(
            !store.evidence_cache[&wallet]
                .1
                .is_fresh(now().unwrap())
                .unwrap()
        );
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let raw = rpc.view(query).await.unwrap();
        let raw_clone = raw.clone();
        let detached = raw.clone();
        assert!(
            raw.balance_certificates[&wallet]
                .is_fresh(now().unwrap())
                .unwrap()
        );
        let merged = store.prepare_evidence_view(raw).unwrap();
        assert!(
            !merged.balance_certificates[&wallet]
                .is_fresh(now().unwrap())
                .unwrap()
        );
        assert!(
            matches!(store.prepare_evidence_view(raw_clone), Err(error) if error.downcast_ref::<EvidenceFailure>() == Some(&EvidenceFailure::Expired))
        );
        let cached = store.evidence_cache.remove(&wallet).unwrap();
        let expected = crate::rotation::manager::PaymentCandidateHandle {
            wallet: wallet.clone(),
            address: detached.observations[&wallet].address.clone(),
        };
        let rejected = store.prepare_payment(
            &pool,
            U256::from(1),
            "detached-expiry",
            detached,
            Some(&expected),
        );
        assert!(
            matches!(rejected, Err(error) if error.downcast_ref::<EvidenceFailure>() == Some(&EvidenceFailure::Expired)),
            "detached raw clone must not regain authority after cache eviction"
        );
        assert!(
            !store.evidence_cache.contains_key(&wallet),
            "rejected detached handle must not poison the cache"
        );
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let newly_acquired = store
            .prepare_evidence_view(rpc.view(query).await.unwrap())
            .unwrap();
        assert!(
            newly_acquired.balance_certificates[&wallet]
                .is_fresh(now().unwrap())
                .unwrap(),
            "new acquisition may establish age after cache eviction"
        );
        store.evidence_cache.insert(wallet.clone(), cached);
        assert!(
            crate::rotation::store::status(&dir.path().join("state"))
                .unwrap()
                .pools[0]
                .addresses[0]
                .runtime_balance_evidence
                .is_none()
        );
        let expired_at = now().unwrap() - 1;
        store
            .evidence_cache
            .get_mut(&wallet)
            .unwrap()
            .1
            .restrict_validity(expired_at)
            .unwrap();
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let equivalent = store
            .prepare_evidence_view(rpc.view(query).await.unwrap())
            .unwrap();
        assert_eq!(
            equivalent.observations[&wallet].valid_until, expired_at,
            "equivalent anchors cannot renew even an expired saved observation"
        );
        *state.lock().unwrap() = (120, 90);
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let mut raced = rpc.view(query).await.unwrap();
        let original_age = raced.observations[&wallet].valid_until;
        let elapsed_expired = raced.balance_certificates[&wallet].clone();
        elapsed_expired.advance_freshness_for_test(std::time::Duration::from_secs(121));
        assert!(now().unwrap() <= original_age);
        assert!(!elapsed_expired.is_fresh(now().unwrap()).unwrap());
        let anchor = |height| Anchor {
            height,
            hash: format!("0x{height:064x}"),
        };
        advance_checkpoint(&store.db, &wallet, &anchor(110), false).unwrap();
        assert_eq!(
            store
                .publish_scoped_evidence(&pool, raced.clone())
                .unwrap_err()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::CompatibilityRequired)
        );
        let certificate = rpc
            .recheck_balance(
                raced.balance_certificates[&wallet].clone(),
                store.capture_evidence(&wallet).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(certificate.observation().valid_until, original_age);
        assert!(
            !certificate.is_fresh(now().unwrap()).unwrap(),
            "header recheck must preserve elapsed expiration"
        );
        raced
            .balance_certificates
            .insert(wallet.clone(), certificate);
        store.publish_scoped_evidence(&pool, raced.clone()).unwrap();
        assert!(store.cached_payment_view(&pool, &wallet).unwrap().is_none());
        let elapsed_status =
            crate::rotation::store::reporting::runtime_status(&store, now().unwrap()).unwrap();
        assert!(
            !elapsed_status.pools[0].addresses[0]
                .runtime_balance_evidence
                .as_ref()
                .unwrap()
                .fresh
        );
        advance_checkpoint(&store.db, &wallet, &anchor(120), true).unwrap();
        assert_eq!(
            store
                .publish_scoped_evidence(&pool, raced)
                .unwrap_err()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::BeforeDebit)
        );
        *state.lock().unwrap() = (140, 80);
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        store
            .publish_scoped_evidence(&pool, rpc.view(query).await.unwrap())
            .unwrap();
        let durable_observation = store.evidence_cache[&wallet].1.observation().clone();
        *state.lock().unwrap() = (140, 70);
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        assert_eq!(
            store
                .publish_scoped_evidence(&pool, rpc.view(query).await.unwrap())
                .unwrap_err()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::AmountContradiction)
        );
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::ZERO);
        assert_eq!(checkpoint(&store.db, &wallet).unwrap(), Some(anchor(120)));
        assert!(!store.evidence_cache.contains_key(&wallet));
        let payer: String = store
            .db
            .query_row(
                "SELECT address FROM wallets WHERE id=?1",
                [&wallet],
                |row| row.get(0),
            )
            .unwrap();
        let nonce = format!("0x{:064x}", 1);
        let valid_before = now().unwrap() + 100;
        store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) VALUES('history-proof',?1,?2,0,'7','fixture','POSSIBLY_SUBMITTED',?3,?4,?5,1)",params![pool,wallet,payer,nonce,i64::try_from(valid_before).unwrap()]).unwrap();
        add_exposure(&store.db, &wallet, U256::from(7)).unwrap();
        let authorization = crate::rotation::base::PendingAuthorization {
            id: "history-proof".into(),
            wallet: wallet.clone(),
            payer,
            nonce,
            valid_before,
        };
        let query = store
            .scoped_chain_query(&pool, vec![], vec![authorization])
            .unwrap();
        let mut historical = rpc.view(query).await.unwrap();
        let original = historical.resolution_certificates["history-proof"].clone();
        store
            .invalidate_wallet_evidence(&wallet, &anchor(140), false, false)
            .unwrap();
        store
            .invalidate_wallet_evidence(&wallet, &anchor(140), false, false)
            .unwrap();
        assert_eq!(
            store
                .publish_scoped_evidence(&pool, historical.clone())
                .unwrap_err()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::Invalidated)
        );
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(7));
        let verified = rpc
            .recheck_resolution(original, store.capture_evidence(&wallet).unwrap())
            .await
            .unwrap();
        historical
            .resolution_certificates
            .insert("history-proof".into(), verified.clone());
        store
            .publish_scoped_evidence(&pool, historical.clone())
            .unwrap();
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::ZERO);
        assert_eq!(checkpoint(&store.db, &wallet).unwrap(), Some(anchor(128)));
        store.publish_scoped_evidence(&pool, historical).unwrap();
        assert_eq!(
            exposure(&store.db, &wallet).unwrap(),
            U256::ZERO,
            "equivalent proof is idempotent"
        );
        store
            .invalidate_endpoint_evidence(verified.endpoint())
            .unwrap();
        assert_eq!(
            rpc.recheck_resolution(verified, store.capture_evidence(&wallet).unwrap())
                .await
                .err()
                .unwrap()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::Invalidated)
        );
        let durable_support = BalanceSupport {
            observation: durable_observation.clone(),
            checkpoint: Some(anchor(120)),
        };
        store.db.execute("INSERT INTO payment_admission_evidence(attempt_id,block_height,block_hash,balance,reserved_before,valid_until,support,latest_height,latest_hash,endpoint,payer_role) VALUES('history-proof',128,?1,'80','0',?2,?3,140,?4,?5,'ACTIVE')",params![durable_observation.confirmed.anchor.hash,durable_observation.valid_until.to_string(),serde_json::to_string(&durable_support).unwrap(),durable_observation.latest.anchor.hash,durable_observation.endpoint]).unwrap();
        let identity = store.id().to_owned();
        drop(store);
        let mut store = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &identity,
        )
        .unwrap();
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        assert_eq!(
            store
                .publish_scoped_evidence(&pool, rpc.view(query).await.unwrap())
                .unwrap_err()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::AmountContradiction)
        );
        let repair: Option<String> = store
            .db
            .query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [&wallet],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            repair.as_deref(),
            Some("chain_repair_required"),
            "endpoint distrust preserves released exposure independently of admission balances"
        );
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(7));
        assert_eq!(checkpoint(&store.db, &wallet).unwrap(), Some(anchor(128)));
        *state.lock().unwrap() = (140, 80);
        let mut wrong_height = durable_support.clone();
        wrong_height.observation.confirmed.anchor.height += 1;
        wrong_height.observation.latest.anchor.height += 1;
        let mut missing_address = serde_json::to_value(&durable_support).unwrap();
        missing_address["observation"]
            .as_object_mut()
            .unwrap()
            .remove("address");
        let mut missing_validity = serde_json::to_value(&durable_support).unwrap();
        missing_validity["observation"]
            .as_object_mut()
            .unwrap()
            .remove("valid_until");
        for (support, failure) in [
            (
                serde_json::to_string(&wrong_height).unwrap(),
                EvidenceFailure::AmountContradiction,
            ),
            ("{broken".into(), EvidenceFailure::MalformedSupport),
            ("{}".into(), EvidenceFailure::MalformedSupport),
            (
                missing_address.to_string(),
                EvidenceFailure::MalformedSupport,
            ),
            (
                missing_validity.to_string(),
                EvidenceFailure::MalformedSupport,
            ),
        ] {
            store.db.execute("UPDATE payment_admission_evidence SET support=?1 WHERE attempt_id='history-proof'",[support]).unwrap();
            let query = store
                .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
                .unwrap();
            assert_eq!(
                store
                    .publish_scoped_evidence(&pool, rpc.view(query).await.unwrap())
                    .unwrap_err()
                    .downcast_ref::<EvidenceFailure>(),
                Some(&failure)
            );
            assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(7));
            assert_eq!(checkpoint(&store.db, &wallet).unwrap(), Some(anchor(128)));
        }
        server.abort();
    }

    #[test]
    fn endpoint_invalidation_rejects_captures_across_groups_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let identity = store.id().to_owned();
        crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "first",
            "1",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "second",
            "1",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        let wallets: Vec<_> = store
            .status()
            .unwrap()
            .pools
            .into_iter()
            .map(|pool| pool.addresses[0].id.clone())
            .collect();
        let captures: Vec<_> = wallets
            .iter()
            .map(|wallet| store.capture_evidence(wallet).unwrap())
            .collect();
        let endpoint = "a".repeat(64);
        let other_endpoint = "b".repeat(64);
        for capture in &captures {
            check_capture(&store.db, store.evidence_namespace, capture, &endpoint, 0).unwrap();
        }
        store.invalidate_endpoint_evidence(&endpoint).unwrap();
        for capture in &captures {
            assert_eq!(
                check_capture(&store.db, store.evidence_namespace, capture, &endpoint, 0)
                    .unwrap_err()
                    .downcast_ref::<EvidenceFailure>(),
                Some(&EvidenceFailure::Invalidated)
            );
            check_capture(
                &store.db,
                store.evidence_namespace,
                capture,
                &other_endpoint,
                0,
            )
            .unwrap();
        }
        let fresh = store.capture_evidence(&wallets[0]).unwrap();
        check_capture(&store.db, store.evidence_namespace, &fresh, &endpoint, 1).unwrap();
        drop(store);
        let store = Store::open(&state, &key, &identity).unwrap();
        assert_eq!(
            check_capture(&store.db, store.evidence_namespace, &fresh, &endpoint, 0)
                .unwrap_err()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::Invalidated)
        );
        assert_eq!(exposure(&store.db, &wallets[0]).unwrap(), U256::ZERO);
    }

    #[test]
    fn financial_lineage_and_debit_watermarks_are_wallet_scoped_and_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let mut store = store;
        crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "first",
            "1",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        let wallets = store.status().unwrap().pools.remove(0).addresses;
        let anchor = |height| Anchor {
            height,
            hash: format!("0x{height:064x}"),
        };
        advance_checkpoint(&store.db, &wallets[0].id, &anchor(20), false).unwrap();
        let debit: Option<i64> = store
            .db
            .query_row(
                "SELECT debit_height FROM wallets WHERE id=?1",
                [&wallets[0].id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(debit, None, "unused expiry never raises debit watermark");
        advance_checkpoint(&store.db, &wallets[0].id, &anchor(15), true).unwrap();
        advance_checkpoint(&store.db, &wallets[0].id, &anchor(10), true).unwrap();
        assert_eq!(
            checkpoint(&store.db, &wallets[0].id).unwrap(),
            Some(anchor(20))
        );
        let debit: i64 = store
            .db
            .query_row(
                "SELECT debit_height FROM wallets WHERE id=?1",
                [&wallets[0].id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(debit, 15);
        assert_eq!(checkpoint(&store.db, &wallets[1].id).unwrap(), None);
        let conflicting = Anchor {
            height: 20,
            hash: format!("0x{:064x}", 999),
        };
        assert!(advance_checkpoint(&store.db, &wallets[0].id, &conflicting, true).is_err());
        assert_eq!(
            checkpoint(&store.db, &wallets[0].id).unwrap(),
            Some(anchor(20))
        );
    }

    #[test]
    fn exposure_arithmetic_is_checked_and_rollback_preserves_total() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "history",
            "1",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        let wallet = store
            .status()
            .unwrap()
            .pools
            .remove(0)
            .addresses
            .remove(0)
            .id;
        let insert = |db: &Connection, id: &str, value: U256| {
            db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) SELECT ?1,pool_id,id,0,?3,'fixture','POSSIBLY_SUBMITTED' FROM wallets WHERE id=?2", params![id,wallet,value.to_string()]).unwrap();
        };
        insert(&store.db, "one", U256::MAX);
        for projection in [
            "malformed",
            "0",
            "99999999999999999999999999999999999999999999999999999999999999999999999999999999999",
        ] {
            store
                .db
                .execute(
                    "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
                    params![wallet, projection],
                )
                .unwrap();
            assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::MAX);
            assert_eq!(
                store.status().unwrap().pools[0]
                    .addresses
                    .iter()
                    .find(|w| w.id == wallet)
                    .unwrap()
                    .signed_exposure_atomic
                    .as_deref(),
                Some(U256::MAX.to_string().as_str())
            );
        }
        insert(&store.db, "two", U256::from(1));
        assert_eq!(
            exposure(&store.db, &wallet)
                .unwrap_err()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::MalformedExposure)
        );
        store
            .db
            .execute("DELETE FROM payment_attempts WHERE id='two'", [])
            .unwrap();
        store
            .db
            .execute(
                "UPDATE payment_attempts SET amount='bad' WHERE id='one'",
                [],
            )
            .unwrap();
        assert!(exposure(&store.db, &wallet).is_err());
        store
            .db
            .execute("UPDATE payment_attempts SET amount='7' WHERE id='one'", [])
            .unwrap();
        store
            .db
            .execute("INSERT INTO pools(id,name) VALUES('other','other')", [])
            .unwrap();
        store
            .db
            .execute(
                "UPDATE payment_attempts SET pool_id='other' WHERE id='one'",
                [],
            )
            .unwrap();
        assert!(exposure(&store.db, &wallet).is_err());
        store.db.execute("UPDATE payment_attempts SET pool_id=(SELECT pool_id FROM wallets WHERE id=wallet_id) WHERE id='one'", []).unwrap();
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(7));
        let tx = store.db.transaction().unwrap();
        insert(&tx, "rolled-back", U256::from(10));
        assert_eq!(exposure(&tx, &wallet).unwrap(), U256::from(17));
        tx.rollback().unwrap();
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(7));
        let plan = store
            .db
            .prepare(&format!("EXPLAIN QUERY PLAN {EXPOSURE_SQL}"))
            .unwrap()
            .query_map([&wallet], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("\n");
        assert!(plan.contains("payment_wallet_state"));
        assert!(plan.contains("payment_wallet_state"));
        assert!(plan.contains("wallet_id=?"));
    }
    #[test]
    fn disputed_membership_is_exact_durable_and_requires_original_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "disputes",
            "1",
            Some(U256::from(100)),
            0,
        )
        .unwrap();
        let wallet: String = store
            .db
            .query_row(
                "SELECT id FROM wallets WHERE pool_id=?1 LIMIT 1",
                [&pool],
                |r| r.get(0),
            )
            .unwrap();
        store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES('old',?1,?2,0,?3,'fixture','RESOLVED')", params![pool,wallet,U256::MAX.to_string()]).unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET payment_resolutions_disputed=1 WHERE id=?1",
                [&wallet],
            )
            .unwrap();
        assert!(
            exposure(&store.db, &wallet).is_err(),
            "missing original proof never becomes a favorable release"
        );
        store.db.execute("INSERT INTO payment_resolutions(attempt_id,outcome,height,hash,block_time,support) VALUES('old','USED',1,'original',1,'immutable-fixture')", []).unwrap();
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::MAX);
        store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES('pending',?1,?2,0,'1','fixture','POSSIBLY_SUBMITTED')", params![pool,wallet]).unwrap();
        assert!(
            exposure(&store.db, &wallet).is_err(),
            "checked sum cannot wrap"
        );
        store
            .db
            .execute("UPDATE payment_attempts SET amount='9' WHERE id='old'", [])
            .unwrap();
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(10));
        let treasury = store.id().to_owned();
        drop(store);
        let store = Store::open(&state, &key, &treasury).unwrap();
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(10));
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT support FROM payment_resolutions WHERE attempt_id='old'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "immutable-fixture"
        );
        store.db.pragma_update(None, "user_version", 16).unwrap();
        drop(store);
        let before = std::fs::read(state.join("state.sqlite")).unwrap();
        assert!(Store::open(&state, &key, &treasury).is_err());
        assert_eq!(std::fs::read(state.join("state.sqlite")).unwrap(), before);
    }
}

pub(super) fn initialize_runtime(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TEMP TABLE evidence_generations(kind TEXT NOT NULL,scope TEXT NOT NULL,generation INTEGER NOT NULL,PRIMARY KEY(kind,scope));")?;
    Ok(())
}
pub(super) fn generation(db: &Connection, kind: &str, scope: &str) -> Result<u64> {
    Ok(db
        .query_row(
            "SELECT generation FROM evidence_generations WHERE kind=?1 AND scope=?2",
            params![kind, scope],
            |row| sql_u64(row, 0),
        )
        .optional()?
        .unwrap_or(0))
}
pub(super) fn checkpoint(db: &Connection, wallet: &str) -> Result<Option<Anchor>> {
    let (height, hash): (Option<i64>, Option<String>) = db.query_row(
        "SELECT financial_height,financial_hash FROM wallets WHERE id=?1",
        [wallet],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    match (height, hash) {
        (None, None) => Ok(None),
        (Some(height), Some(hash)) => Ok(Some(Anchor {
            height: height.try_into()?,
            hash,
        })),
        _ => anyhow::bail!("incomplete financial checkpoint"),
    }
}
impl Store {
    /// A payment can use remaining capacity while every historical release is
    /// reserved again. This proof cannot establish historical funding or nonce facts.
    pub(crate) fn capture_payment_evidence(
        &self,
        wallet: &str,
    ) -> Result<crate::rotation::base::EvidenceCapture> {
        let mut capture = self.capture_evidence(wallet)?;
        let repair: Option<String> = self.db.query_row(
            "SELECT evidence_repair FROM wallets WHERE id=?1",
            [wallet],
            |row| row.get(0),
        )?;
        if repair.as_deref() == Some("chain_repair_required") {
            check_reserved_releases(&self.db, wallet)?;
            capture.capacity_snapshot = Some((
                checkpoint(&self.db, wallet)?,
                debit_checkpoint(&self.db, wallet)?,
            ));
            capture.checkpoint = None;
            capture.previous = None;
            capture.previous_confirmed_covered = false;
        }
        Ok(capture)
    }

    pub(crate) fn capture_evidence(
        &self,
        wallet: &str,
    ) -> Result<crate::rotation::base::EvidenceCapture> {
        let endpoints = self
            .db
            .prepare("SELECT scope,generation FROM evidence_generations WHERE kind='endpoint'")?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, sql_u64(row, 1)?)))?
            .collect::<rusqlite::Result<std::collections::BTreeMap<_, _>>>()?;
        let checkpoint = checkpoint(&self.db, wallet)?;
        let previous_confirmed_covered =
            self.evidence_cache
                .get(wallet)
                .is_some_and(|(_, certificate)| {
                    checkpoint.as_ref().is_some_and(|checkpoint| {
                        certificate.observation().confirmed.anchor.height <= checkpoint.height
                            && certificate.compatible(Some(checkpoint))
                    })
                });
        // Current capacity does not depend on archival funding/retirement
        // balances. Financial checkpoint and debit support remain independent.
        Ok(crate::rotation::base::EvidenceCapture {
            capacity_snapshot: None,
            durable_balance_anchors: Vec::new(),
            previous_confirmed_covered,
            namespace: self.evidence_namespace,
            wallet: wallet.into(),
            generation: generation(&self.db, "wallet", wallet)?,
            endpoint_generations: endpoints,
            checkpoint,
            previous: self
                .evidence_cache
                .get(wallet)
                .map(|(_, certificate)| certificate.observation().clone()),
        })
    }
}

const UNDISPUTED_SQL: &str = "SELECT EXISTS(SELECT 1 FROM wallets w WHERE w.id=?1 AND w.payment_resolutions_disputed=0 AND EXISTS(SELECT 1 FROM payment_attempts a WHERE a.wallet_id=w.id AND a.state='RESOLVED'))";

fn debit_checkpoint(db: &Connection, wallet: &str) -> Result<Option<Anchor>> {
    let (height, hash): (Option<i64>, Option<String>) = db.query_row(
        "SELECT debit_height,debit_hash FROM wallets WHERE id=?1",
        [wallet],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    match (height, hash) {
        (None, None) => Ok(None),
        (Some(height), Some(hash)) => Ok(Some(Anchor {
            height: height.try_into()?,
            hash,
        })),
        _ => anyhow::bail!("incomplete debit checkpoint"),
    }
}

fn check_reserved_releases(db: &Connection, wallet: &str) -> Result<()> {
    let repair: Option<String> = db.query_row(
        "SELECT evidence_repair FROM wallets WHERE id=?1",
        [wallet],
        |row| row.get(0),
    )?;
    ensure!(
        repair.as_deref() == Some("chain_repair_required"),
        EvidenceFailure::ChainRepairRequired
    );
    let undisputed: bool = db.query_row(UNDISPUTED_SQL, [wallet], |row| row.get(0))?;
    ensure!(!undisputed, EvidenceFailure::ChainRepairRequired);
    exposure(db, wallet)?;
    Ok(())
}

pub(super) fn check_payment_balance(
    db: &Connection,
    namespace: Uuid,
    certificate: &crate::rotation::base::BalanceCertificate,
) -> Result<BalanceSupport> {
    let Some((financial, debit)) = &certificate.capture().capacity_snapshot else {
        return check_balance(db, namespace, certificate);
    };
    check_balance_integrity(db, namespace, certificate)?;
    let observation = certificate.observation();
    check_reserved_releases(db, &observation.wallet)?;
    ensure!(
        checkpoint(db, &observation.wallet)? == *financial
            && debit_checkpoint(db, &observation.wallet)? == *debit,
        EvidenceFailure::Invalidated
    );
    ensure!(
        debit
            .as_ref()
            .is_none_or(|anchor| observation.confirmed.anchor.height >= anchor.height),
        EvidenceFailure::BeforeDebit
    );
    // No historical lineage is asserted by a capacity-only support value.
    Ok(BalanceSupport {
        observation: observation.clone(),
        checkpoint: None,
    })
}

pub(super) fn check_capture(
    db: &Connection,
    namespace: Uuid,
    capture: &crate::rotation::base::EvidenceCapture,
    endpoint: &str,
    endpoint_generation: u64,
) -> Result<()> {
    ensure!(
        capture.namespace == namespace
            && capture.generation == generation(db, "wallet", &capture.wallet)?
            && endpoint_generation == generation(db, "endpoint", endpoint)?,
        EvidenceFailure::Invalidated
    );
    Ok(())
}

// A capacity/role decision still needs repair clearance. Independently bound
// nonce observations do not grant that clearance or depend on it.
fn check_capacity_repair(db: &Connection, wallet: &str) -> Result<()> {
    let repair: Option<String> = db.query_row(
        "SELECT evidence_repair FROM wallets WHERE id=?1",
        [wallet],
        |row| row.get(0),
    )?;
    if let Some(reason) = repair {
        anyhow::bail!(if reason == "evidence_repair_required" {
            EvidenceFailure::EvidenceRepairRequired
        } else {
            EvidenceFailure::ChainRepairRequired
        });
    }
    Ok(())
}
pub(super) fn check_balance_integrity(
    db: &Connection,
    namespace: Uuid,
    certificate: &crate::rotation::base::BalanceCertificate,
) -> Result<()> {
    let observation = certificate.observation();
    let capture = certificate.capture();
    observation.validate()?;
    check_capture(
        db,
        namespace,
        capture,
        &observation.endpoint,
        certificate.endpoint_generation(),
    )?;
    ensure!(
        capture.wallet == observation.wallet,
        EvidenceFailure::ScopeMismatch
    );
    let address: String = db.query_row(
        "SELECT address FROM wallets WHERE id=?1",
        [&capture.wallet],
        |row| row.get(0),
    )?;
    ensure!(
        address.parse::<alloy_primitives::Address>()?
            == observation.address.parse::<alloy_primitives::Address>()?,
        EvidenceFailure::ScopeMismatch
    );
    Ok(())
}
pub(super) fn check_balance(
    db: &Connection,
    namespace: Uuid,
    certificate: &crate::rotation::base::BalanceCertificate,
) -> Result<BalanceSupport> {
    ensure!(
        certificate.capture().capacity_snapshot.is_none(),
        EvidenceFailure::CompatibilityRequired
    );
    check_balance_integrity(db, namespace, certificate)?;
    check_capacity_repair(db, &certificate.capture().wallet)?;
    let observation = certificate.observation();
    let current = checkpoint(db, &observation.wallet)?;
    let debit: Option<i64> = db.query_row(
        "SELECT debit_height FROM wallets WHERE id=?1",
        [&observation.wallet],
        |row| row.get(0),
    )?;
    if let Some(debit) = debit {
        ensure!(
            current
                .as_ref()
                .is_some_and(|checkpoint| i64::try_from(checkpoint.height)
                    .is_ok_and(|height| height >= debit)),
            EvidenceFailure::ChainRepairRequired
        );
        if current
            .as_ref()
            .is_some_and(|checkpoint| i64::try_from(checkpoint.height) == Ok(debit))
        {
            let hash: Option<String> = db.query_row(
                "SELECT debit_hash FROM wallets WHERE id=?1",
                [&observation.wallet],
                |row| row.get(0),
            )?;
            ensure!(
                hash.as_ref() == current.as_ref().map(|checkpoint| &checkpoint.hash),
                EvidenceFailure::ChainRepairRequired
            );
        }
        ensure!(
            i64::try_from(observation.confirmed.anchor.height).is_ok_and(|height| height >= debit),
            EvidenceFailure::BeforeDebit
        );
    }
    ensure!(
        certificate.compatible(current.as_ref()),
        EvidenceFailure::CompatibilityRequired
    );
    Ok(BalanceSupport {
        observation: observation.clone(),
        checkpoint: current,
    })
}
#[derive(Serialize)]
struct ResolutionSupport<'a> {
    wallet: &'a str,
    payer: &'a str,
    nonce: &'a str,
    endpoint: &'a str,
    network: &'static str,
    asset: &'static str,
    anchor: &'a Anchor,
    outcome: &'a crate::rotation::base::AuthorizationOutcome,
    block_time: u64,
    checkpoint: Option<Anchor>,
}
pub(super) fn check_resolution(
    db: &Connection,
    namespace: Uuid,
    certificate: &crate::rotation::base::ResolutionCertificate,
) -> Result<String> {
    ensure!(
        certificate.capture().capacity_snapshot.is_none(),
        EvidenceFailure::CompatibilityRequired
    );
    check_capture(
        db,
        namespace,
        certificate.capture(),
        certificate.endpoint(),
        certificate.endpoint_generation(),
    )?;
    let auth = certificate.authorization();
    ensure!(
        auth.wallet == certificate.capture().wallet,
        EvidenceFailure::ScopeMismatch
    );
    let current = checkpoint(db, &auth.wallet)?;
    ensure!(
        certificate.compatible(current.as_ref()),
        EvidenceFailure::CompatibilityRequired
    );
    let saved: (String, String, String, u64) = db.query_row(
        "SELECT wallet_id,payer,nonce,valid_before FROM payment_attempts WHERE id=?1",
        [&auth.id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, sql_u64(row, 3)?)),
    )?;
    ensure!(
        saved
            == (
                auth.wallet.clone(),
                auth.payer.clone(),
                auth.nonce.clone(),
                auth.valid_before
            ),
        "authorization_evidence_binding_mismatch"
    );
    if *certificate.outcome() == crate::rotation::base::AuthorizationOutcome::ExpiredUnused {
        ensure!(
            certificate.block_time() > auth.valid_before,
            "authorization_not_expired"
        );
    }
    Ok(serde_json::to_string(&ResolutionSupport {
        wallet: &auth.wallet,
        payer: &auth.payer,
        nonce: &auth.nonce,
        endpoint: certificate.endpoint(),
        network: "eip155:8453",
        asset: crate::payment::USDC,
        anchor: certificate.anchor(),
        outcome: certificate.outcome(),
        block_time: certificate.block_time(),
        checkpoint: current,
    })?)
}
pub(super) fn advance_checkpoint(
    db: &Connection,
    wallet: &str,
    anchor: &Anchor,
    used: bool,
) -> Result<()> {
    if let Some(current) = checkpoint(db, wallet)? {
        ensure!(
            current.height != anchor.height || current.hash == anchor.hash,
            EvidenceFailure::ChainRepairRequired
        );
    }
    let debit: (Option<i64>, Option<String>) = db.query_row(
        "SELECT debit_height,debit_hash FROM wallets WHERE id=?1",
        [wallet],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    ensure!(
        debit.0 != Some(i64::try_from(anchor.height)?) || debit.1.as_ref() == Some(&anchor.hash),
        EvidenceFailure::ChainRepairRequired
    );
    db.execute("UPDATE wallets SET financial_height=?2,financial_hash=?3 WHERE id=?1 AND (financial_height IS NULL OR financial_height<?2)",params![wallet,i64::try_from(anchor.height)?,anchor.hash])?;
    if used {
        db.execute("UPDATE wallets SET debit_height=?2,debit_hash=?3 WHERE id=?1 AND (debit_height IS NULL OR debit_height<?2)",params![wallet,i64::try_from(anchor.height)?,anchor.hash])?;
    }
    Ok(())
}

/// Fixed indexed lookups shared by every publication. Building their JSON
/// predicates once keeps statement construction out of the serialized hot path.
fn balance_support_queries() -> &'static [String; 8] {
    static QUERIES: std::sync::LazyLock<[String; 8]> = std::sync::LazyLock::new(|| {
        let required = [
            ("observation.wallet", "text"),
            ("observation.address", "text"),
            ("observation.network", "text"),
            ("observation.asset", "text"),
            ("observation.endpoint", "text"),
            ("observation.confirmations", "integer"),
            ("observation.confirmed.amount", "text"),
            ("observation.confirmed.anchor.height", "integer"),
            ("observation.confirmed.anchor.hash", "text"),
            ("observation.latest.amount", "text"),
            ("observation.latest.anchor.height", "integer"),
            ("observation.latest.anchor.hash", "text"),
            ("observation.valid_until", "integer"),
        ]
        .into_iter()
        .map(|(field, kind)| format!("json_type(e.support,'$.{field}') IS NOT '{kind}'"))
        .collect::<Vec<_>>()
        .join(" OR ");
        let malformed = format!(
            "({required} OR (json_type(e.support,'$.checkpoint') IS NOT 'null' AND json_type(e.support,'$.checkpoint') IS NOT 'object') OR (json_type(e.support,'$.checkpoint')='object' AND (json_type(e.support,'$.checkpoint.height') IS NOT 'integer' OR json_type(e.support,'$.checkpoint.hash') IS NOT 'text')))"
        );
        let sources = [
            (
                "payment_admission_evidence e JOIN payment_attempts a ON a.id=e.attempt_id",
                "a.wallet_id",
                0,
            ),
            (
                "(SELECT wallet_id,readiness_support AS support,readiness_hash AS block_hash,readiness_latest_hash AS latest_hash FROM funding_jobs) e",
                "e.wallet_id",
                1,
            ),
            (
                "(SELECT wallet_id,support,confirmed_hash AS block_hash,latest_hash FROM wallet_retirements) e",
                "e.wallet_id",
                1,
            ),
            (
                "(SELECT wallet_id,balance_support AS support,confirmed_hash AS block_hash,latest_hash FROM payment_resolution_repairs WHERE activated=1) e",
                "e.wallet_id",
                1,
            ),
        ];
        std::array::from_fn(|index| {
            let (column, path) =
                [("block_hash", "confirmed"), ("latest_hash", "latest")][index / sources.len()];
            let (source, owner, missing) = sources[index % sources.len()];
            format!(
                "SELECT COALESCE(e.support,''),CASE WHEN json_valid(e.support) THEN {malformed} ELSE 1 END FROM {source} WHERE {owner}=?1 AND e.{column}=?2 AND CASE WHEN e.support IS NULL THEN {missing} WHEN NOT json_valid(e.support) THEN 1 ELSE {malformed} OR json_extract(e.support,'$.observation.{path}.amount') IS NOT ?3 OR json_extract(e.support,'$.observation.{path}.anchor.height') IS NOT ?4 OR json_extract(e.support,'$.observation.{path}.anchor.hash') IS NOT ?2 END LIMIT 1"
            )
        })
    });
    &QUERIES
}

impl Store {
    /// A conflicting durable nonce proof invalidates its original wallet anchor.
    fn check_resolution_or_invalidate(
        &mut self,
        attempt: &str,
        certificate: &crate::rotation::base::ResolutionCertificate,
    ) -> Result<bool> {
        check_resolution(&self.db, self.evidence_namespace, certificate)?;
        let existing:Option<(String,i64,String,u64)> = self.db.query_row("SELECT outcome,height,hash,block_time FROM effective_payment_resolutions WHERE attempt_id=?1",[attempt],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,sql_u64(row,3)?))).optional()?;
        if let Some((outcome, height, hash, block_time)) = existing {
            let expected =
                if *certificate.outcome() == crate::rotation::base::AuthorizationOutcome::Used {
                    "USED"
                } else {
                    "EXPIRED_UNUSED"
                };
            // A second canonical observation is not a second disposition. Its
            // height is an observation anchor, not the transaction's inclusion
            // height. Preserve the first proof and never release exposure twice.
            let resolved: bool = self.db.query_row(
                "SELECT state='RESOLVED' FROM payment_attempts WHERE id=?1",
                [attempt],
                |r| r.get(0),
            )?;
            if resolved
                && outcome == expected
                && height != i64::try_from(certificate.anchor().height)?
                && hash != certificate.anchor().hash
            {
                return Ok(true);
            }
            if outcome != expected
                || height != i64::try_from(certificate.anchor().height)?
                || hash != certificate.anchor().hash
                || block_time != certificate.block_time()
            {
                self.invalidate_wallet_evidence(
                    &certificate.authorization().wallet,
                    &Anchor {
                        height: height.try_into()?,
                        hash,
                    },
                    true,
                    false,
                )?;
                anyhow::bail!(EvidenceFailure::ResolutionConflict);
            }
        }
        Ok(false)
    }

    fn check_journal_support_or_invalidate(
        &mut self,
        wallet: &str,
        observation: &BalanceObservation,
    ) -> Result<()> {
        // Journal support survives cache eviction. Indexed hash lookups return
        // only a conflicting record; ordinary acquisition never enumerates history.
        for point in [&observation.confirmed, &observation.latest] {
            let encoded = serde_json::to_value(point.amount)?;
            let amount_text = encoded.as_str().context("invalid encoded balance amount")?;
            // SQLite flattens the funding/retirement projections so each
            // lookup retains its dedicated hash index and malformed-JSON guard.
            for sql in balance_support_queries() {
                let conflicting: Option<(String, bool)> = self
                    .db
                    .prepare_cached(sql)?
                    .query_row(
                        params![
                            wallet,
                            point.anchor.hash,
                            amount_text,
                            i64::try_from(point.anchor.height)?
                        ],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                if let Some((support, malformed)) = conflicting {
                    let exact = serde_json::from_str::<BalanceSupport>(&support)
                        .ok()
                        .filter(|support| support.observation.validate().is_ok());
                    self.invalidate_wallet_evidence(wallet, &point.anchor, false, true)?;
                    if malformed || exact.is_none() {
                        anyhow::bail!(EvidenceFailure::MalformedSupport);
                    }
                    anyhow::bail!(EvidenceFailure::AmountContradiction);
                }
            }
        }
        Ok(())
    }

    /// Cached contradictions invalidate only the anchor supporting the failure.
    pub(super) fn compare_cached_balance_or_invalidate(
        &mut self,
        wallet: &str,
        observation: &BalanceObservation,
        saved: &BalanceObservation,
    ) -> Result<std::cmp::Ordering> {
        match observation.compare(saved) {
            Ok(order) => Ok(order),
            Err(failure) => {
                if matches!(
                    failure,
                    EvidenceFailure::AmountContradiction
                        | EvidenceFailure::ConfirmedReplacement
                        | EvidenceFailure::LatestReplacement
                ) {
                    let anchor = if failure == EvidenceFailure::AmountContradiction {
                        [&observation.confirmed, &observation.latest]
                            .into_iter()
                            .find(|point| {
                                [&saved.confirmed, &saved.latest].into_iter().any(|old| {
                                    point.anchor.hash == old.anchor.hash
                                        && (point.amount != old.amount
                                            || point.anchor.height != old.anchor.height)
                                })
                            })
                            .map(|point| &point.anchor)
                            .context("missing conflicting balance anchor")?
                    } else if failure == EvidenceFailure::LatestReplacement {
                        &saved.latest.anchor
                    } else {
                        &saved.confirmed.anchor
                    };
                    self.invalidate_wallet_evidence(
                        wallet,
                        anchor,
                        false,
                        failure == EvidenceFailure::AmountContradiction,
                    )?;
                }
                Err(failure.into())
            }
        }
    }

    /// Complete observations are retained only as a bounded optimization. Exact
    /// financial support is separately committed with each durable disposition.
    pub(crate) fn prepare_evidence_view(
        &mut self,
        view: crate::rotation::base::ChainView,
    ) -> Result<crate::rotation::base::ChainView> {
        self.prepare_evidence_view_inner(view, false)
    }

    pub(crate) fn prepare_payment_evidence_view(
        &mut self,
        view: crate::rotation::base::ChainView,
    ) -> Result<crate::rotation::base::ChainView> {
        self.prepare_evidence_view_inner(view, true)
    }

    fn prepare_evidence_view_inner(
        &mut self,
        mut view: crate::rotation::base::ChainView,
        payment: bool,
    ) -> Result<crate::rotation::base::ChainView> {
        let validate_balance = if payment {
            check_payment_balance
        } else {
            check_balance
        };
        let capacity_only = view
            .balance_certificates
            .values()
            .any(|certificate| certificate.capture().capacity_snapshot.is_some());
        ensure!(
            !capacity_only
                || (payment
                    && view.resolution_certificates.is_empty()
                    && view.resolutions.is_empty()
                    && view.released.is_empty()),
            EvidenceFailure::CompatibilityRequired
        );
        // Validate all cross-map bindings before any contradiction can invalidate
        // durable evidence. A malformed view is not proof of a chain conflict.
        for (attempt, certificate) in &view.resolution_certificates {
            let resolution = view
                .resolutions
                .get(attempt)
                .context("resolution_certificate_binding_mismatch")?;
            ensure!(
                certificate.authorization().id == *attempt
                    && view.released.iter().filter(|id| *id == attempt).count() == 1
                    && certificate.anchor() == resolution.anchor.as_ref().unwrap_or(&view.anchor)
                    && certificate.outcome() == &resolution.outcome
                    && certificate.block_time() == resolution.block_time,
                "resolution_certificate_binding_mismatch"
            );
        }
        let mut redundant = Vec::new();
        for (attempt, certificate) in &view.resolution_certificates {
            if self.check_resolution_or_invalidate(attempt, certificate)? {
                redundant.push(attempt.clone());
            }
        }
        for attempt in redundant {
            view.resolution_certificates.remove(&attempt);
            view.resolutions.remove(&attempt);
            view.released.retain(|id| id != &attempt);
        }
        for (wallet, incoming) in &mut view.balance_certificates {
            let observation = view
                .observations
                .get(wallet)
                .context("missing exact balance observation")?;
            ensure!(
                incoming.observation() == observation,
                "balance_certificate_binding_mismatch"
            );
            // A detached pre-merge handle has surrendered its own lease. It
            // cannot become cache authority again after canonical eviction.
            ensure!(!incoming.is_superseded()?, EvidenceFailure::Expired);
            check_balance_integrity(&self.db, self.evidence_namespace, incoming)?;
            self.check_journal_support_or_invalidate(wallet, observation)?;

            if let Some((_, saved)) = self.evidence_cache.get(wallet).cloned() {
                let order = self.compare_cached_balance_or_invalidate(
                    wallet,
                    observation,
                    saved.observation(),
                )?;
                if order == std::cmp::Ordering::Equal {
                    incoming.retain_freshness(&saved)?;
                    view.observations
                        .insert(wallet.clone(), incoming.observation().clone());
                }
                if order != std::cmp::Ordering::Greater
                    && saved.is_fresh(crate::rotation::base::now()?)?
                    && validate_balance(&self.db, self.evidence_namespace, &saved).is_ok()
                {
                    *incoming = saved;
                    view.observations
                        .insert(wallet.clone(), incoming.observation().clone());
                    view.balances
                        .insert(wallet.clone(), incoming.observation().available());
                }
            }
            validate_balance(&self.db, self.evidence_namespace, incoming)?;
            self.evidence_order = self.evidence_order.saturating_add(1);
            self.evidence_cache
                .insert(wallet.clone(), (self.evidence_order, incoming.clone()));
            // A wakeup grants no authority; the waiter rechecks under its group
            // gate after this accepted store job has finished.
            let pool: String =
                self.db
                    .query_row("SELECT pool_id FROM wallets WHERE id=?1", [wallet], |row| {
                        row.get(0)
                    })?;
            self.notify_capacity_changed(&pool);
            if self.evidence_cache.len() > 256 {
                let oldest = self
                    .evidence_cache
                    .iter()
                    .min_by_key(|(_, (order, _))| *order)
                    .map(|(wallet, _)| wallet.clone())
                    .expect("nonempty observation cache");
                self.evidence_cache.remove(&oldest);
            }
        }
        Ok(view)
    }
}

/// Endpoint identity belongs to effective resolution and adopted checkpoint support.
/// Guard extraction in the expression itself: SQLite may reorder predicates.
/// Unknown provenance is contained to its owner, never treated as an absent debt.
fn payment_endpoint_dependents(
    db: &Connection,
    endpoint: &str,
) -> Result<std::collections::BTreeMap<String, bool>> {
    let mut query = db.prepare("WITH supports AS (SELECT a.wallet_id,r.endpoint FROM payment_attempts a LEFT JOIN effective_payment_resolutions r ON r.attempt_id=a.id WHERE a.state='RESOLVED' UNION ALL SELECT r.wallet_id,r.endpoint FROM payment_resolution_repairs r WHERE r.activated=1 AND r.sequence=(SELECT n.sequence FROM payment_resolution_repairs n WHERE n.wallet_id=r.wallet_id AND n.activated=1 ORDER BY n.sequence DESC LIMIT 1)) SELECT wallet_id,endpoint IS NULL OR length(endpoint)!=64 OR endpoint GLOB '*[^0-9a-fA-F]*' AS malformed FROM supports WHERE endpoint=?1 OR endpoint IS NULL OR length(endpoint)!=64 OR endpoint GLOB '*[^0-9a-fA-F]*'")?;
    let mut wallets = std::collections::BTreeMap::new();
    for row in query.query_map([endpoint], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
    })? {
        let (wallet, malformed) = row?;
        *wallets.entry(wallet).or_insert(false) |= malformed;
    }
    Ok(wallets)
}

fn bump_generation(db: &Connection, kind: &str, scope: &str) -> Result<()> {
    let next = generation(db, kind, scope)?
        .checked_add(1)
        .context("evidence generation exhausted")?;
    db.execute("INSERT INTO evidence_generations(kind,scope,generation) VALUES(?1,?2,?3) ON CONFLICT(kind,scope) DO UPDATE SET generation=excluded.generation",params![kind,scope,i64::try_from(next)?])?;
    Ok(())
}
impl Store {
    fn invalidate_wallet_evidence(
        &mut self,
        wallet: &str,
        anchor: &Anchor,
        financial_checkpoint: bool,
        amount_conflict: bool,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        // An amount observation never supplies authorizationState. An intact
        // nonce proof does not inherit its former admission amount dependency.
        let dependent: bool = if amount_conflict {
            false
        } else {
            tx.query_row("SELECT EXISTS(SELECT 1 FROM effective_payment_resolutions r JOIN payment_attempts a ON a.id=r.attempt_id WHERE a.wallet_id=?1 AND r.hash=?2)",params![wallet,anchor.hash],|row|row.get(0))?
        };
        if financial_checkpoint || dependent {
            if !amount_conflict {
                // A failed checkpoint does not identify the fork's starting
                // height. Restore every released liability for this wallet;
                // the original proofs remain immutable audit evidence.
                tx.execute(
                    "UPDATE wallets SET payment_resolutions_disputed=1 WHERE id=?1",
                    [wallet],
                )?;
            }
            tx.execute(
                "UPDATE wallets SET evidence_repair=CASE WHEN evidence_repair='evidence_repair_required' THEN evidence_repair ELSE ?2 END WHERE id=?1",
                params![
                    wallet,
                    if amount_conflict {
                        "evidence_repair_required"
                    } else {
                        "chain_repair_required"
                    }
                ],
            )?;
        }
        super::funding::dispute_support(
            &tx,
            wallet,
            &anchor.hash,
            if amount_conflict {
                "evidence_repair_required"
            } else {
                "chain_repair_required"
            },
        )?;
        super::capacity::dispute_support(
            &tx,
            wallet,
            &anchor.hash,
            if amount_conflict {
                "evidence_repair_required"
            } else {
                "chain_repair_required"
            },
        )?;
        bump_generation(&tx, "wallet", wallet)?;
        tx.commit()?;
        self.evidence_cache.remove(wallet);
        Ok(())
    }
    /// Shared-store serialization reaches captures from every group, even while
    /// their callers hold different group gates. Failed acquisitions do not call it.
    pub(crate) fn invalidate_endpoint_evidence(&mut self, endpoint: &str) -> Result<()> {
        ensure!(
            endpoint.len() == 64 && endpoint.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid endpoint identity"
        );
        let tx = self.db.transaction()?;
        let payment_wallets = payment_endpoint_dependents(&tx, endpoint)?;
        for wallet in payment_wallets.keys() {
            // Losing effective resolution or adopted checkpoint provenance
            // invalidates every release for this wallet; the
            // original proof, financial/debit anchors and attempt stay intact.
            tx.execute(
                "UPDATE wallets SET payment_resolutions_disputed=1 WHERE id=?1",
                [wallet],
            )?;
            // Unknown historical endpoint provenance does not make the exact
            // authorization amount unknown. Reserve it and reacquire proof;
            // independent amount contradictions retain their stronger stop.
            let changed = tx.execute("UPDATE wallets SET evidence_repair=CASE WHEN evidence_repair='evidence_repair_required' THEN evidence_repair ELSE 'chain_repair_required' END WHERE id=?1", [wallet])?;
            ensure!(changed == 1, EvidenceFailure::ScopeMismatch);
            bump_generation(&tx, "wallet", wallet)?;
        }
        super::funding::dispute_endpoint_support(&tx, endpoint)?;
        super::capacity::dispute_endpoint_support(&tx, endpoint)?;
        bump_generation(&tx, "endpoint", endpoint)?;
        tx.commit()?;
        self.evidence_cache.retain(|wallet, (_, certificate)| {
            certificate.observation().endpoint != endpoint && !payment_wallets.contains_key(wallet)
        });
        let malformed_wallets = payment_wallets
            .values()
            .filter(|malformed| **malformed)
            .count();
        if malformed_wallets > 0 {
            tracing::warn!(
                category = "malformed_payment_resolution_support",
                affected_wallets = malformed_wallets,
                "Payment resolution endpoint provenance requires wallet-scoped evidence repair"
            );
        }
        Ok(())
    }
    pub(crate) fn record_base_failure(&mut self, error: &anyhow::Error) -> Result<()> {
        if let Some(failure) = error.downcast_ref::<crate::rotation::base::EndpointMismatch>() {
            self.invalidate_endpoint_evidence(&failure.endpoint)?;
        }
        if let Some(failure) =
            error.downcast_ref::<crate::rotation::base::historical::HistoricalAmountConflict>()
        {
            self.invalidate_wallet_evidence(&failure.wallet, &failure.anchor, false, true)?;
        }
        if let Some(failure) = error.downcast_ref::<crate::rotation::base::EvidenceConflict>() {
            ensure!(failure.endpoint.len() == 64, "invalid endpoint identity");
            self.invalidate_wallet_evidence(
                &failure.wallet,
                &failure.anchor,
                failure.financial_checkpoint,
                false,
            )?;
        }
        Ok(())
    }
    pub fn record_verified_credit(
        &mut self,
        wallet: &str,
        view: crate::rotation::base::ChainView,
    ) -> Result<BalanceSupport> {
        self.record_verified_funding_credit(wallet, None, view)
    }
    pub(crate) fn record_verified_funding_credit(
        &mut self,
        wallet: &str,
        operation: Option<(&str, &str)>,
        view: crate::rotation::base::ChainView,
    ) -> Result<BalanceSupport> {
        let view = self.prepare_evidence_view(view)?;
        let certificate = view
            .balance_certificates
            .get(wallet)
            .context("missing verified funding evidence")?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((job, operation)) = operation {
            let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1 AND f.wallet_id=?2 AND j.operation_id=?3)",params![job,wallet,operation],|r|r.get(0))?;
            ensure!(bound, "funding operation binding changed");
        }
        let support = check_balance(&tx, self.evidence_namespace, certificate)?;
        tx.execute(
            "UPDATE wallets SET balance=?2,block_hash=?3,block_height=?4 WHERE id=?1",
            params![
                wallet,
                support.observation.available().to_string(),
                support.observation.confirmed.anchor.hash,
                i64::try_from(support.observation.confirmed.anchor.height)?
            ],
        )?;
        if super::funding::record_readiness(&tx, wallet, &support)? {
            super::funding::publish_destination_readiness(&tx, wallet)?;
        }
        tx.commit()?;
        Ok(support)
    }
}

impl Store {
    /// An endpoint dispute requires exact historical support, even after the
    /// destination's present balance has been spent. Other disputes need repair.
    pub(crate) fn funding_readiness_reverification(
        &self,
        job: &str,
        operation: &str,
    ) -> Result<Option<crate::rotation::base::historical::HistoricalBalanceRequest>> {
        let (wallet,support,dispute):(String,Option<String>,Option<String>) = self.db.query_row(
            "SELECT f.wallet_id,f.readiness_support,f.readiness_disputed FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1 AND j.operation_id=?2", params![job,operation],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        let Some(dispute) = dispute else {
            return Ok(None);
        };
        ensure!(
            dispute == "endpoint_reverification_required",
            EvidenceFailure::EvidenceRepairRequired
        );
        let support: BalanceSupport = serde_json::from_str(
            support
                .as_deref()
                .ok_or(EvidenceFailure::MalformedSupport)?,
        )
        .map_err(|_| EvidenceFailure::MalformedSupport)?;
        support.observation.validate()?;
        ensure!(
            support.observation.wallet == wallet,
            EvidenceFailure::ScopeMismatch
        );
        let address: String = self.db.query_row(
            "SELECT address FROM wallets WHERE id=?1",
            [&wallet],
            |row| row.get(0),
        )?;
        ensure!(
            support
                .observation
                .address
                .parse::<alloy_primitives::Address>()?
                == address.parse::<alloy_primitives::Address>()?,
            EvidenceFailure::ScopeMismatch
        );
        Ok(Some(
            crate::rotation::base::historical::HistoricalBalanceRequest {
                capture: self.capture_evidence(&wallet)?,
                observation: support.observation,
                checkpoint: support.checkpoint,
            },
        ))
    }

    pub(crate) fn restore_funding_readiness_support(
        &mut self,
        job: &str,
        operation: &str,
        certificate: crate::rotation::base::historical::HistoricalBalanceCertificate,
    ) -> Result<()> {
        let request = certificate.request();
        let tx = self.db.transaction()?;
        check_capacity_repair(&tx, &request.capture.wallet)?;
        check_capture(
            &tx,
            self.evidence_namespace,
            &request.capture,
            &request.observation.endpoint,
            certificate.endpoint_generation(),
        )?;
        ensure!(
            checkpoint(&tx, &request.capture.wallet)? == request.capture.checkpoint,
            EvidenceFailure::CompatibilityRequired
        );
        let (wallet,support,dispute):(String,String,Option<String>)=tx.query_row(
            "SELECT f.wallet_id,f.readiness_support,f.readiness_disputed FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1 AND j.operation_id=?2",params![job,operation],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        ensure!(
            dispute.as_deref() == Some("endpoint_reverification_required"),
            EvidenceFailure::Invalidated
        );
        let support: BalanceSupport =
            serde_json::from_str(&support).map_err(|_| EvidenceFailure::MalformedSupport)?;
        ensure!(
            wallet == request.capture.wallet
                && support.observation == request.observation
                && support.checkpoint == request.checkpoint,
            EvidenceFailure::ScopeMismatch
        );
        // Never alter the original D amount, anchors, timestamp, operation or
        // qualification capability; this restores only the trust annotation.
        tx.execute("UPDATE funding_jobs SET readiness_disputed=NULL WHERE id=?1 AND readiness_disputed='endpoint_reverification_required'",[job])?;
        super::funding::publish_destination_readiness(&tx, &wallet)?;
        let pool: String = tx.query_row(
            "SELECT pool_id FROM wallets WHERE id=?1",
            [&wallet],
            |row| row.get(0),
        )?;
        tx.commit()?;
        // Restoration can make an existing destination usable. Wake only its
        // pool after commit; waiters must still acquire fresh admission evidence.
        self.notify_capacity_changed(&pool);
        Ok(())
    }
}

#[cfg(test)]
mod funding_tests;

#[cfg(test)]
mod retirement_tests;

#[cfg(test)]
mod preparation_tests;

#[cfg(test)]
mod endpoint_tests;
