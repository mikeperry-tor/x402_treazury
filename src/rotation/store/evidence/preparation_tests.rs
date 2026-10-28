use super::*;
use crate::rotation::base::{BaseRpc, ChainQuery, PendingAuthorization, now};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct Fixture {
    store: Store,
    rpc: BaseRpc,
    // Head, balance, optional replacement block height.
    chain: Arc<Mutex<(u64, u64, Option<u64>)>>,
    nonce: Arc<std::sync::atomic::AtomicU64>,
    lagging_finality: Arc<std::sync::atomic::AtomicBool>,
    stamp: Arc<std::sync::atomic::AtomicU64>,
    wallets: Vec<String>,
    pool: String,
    server: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let chain = Arc::new(Mutex::new((100u64, 100u64, None)));
        let state = chain.clone();
        let nonce = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let stamp = Arc::new(std::sync::atomic::AtomicU64::new(now().unwrap()));
        let lagging_finality = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finality_state = lagging_finality.clone();
        let (nonce_state, stamp_state) = (nonce.clone(), stamp.clone());
        let app = Router::new().route("/", post(move |Json(request): Json<Value>| {
            let state = state.clone();
            let (nonce,stamp)=(nonce_state.clone(),stamp_state.clone());
            let finality = finality_state.clone();
            async move {
                let (head, amount, replacement) = *state.lock().unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let height = if tag == "finalized" && finality.load(std::sync::atomic::Ordering::SeqCst) { 88 } else if matches!(tag, "latest" | "finalized") { head } else { u64::from_str_radix(&tag[2..], 16).unwrap() };
                        let hash = height + if replacement == Some(height) { 1000 } else { 0 };
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{hash:064x}"),"timestamp":format!("0x{:x}",stamp.load(std::sync::atomic::Ordering::SeqCst))})
                    }
                    "eth_call" => {
                        assert_eq!(request["params"][1]["requireCanonical"], true);
                        let value = if request["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231") {
                            let hash = request["params"][1]["blockHash"].as_str().unwrap();
                            let height = u64::from_str_radix(&hash[2..], 16).unwrap();
                            if finality.load(std::sync::atomic::Ordering::SeqCst) && height <= 100 { 0 } else { amount }
                        } else { nonce.load(std::sync::atomic::Ordering::SeqCst) };
                        json!(format!("0x{value:064x}"))
                    }
                    _ => panic!("unexpected RPC mutation"),
                };
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &directory.path().join("state"),
            &directory.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "preparation",
            "1",
            Some(U256::from(100_000_000)),
            0,
        )
        .unwrap();
        let wallets = store
            .status()
            .unwrap()
            .pools
            .remove(0)
            .addresses
            .into_iter()
            .map(|wallet| wallet.id)
            .collect();
        Self {
            store,
            rpc,
            chain,
            nonce,
            lagging_finality,
            stamp,
            wallets,
            pool,
            server,
            _directory: directory,
        }
    }

    fn query(&self) -> ChainQuery {
        self.store
            .scoped_chain_query(&self.pool, self.wallets.clone(), vec![])
            .unwrap()
    }

    fn authorization(&mut self) -> PendingAuthorization {
        let wallet = &self.wallets[0];
        let payer: String = self
            .store
            .db
            .query_row("SELECT address FROM wallets WHERE id=?1", [wallet], |r| {
                r.get(0)
            })
            .unwrap();
        let nonce = format!("0x{:064x}", 1);
        let valid_before = now().unwrap() + 100;
        self.store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) VALUES('proof',?1,?2,0,'7','fixture','POSSIBLY_SUBMITTED',?3,?4,?5,1)",params![self.pool,wallet,payer,nonce,i64::try_from(valid_before).unwrap()]).unwrap();
        add_exposure(&self.store.db, wallet, U256::from(7)).unwrap();
        PendingAuthorization {
            id: "proof".into(),
            wallet: wallet.clone(),
            payer,
            nonce,
            valid_before,
        }
    }

    fn saved_resolution(&self, outcome: &str, anchor: &Anchor, block_time: u64) {
        self.store.db.execute("INSERT INTO payment_resolutions(attempt_id,outcome,height,hash,block_time) VALUES('proof',?1,?2,?3,?4)",params![outcome,i64::try_from(anchor.height).unwrap(),anchor.hash,i64::try_from(block_time).unwrap()]).unwrap();
    }

    fn assert_local_invalidation(&self, sibling: &BalanceObservation, archived: bool) {
        assert!(!self.store.evidence_cache.contains_key(&self.wallets[0]));
        assert_eq!(
            self.store.evidence_cache[&self.wallets[1]].1.observation(),
            sibling
        );
        assert_eq!(
            generation(&self.store.db, "wallet", &self.wallets[0]).unwrap(),
            1
        );
        assert_eq!(
            generation(&self.store.db, "wallet", &self.wallets[1]).unwrap(),
            0
        );
        let reasons: Vec<Option<String>> = self
            .wallets
            .iter()
            .map(|wallet| {
                self.store
                    .db
                    .query_row(
                        "SELECT evidence_repair FROM wallets WHERE id=?1",
                        [wallet],
                        |r| r.get(0),
                    )
                    .unwrap()
            })
            .collect();
        assert_eq!(reasons, [None, None]);
        assert_eq!(
            self.store
                .db
                .query_row("SELECT COUNT(*) FROM payment_resolution_history", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            i64::from(archived)
        );
        assert_eq!(
            exposure(&self.store.db, &self.wallets[0]).unwrap(),
            U256::from(7)
        );
        assert_eq!(checkpoint(&self.store.db, &self.wallets[0]).unwrap(), None);
    }
}

