//! Real signing and HTTP dispatch with advisory data unavailable.
use super::*;
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    dir: tempfile::TempDir,
    pool: ManagedPool,
    payer: PaymentCandidateHandle,
    url: String,
    signed: Arc<AtomicUsize>,
    received: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
    worker: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.path().join("state/state.sqlite")).unwrap()
    }
    async fn pay(&self) -> Result<(reqwest::Response, bool)> {
        let http = crate::network::NetworkContext::new(Default::default())?.http(
            &crate::network::IsolationId::evm(&self.payer.address)?,
            &self.url,
            None,
        )?;
        let retry = http.get(&self.url).build()?;
        let challenge = http.execute(retry.try_clone().unwrap()).await?;
        self.pool
            .pay(&http, self.payer.clone(), retry, challenge)
            .await
    }
    async fn close(self) {
        self.pool.drain_direct_payments().await;
        drop(self.pool);
        self.worker.await.unwrap();
        self.server.abort();
    }
}
async fn fixture() -> Fixture {
    fixture_with_held_response(false).await
}
async fn fixture_with_held_response(hold: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut store = super::super::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        store,
        "advisory",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(100)),
        0
    )
    .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let (wallet, address): (String, String) = db
        .query_row(
            "SELECT id,address FROM wallets ORDER BY sequence LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    db.execute(
        "UPDATE wallets SET role='ACTIVE',balance='2' WHERE id=?1",
        [&wallet],
    )
    .unwrap();
    db.execute(
        "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1 WHERE wallet_id=?1",
        [&wallet],
    )
    .unwrap();
    let signed = Arc::new(AtomicUsize::new(0));
    let observed = signed.clone();
    let received = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let (notify, barrier) = (received.clone(), release.clone());
    let state_path = dir.path().join("state/state.sqlite");
    let app = Router::new().route("/pay",get(move |headers: HeaderMap| {
        let (signed,path,notify,barrier) = (observed.clone(),state_path.clone(),notify.clone(),barrier.clone());
        async move {
            if let Some(signature) = headers.get("payment-signature") {
                let payload: Value = serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
                let nonce = payload["payload"]["authorization"]["nonce"].as_str().unwrap();
                let db = rusqlite::Connection::open(path).unwrap();
                assert_eq!(db.query_row("SELECT count(*) FROM payment_attempts WHERE nonce=?1",[nonce],|r|r.get::<_,i64>(0)).unwrap(),1);
                signed.fetch_add(1,Ordering::SeqCst);
                notify.notify_one();
                if hold { barrier.acquire().await.unwrap().forget(); }
                return "paid".into_response();
            }
            let challenge = serde_json::json!({"x402Version":2,"resource":{"url":"https://example.com/pay"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"7","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
        }
    }));
    let app = app.route("/rpc", post(|Json(request): Json<Value>| async move {
        let result = match request["method"].as_str().unwrap() {
            "eth_chainId" => serde_json::json!("0x2105"),
            "eth_getBlockByNumber" => {
                let tag = request["params"][0].as_str().unwrap();
                let height = if tag == "latest" { 100 } else { u64::from_str_radix(&tag[2..], 16).unwrap() };
                serde_json::json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
            },
            "eth_call" => {
                let value = if request["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231") { 0 } else { 1 };
                serde_json::json!(format!("0x{value:064x}"))
            },
            method => panic!("unexpected fixture method {method}"),
        };
        Json(serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/pay", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (store, worker) = StoreHandle::spawn(store);
    let pool = ManagedPool::new(
        store,
        pool,
        BaseRpc::new("http://127.0.0.1:1", 12).unwrap(),
        SpendPolicy {
            max_atomic: Some(U256::from(100)),
        },
    )
    .unwrap();
    Fixture {
        dir,
        pool,
        payer: PaymentCandidateHandle { wallet, address },
        url,
        signed,
        received,
        release,
        worker,
        server,
    }
}

#[tokio::test]
async fn qualification_canonical_loop_ignores_bad_historical_readiness_and_resolves_later_wallet() {
    let mut f = fixture().await;
    f.pool.base = BaseRpc::new(&f.url.replace("/pay", "/rpc"), 12).unwrap();
    f.pay().await.unwrap().0.text().await.unwrap();
    let other: PaymentCandidateHandle = f
        .db()
        .query_row(
            "SELECT id,address FROM wallets WHERE id!=?1",
            [&f.payer.wallet],
            |r| {
                Ok(PaymentCandidateHandle {
                    wallet: r.get(0)?,
                    address: r.get(1)?,
                })
            },
        )
        .unwrap();
    let wallet = other.wallet.clone();
    f.pool
        .store
        .call(move |s| s.record_credit(&wallet, "5000000", "fixture", 100))
        .await
        .unwrap();
    let http = crate::network::NetworkContext::new(Default::default())
        .unwrap()
        .http(
            &crate::network::IsolationId::evm(&other.address).unwrap(),
            &f.url,
            None,
        )
        .unwrap();
    let request = http.get(&f.url).build().unwrap();
    let challenge = http.execute(request.try_clone().unwrap()).await.unwrap();
    f.pool
        .pay(&http, other, request, challenge)
        .await
        .unwrap()
        .0
        .text()
        .await
        .unwrap();
    f.db().execute("UPDATE funding_jobs SET readiness_requirement='1',readiness_support='malformed',readiness_disputed=NULL WHERE wallet_id=?1",[&f.payer.wallet]).unwrap();
    // Invoke the same qualification observation loop used by maintenance, without
    // installing a process-global spending guard in this unfunded fixture.
    f.pool.qualification_observation(true).await.unwrap();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM payment_resolutions WHERE outcome='USED'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    f.close().await;
}
#[tokio::test]
async fn malformed_text_address_hands_off_without_signing_to_a_healthy_sibling() {
    for estimate in ["100", "0"] {
        let mut f = fixture().await;
        let other: PaymentCandidateHandle = f
            .db()
            .query_row(
                "SELECT id,address FROM wallets WHERE id!=?1",
                [&f.payer.wallet],
                |r| {
                    Ok(PaymentCandidateHandle {
                        wallet: r.get(0)?,
                        address: r.get(1)?,
                    })
                },
            )
            .unwrap();
        let healthy = other.wallet.clone();
        f.pool
            .store
            .call(move |s| s.record_credit(&healthy, "5000000", "fixture", 100))
            .await
            .unwrap();
        f.db()
            .execute("UPDATE wallets SET balance=?1", [estimate])
            .unwrap();
        f.db()
            .execute(
                "UPDATE wallets SET address='invalid EVM address' WHERE id=?1",
                [&f.payer.wallet],
            )
            .unwrap();
        let initial = f
            .pool
            .next_candidate(U256::ZERO, i64::MAX, &[])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(initial.wallet, other.wallet);
        assert_eq!(initial.address, other.address);
        let error = f.pay().await.err().unwrap();
        let selected = error.downcast_ref::<PayerHandoff>().unwrap().0.clone();
        assert_eq!(selected.wallet, other.wallet);
        assert_eq!(selected.address, other.address);
        assert_eq!(f.signed.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.db()
                .query_row("SELECT count(*) FROM payment_attempts", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        f.payer = selected;
        assert!(f.pay().await.unwrap().0.status().is_success());
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        let attempted: Vec<String> = f
            .db()
            .prepare("SELECT wallet_id FROM payment_attempts")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(attempted, vec![other.wallet]);
        f.close().await;
    }
}

#[tokio::test]
async fn old_low_missing_and_malformed_estimates_and_readiness_do_not_gate_signing() {
    let f = fixture().await;
    for estimate in ["100", "2", "", "malformed"] {
        let db = f.db();
        db.execute("UPDATE wallets SET balance=?1,block_height=1,block_hash='old',unresolved_exposure='malformed' WHERE id=?2",rusqlite::params![estimate,f.payer.wallet]).unwrap();
        db.execute(
            "UPDATE funding_jobs SET readiness_support='malformed',readiness_disputed='old'",
            [],
        )
        .unwrap();
        drop(db);
        assert!(f.pay().await.unwrap().0.status().is_success());
        let after: String = f
            .db()
            .query_row(
                "SELECT balance FROM wallets WHERE id=?1",
                [&f.payer.wallet],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            after,
            match estimate {
                "100" => "93",
                "2" => "0",
                other => other,
            }
        );
    }
    assert_eq!(f.signed.load(Ordering::SeqCst), 4);
    f.db()
        .execute(
            "UPDATE wallets SET balance=x'ff' WHERE id=?1",
            [&f.payer.wallet],
        )
        .unwrap();
    assert!(f.pay().await.unwrap().0.status().is_success());
    assert_eq!(f.signed.load(Ordering::SeqCst), 5);
    f.close().await;
}

#[tokio::test]
async fn failed_journal_rolls_back_estimate_and_discloses_no_signature() {
    let f = fixture().await;
    f.db().execute_batch("CREATE TRIGGER reject_attempt BEFORE INSERT ON payment_attempts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f.pay().await.is_err());
    assert_eq!(f.signed.load(Ordering::SeqCst), 0);
    let (balance, count): (String, i64) = f
        .db()
        .query_row(
            "SELECT balance,(SELECT count(*) FROM payment_attempts) FROM wallets WHERE id=?1",
            [&f.payer.wallet],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((balance.as_str(), count), ("2", 0));
    f.close().await;
}

#[tokio::test]
async fn concurrent_signatures_keep_unique_journals_and_one_debit_each() {
    let f = fixture().await;
    f.db()
        .execute(
            "UPDATE wallets SET balance='100' WHERE id=?1",
            [&f.payer.wallet],
        )
        .unwrap();
    let (a, b) = tokio::join!(f.pay(), f.pay());
    a.unwrap();
    b.unwrap();
    let (balance,count,nonces): (String,i64,i64) = f.db().query_row("SELECT balance,(SELECT count(*) FROM payment_attempts),(SELECT count(DISTINCT nonce) FROM payment_attempts) FROM wallets WHERE id=?1",[&f.payer.wallet],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!((balance.as_str(), count, nonces), ("86", 2, 2));
    f.close().await;
}

#[tokio::test]
async fn estimate_replacement_during_signing_is_used_without_revision_gate() {
    let mut f = fixture().await;
    let path = f.dir.path().join("state/state.sqlite");
    let wallet = f.payer.wallet.clone();
    f.pool.test_admission_hook = Some(Arc::new(move |stage| {
        let (path, wallet) = (path.clone(), wallet.clone());
        Box::pin(async move {
            if let AdmissionTestStage::Signed {
                nonce,
                valid_before,
            } = stage
            {
                assert!(!nonce.is_empty());
                assert!(valid_before > now().unwrap());
                let db = rusqlite::Connection::open(path).unwrap();
                db.execute("UPDATE wallets SET balance='50' WHERE id=?1", [wallet])
                    .unwrap();
            }
        })
    }));
    assert!(f.pay().await.unwrap().0.status().is_success());
    assert_eq!(
        f.db()
            .query_row(
                "SELECT balance FROM wallets WHERE id=?1",
                [&f.payer.wallet],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "43"
    );
    f.close().await;
}

#[tokio::test]
async fn revoked_authority_at_commit_still_prevents_disclosure() {
    let mut f = fixture().await;
    let path = f.dir.path().join("state/state.sqlite");
    f.pool.test_admission_hook = Some(Arc::new(move |stage| {
        let path = path.clone();
        Box::pin(async move {
            if let AdmissionTestStage::Signed { .. } = stage {
                rusqlite::Connection::open(path)
                    .unwrap()
                    .execute("UPDATE pools SET payment_limit='1'", [])
                    .unwrap();
            }
        })
    }));
    assert!(f.pay().await.is_err());
    assert_eq!(f.signed.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.close().await;
}

#[tokio::test]
async fn new_advisory_authorization_can_resolve_and_unknown_estimate_refreshes_to_zero() {
    let mut f = fixture().await;
    f.db()
        .execute(
            "UPDATE wallets SET balance='' WHERE id=?1",
            [&f.payer.wallet],
        )
        .unwrap();
    assert!(f.pay().await.unwrap().0.status().is_success());
    let endpoint = f.url.replace("/pay", "/rpc");
    f.pool.base = BaseRpc::new(&endpoint, 12).unwrap();
    let pool = f.pool.pool.clone();
    let query = f
        .pool
        .store
        .call(move |s| s.chain_query(&pool))
        .await
        .unwrap();
    let view = f.pool.base.view(query).await.unwrap();
    let pool = f.pool.pool.clone();
    f.pool
        .store
        .call(move |s| s.reconcile_pool(&pool, view))
        .await
        .unwrap();
    let (balance, pending, resolutions): (String,i64,i64) = f.db().query_row("SELECT balance,(SELECT count(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'),(SELECT count(*) FROM payment_resolutions) FROM wallets WHERE id=?1",[&f.payer.wallet],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!((balance.as_str(), pending, resolutions), ("0", 0, 1));
    f.close().await;
}

#[cfg(unix)]
#[path = "crash_tests.rs"]
mod crash_tests;
#[cfg(unix)]
#[path = "io_failure_tests.rs"]
mod io_failure_tests;
#[path = "ownership_tests.rs"]
mod ownership_tests;
