//! Event barriers exercise the actual shared RPC and retained reconciliation owner.
use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[derive(Clone)]
struct Rpc {
    address: String,
    calls: Arc<AtomicUsize>,
    blocked: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    signed: Arc<std::sync::Mutex<Vec<String>>>,
    rejects: Arc<AtomicUsize>,
}
struct Fixture {
    dir: tempfile::TempDir,
    id: String,
    group: String,
    wallet: String,
    endpoint: String,
    pool: Arc<ManagedPool>,
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    rpc: Rpc,
}
impl Fixture {
    async fn status(&self) -> crate::rotation::store::AddressStatus {
        let wallet = self.wallet.clone();
        self.store
            .call(move |s| {
                Ok(s.status()?
                    .pools
                    .remove(0)
                    .addresses
                    .into_iter()
                    .find(|a| a.id == wallet)
                    .unwrap())
            })
            .await
            .unwrap()
    }
    async fn close(self) {
        self.rpc.blocked.store(false, Ordering::SeqCst);
        self.rpc.release.notify_waiters();
        drop(self.pool);
        drop(self.store);
        self.worker.await.unwrap();
        self.server.abort();
    }
    async fn reopen(self) -> Self {
        let Self {
            dir,
            id,
            group,
            wallet,
            endpoint,
            pool,
            store,
            worker,
            server,
            rpc,
        } = self;
        drop(pool);
        drop(store);
        worker.await.unwrap();
        let state = crate::rotation::store::Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &id,
        )
        .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let pool = Arc::new(
            ManagedPool::new(
                store.clone(),
                group.clone(),
                BaseRpc::new(&format!("{endpoint}/rpc"), 12).unwrap(),
                crate::payment::SpendPolicy {
                    max_atomic: Some(U256::from(100)),
                },
            )
            .unwrap(),
        );
        Self {
            dir,
            id,
            group,
            wallet,
            endpoint,
            pool,
            store,
            worker,
            server,
            rpc,
        }
    }
}
async fn fixture(blocked: bool, fail: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let id = state.id().to_owned();
    let group = crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "advisory_maintenance",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(100)),
        0
    )
    .unwrap();
    for job in state.funding_jobs().unwrap() {
        state
            .record_credit(&job.wallet_id, "5000000", "fixture", 100)
            .unwrap();
    }
    let first = state.status().unwrap().pools.remove(0).addresses.remove(0);
    let wallet = first.id;
    state.low_balance(&group, &wallet).unwrap();
    let rpc = Rpc {
        address: first.address,
        calls: Arc::default(),
        blocked: Arc::new(AtomicBool::new(blocked)),
        fail: Arc::new(AtomicBool::new(fail)),
        started: Arc::default(),
        release: Arc::default(),
        signed: Arc::default(),
        rejects: Arc::default(),
    };
    let app=Router::new().route("/rpc",post(|State(rpc):State<Rpc>,Json(v):Json<Value>|async move {
        assert_eq!(v["method"],"eth_call");assert_eq!(v["params"][1],"latest");
        let address=format!("0x{}",&v["params"][0]["data"].as_str().unwrap()[34..]);
        let ours=address.eq_ignore_ascii_case(&rpc.address);
        let count=if ours{rpc.calls.fetch_add(1,Ordering::SeqCst)+1}else{0};
        if ours && count==1 {
            let released=rpc.release.notified();tokio::pin!(released);released.as_mut().enable();
            rpc.started.notify_one();
            if rpc.blocked.load(Ordering::SeqCst){released.await;}
        }
        if ours && rpc.fail.load(Ordering::SeqCst){return Json(json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32000,"message":"fixture unavailable"}}));}
        let balance=if ours && count>1{33}else{17};
        Json(json!({"jsonrpc":"2.0","id":v["id"],"result":format!("0x{balance:064x}")}))
    })).route("/pay",get(|State(rpc):State<Rpc>,headers:HeaderMap|async move {
        if let Some(value)=headers.get("payment-signature") {
            let v:Value=serde_json::from_slice(&STANDARD.decode(value.as_bytes()).unwrap()).unwrap();
            rpc.signed.lock().unwrap().push(v["payload"]["authorization"]["from"].as_str().unwrap().to_owned());
            if rpc.rejects.try_update(Ordering::SeqCst,Ordering::SeqCst,|remaining|remaining.checked_sub(1)).is_ok() {
                let v=json!({"success":false,"errorReason":"insufficient_funds","network":"eip155:8453","transaction":""});
                return (StatusCode::PAYMENT_REQUIRED,[("payment-response",STANDARD.encode(v.to_string()))],"{}").into_response();
            }
            return "paid".into_response();
        }
        let v=json!({"x402Version":2,"resource":{"url":"https://example.com/pay"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"7","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
        (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(v.to_string()))]).into_response()
    })).with_state(rpc.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (store, worker) = StoreHandle::spawn(state);
    let pool = Arc::new(
        ManagedPool::new(
            store.clone(),
            group.clone(),
            BaseRpc::new(&format!("{endpoint}/rpc"), 12).unwrap(),
            crate::payment::SpendPolicy {
                max_atomic: Some(U256::from(100)),
            },
        )
        .unwrap(),
    );
    Fixture {
        dir,
        id,
        group,
        wallet,
        endpoint,
        pool,
        store,
        worker,
        server,
        rpc,
    }
}

#[tokio::test]
async fn qualification_history_pages_keep_bad_rows_scoped_and_include_retired_attempts() {
    let f = fixture(false, false).await;
    let db = rusqlite::Connection::open(f.dir.path().join("state/state.sqlite")).unwrap();
    for sequence in 1..=70 {
        db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) VALUES(?1,?2,?3,0,'7','fixture','POSSIBLY_SUBMITTED',?4,?7,?5,?6)",rusqlite::params![format!("attempt-{sequence}"),f.group,f.wallet,f.rpc.address,if sequence==1 {-1}else{100},sequence,if sequence==3 {"malformed".into()}else{format!("0x{sequence:064x}")}]).unwrap();
    }
    db.execute("UPDATE wallets SET role='RETIRED' WHERE id=?1", [&f.wallet])
        .unwrap();
    let pool = f.group.clone();
    f.store
        .call(move |s| {
            let through = s.qualification_pending_boundary(&pool)?;
            let first = s.qualification_pending_page(&pool, 0, through)?;
            assert_eq!(first.len(), 64);
            assert!(first[0].1.is_err());
            assert!(first[1].1.is_ok());
            assert!(first[2].1.is_err());
            let next = s.qualification_pending_page(&pool, first.last().unwrap().0, through)?;
            assert_eq!(next.len(), 6);
            assert!(next.iter().all(|(_, auth)| auth.is_ok()));
            Ok(())
        })
        .await
        .unwrap();
    drop(db);
    f.close().await;
}
#[tokio::test]
async fn cancelled_waiter_keeps_owned_read_and_new_failure_requires_follow_up() {
    let f = fixture(true, false).await;
    let pool = f.pool.clone();
    let waiter = tokio::spawn(async move { pool.reconcile().await });
    tokio::time::timeout(Duration::from_secs(10), f.rpc.started.notified())
        .await
        .unwrap();
    f.pool.note_insufficient(f.wallet.clone()).await.unwrap();
    waiter.abort();
    let _ = waiter.await;
    assert!(
        f.pool
            .reconcile()
            .await
            .unwrap_err()
            .is::<ReconciliationInProgress>()
    );
    f.rpc.blocked.store(false, Ordering::SeqCst);
    f.rpc.release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if !f.status().await.low_balance {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let status = f.status().await;
    assert_eq!(status.confirmed_balance, "33");
    assert!(status.balance_refresh_error.is_none());
    assert!(f.rpc.calls.load(Ordering::SeqCst) >= 2);
    // Wait for the retained pass to finish before closing its Store owner.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if f.pool.reconciliation.try_lock().is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.close().await;
}
#[tokio::test]
async fn failed_refresh_preserves_estimate_and_survives_restart_then_wallet_is_reusable() {
    let f = fixture(false, true).await;
    assert!(f.pool.reconcile().await.is_err());
    let before = f.status().await;
    assert!(before.low_balance);
    assert_eq!(before.confirmed_balance, "5000000");
    assert_eq!(
        before.balance_refresh_error.as_deref(),
        Some("observation_unavailable")
    );
    let f = f.reopen().await;
    assert!(f.status().await.low_balance);
    f.rpc.fail.store(false, Ordering::SeqCst);
    f.pool.reconcile().await.unwrap();
    let after = f.status().await;
    assert!(!after.low_balance);
    assert_eq!(after.confirmed_balance, "33");
    assert!(after.balance_refresh_error.is_none());
    let client = crate::payment::PaidClient::managed(f.pool.clone());
    client
        .execute(crate::catalog::RoutedRequest {
            method: "GET".into(),
            url: format!("{}/pay", f.endpoint),
            query: Default::default(),
            body: None,
        })
        .await
        .unwrap();
    client.drain().await;
    drop(client);
    let signed = f.rpc.signed.lock().unwrap().clone();
    assert_eq!(signed.len(), 1);
    assert!(signed[0].eq_ignore_ascii_case(&f.rpc.address));
    f.close().await;
}

#[tokio::test]
async fn slow_balance_read_does_not_delay_paid_wallet_fallback() {
    let f = fixture(true, false).await;
    let pool = f.pool.clone();
    let observer = tokio::spawn(async move { pool.reconcile().await });
    tokio::time::timeout(Duration::from_secs(10), f.rpc.started.notified())
        .await
        .unwrap();
    f.rpc.rejects.store(1, Ordering::SeqCst);
    let client = crate::payment::PaidClient::managed(f.pool.clone());
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        client.execute(crate::catalog::RoutedRequest {
            method: "GET".into(),
            url: format!("{}/pay", f.endpoint),
            query: Default::default(),
            body: None,
        }),
    )
    .await;
    f.rpc.blocked.store(false, Ordering::SeqCst);
    f.rpc.release.notify_waiters();
    outcome.unwrap().unwrap();
    client.drain().await;
    drop(client);
    observer.await.unwrap().unwrap();
    let signed = f.rpc.signed.lock().unwrap().clone();
    assert_eq!(signed.len(), 2);
    assert!(!signed[0].eq_ignore_ascii_case(&signed[1]));
    f.close().await;
}

