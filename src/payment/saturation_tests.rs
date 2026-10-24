//! Process-isolated foreground resource ownership; balance RPC is not admission.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    manager::ManagedPool,
    store::{Store, StoreHandle},
};
use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;

#[test]
fn concurrent_groups_keep_owned_calls_through_foreground_saturation() {
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
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state/state.sqlite");
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
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let rpc = BaseRpc::new(&format!("{origin}/rpc"), 12).unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
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
        for job in state
            .funding_jobs()
            .unwrap()
            .into_iter()
            .filter(|job| job.pool_id == group)
        {
            state
                .record_credit(&job.wallet_id, "10000", "fixture", 100)
                .unwrap();
        }
        groups.push(group);
    }
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
    let mut waiters = Vec::new();
    for n in 0..128 {
        let (client, request) = (clients[n % 2].clone(), route());
        waiters.push(tokio::spawn(async move {
            let execution = async move {
                let owned_client = client.clone();
                client
                    .run_owned(async move {
                        let progress = SubmissionProgress::default();
                        let result = owned_client
                            .execute_response_observed(request, &progress)
                            .await;
                        assert!(result.is_ok(), "{result:?}");
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
    challenges.add_permits(128);
    until("all owned signed calls", || {
        signed.load(Ordering::SeqCst) == 128
    })
    .await;
    for (index, waiter) in waiters.into_iter().enumerate() {
        match waiter.await {
            Err(error) => assert!(index < 4 && error.is_cancelled()),
            Ok(result) => {
                assert!(index >= 4);
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
    let permits: Vec<_> = (0..128).map(|_| ownership::foreground().unwrap()).collect();
    let (result, logs) = crate::limits::tests::capture(async { ownership::foreground() }).await;
    assert!(result.is_err());
    assert!(
        logs.contains("WARN")
            && logs.contains("foreground_work_busy")
            && logs.contains("limit=128"),
        "{logs}"
    );
    drop(permits);
    // Both groups can accept another call after retained foreground work drains.
    for pool in &pools {
        challenges.add_permits(1);
        let client = PaidClient::managed(pool.clone());
        client.execute_response(route()).await.unwrap();
        client.drain().await;
    }
    assert_eq!(signed.load(Ordering::SeqCst), 130);
    assert_eq!(unsigned.load(Ordering::SeqCst), 130);
    let db = rusqlite::Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*), COUNT(DISTINCT nonce) FROM payment_attempts",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        (130, 130)
    );
    assert_eq!(
        db.query_row(
            "SELECT SUM(CAST(amount AS INTEGER)) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        130
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
