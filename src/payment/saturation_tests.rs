//! Process-isolated exercise of the actual global admission and RPC bounds.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    manager::ManagedPool,
    store::{Store, StoreHandle},
};
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::json;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

#[test]
fn concurrent_groups_recover_from_global_capacity_saturation() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "payment::saturation_tests::saturation_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn until(label: &str, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {label}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "isolated process-wide capacity; exercised by parent"]
async fn saturation_child() {
    let unsigned = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let challenges = Arc::new(Semaphore::new(0));
    let held = Arc::new(AtomicBool::new(false));
    let initializing = Arc::new(AtomicBool::new(true));
    let release = Arc::new(Notify::new());
    let active_addresses = Arc::new(Mutex::new(std::collections::BTreeSet::<String>::new()));
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state/state.sqlite");
    let block_timestamp = crate::rotation::base::now().unwrap();
    let app = Router::new().route("/pay", get({
        let (unsigned, signed, challenges, db_path) = (unsigned.clone(), signed.clone(), challenges.clone(), db_path.clone());
        move |headers: HeaderMap| {
            let (unsigned, signed, challenges, db_path) = (unsigned.clone(), signed.clone(), challenges.clone(), db_path.clone());
            async move {
                if let Some(signature) = headers.get("payment-signature") {
                    let payload: Value = serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
                    let nonce = payload["payload"]["authorization"]["nonce"].as_str().unwrap();
                    let db = rusqlite::Connection::open(db_path).unwrap();
                    assert_eq!(db.query_row("SELECT COUNT(*) FROM payment_attempts WHERE nonce=?1 AND state='POSSIBLY_SUBMITTED'", [nonce], |r| r.get::<_, i64>(0)).unwrap(), 1);
                    signed.fetch_add(1, Ordering::SeqCst);
                    return "paid".into_response();
                }
                unsigned.fetch_add(1, Ordering::SeqCst);
                challenges.acquire().await.unwrap().forget();
                let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/pay"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":600,"extra":{"name":"USD Coin","version":"2"}}]});
                (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
            }
        }
    })).route("/rpc", post({
        let (held, release, initializing, active_addresses) = (held.clone(), release.clone(), initializing.clone(), active_addresses.clone());
        move |Json(v): Json<Value>| {
            let (held, release, initializing, active_addresses) = (held.clone(), release.clone(), initializing.clone(), active_addresses.clone());
            async move {
                let notified = release.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if held.load(Ordering::SeqCst) { notified.await; }
                let result = match v["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = v["params"][0].as_str().unwrap();
                        let height = if tag == "latest" { if initializing.load(Ordering::SeqCst) {100} else {200} } else {u64::from_str_radix(&tag[2..], 16).unwrap()};
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{block_timestamp:x}")})
                    }
                    "eth_call" => {
                        let data = v["params"][0]["data"].as_str().unwrap();
                        let balance = if data.starts_with("0x70a08231") {
                            let address = format!("0x{}", &data[data.len()-40..]).to_lowercase();
                            let height = u64::from_str_radix(v["params"][1]["blockHash"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
                            if height <= 100 || active_addresses.lock().unwrap().contains(&address) {10_000} else {0}
                        } else {0};
                        json!(format!("0x{balance:064x}"))
                    }
                    method => panic!("unexpected mutation {method}"),
                };
                Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let rpc = BaseRpc::new(&format!("{origin}/rpc"), 12, 120).unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let treasury_id = state.id().to_owned();
    let mut groups = Vec::new();
    for name in ["one", "two"] {
        let group = crate::rotation::test_capacity::declare_and_allocate!(
            state,
            name,
            crate::rotation::test_capacity::two_reserves("0.01"),
            Some(U256::from(10_000)),
            0
        )
        .unwrap();
        let view = rpc.view(state.chain_query(&group).unwrap()).await.unwrap();
        state.reconcile_pool(&group, view).unwrap();
        let active = state.payment_candidate(&group).unwrap();
        active_addresses
            .lock()
            .unwrap()
            .insert(active.address.to_lowercase());
        groups.push(group);
    }
    // Preserve real immutable readiness but start with no runtime balance cache.
    drop(state);
    let state = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury_id,
    )
    .unwrap();
    initializing.store(false, Ordering::SeqCst);
    let (store, worker) = StoreHandle::spawn(state);
    let pools: Vec<_> = groups
        .iter()
        .map(|group| {
            Arc::new(
                ManagedPool::new(
                    store.clone(),
                    group.clone(),
                    rpc.clone(),
                    SpendPolicy::dollars("0.01").unwrap(),
                )
                .unwrap(),
            )
        })
        .collect();
    let clients: Vec<_> = pools
        .iter()
        .map(|pool| PaidClient::managed(pool.clone()))
        .collect();
    let route = || RoutedRequest {
        method: "GET".into(),
        url: format!("{origin}/pay"),
        query: Default::default(),
        body: None,
    };
    let failed = Arc::new(AtomicUsize::new(0));
    let mut waiters = Vec::new();
    for n in 0..128 {
        let (client, request, failed) = (clients[n % 2].clone(), route(), failed.clone());
        waiters.push(tokio::spawn(async move {
            let execution = async move {
                let owned_client = client.clone();
                client
                    .run_owned(async move {
                        let progress = SubmissionProgress::default();
                        let result = owned_client
                            .execute_response_observed(request, &progress)
                            .await;
                        if let Err(error) = &result {
                            assert_eq!(
                                error.downcast_ref::<crate::rotation::error::AdmissionError>(),
                                Some(&crate::rotation::error::AdmissionError::WalletNotReady(
                                    "candidate evidence acquisition incomplete; no payment signed"
                                ))
                            );
                            assert!(
                                !progress.possible(),
                                "capacity rejection must stay unsigned"
                            );
                            failed.fetch_add(1, Ordering::SeqCst);
                        }
                        result
                    })
                    .await
                    .unwrap()
            };
            // Exercise both direct callers and callers already retained by a
            // listener. A Work token cannot bypass resource admission.
            if n % 2 == 0 {
                crate::server::work::Work::default()
                    .run(execution)
                    .await
                    .unwrap()
            } else {
                execution.await
            }
        }));
    }
    until("all foreground owners", || {
        unsigned.load(Ordering::SeqCst) == 128
    })
    .await;
    // Cancellation of response waiters cannot free retained foreground owners.
    for waiter in waiters.iter().take(4) {
        waiter.abort();
    }
    let excess = clients[0].execute_response(route()).await.unwrap_err();
    assert!(format!("{excess:#}").contains("managed payment queue capacity unavailable"));
    assert_eq!(unsigned.load(Ordering::SeqCst), 128);
    held.store(true, Ordering::SeqCst);
    challenges.add_permits(32);
    until("Base query saturation", || {
        crate::rotation::base::available_queries() == 0
    })
    .await;
    for pool in &pools {
        let error = pool.reconcile().await.unwrap_err();
        assert!(format!("{error:#}").contains("Base query capacity unavailable"));
    }
    let stop = CancellationToken::new();
    let mut reconciliation = Vec::new();
    for pool in &pools {
        let (pool, stop) = (pool.clone(), stop.clone());
        reconciliation.push(tokio::spawn(
            async move { pool.run_reconciliation(stop).await },
        ));
    }
    // Another actual challenged call must fail unsigned when all query owners
    // are occupied, without blocking the shared store or creating a liability.
    challenges.add_permits(1);
    until("unsigned capacity rejection", || {
        failed.load(Ordering::SeqCst) == 1
    })
    .await;
    let clean = store.call(|s| s.status()).await.unwrap();
    assert!(clean.treasury_operations.is_empty());
    let db = rusqlite::Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(signed.load(Ordering::SeqCst), 0);
    held.store(false, Ordering::SeqCst);
    release.notify_waiters();
    until("initial signed calls", || {
        signed.load(Ordering::SeqCst) == 32
    })
    .await;
    // Cached independently verified capacity now serves the remaining callers.
    challenges.add_permits(95);
    until("remaining signed calls", || {
        signed.load(Ordering::SeqCst) == 127
    })
    .await;
    for (index, waiter) in waiters.into_iter().enumerate() {
        match waiter.await {
            Err(error) => assert!(index < 4 && error.is_cancelled()),
            Ok(result) => {
                assert!(index >= 4);
                // The owned workflow already checked the one unsigned rejection.
                if let Ok(output) = result {
                    assert_eq!(
                        output
                            .render(None, &Default::default())
                            .unwrap()
                            .into_text()
                            .unwrap(),
                        "paid"
                    );
                }
            }
        }
    }
    for client in &clients {
        client.drain().await;
    }
    stop.cancel();
    for task in reconciliation {
        task.await.unwrap().unwrap();
    }
    until("query owner drain", || {
        crate::rotation::base::available_queries() == 64
    })
    .await;
    assert_eq!(failed.load(Ordering::SeqCst), 1);
    let permits: Vec<_> = (0..128).map(|_| ownership::foreground().unwrap()).collect();
    assert!(ownership::foreground().is_err());
    drop(permits);
    // Both groups can reconcile and accept a new payment after saturation.
    for pool in &pools {
        pool.reconcile().await.unwrap();
        challenges.add_permits(1);
        let client = PaidClient::managed(pool.clone());
        client.execute_response(route()).await.unwrap();
        client.drain().await;
    }
    assert_eq!(signed.load(Ordering::SeqCst), 129);
    assert_eq!(unsigned.load(Ordering::SeqCst), 130);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*), COUNT(DISTINCT nonce) FROM payment_attempts",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        (129, 129)
    );
    assert_eq!(
        db.query_row(
            "SELECT SUM(CAST(amount AS INTEGER)) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        129
    );
    for group in &groups {
        assert!(
            db.query_row(
                "SELECT COUNT(*) FROM payment_attempts WHERE pool_id=?1",
                [group],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
                > 0
        );
    }
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM reserve_claims WHERE wallet_id IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    drop(db);
    drop(clients);
    drop(pools);
    drop(store);
    worker.await.unwrap();
    server.abort();
}