#[tokio::test]
async fn older_valid_observation_reuses_cached_certificate_without_relabeling() {
    let mut fixture = Fixture::new().await;
    let captured = fixture.query();
    let old = fixture.rpc.view(captured.clone()).await.unwrap();
    *fixture.chain.lock().unwrap() = (120, 80, None);
    let newer = fixture.rpc.view(captured).await.unwrap();
    fixture.store.prepare_evidence_view(newer.clone()).unwrap();
    let prepared = fixture.store.prepare_evidence_view(old).unwrap();
    for wallet in &fixture.wallets {
        assert_eq!(prepared.observations[wallet], newer.observations[wallet]);
        assert_eq!(
            prepared.balance_certificates[wallet].observation(),
            &newer.observations[wallet]
        );
        assert_eq!(prepared.balances[wallet], U256::from(80));
        assert_eq!(
            fixture.store.evidence_cache[wallet].1.observation(),
            &newer.observations[wallet]
        );
        assert_eq!(generation(&fixture.store.db, "wallet", wallet).unwrap(), 0);
    }
}

#[tokio::test]
async fn cached_replacement_invalidates_only_its_saved_anchor_and_wallet() {
    for (height, failure) in [
        (88, EvidenceFailure::ConfirmedReplacement),
        (100, EvidenceFailure::LatestReplacement),
    ] {
        // A durable nonce proof at the replaced anchor must be marked for repair;
        // one at the other anchor must stay intact. Both retain the liability.
        for dependent in [true, false] {
            let mut fixture = Fixture::new().await;
            fixture.authorization();
            let captured = fixture.query();
            let saved = fixture.rpc.view(captured.clone()).await.unwrap();
            fixture.store.prepare_evidence_view(saved.clone()).unwrap();
            let observation = &saved.observations[&fixture.wallets[0]];
            let anchor = if (height == 88) == dependent {
                &observation.confirmed.anchor
            } else {
                &observation.latest.anchor
            };
            fixture.saved_resolution("USED", anchor, now().unwrap());
            *fixture.chain.lock().unwrap() = (100, 100, Some(height));
            // Use a capture from before cache publication, so this specifically
            // exercises the store comparison rather than acquisition's recheck.
            let mut changed = fixture.rpc.view(captured).await.unwrap();
            changed
                .balance_certificates
                .retain(|wallet, _| wallet == &fixture.wallets[0]);
            let error = fixture.store.prepare_evidence_view(changed).err().unwrap();
            assert_eq!(error.downcast_ref::<EvidenceFailure>(), Some(&failure));
            fixture.assert_local_invalidation(&saved.observations[&fixture.wallets[1]], dependent);
        }
    }
}