#[tokio::test]
async fn malformed_advisory_row_does_not_hide_independent_wallet_observations() {
    let f = fixture(false, false).await;
    let db = rusqlite::Connection::open(f.dir.path().join("state/state.sqlite")).unwrap();
    db.execute("UPDATE wallets SET address=x'ff' WHERE id=?1", [&f.wallet])
        .unwrap();
    assert!(
        f.pool
            .reconcile()
            .await
            .unwrap_err()
            .is::<crate::rotation::store::balances::AdvisoryIdentityUnavailable>()
    );
    let siblings: i64 = db
        .query_row(
            "SELECT count(*) FROM wallets WHERE id!=?1 AND balance='17'",
            [&f.wallet],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(siblings, 1);
    assert_eq!(f.rpc.calls.load(Ordering::SeqCst), 0);
    let (estimate, error): (String, String) = db
        .query_row(
            "SELECT balance,balance_refresh_error FROM wallets WHERE id=?1",
            [&f.wallet],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(estimate, "5000000");
    assert_eq!(error, "observation_unavailable");
    db.execute(
        "UPDATE wallets SET address=?2 WHERE id=?1",
        rusqlite::params![f.wallet, f.rpc.address],
    )
    .unwrap();
    drop(db);
    f.pool.reconcile().await.unwrap();
    assert!(!f.status().await.low_balance);
    f.close().await;
}

#[tokio::test]
async fn exited_reconciliation_task_reacquires_same_gate_and_stops_without_another_owner() {
    let f = fixture(false, false).await;
    let (entered, mut owners) = tokio::sync::mpsc::unbounded_channel();
    let first = Arc::new(AtomicBool::new(true));
    let mut pool = (*f.pool).clone();
    pool.reconciliation_test_hook = Some(Arc::new(move || {
        entered.send(tokio::task::id()).unwrap();
        assert!(!first.swap(false, Ordering::SeqCst), "fixture owner exit");
    }));
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let running = tokio::spawn(async move { pool.run_reconciliation(stopped).await });
    let original = owners.recv().await.unwrap();
    let replacement = tokio::time::timeout(Duration::from_secs(10), owners.recv())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(original, replacement);
    stop.cancel();
    running.await.unwrap().unwrap();
    assert!(owners.try_recv().is_err());
    assert!(f.pool.reconciliation.try_lock().is_ok());
    f.close().await;
}
