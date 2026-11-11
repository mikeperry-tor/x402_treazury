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

#[tokio::test]
async fn explicit_deferred_relay_reports_demand_and_retains_its_selected_payer() {
    use crate::payment::SpendPolicy;
    use tracing::instrument::WithSubscriber;

    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let reserves = crate::rotation::test_capacity::small_reserve("2");
    let bootstrap = state
        .ensure_capacity_group("bootstrap", &reserves, None, 0)
        .unwrap();
    let destination = state
        .ensure_capacity_group("destination", &reserves, None, 0)
        .unwrap();
    let sibling = state
        .ensure_capacity_group("sibling", &reserves, None, 0)
        .unwrap();
    state
        .allocate_capacity_kind(&bootstrap, AllocationKind::Small, true)
        .unwrap();
    let job = state.funding_jobs().unwrap().remove(0);
    state
        .record_credit(&job.wallet_id, "2000000", "fixture", 100)
        .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let managed = |pool| {
        Arc::new(
            ManagedPool::new(
                store.clone(),
                pool,
                BaseRpc::new("http://127.0.0.1:1").unwrap(),
                SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        )
    };
    let client = PaidClient::managed(managed(destination.clone()))
        .with_bootstrap(managed(bootstrap.clone()));
    let sibling_client =
        PaidClient::managed(managed(sibling.clone())).with_bootstrap(managed(bootstrap));
    let (transport, calls, server) = fixture(envelope(TARGET), false, false).await;
    let policy = Policy {
        wallet: Some("destination".into()),
        relays: BTreeMap::from([(
            "GET".into(),
            vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/provider.toml")],
        )]),
        ..Default::default()
    };
    let mut relay = build(
        &policy,
        &dir.path().join("deployment.toml"),
        &[("api".into(), "destination".into())].into(),
        &[("destination".into(), client.clone())].into(),
    )
    .await
    .unwrap()
    .remove("api")
    .unwrap();
    Arc::get_mut(&mut relay).unwrap().sequence = transport.sequence.clone();
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        relay.fetch_when_ready_bounded(
            TARGET,
            4096,
            "max_spec_bytes",
            None,
            CatalogReadiness {
                activation_blocked: Some(tokio::sync::watch::channel(Some(Vec::new())).1),
                ..Default::default()
            },
            None,
        ),
    )
    .await
    .unwrap()
    .err()
    .unwrap();
    assert!(
        format!("{error:#}").contains("no endpoint can activate it while this catalog is pending")
    );
    assert_eq!(calls.calls.load(Ordering::SeqCst), 0);
    // Only a changed topology decision can revoke a paused proof. Repeated
    // publication of the same proof cannot require sibling quiet to settle.
    for revoke in [false, true] {
        let (decision, changed) = tokio::sync::watch::channel(Some(vec![relay.clone()]));
        let readiness = CatalogReadiness {
            activation_blocked: Some(changed),
            ..Default::default()
        };
        let mut reported = false;
        let mut checking = Box::pin(relay.report_dependency(&readiness, false, &mut reported));
        assert!(futures_util::poll!(&mut checking).is_pending());
        store.call(|_| Ok(())).await.unwrap();
        let (entered, waiting_for_block) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let holding = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .call(move |_| {
                        entered.send(()).unwrap();
                        held.recv().unwrap();
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
        });
        waiting_for_block.await.unwrap();
        assert!(futures_util::poll!(&mut checking).is_pending());
        publish_activation_decision(
            &decision,
            if revoke {
                None
            } else {
                Some(vec![relay.clone()])
            },
        );
        release.send(()).unwrap();
        let result = checking.await;
        assert_eq!(result.is_ok(), revoke);
        if let Err(error) = result {
            assert!(error.is::<crate::catalog::diagnostics::Problem>());
        }
        holding.await.unwrap();
        assert_eq!(calls.calls.load(Ordering::SeqCst), 0);
    }

    let mut sibling_policy = policy.clone();
    sibling_policy.wallet = Some("sibling".into());
    let sibling_relay = Relay::new(
        &sibling_policy,
        &dir.path().join("deployment.toml"),
        sibling_client.clone(),
    )
    .await
    .unwrap();
    let (_decision, changed) = tokio::sync::watch::channel(Some(vec![sibling_relay]));
    let readiness = CatalogReadiness {
        activation_blocked: Some(changed),
        ..Default::default()
    };
    let mut reported = false;
    let mut checking = Box::pin(relay.report_dependency(&readiness, false, &mut reported));
    assert!(futures_util::poll!(&mut checking).is_pending());
    store.call(|_| Ok(())).await.unwrap();
    // Its status still says waiting, but a trusted sibling call has already
    // recorded durable demand. This old proof cannot fail the target.
    sibling_client.capture_effective(true).await.unwrap();
    checking.await.unwrap();
    assert_eq!(calls.calls.load(Ordering::SeqCst), 0);
    drop((_decision, readiness, sibling_client));
    let waiting = super::tests::dependency_wait_signal(&relay).await;
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let fetch = tokio::spawn({
        let relay = relay.clone();
        async move { relay.fetch_when_ready(TARGET, 4096, "max_spec_bytes").await }
            .with_subscriber(subscriber)
    });
    tokio::time::timeout(Duration::from_secs(5), waiting.notified())
        .await
        .unwrap();
    assert!(
        std::fs::read_to_string(log.path())
            .unwrap()
            .contains("explicit relay wallet that needs first-use activation")
    );
    assert_eq!(calls.calls.load(Ordering::SeqCst), 0);
    let pool = destination.clone();
    store
        .call(move |s| {
            assert!(!s.has_bootstrap_demand(&pool)?);
            assert_eq!(s.funding_jobs()?.len(), 1);
            Ok(())
        })
        .await
        .unwrap();

    // A separate invocation supplies demand. The original waiter resumes on
    // ordinary capacity publication without substituting bootstrap authority.
    client.capture_effective(true).await.unwrap();
    let recipient = store
        .call(move |s| {
            assert!(s.has_bootstrap_demand(&destination)?);
            let job = s
                .allocate_capacity_kind(&destination, AllocationKind::Small, true)?
                .unwrap();
            let job = s.funding_job(&job)?.unwrap();
            s.record_credit(&job.wallet_id, "2000000", "fixture", 101)?;
            Ok(job.recipient.to_ascii_lowercase())
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), fetch)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(calls.signed.load(Ordering::SeqCst), 1);
    assert_eq!(calls.payers.lock().unwrap().as_slice(), [recipient]);
    relay.drain().await;
    drop((relay, client, store));
    worker.await.unwrap();
    server.abort();
}