#[tokio::test]
async fn conflicting_resolution_rejects_each_changed_field_before_balance_publication() {
    for field in ["outcome", "height", "hash", "block_time"] {
        let mut fixture = Fixture::new().await;
        let authorization = fixture.authorization();
        // Both outcomes must be intrinsically possible at the saved block time;
        // impossible expiry tuples are repaired as malformed derived evidence.
        fixture.stamp.store(
            authorization.valid_before + 1,
            std::sync::atomic::Ordering::SeqCst,
        );
        let captured = fixture.query();
        let saved = fixture.rpc.view(captured).await.unwrap();
        fixture.store.prepare_evidence_view(saved.clone()).unwrap();
        let query = fixture
            .store
            .scoped_chain_query(&fixture.pool, fixture.wallets.clone(), vec![authorization])
            .unwrap();
        let mut incoming = fixture.rpc.view(query).await.unwrap();
        let certificate = &incoming.resolution_certificates["proof"];
        let mut anchor = certificate.anchor().clone();
        let mut block_time = certificate.block_time();
        let outcome = if field == "outcome" {
            "EXPIRED_UNUSED"
        } else {
            "USED"
        };
        if field == "height" {
            anchor.height += 1;
        }
        if field == "hash" {
            anchor.hash = format!("0x{:064x}", 999);
        }
        if field == "block_time" {
            block_time += 1;
        }
        fixture.saved_resolution(outcome, &anchor, block_time);
        // Both wallets reference the saved resolution anchor. Only the failed
        // wallet's matching durable support may be disputed, never its sibling's.
        fixture
            .store
            .db
            .execute(
                "UPDATE funding_jobs SET readiness_hash=?1 WHERE pool_id=?2",
                params![anchor.hash, fixture.pool],
            )
            .unwrap();
        // Malformed balance binding would fail if balance processing preceded
        // the resolution conflict or if processing continued after its error.
        incoming.observations.clear();
        let order = fixture.store.evidence_order;
        let error = fixture.store.prepare_evidence_view(incoming).err().unwrap();
        assert_eq!(
            error.downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::ResolutionConflict),
            "{field}"
        );
        assert_eq!(fixture.store.evidence_order, order);
        fixture.assert_local_invalidation(&saved.observations[&fixture.wallets[1]], true);
        for wallet in &fixture.wallets {
            let dispute: Option<String> = fixture
                .store
                .db
                .query_row(
                    "SELECT readiness_disputed FROM funding_jobs WHERE wallet_id=?1",
                    [wallet],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                dispute.is_none(),
                "a chain conflict cannot reopen the initial claim"
            );
        }
        let retained: (String, i64, String, u64) = fixture.store.db.query_row("SELECT outcome,height,hash,block_time FROM payment_resolution_history WHERE attempt_id='proof'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,sql_u64(r,3)?))).unwrap();
        assert_eq!(
            retained,
            (
                outcome.into(),
                anchor.height as i64,
                anchor.hash,
                block_time
            )
        );
    }
}

