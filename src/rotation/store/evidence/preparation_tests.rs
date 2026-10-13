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
        let app = Router::new().route("/", post(move |Json(request): Json<Value>| {
            let state = state.clone();
            async move {
                let (head, amount, replacement) = *state.lock().unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let height = if tag == "latest" { head } else { u64::from_str_radix(&tag[2..], 16).unwrap() };
                        let hash = height + if replacement == Some(height) { 1000 } else { 0 };
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{hash:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
                    }
                    "eth_call" => {
                        assert_eq!(request["params"][1]["requireCanonical"], true);
                        let value = if request["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231") { amount } else { 1 };
                        json!(format!("0x{value:064x}"))
                    }
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

    fn assert_local_invalidation(&self, sibling: &BalanceObservation, repair: Option<&str>) {
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
        assert_eq!(reasons, [repair.map(str::to_owned), None]);
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
            fixture.assert_local_invalidation(
                &saved.observations[&fixture.wallets[1]],
                dependent.then_some("chain_repair_required"),
            );
        }
    }
}

#[tokio::test]
async fn conflicting_resolution_rejects_each_changed_field_before_balance_publication() {
    for field in ["outcome", "height", "hash", "block_time"] {
        let mut fixture = Fixture::new().await;
        let authorization = fixture.authorization();
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
        fixture.assert_local_invalidation(
            &saved.observations[&fixture.wallets[1]],
            Some("chain_repair_required"),
        );
        for (index, wallet) in fixture.wallets.iter().enumerate() {
            let dispute: Option<String> = fixture
                .store
                .db
                .query_row(
                    "SELECT readiness_disputed FROM funding_jobs WHERE wallet_id=?1",
                    [wallet],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                dispute.as_deref(),
                (index == 0).then_some("chain_repair_required")
            );
        }
        let retained: (String, i64, String, u64) = fixture.store.db.query_row("SELECT outcome,height,hash,block_time FROM payment_resolutions WHERE attempt_id='proof'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,sql_u64(r,3)?))).unwrap();
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
