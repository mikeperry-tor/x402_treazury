use super::tests::{TARGET, envelope, fixture};
use super::*;
use crate::rotation::{
    base::BaseRpc,
    manager::ManagedPool,
    store::{AllocationKind, Store, StoreHandle},
};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::{sync::atomic::Ordering, time::Duration};

async fn rpc(Json(v): Json<Value>) -> Json<Value> {
    let result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = v["params"][0].as_str().unwrap();
            let h = if tag == "latest" {
                100
            } else {
                u64::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap()
            };
            json!({"number":format!("0x{h:x}"),"hash":format!("0x{h:064x}"),"timestamp":format!("0x{:x}",crate::rotation::base::now().unwrap())})
        }
        "eth_call" => json!(format!(
            "0x{:064x}",
            if v["params"][0]["data"]
                .as_str()
                .unwrap()
                .starts_with("0x70a08231")
            {
                5_000_000
            } else {
                0
            }
        )),
        other => panic!("unexpected RPC {other}"),
    };
    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
}

#[tokio::test]
async fn empty_relay_payer_waits_without_attempts_and_same_owner_resumes_after_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let snapshot = state.snapshot().unwrap();
    let reserves = serde_json::from_value(json!({"small":{"funding_amount_usdc":"5"}})).unwrap();
    let pool = state
        .ensure_capacity_group("relay", &reserves, None, 0)
        .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let rpc_task = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/", post(rpc)))
            .await
            .unwrap()
    });
    let manager = Arc::new(
        ManagedPool::new(
            store.clone(),
            pool.clone(),
            BaseRpc::new(&base, 12, 120).unwrap(),
            crate::payment::SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap(),
    );
    let (mut relay, f, task) = fixture(envelope(TARGET), false, false).await;
    Arc::get_mut(&mut relay).unwrap().client = PaidClient::managed(manager.clone());
    for _ in 0..8 {
        let error = relay
            .fetch(TARGET, 200, 4096, "max_spec_bytes")
            .await
            .err()
            .unwrap();
        assert_eq!(
            RetryDisposition::from_error(&error),
            RetryDisposition::DependencyWait
        );
        let state = relay.state.lock().await;
        assert!(!state.stopped);
        assert!(state.attempts.is_empty());
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    }
    assert!(store.call(|s| s.funding_jobs()).await.unwrap().is_empty());
    relay.state.lock().await.stopped = true;
    let error = relay
        .fetch(TARGET, 200, 4096, "max_spec_bytes")
        .await
        .err()
        .unwrap();
    assert_eq!(
        RetryDisposition::from_error(&error),
        RetryDisposition::Terminal
    );
    relay.state.lock().await.stopped = false;
    // Explicit fixture funding and canonical evidence; the readiness check itself
    // is forbidden from creating this capacity.
    let saved = pool.clone();
    store
        .call(move |s| s.allocate_capacity_kind(&saved, AllocationKind::Small, true))
        .await
        .unwrap()
        .unwrap();
    manager.reconcile().await.unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let original_role: String = db
        .query_row("SELECT role FROM wallets WHERE pool_id=?1", [&pool], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute(
        "UPDATE wallets SET role='RETAINED' WHERE pool_id=?1",
        [&pool],
    )
    .unwrap();
    assert!(relay.client.discovery_ready().await.unwrap());
    let role: String = db
        .query_row("SELECT role FROM wallets WHERE pool_id=?1", [&pool], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(role, "RETAINED", "readiness cannot promote a role");
    // Restore the complete READY claim/role binding before executing payment;
    // this isolated read-only role check must not manufacture a new claim shape.
    db.execute(
        "UPDATE wallets SET role=?2 WHERE pool_id=?1",
        rusqlite::params![pool, original_role],
    )
    .unwrap();
    relay
        .fetch(TARGET, 200, 4096, "max_spec_bytes")
        .await
        .unwrap();
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    let cap: String = db
        .query_row(
            "SELECT payment_limit FROM pools WHERE id=?1",
            [&pool],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "UPDATE pools SET payment_limit='malformed' WHERE id=?1",
        [&pool],
    )
    .unwrap();
    let error = relay
        .fetch(
            "https://example.com/other.json",
            200,
            4096,
            "max_spec_bytes",
        )
        .await
        .err()
        .unwrap();
    assert_eq!(
        RetryDisposition::from_error(&error),
        RetryDisposition::Terminal
    );
    assert_eq!(relay.state.lock().await.attempts.len(), 1);
    assert!(!relay.state.lock().await.stopped);
    assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    db.execute(
        "UPDATE pools SET payment_limit=?2 WHERE id=?1",
        rusqlite::params![pool, cap],
    )
    .unwrap();
    assert_eq!(
        relay.state.lock().await.attempts[&(relay.key.clone(), TARGET.into())].0,
        1
    );
    assert_eq!(store.call(|s| s.snapshot()).await.unwrap(), snapshot);
    assert_eq!(store.call(|s| s.funding_jobs()).await.unwrap().len(), 1);
    relay.drain().await;
    drop(relay);
    drop(manager);
    drop(store);
    worker.await.unwrap();
    rpc_task.abort();
    task.abort();
}

#[tokio::test]
async fn retry_disposition_keeps_original_attempt_bound_and_backoff() {
    let (mut relay, f, task) = fixture(envelope(TARGET), false, false).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    Arc::get_mut(&mut relay).unwrap().endpoint =
        format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    for attempt in 1..=3 {
        let error = relay
            .fetch(TARGET, 200, 4096, "max_spec_bytes")
            .await
            .err()
            .unwrap();
        assert_eq!(
            matches!(
                RetryDisposition::from_error(&error),
                RetryDisposition::Backoff(_)
            ),
            attempt < 3
        );
        let error = relay
            .fetch(TARGET, 200, 4096, "max_spec_bytes")
            .await
            .err()
            .unwrap();
        assert_eq!(
            matches!(
                RetryDisposition::from_error(&error),
                RetryDisposition::Backoff(_)
            ),
            attempt < 3
        );
        let mut state = relay.state.lock().await;
        let saved = state
            .attempts
            .get_mut(&(relay.key.clone(), TARGET.into()))
            .unwrap();
        assert_eq!(saved.0, attempt);
        saved.1 = Instant::now() - Duration::from_secs(2);
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        relay.state.lock().await.attempts[&(relay.key.clone(), TARGET.into())].0,
        3
    );
    task.abort();
}

#[tokio::test]
async fn immutable_unavailability_is_terminal_and_drain_retains_cancelled_paid_owner() {
    let (mut relay, f, task) = fixture(envelope(TARGET), false, false).await;
    Arc::get_mut(&mut relay).unwrap().client = PaidClient::unavailable(
        crate::rotation::error::AdmissionError::WalletNotReady("fixture configuration unavailable"),
    );
    let error = relay
        .fetch(TARGET, 200, 4096, "max_spec_bytes")
        .await
        .err()
        .unwrap();
    assert_eq!(
        RetryDisposition::from_error(&error),
        RetryDisposition::Terminal
    );
    assert!(relay.state.lock().await.attempts.is_empty());
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    task.abort();

    let (mut relay, f, task) = fixture(envelope(TARGET), false, true).await;
    let inner = Arc::get_mut(&mut relay).unwrap();
    inner.client = inner
        .client
        .clone()
        .with_timeout(Duration::from_millis(200));
    let accepted = relay.clone();
    let request =
        tokio::spawn(async move { accepted.fetch(TARGET, 200, 4096, "max_spec_bytes").await });
    tokio::time::timeout(Duration::from_secs(2), f.arrived.notified())
        .await
        .unwrap();
    request.abort();
    let _ = request.await;
    tokio::time::timeout(Duration::from_secs(2), relay.drain())
        .await
        .unwrap();
    assert!(relay.state.lock().await.stopped);
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    assert!(
        relay
            .fetch("https://other.example/spec", 200, 4096, "max_spec_bytes")
            .await
            .is_err()
    );
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    task.abort();
}