fn publication_rows(store: &Store) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "wallets",
        "payment_attempts",
        "payment_resolutions",
        "funding_jobs",
        "funding_progress",
        "pools",
        "reserve_claims",
    ]
    .into_iter()
    .map(|table| {
        let mut query = store
            .db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = query.column_count();
        query
            .query_map([], |row| {
                (0..columns)
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}

#[tokio::test]
async fn publication_rolls_back_resolution_and_first_balance_after_late_failure() {
    let mut fixture = Fixture::new().await;
    fixture.chain.lock().unwrap().1 = 1_000_000;
    let authorization = fixture.authorization();
    let query = fixture
        .store
        .scoped_chain_query(&fixture.pool, fixture.wallets.clone(), vec![authorization])
        .unwrap();
    let view = fixture.rpc.view(query).await.unwrap();
    let before = publication_rows(&fixture.store);
    let sibling = &fixture.wallets[1];
    fixture.store.db.execute_batch(&format!(
        "CREATE TEMP TRIGGER reject_late_publication BEFORE UPDATE OF balance ON wallets
         WHEN NEW.id='{sibling}' AND EXISTS(SELECT 1 FROM payment_resolutions WHERE attempt_id='proof')
         AND EXISTS(SELECT 1 FROM wallets WHERE id='{}' AND balance='1000000')
         AND EXISTS(SELECT 1 FROM funding_jobs WHERE readiness_requirement IS NOT NULL)
         BEGIN SELECT RAISE(ABORT,'fixture_late_publication'); END;", fixture.wallets[0])).unwrap();
    let error = fixture
        .store
        .reconcile_pool(&fixture.pool, view.clone())
        .unwrap_err();
    assert_eq!(error.to_string(), "fixture_late_publication");
    assert_eq!(publication_rows(&fixture.store), before);
    assert_eq!(
        exposure(&fixture.store.db, &fixture.wallets[0]).unwrap(),
        U256::from(7)
    );
    assert_eq!(
        checkpoint(&fixture.store.db, &fixture.wallets[0]).unwrap(),
        None
    );
    fixture
        .store
        .db
        .execute_batch("DROP TRIGGER reject_late_publication")
        .unwrap();
    fixture
        .store
        .reconcile_pool(&fixture.pool, view.clone())
        .unwrap();
    assert_eq!(
        exposure(&fixture.store.db, &fixture.wallets[0]).unwrap(),
        U256::ZERO
    );
    assert_eq!(
        checkpoint(&fixture.store.db, &fixture.wallets[0]).unwrap(),
        Some(
            view.resolutions["proof"]
                .anchor
                .clone()
                .unwrap_or(view.anchor.clone())
        )
    );
    let after = publication_rows(&fixture.store);
    fixture.store.reconcile_pool(&fixture.pool, view).unwrap();
    assert_eq!(publication_rows(&fixture.store), after);
}

#[tokio::test]
async fn scoped_publication_leaves_sibling_untouched_and_complete_requires_it() {
    let mut fixture = Fixture::new().await;
    let query = fixture
        .store
        .scoped_chain_query(&fixture.pool, vec![fixture.wallets[0].clone()], vec![])
        .unwrap();
    let view = fixture.rpc.view(query).await.unwrap();
    let before = publication_rows(&fixture.store);
    assert_eq!(
        fixture
            .store
            .reconcile_pool(&fixture.pool, view.clone())
            .unwrap_err()
            .to_string(),
        "incomplete Base view"
    );
    assert_eq!(publication_rows(&fixture.store), before);
    fixture
        .store
        .publish_scoped_evidence(&fixture.pool, view)
        .unwrap();
    let after = publication_rows(&fixture.store);
    assert_ne!(after[0][0], before[0][0]);
    assert_eq!(after[0][1], before[0][1]);
    assert_eq!(after[1], before[1]);
    assert_eq!(after[2], before[2]);
    let next = fixture.rpc.view(fixture.query()).await.unwrap();
    fixture.store.reconcile_pool(&fixture.pool, next).unwrap();
    let balance: String = fixture
        .store
        .db
        .query_row(
            "SELECT balance FROM wallets WHERE id=?1",
            [&fixture.wallets[1]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(balance, "100");
}

#[tokio::test]
async fn resolved_history_is_rechecked_before_reacquisition_at_a_newer_head() {
    let mut fixture = Fixture::new().await;
    let authorization = fixture.authorization();
    let query = fixture
        .store
        .scoped_chain_query(&fixture.pool, vec![], vec![authorization.clone()])
        .unwrap();
    let first = fixture.rpc.view(query).await.unwrap();
    fixture
        .store
        .publish_scoped_evidence(&fixture.pool, first)
        .unwrap();
    let original: (String, i64, String, String) = fixture
        .store
        .db
        .query_row(
            "SELECT outcome,height,hash,support FROM payment_resolutions WHERE attempt_id='proof'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    fixture.chain.lock().unwrap().0 = 101;
    let query = fixture
        .store
        .scoped_chain_query(&fixture.pool, vec![], vec![authorization.clone()])
        .unwrap();
    assert!(query.pending.is_empty());
    assert!(query.captures.is_empty());
    let retained: (String, i64, String, String) = fixture
        .store
        .db
        .query_row(
            "SELECT outcome,height,hash,support FROM payment_resolutions WHERE attempt_id='proof'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(original, retained);
    assert_eq!(
        exposure(&fixture.store.db, &fixture.wallets[0]).unwrap(),
        U256::ZERO
    );
    let repair: Option<String> = fixture
        .store
        .db
        .query_row(
            "SELECT evidence_repair FROM wallets WHERE id=?1",
            [&fixture.wallets[0]],
            |r| r.get(0),
        )
        .unwrap();
    assert!(repair.is_none());
    for field in ["wallet", "payer", "nonce", "valid_before"] {
        let mut wrong = authorization.clone();
        match field {
            "wallet" => wrong.wallet = "wrong".into(),
            "payer" => wrong.payer = "wrong".into(),
            "nonce" => wrong.nonce = "wrong".into(),
            _ => wrong.valid_before += 1,
        }
        assert!(
            fixture
                .store
                .scoped_chain_query(&fixture.pool, vec![], vec![wrong])
                .is_err()
        );
    }
    assert!(
        fixture
            .store
            .scoped_chain_query("wrong-pool", vec![], vec![authorization])
            .is_err()
    );
}

#[tokio::test]
async fn overlapping_resolution_reads_preserve_first_proof_in_either_completion_order() {
    for newer_first in [false, true] {
        let mut fixture = Fixture::new().await;
        let auth = fixture.authorization();
        let query = fixture
            .store
            .scoped_chain_query(&fixture.pool, vec![], vec![auth])
            .unwrap();
        let older = fixture.rpc.view(query.clone()).await.unwrap();
        fixture.chain.lock().unwrap().0 = 101;
        let newer = fixture.rpc.view(query).await.unwrap();
        let (first, second) = if newer_first {
            (newer, older)
        } else {
            (older, newer)
        };
        fixture
            .store
            .publish_scoped_evidence(&fixture.pool, first)
            .unwrap();
        let saved: (String,String,i64,String) = fixture.store.db.query_row(
            "SELECT outcome,hash,height,support FROM payment_resolutions WHERE attempt_id='proof'",[],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        // A concurrent checkpoint change can reject an old capture. It does
        // not overwrite the first proof or demand a foreground repair read.
        let error = fixture
            .store
            .publish_scoped_evidence(&fixture.pool, second)
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::CompatibilityRequired)
        );
        let retained: (String,String,i64,String) = fixture.store.db.query_row(
            "SELECT outcome,hash,height,support FROM payment_resolutions WHERE attempt_id='proof'",[],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(retained, saved);
        assert_eq!(
            exposure(&fixture.store.db, &fixture.wallets[0]).unwrap(),
            U256::ZERO
        );
        let repair: Option<String> = fixture
            .store
            .db
            .query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [&fixture.wallets[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert!(repair.is_none());
        assert_eq!(
            checkpoint(&fixture.store.db, &fixture.wallets[0])
                .unwrap()
                .unwrap()
                .height,
            saved.2 as u64
        );
    }
}

#[tokio::test]
async fn malformed_resolution_bindings_cannot_invalidate_financial_evidence() {
    for mutation in ["id", "outcome", "time", "anchor", "missing", "duplicate"] {
        let mut fixture = Fixture::new().await;
        let auth = fixture.authorization();
        let query = fixture
            .store
            .scoped_chain_query(&fixture.pool, vec![], vec![auth])
            .unwrap();
        let mut view = fixture.rpc.view(query).await.unwrap();
        match mutation {
            "id" => {
                let cert = view.resolution_certificates.remove("proof").unwrap();
                view.resolution_certificates.insert("other".into(), cert);
            }
            "outcome" => {
                view.resolutions.get_mut("proof").unwrap().outcome =
                    crate::rotation::base::AuthorizationOutcome::ExpiredUnused
            }
            "time" => view.resolutions.get_mut("proof").unwrap().block_time += 1,
            "anchor" => {
                view.resolutions
                    .get_mut("proof")
                    .unwrap()
                    .anchor
                    .as_mut()
                    .unwrap()
                    .height += 1
            }
            "missing" => view.released.clear(),
            _ => view.released.push("proof".into()),
        }
        assert!(
            fixture
                .store
                .publish_scoped_evidence(&fixture.pool, view)
                .is_err()
        );
        assert_eq!(
            exposure(&fixture.store.db, &fixture.wallets[0]).unwrap(),
            U256::from(7)
        );
        let (proofs, repairs): (i64,i64) = fixture.store.db.query_row("SELECT (SELECT COUNT(*) FROM payment_resolutions),(SELECT COUNT(*) FROM wallets WHERE evidence_repair IS NOT NULL)",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((proofs, repairs), (0, 0));
    }
}

#[path = "reorg_tests.rs"]
mod reorg_tests;

#[tokio::test]
async fn historical_balance_support_is_audit_only_and_cannot_veto_canonical_proof() {
    let mut fixture = Fixture::new().await;
    let original = fixture.authorization();
    fixture.store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) SELECT 'later-proof',pool_id,wallet_id,generation,amount,requirements_hash,state,payer,?1,valid_before,2 FROM payment_attempts WHERE id='proof'", [format!("0x{:064x}", 2)]).unwrap();
    let wallet = original.wallet.clone();
    let initial = fixture.rpc.view(fixture.query()).await.unwrap();
    let observation = initial.observations[&wallet].clone();
    let mut contradiction = BalanceSupport {
        observation: observation.clone(),
        checkpoint: None,
    };
    contradiction.observation.confirmed.amount += U256::from(1);
    let valid_conflict = serde_json::to_string(&contradiction).unwrap();
    for (attempt, support) in [
        ("proof", "{broken"),
        ("later-proof", valid_conflict.as_str()),
    ] {
        fixture.store.db.execute("INSERT INTO payment_admission_evidence(attempt_id,block_height,block_hash,balance,reserved_before,support,latest_height,latest_hash,endpoint,payer_role) VALUES(?1,?2,?3,'100','0',?4,?5,?6,?7,'RETAINED')", params![attempt,i64::try_from(observation.confirmed.anchor.height).unwrap(),observation.confirmed.anchor.hash,support,i64::try_from(observation.latest.anchor.height).unwrap(),observation.latest.anchor.hash,observation.endpoint]).unwrap();
    }
    fixture
        .store
        .publish_scoped_evidence(&fixture.pool, initial)
        .unwrap();
    assert_eq!(
        exposure(&fixture.store.db, &wallet).unwrap(),
        U256::from(14)
    );
    assert_eq!(generation(&fixture.store.db, "wallet", &wallet).unwrap(), 0);
    let stored: Vec<String> = fixture
        .store
        .db
        .prepare("SELECT support FROM payment_admission_evidence ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(stored, vec!["{broken".to_owned(), valid_conflict]);

    // With only malformed optional history, the very same canonical block is
    // immediately usable. Neither an advancing head nor repeated invalidation
    // is needed, and every original authorization stays potentially chargeable.
    fixture.store.db.execute("UPDATE payment_admission_evidence SET support='{also broken' WHERE attempt_id='later-proof'", []).unwrap();
    fixture.store.db.execute("UPDATE funding_jobs SET readiness_support='{broken funding history',readiness_hash=?2,readiness_latest_hash=?3 WHERE wallet_id=?1", params![wallet,observation.confirmed.anchor.hash,observation.latest.anchor.hash]).unwrap();
    for _ in 0..2 {
        let view = fixture.rpc.view(fixture.query()).await.unwrap();
        assert_eq!(view.observations[&wallet], observation);
        fixture
            .store
            .publish_scoped_evidence(&fixture.pool, view)
            .unwrap();
    }
    assert_eq!(generation(&fixture.store.db, "wallet", &wallet).unwrap(), 0);
    assert_eq!(
        exposure(&fixture.store.db, &wallet).unwrap(),
        U256::from(14)
    );
    assert_eq!(
        fixture
            .store
            .db
            .query_row(
                "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    for table in [
        "payment_resolutions",
        "payment_resolution_history",
        "outgoing",
        "operation_network",
        "wallet_retirements",
    ] {
        assert_eq!(
            fixture
                .store
                .db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "forbidden side effect in {table}"
        );
    }
    let stored: Vec<String> = fixture
        .store
        .db
        .prepare("SELECT support FROM payment_admission_evidence ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(stored, vec!["{broken", "{also broken"]);
    let funding_history: (String, Option<String>) = fixture
        .store
        .db
        .query_row(
            "SELECT readiness_support,readiness_disputed FROM funding_jobs WHERE wallet_id=?1",
            [&wallet],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(funding_history, ("{broken funding history".into(), None));
}

#[tokio::test]
async fn newer_latest_credit_prevents_retirement_during_finality_lag_and_late_publication() {
    let mut f = Fixture::new().await;
    let job_id: String = f
        .store
        .db
        .query_row(
            "SELECT id FROM funding_jobs WHERE allocation_kind='large'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let job = f.store.funding_job(&job_id).unwrap().unwrap();
    // An observed external first credit consumes this claim; removing its large
    // reserve creates a retained wallet eligible at a one-unit tolerance.
    f.store
        .record_funding_balance(
            &job.id,
            &job.operation_id,
            &job.wallet_id,
            &job.recipient,
            U256::from(1),
        )
        .unwrap();
    f.store
        .configure_capacity_group(
            &f.pool,
            Some(U256::from(100_000_000)),
            1,
            &crate::rotation::store::test_support::reserves("1"),
        )
        .unwrap();
    assert!(
        f.store
            .retirement_eligible(&f.pool, &job.wallet_id)
            .unwrap()
    );
    f.lagging_finality
        .store(true, std::sync::atomic::Ordering::SeqCst);
    *f.chain.lock().unwrap() = (100, 0, None);
    let query = f
        .store
        .scoped_chain_query(&f.pool, vec![job.wallet_id.clone()], vec![])
        .unwrap();
    let old = f.rpc.view(query.clone()).await.unwrap();
    *f.chain.lock().unwrap() = (101, 7, None);
    let newer = f.rpc.view(query).await.unwrap();
    assert_eq!(
        old.observations[&job.wallet_id].confirmed,
        newer.observations[&job.wallet_id].confirmed
    );
    assert_eq!(
        newer.observations[&job.wallet_id].latest.amount,
        U256::from(7)
    );
    // Cache publication is independent of the later retirement transaction.
    // Seed the old zero certificate so an Equal comparison would reuse it.
    f.store.prepare_evidence_view(old.clone()).unwrap();
    f.store
        .publish_scoped_evidence(&f.pool, newer.clone())
        .unwrap();
    let published = f.store.publish_scoped_view(&f.pool, old).unwrap();
    assert_eq!(
        published.observations[&job.wallet_id],
        newer.observations[&job.wallet_id]
    );
    assert_eq!(
        f.store
            .db
            .query_row(
                "SELECT role FROM wallets WHERE id=?1",
                [&job.wallet_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "RETAINED"
    );
    assert_eq!(
        f.store
            .db
            .query_row(
                "SELECT COUNT(*) FROM wallet_retirements WHERE wallet_id=?1",
                [&job.wallet_id],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn committed_retirement_discards_demand_from_each_publisher_and_late_completions() {
    for scoped in [true, false] {
        let mut f = Fixture::new().await;
        let wallet = f.wallets[0].clone();
        let sibling = f.wallets[1].clone();
        for id in &f.wallets {
            f.store.record_credit(id, "100", "fixture", 1).unwrap();
        }
        f.store
            .db
            .execute_batch("UPDATE reserve_claims SET enabled=0; UPDATE wallets SET role='RETAINED',balance='0';")
            .unwrap();
        for id in &f.wallets {
            f.store.consider_retirement(&f.pool, id).unwrap();
        }
        assert!(f.store.begin_retirement(&f.pool, &wallet).unwrap());
        f.store.finish_retirement(
            &f.pool,
            &sibling,
            true,
            true,
            Some("canonical_observation_unavailable"),
            None,
        );
        let sibling_due = f.store.retirement_due(&sibling);
        // An independently reused payer keeps its existing marker until local
        // eligibility reevaluation; retirement of another wallet cannot erase it.
        f.store
            .db
            .execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1", [&sibling])
            .unwrap();
        f.chain.lock().unwrap().1 = 0;
        let query = if scoped {
            f.store
                .scoped_chain_query(&f.pool, vec![wallet.clone()], vec![])
                .unwrap()
        } else {
            f.store.chain_query(&f.pool).unwrap()
        };
        let view = f.rpc.view(query).await.unwrap();
        f.store.db.execute_batch("CREATE TRIGGER reject_retirement BEFORE INSERT ON wallet_retirements BEGIN SELECT RAISE(ABORT,'fixture retirement commit failure'); END;").unwrap();
        let failed = if scoped {
            f.store.publish_scoped_evidence(&f.pool, view.clone())
        } else {
            f.store.reconcile_pool(&f.pool, view.clone())
        };
        assert!(failed.is_err());
        assert!(f.store.retirement_work.contains_key(&wallet));
        assert_eq!(
            f.store
                .db
                .query_row("SELECT role FROM wallets WHERE id=?1", [&wallet], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "RETAINED"
        );
        f.store
            .db
            .execute_batch("DROP TRIGGER reject_retirement")
            .unwrap();
        if scoped {
            f.store.publish_scoped_evidence(&f.pool, view).unwrap();
        } else {
            f.store.reconcile_pool(&f.pool, view).unwrap();
        }
        assert_eq!(
            f.store
                .db
                .query_row("SELECT role FROM wallets WHERE id=?1", [&wallet], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "RETIRED"
        );
        assert!(!f.store.retirement_work.contains_key(&wallet));
        // The maintenance owner may finish after another canonical publisher.
        f.store.finish_retirement(
            &f.pool,
            &wallet,
            true,
            true,
            Some("canonical_observation_unavailable"),
            None,
        );
        f.store
            .finish_balance(&f.pool, &wallet, Some(U256::ZERO))
            .unwrap();
        f.store.consider_retirement(&f.pool, &wallet).unwrap();
        assert!(!f.store.retirement_work.contains_key(&wallet));
        assert!(f.store.retirement_requested(&sibling));
        assert_eq!(f.store.retirement_due(&sibling), sibling_due);
    }
}