pub(crate) async fn rpc(timestamp: u64, Json(v): Json<Value>) -> Json<Value> {
    let result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = v["params"][0].as_str().unwrap();
            let h = if matches!(tag, "latest" | "finalized") {
                100
            } else {
                u64::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap()
            };
            json!({"number":format!("0x{h:x}"),"hash":format!("0x{h:064x}"),"timestamp":format!("0x{timestamp:x}")})
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
async fn bootstrap_only_cap_refusal_waits_for_credit_without_publishing_failure() {
    use crate::payment::SpendPolicy;

    for (destination_cap, deferred, handoff) in [
        ("0.02", true, true),
        ("0.006", true, false),
        ("0.02", false, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let reserves = crate::rotation::test_capacity::small_reserve("2");
        let [bootstrap, destination] = [("bootstrap", "0.005"), ("destination", destination_cap)]
            .map(|(name, cap)| {
                let policy = SpendPolicy::dollars(cap).unwrap();
                let pool = state
                    .ensure_capacity_group(name, &reserves, policy.max_atomic, 0)
                    .unwrap();
                (pool, policy)
            });
        let job = state
            .allocate_capacity_kind(&bootstrap.0, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let bootstrap_job = state.funding_job(&job).unwrap().unwrap();
        state
            .record_credit(&bootstrap_job.wallet_id, "1000000", "fixture", 100)
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let managed = |(pool, policy): &(String, SpendPolicy)| {
            Arc::new(
                ManagedPool::new(
                    store.clone(),
                    pool.clone(),
                    BaseRpc::new("http://127.0.0.1:1").unwrap(),
                    policy.clone(),
                )
                .unwrap(),
            )
        };
        let (mut relay, calls, server) = fixture(envelope(TARGET), false, false).await;
        Arc::get_mut(&mut relay).unwrap().client =
            PaidClient::managed(managed(&destination)).with_bootstrap(managed(&bootstrap));
        if handoff {
            // A strict pricing acquisition uses the same scope and URL. Its
            // refusal remains useful to the deferred catalog aliases below.
            let error = relay
                .fetch(TARGET, 4096, "max_spec_bytes")
                .await
                .err()
                .unwrap();
            assert_eq!(RelayFailure::from_error(&error).kind(), "price_limit");
        }
        let waiting = super::tests::dependency_wait_signal(&relay).await;
        let (status, operator) = tokio::sync::watch::channel(CatalogDependency::default());
        let fetch = tokio::spawn({
            let relay = relay.clone();
            async move {
                if deferred {
                    relay
                        .fetch_when_ready_bounded(
                            TARGET,
                            4096,
                            "max_spec_bytes",
                            None,
                            CatalogReadiness {
                                status: Some(status),
                                ..Default::default()
                            },
                            None,
                        )
                        .await
                } else {
                    relay.fetch(TARGET, 4096, "max_spec_bytes").await
                }
            }
        });
        let joined = if handoff {
            tokio::time::timeout(Duration::from_secs(5), waiting.notified())
                .await
                .unwrap();
            assert!(
                operator.borrow().operator_required,
                "the unavailable destination allocation needs operator action"
            );
            {
                let state = relay.state.lock().await;
                assert_eq!(state.outcomes.len(), 1);
                assert!(state.outcomes.values().all(|outcome| {
                    outcome
                        .as_ref()
                        .err()
                        .is_some_and(|failure| failure.bootstrap_cap && !failure.possible)
                }));
                assert!(state.stopped.is_empty());
            }
            assert_eq!(calls.calls.load(Ordering::SeqCst), 1);
            assert_eq!(calls.signed.load(Ordering::SeqCst), 0);
            store
                .call(move |s| s.low_balance(&bootstrap_job.pool_id, &bootstrap_job.wallet_id))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), waiting.notified())
                .await
                .unwrap();
            assert_eq!(
                calls.calls.load(Ordering::SeqCst),
                1,
                "an unrelated wallet wake cannot repeat the refused relay POST"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(20), waiting.notified())
                    .await
                    .is_err()
            );
            let alias = tokio::spawn({
                let relay = relay.clone();
                async move { relay.fetch_when_ready(TARGET, 4096, "max_spec_bytes").await }
            });
            tokio::time::timeout(Duration::from_secs(5), waiting.notified())
                .await
                .unwrap();
            assert_eq!(
                calls.calls.load(Ordering::SeqCst),
                1,
                "catalog aliases share the refusal"
            );
            let pool = destination.0.clone();
            let recipient = store
                .call(move |s| {
                    assert!(!s.has_bootstrap_demand(&pool)?);
                    let job = s
                        .allocate_capacity_kind(&pool, AllocationKind::Small, true)?
                        .unwrap();
                    let job = s.funding_job(&job)?.unwrap();
                    s.finish_balance(
                        &pool,
                        &job.wallet_id,
                        Some(alloy_primitives::U256::from(1_000_000)),
                    )?;
                    Ok(job.recipient.to_ascii_lowercase())
                })
                .await
                .unwrap();
            Some((alias, recipient))
        } else {
            None
        };
        let result = tokio::time::timeout(Duration::from_secs(5), fetch)
            .await
            .unwrap()
            .unwrap();
        if handoff {
            let response = result.unwrap();
            let (alias, recipient) = joined.unwrap();
            let joined = tokio::time::timeout(Duration::from_secs(5), alias)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(Arc::ptr_eq(&response, &joined));
            assert_eq!(calls.payers.lock().unwrap().as_slice(), [recipient]);
            assert_eq!(calls.calls.load(Ordering::SeqCst), 3);

            // After permanent handoff, a retired destination cannot supply a
            // signer and must not hide the need for a new wallet.
            let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
            db.execute(
                "UPDATE wallets SET role='RETIRED' WHERE pool_id=?1",
                [&destination.0],
            )
            .unwrap();
            drop(db);
            let (status, operator) = tokio::sync::watch::channel(CatalogDependency::default());
            let next = tokio::spawn({
                let relay = relay.clone();
                async move {
                    relay
                        .fetch_when_ready_bounded(
                            "https://example.com/another.json",
                            4096,
                            "max_spec_bytes",
                            None,
                            CatalogReadiness {
                                status: Some(status),
                                ..Default::default()
                            },
                            None,
                        )
                        .await
                }
            });
            tokio::time::timeout(Duration::from_secs(5), waiting.notified())
                .await
                .unwrap();
            assert!(
                operator.borrow().operator_required,
                "retired wallet history cannot clear operator action"
            );
            assert_eq!(calls.calls.load(Ordering::SeqCst), 3);
            next.abort();
            let _ = next.await;
        } else {
            assert_eq!(
                RelayFailure::from_error(&result.err().unwrap()).kind(),
                "price_limit"
            );
            assert_eq!(calls.calls.load(Ordering::SeqCst), 1);
            assert_eq!(calls.signed.load(Ordering::SeqCst), 0);
        }
        relay.drain().await;
        drop((relay, store));
        worker.await.unwrap();
        server.abort();
    }
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
    let listener = crate::test_tls::listener().await;
    let base = format!("https://{}", listener.local_addr().unwrap());
    let timestamp = crate::rotation::base::now().unwrap();
    let rpc_task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/", post(move |request| rpc(timestamp, request))),
        )
        .await
        .unwrap()
    });
    let manager = Arc::new(
        ManagedPool::new(
            store.clone(),
            pool.clone(),
            BaseRpc::new(&base).unwrap(),
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
    let capacity_changes = relay.client.discovery_changes().await.unwrap().remove(0);
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
    let listener = crate::test_tls::listener().await;
    set_endpoint(
        &mut relay,
        format!("https://{}/curl", listener.local_addr().unwrap()),
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
            BaseRpc::new("http://127.0.0.1:1").unwrap(),
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
    let gate = Arc::new(crate::network::DiscoveryCapacity::new(1));
    let mut waiters = Vec::new();
    for _ in 0..140 {
        let relay = waiting.clone();
        let gate = gate.clone();
        waiters.push(tokio::spawn(async move {
            relay
                .fetch_when_ready_bounded(
                    TARGET,
                    4096,
                    "max_spec_bytes",
                    Some(gate),
                    CatalogReadiness::default(),
                    None,
                )
                .await
        }));
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        ready.fetch_when_ready_bounded(
            TARGET,
            4096,
            "max_spec_bytes",
            Some(gate),
            CatalogReadiness::default(),
            None,
        ),
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

#[tokio::test]
async fn bootstrap_relay_waiters_handoff_on_first_credit_without_demand_or_duplicate_io() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let reserves = crate::rotation::test_capacity::small_reserve("2");
    let bootstrap = state
        .ensure_capacity_group(
            "bootstrap",
            &reserves,
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0,
        )
        .unwrap();
    let destination = state
        .ensure_capacity_group(
            "destination",
            &reserves,
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0,
        )
        .unwrap();
    state
        .allocate_capacity_kind(&destination, AllocationKind::Small, true)
        .unwrap();
    let job = state.funding_jobs().unwrap().remove(0);
    let (store, worker) = StoreHandle::spawn(state);
    let manager_store = store.clone();
    let manager = move |pool| {
        Arc::new(
            ManagedPool::new(
                manager_store.clone(),
                pool,
                BaseRpc::new("http://127.0.0.1:1").unwrap(),
                crate::payment::SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        )
    };
    let participant = PaidClient::managed(manager(destination.clone()))
        .with_bootstrap(manager(bootstrap.clone()));
    let (mut implicit, implicit_calls, implicit_server) =
        fixture(envelope(TARGET), false, false).await;
    Arc::get_mut(&mut implicit).unwrap().client = participant.clone();
    let dependency_wait = super::tests::dependency_wait_signal(&implicit).await;
    let gate = Arc::new(crate::network::DiscoveryCapacity::new(1));
    let mut waiting = Vec::new();
    for _ in 0..2 {
        let relay = implicit.clone();
        let gate = gate.clone();
        waiting.push(tokio::spawn(async move {
            relay
                .fetch_when_ready_bounded(
                    TARGET,
                    4096,
                    "max_spec_bytes",
                    Some(gate),
                    CatalogReadiness::default(),
                    None,
                )
                .await
        }));
        // This existing hook runs after both route notices have been enabled and
        // the empty bootstrap has returned DependencyWait. No elapsed-time guess.
        tokio::time::timeout(Duration::from_secs(5), dependency_wait.notified())
            .await
            .unwrap();
    }
    assert_eq!(implicit_calls.calls.load(Ordering::SeqCst), 0);

    // Exercise explicit-wallet resolution and production relay construction. Only
    // the transport sequence is replaced with the existing local TLS fixture.
    let (explicit_fixture, explicit_calls, explicit_server) =
        fixture(envelope(TARGET), false, false).await;
    let config: crate::deployment::MetaConfig = serde_json::from_value(json!({
        "version":1,
        "wallets":{
            "destination":{"mode":"zcash_rotation","max_api_payment_usdc":"1"},
            "operator":{"mode":"static","private_key_env":"UNREAD"}},
        "sources":{"api":{"spec":"unused.json"}},
        "servers":{"one":{"listen":"127.0.0.1:0","wallet":"destination","sources":["api"]}}
    }))
    .unwrap();
    let policy = Policy {
        wallet: Some("operator".into()),
        relays: BTreeMap::from([(
            "GET".into(),
            vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/provider.toml")],
        )]),
        ..Default::default()
    };
    let bindings = resolve_wallets(
        &policy,
        &config,
        &crate::rotation::assignment::resolve(&config).unwrap(),
        ["api".into()],
    )
    .unwrap();
    assert_eq!(bindings["api"], "operator");
    let clients = BTreeMap::from([
        ("operator".into(), explicit_fixture.client.clone()),
        ("destination".into(), participant.clone()),
    ]);
    let mut explicit = build(
        &policy,
        &dir.path().join("deployment.toml"),
        &bindings,
        &clients,
    )
    .await
    .unwrap()
    .remove("api")
    .unwrap();
    Arc::get_mut(&mut explicit).unwrap().sequence = explicit_fixture.sequence.clone();
    explicit
        .fetch_when_ready_bounded(
            TARGET,
            4096,
            "max_spec_bytes",
            Some(gate),
            CatalogReadiness::default(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(explicit_calls.signed.load(Ordering::SeqCst), 1);
    assert_eq!(implicit_calls.calls.load(Ordering::SeqCst), 0);
    assert!(waiting.iter().all(|waiter| !waiter.is_finished()));

    // One atomic unit permanently hands off; payment amounts remain advisory
    // selection inputs, so this does not introduce a minimum-credit gate.
    store
        .call(move |state| {
            state.record_funding_balance(
                &job.id,
                &job.operation_id,
                &job.wallet_id,
                &job.recipient,
                alloy_primitives::U256::ONE,
            )
        })
        .await
        .unwrap();
    for waiter in waiting {
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    assert_eq!(implicit_calls.calls.load(Ordering::SeqCst), 2);
    assert_eq!(implicit_calls.signed.load(Ordering::SeqCst), 1);
    store
        .call(move |state| {
            assert!(!state.has_bootstrap_demand(&destination)?);
            assert!(!state.has_bootstrap_demand(&bootstrap)?);
            assert_eq!(state.funding_jobs()?.len(), 1);
            Ok(())
        })
        .await
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let journal: (i64, String) = db
        .query_row(
            "SELECT COUNT(*),p.name FROM payment_attempts a JOIN pools p ON p.id=a.pool_id GROUP BY p.name",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(journal, (1, "destination".into()));
    drop(db);
    implicit.drain().await;
    explicit.drain().await;
    drop((implicit, explicit, explicit_fixture, clients, participant));
    drop(manager);
    drop(store);
    worker.await.unwrap();
    implicit_server.abort();
    explicit_server.abort();
}
