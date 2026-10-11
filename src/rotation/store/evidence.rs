//! Exact supporting values live with the journal; runtime credentials never survive reopen.
use super::*;
use crate::rotation::base::{
    Anchor,
    evidence::{BalanceObservation, EvidenceFailure},
};

pub(super) fn initialize(db: &Connection) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    initialize_in_transaction(&tx)?;
    tx.commit()?;
    Ok(())
}

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

pub(super) fn exposure(db: &Connection, wallet: &str) -> Result<U256> {
    let value: String = db.query_row(
        "SELECT unresolved_exposure FROM wallets WHERE id=?1",
        [wallet],
        |row| row.get(0),
    )?;
    amount(&value)
}
pub(super) fn add_exposure(db: &Connection, wallet: &str, value: U256) -> Result<()> {
    let total = exposure(db, wallet)?
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
pub(super) fn subtract_exposure(db: &Connection, wallet: &str, value: U256) -> Result<()> {
    let total = exposure(db, wallet)?
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
        let pool = store.ensure_pool("history", "1").unwrap();
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
        let pool = store.ensure_pool("first", "1").unwrap();
        let wallet = store
            .status()
            .unwrap()
            .pools
            .remove(0)
            .addresses
            .remove(0)
            .id;
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let first = rpc.view(query).await.unwrap();
        store.publish_scoped_evidence(&pool, first).unwrap();
        let expired_at = now().unwrap() - 1;
        store
            .evidence_cache
            .get_mut(&wallet)
            .unwrap()
            .1
            .restrict_validity(expired_at);
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
        raced
            .balance_certificates
            .insert(wallet.clone(), certificate);
        store.publish_scoped_evidence(&pool, raced.clone()).unwrap();
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
        store.db.execute("INSERT INTO payment_admission_evidence(attempt_id,block_height,block_hash,balance,reserved_before,valid_until,support,latest_height,latest_hash,endpoint) VALUES('history-proof',128,?1,'80','0',?2,?3,140,?4,?5)",params![durable_observation.confirmed.anchor.hash,durable_observation.valid_until.to_string(),serde_json::to_string(&durable_support).unwrap(),durable_observation.latest.anchor.hash,durable_observation.endpoint]).unwrap();
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
            repair, None,
            "independently intact nonce proof does not depend on its admission balance"
        );
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::ZERO);
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
            assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::ZERO);
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
        store.ensure_pool("first", "1").unwrap();
        store.ensure_pool("second", "1").unwrap();
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
        store.ensure_pool("first", "1").unwrap();
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
        store.ensure_pool("history", "1").unwrap();
        let wallet = store
            .status()
            .unwrap()
            .pools
            .remove(0)
            .addresses
            .remove(0)
            .id;
        add_exposure(&store.db, &wallet, U256::MAX).unwrap();
        assert!(add_exposure(&store.db, &wallet, U256::from(1)).is_err());
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::MAX);
        subtract_exposure(&store.db, &wallet, U256::MAX).unwrap();
        assert!(subtract_exposure(&store.db, &wallet, U256::from(1)).is_err());
        let tx = store.db.transaction().unwrap();
        add_exposure(&tx, &wallet, U256::from(10)).unwrap();
        tx.rollback().unwrap();
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::ZERO);
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
        // Exactly one funding job owns a wallet. This keyed read is bounded;
        // immutable support remains relevant after ordinary cache eviction.
        let saved:Option<(Option<String>,Option<String>)>=self.db.query_row(
            "SELECT readiness_requirement,readiness_support FROM funding_jobs WHERE wallet_id=?1",[wallet],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let mut durable_balance_anchors = Vec::new();
        if let Some((Some(_), support)) = saved {
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
            let address: String =
                self.db
                    .query_row("SELECT address FROM wallets WHERE id=?1", [wallet], |row| {
                        row.get(0)
                    })?;
            ensure!(
                support
                    .observation
                    .address
                    .parse::<alloy_primitives::Address>()?
                    == address.parse::<alloy_primitives::Address>()?,
                EvidenceFailure::ScopeMismatch
            );
            durable_balance_anchors.push(support.observation.confirmed.anchor);
            durable_balance_anchors.push(support.observation.latest.anchor);
            if let Some(anchor) = support.checkpoint {
                durable_balance_anchors.push(anchor);
            }
        }
        Ok(crate::rotation::base::EvidenceCapture {
            durable_balance_anchors,
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
    let repair: Option<String> = db.query_row(
        "SELECT evidence_repair FROM wallets WHERE id=?1",
        [&capture.wallet],
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
    check_balance_integrity(db, namespace, certificate)?;
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

impl Store {
    /// Complete observations are retained only as a bounded optimization. Exact
    /// financial support is separately committed with each durable disposition.
    pub(crate) fn prepare_evidence_view(
        &mut self,
        mut view: crate::rotation::base::ChainView,
    ) -> Result<crate::rotation::base::ChainView> {
        for (attempt, certificate) in &view.resolution_certificates {
            check_resolution(&self.db, self.evidence_namespace, certificate)?;
            let existing:Option<(String,i64,String,u64)> = self.db.query_row("SELECT outcome,height,hash,block_time FROM payment_resolutions WHERE attempt_id=?1",[attempt],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,sql_u64(row,3)?))).optional()?;
            if let Some((outcome, height, hash, block_time)) = existing {
                let expected = if *certificate.outcome()
                    == crate::rotation::base::AuthorizationOutcome::Used
                {
                    "USED"
                } else {
                    "EXPIRED_UNUSED"
                };
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
            check_balance_integrity(&self.db, self.evidence_namespace, incoming)?;
            // Journal support survives cache eviction. Indexed hash lookups return
            // only a conflicting record; ordinary acquisition never enumerates history.
            for point in [&observation.confirmed, &observation.latest] {
                let encoded = serde_json::to_value(point.amount)?;
                let amount_text = encoded.as_str().context("invalid encoded balance amount")?;
                for (column, path) in [("block_hash", "confirmed"), ("latest_hash", "latest")] {
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
                    .map(|(field, kind)| {
                        format!("json_type(e.support,'$.{field}') IS NOT '{kind}'")
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                    let malformed = format!(
                        "({required} OR (json_type(e.support,'$.checkpoint') IS NOT 'null' AND json_type(e.support,'$.checkpoint') IS NOT 'object') OR (json_type(e.support,'$.checkpoint')='object' AND (json_type(e.support,'$.checkpoint.height') IS NOT 'integer' OR json_type(e.support,'$.checkpoint.hash') IS NOT 'text')))"
                    );
                    // Every source projects the same exact support columns.
                    // SQLite flattens the funding projection so both lookups use
                    // their dedicated hash indexes; no historical JSON sweep.
                    for (source, owner, missing) in [
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
                    ] {
                        let sql = format!(
                            "SELECT COALESCE(e.support,''),CASE WHEN json_valid(e.support) THEN {malformed} ELSE 1 END FROM {source} WHERE {owner}=?1 AND e.{column}=?2 AND CASE WHEN e.support IS NULL THEN {missing} WHEN NOT json_valid(e.support) THEN 1 ELSE {malformed} OR json_extract(e.support,'$.observation.{path}.amount') IS NOT ?3 OR json_extract(e.support,'$.observation.{path}.anchor.height') IS NOT ?4 OR json_extract(e.support,'$.observation.{path}.anchor.hash') IS NOT ?2 END LIMIT 1"
                        );
                        let conflicting: Option<(String, bool)> = self
                            .db
                            .query_row(
                                &sql,
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
            }

            if let Some((_, saved)) = self.evidence_cache.get(wallet).cloned() {
                let order = match observation.compare(saved.observation()) {
                    Ok(order) => order,
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
                                        [
                                            &saved.observation().confirmed,
                                            &saved.observation().latest,
                                        ]
                                        .into_iter()
                                        .any(|old| {
                                            point.anchor.hash == old.anchor.hash
                                                && (point.amount != old.amount
                                                    || point.anchor.height != old.anchor.height)
                                        })
                                    })
                                    .map(|point| &point.anchor)
                                    .context("missing conflicting balance anchor")?
                            } else if failure == EvidenceFailure::LatestReplacement {
                                &saved.observation().latest.anchor
                            } else {
                                &saved.observation().confirmed.anchor
                            };
                            self.invalidate_wallet_evidence(
                                wallet,
                                anchor,
                                false,
                                failure == EvidenceFailure::AmountContradiction,
                            )?;
                        }
                        return Err(failure.into());
                    }
                };
                if order == std::cmp::Ordering::Equal {
                    incoming.restrict_validity(saved.observation().valid_until);
                    view.observations
                        .insert(wallet.clone(), incoming.observation().clone());
                }
                if order != std::cmp::Ordering::Greater
                    && crate::rotation::base::now()? <= saved.observation().valid_until
                    && check_balance(&self.db, self.evidence_namespace, &saved).is_ok()
                {
                    *incoming = saved;
                    view.observations
                        .insert(wallet.clone(), incoming.observation().clone());
                    view.balances
                        .insert(wallet.clone(), incoming.observation().available());
                }
            }
            check_balance(&self.db, self.evidence_namespace, incoming)?;
            self.evidence_order = self.evidence_order.saturating_add(1);
            self.evidence_cache
                .insert(wallet.clone(), (self.evidence_order, incoming.clone()));
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
            tx.query_row("SELECT EXISTS(SELECT 1 FROM payment_resolutions r JOIN payment_attempts a ON a.id=r.attempt_id WHERE a.wallet_id=?1 AND r.hash=?2)",params![wallet,anchor.hash],|row|row.get(0))?
        };
        if financial_checkpoint || dependent {
            tx.execute(
                "UPDATE wallets SET evidence_repair=?2 WHERE id=?1",
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
        super::funding::dispute_endpoint_support(&tx, endpoint)?;
        bump_generation(&tx, "endpoint", endpoint)?;
        tx.commit()?;
        self.evidence_cache
            .retain(|_, (_, certificate)| certificate.observation().endpoint != endpoint);
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
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod funding_tests;
