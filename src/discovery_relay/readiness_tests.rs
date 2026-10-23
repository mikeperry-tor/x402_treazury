use super::tests::{TARGET, envelope, fixture, set_endpoint, set_stopped};
use super::*;
use crate::rotation::{
    base::BaseRpc,
    manager::ManagedPool,
    store::{AllocationKind, Store, StoreHandle},
};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::{sync::atomic::Ordering, time::Duration};

pub(crate) async fn rpc(Json(v): Json<Value>) -> Json<Value> {
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
            BaseRpc::new(&base, 12).unwrap(),
            crate::payment::SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap(),
    );
    let (mut relay, f, task) = fixture(envelope(TARGET), false, false).await;
    Arc::get_mut(&mut relay).unwrap().client = PaidClient::managed(manager.clone());
    for _ in 0..8 {
        let error = relay
            .fetch(TARGET, 4096, "max_spec_bytes")
            .await
            .err()
            .unwrap();
        assert_eq!(
            RetryDisposition::from_error(&error),
            RetryDisposition::DependencyWait
        );
        let state = relay.state.lock().await;
        assert!(state.stopped.is_empty());
        assert!(state.outcomes.is_empty());
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.signed.load(Ordering::SeqCst), 0);
    }
    assert!(store.call(|s| s.funding_jobs()).await.unwrap().is_empty());
    set_stopped(&relay, true).await;
    let error = relay
        .fetch(TARGET, 4096, "max_spec_bytes")
        .await
        .err()
        .unwrap();
    assert_eq!(
        RetryDisposition::from_error(&error),
        RetryDisposition::Terminal
    );
    set_stopped(&relay, false).await;
    let mut blocked = Box::pin(relay.fetch_when_ready(TARGET, 4096, "max_spec_bytes"));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut blocked)
            .await
            .is_err()
    );
    let (mut sibling, sibling_fixture, sibling_task) = fixture(envelope(TARGET), true, false).await;
    Arc::get_mut(&mut sibling).unwrap().state = relay.state.clone();
    assert!(sibling.fetch(TARGET, 4096, "max_spec_bytes").await.is_err());
    let error = tokio::time::timeout(Duration::from_secs(5), &mut blocked)
        .await
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(
        RetryDisposition::from_error(&error),
        RetryDisposition::Terminal
    );
    assert_eq!(sibling_fixture.signed.load(Ordering::SeqCst), 1);
    drop(blocked);
    sibling.drain().await;
    sibling_task.abort();
    // Isolated breaker fixture completed. Resume the independent readiness case.
    *relay.state.lock().await = State::default();
    // Cancelling an unsigned dependency waiter owns no financial work or attempt.
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            relay.fetch_when_ready(TARGET, 4096, "max_spec_bytes")
        )
        .await
        .is_err()
    );
    {
        let state = relay.state.lock().await;
        assert!(state.outcomes.is_empty());
        assert!(state.stopped.is_empty());
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.signed.load(Ordering::SeqCst), 0);
    let mut waiting = Box::pin(relay.fetch_when_ready(TARGET, 4096, "max_spec_bytes"));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut waiting)
            .await
            .is_err()
    );
    // Explicit fixture funding and canonical evidence; the readiness check itself
    // is forbidden from creating this capacity.
    let saved = pool.clone();
    store
        .call(move |s| s.allocate_capacity_kind(&saved, AllocationKind::Small, true))
        .await
        .unwrap()
        .unwrap();
    store
        .call(|s| {
            for job in s.funding_jobs()? {
                s.record_funding_balance(
                    &job.id,
                    &job.operation_id,
                    &job.wallet_id,
                    &job.recipient,
                    alloy_primitives::U256::from(5_000_000),
                )?;
            }
            Ok(())
        })
        .await
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
    // Reuse this existing funded identity to exercise capacity disappearing
    // during a real server delay. No replacement allocation or funding is needed.
    drop(waiting);
    assert!(relay.client.discovery_ready().await.unwrap());
    let mut delayed = Box::pin(relay.fetch_with_capacity(
        TARGET,
        4096,
        "max_spec_bytes",
        Vec::new(),
        Some(crate::network::RetryAfter::after(Duration::from_secs(1))),
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut delayed)
            .await
            .is_err()
    );
    db.execute("UPDATE pools SET enabled=0 WHERE id=?1", [&pool])
        .unwrap();
    assert!(!relay.client.discovery_ready().await.unwrap());
    let error = tokio::time::timeout(Duration::from_secs(2), &mut delayed)
        .await
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(
        RetryDisposition::from_error(&error),
        RetryDisposition::DependencyWait
    );
    drop(delayed);
    {
        let state = relay.state.lock().await;
        assert!(
            state.outcomes.is_empty(),
            "lost readiness cannot publish a terminal provider failure"
        );
        assert!(state.stopped.is_empty());
    }
    assert!(super::tests::is_idle(&relay));
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.signed.load(Ordering::SeqCst), 0);

    let dependency_wait = super::tests::dependency_wait_signal(&relay).await;
    let capacity_changes = relay.client.discovery_changes().await.unwrap().unwrap();
    let waiting = tokio::spawn({
        let relay = relay.clone();
        async move { relay.fetch_when_ready(TARGET, 4096, "max_spec_bytes").await }
    });
    tokio::time::timeout(Duration::from_secs(2), dependency_wait.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), dependency_wait.notified())
            .await
            .is_err(),
        "a dependency wait cannot notify itself without a changed outcome/capacity fact"
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.signed.load(Ordering::SeqCst), 0);
    db.execute("UPDATE pools SET enabled=1 WHERE id=?1", [&pool])
        .unwrap();
    capacity_changes.notify_waiters();
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(f.calls.load(Ordering::SeqCst), 2);
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
        .fetch("https://example.com/other.json", 4096, "max_spec_bytes")
        .await
        .err()
        .unwrap();
    assert_eq!(
        RetryDisposition::from_error(&error),
        RetryDisposition::Terminal
    );
    {
        let state = relay.state.lock().await;
        assert_eq!(state.outcomes.len(), 1);
        assert!(state.outcomes.values().all(Result::is_ok));
        assert!(state.stopped.is_empty());
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    db.execute(
        "UPDATE pools SET payment_limit=?2 WHERE id=?1",
        rusqlite::params![pool, cap],
    )
    .unwrap();
    relay.fetch(TARGET, 4096, "max_spec_bytes").await.unwrap();
    assert_eq!(
        f.calls.load(Ordering::SeqCst),
        2,
        "published success must not dispatch again"
    );
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
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
async fn definitive_unsigned_failure_is_published_once_without_a_second_retry_loop() {
    let (mut relay, f, unused) = fixture(envelope(TARGET), false, false).await;
    unused.abort();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    set_endpoint(
        &mut relay,
        format!("http://{}/curl", listener.local_addr().unwrap()),
    );
    let app = Router::new().route(
        "/curl",
        post({
            let calls = calls.clone();
            move |headers: axum::http::HeaderMap| {
                let calls = calls.clone();
                async move {
                    assert!(!headers.contains_key("payment-signature"));
                    calls.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                }
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for _ in 0..3 {
        let error = relay
            .fetch(TARGET, 4096, "max_spec_bytes")
            .await
            .err()
            .unwrap();
        assert_eq!(
            RetryDisposition::from_error(&error),
            RetryDisposition::Terminal
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "published failure cannot redispatch accepted work"
        );
    }
    assert!(
        relay.state.lock().await.stopped.is_empty(),
        "definitive unsigned failure cannot poison the provider"
    );
    let error = relay
        .fetch(
            "https://example.com/independent.json",
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
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "an independent target retains its acquisition"
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.signed.load(Ordering::SeqCst), 0);
    relay.drain().await;
    server.abort();
}

#[tokio::test]
async fn immutable_unavailability_is_terminal_and_drain_retains_cancelled_paid_owner() {
    let (mut relay, f, task) = fixture(envelope(TARGET), false, false).await;
    Arc::get_mut(&mut relay).unwrap().client = PaidClient::unavailable(
        crate::rotation::error::AdmissionError::WalletNotReady("fixture configuration unavailable"),
    );
    let error = relay
        .fetch(TARGET, 4096, "max_spec_bytes")
        .await
        .err()
        .unwrap();
    assert_eq!(
        RetryDisposition::from_error(&error),
        RetryDisposition::Terminal
    );
    {
        let state = relay.state.lock().await;
        assert!(state.outcomes.is_empty());
        assert!(state.stopped.is_empty());
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.signed.load(Ordering::SeqCst), 0);
    task.abort();

    let (mut relay, f, task) = fixture(envelope(TARGET), false, true).await;
    // Relay transport inherits the provider's read-inactivity setting; an
    // unrelated payer-client timeout is not the provider configuration.
    Arc::get_mut(&mut relay)
        .unwrap()
        .sequence
        .entries
        .get_mut("GET")
        .unwrap()[0]
        .cfg
        .read_timeout_seconds = Some(0.2);
    let accepted = relay.clone();
    let request = tokio::spawn(async move {
        accepted
            .fetch_when_ready(TARGET, 4096, "max_spec_bytes")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), f.arrived.notified())
        .await
        .unwrap();
    request.abort();
    let _ = request.await;
    tokio::time::timeout(Duration::from_secs(2), relay.drain())
        .await
        .unwrap();
    assert!(!relay.state.lock().await.stopped.is_empty());
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    assert!(
        relay
            .fetch("https://other.example.com/spec", 4096, "max_spec_bytes")
            .await
            .is_err()
    );
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn bounded_deferred_waiters_leave_room_for_an_independent_ready_wallet() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let reserves = serde_json::from_value(json!({"small":{"funding_amount_usdc":"5"}})).unwrap();
    let pool = state
        .ensure_capacity_group("waiting", &reserves, None, 0)
        .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let manager = Arc::new(
        ManagedPool::new(
            store.clone(),
            pool,
            BaseRpc::new("http://127.0.0.1:1", 12).unwrap(),
            crate::payment::SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap(),
    );
    let (mut waiting, empty, empty_task) = fixture(envelope(TARGET), false, false).await;
    Arc::get_mut(&mut waiting).unwrap().client = PaidClient::managed(manager.clone());
    let (mut ready, paid, paid_task) = fixture(envelope(TARGET), false, false).await;
    let ready_inner = Arc::get_mut(&mut ready).unwrap();
    ready_inner.key = "independent".into();
    ready_inner.state = waiting.state.clone();
    let gate = Arc::new(tokio::sync::Semaphore::new(1));
    let mut waiters = Vec::new();
    for _ in 0..140 {
        let relay = waiting.clone();
        let gate = gate.clone();
        waiters.push(tokio::spawn(async move {
            relay
                .fetch_when_ready_bounded(TARGET, 4096, "max_spec_bytes", Some(gate), None, None)
                .await
        }));
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        ready.fetch_when_ready_bounded(TARGET, 4096, "max_spec_bytes", Some(gate), None, None),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(waiters.iter().all(|waiter| !waiter.is_finished()));
    assert_eq!(empty.signed.load(Ordering::SeqCst), 0);
    assert_eq!(paid.signed.load(Ordering::SeqCst), 1);
    for waiter in waiters {
        waiter.abort();
        let _ = waiter.await;
    }
    waiting.drain().await;
    ready.drain().await;
    assert!(store.call(|s| s.funding_jobs()).await.unwrap().is_empty());
    drop(waiting);
    drop(ready);
    drop(manager);
    drop(store);
    worker.await.unwrap();
    empty_task.abort();
    paid_task.abort();
}
