use super::*;
use crate::network::{HttpPolicy, IsolationId, Mode, NetworkContext, NetworkPolicy};

fn local_client() -> reqwest::Client {
    NetworkContext::new(Default::default())
        .unwrap()
        .discovery("https://api.example.com", None)
        .unwrap()
}

fn owner(name: &str) -> Owner {
    let identity = IsolationId::treasury(name);
    Owner {
        runtime: tokio::runtime::Handle::current().id(),
        scope: identity.clone(),
        identity,
        origin: "https://api.example.com".into(),
        transport: HttpPolicy::default(),
        public_only: false,
        read_timeout: std::time::Duration::from_secs(1),
    }
}
fn scope() -> Scope {
    Scope {
        listener: "fixture".into(),
        source: "fixture".into(),
    }
}

#[tokio::test]
async fn cleanup_ownership_outlives_zero_counters_and_prevents_replacement() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let engine = Engine::new(Limits {
        max_owners: 1,
        ..Default::default()
    });
    let mut config = crate::cover::tests::example_config();
    config.ranges_enabled = false;
    let config = Arc::new(config);
    let first = owner("first");
    let next = owner("next");
    let http = local_client();
    let call = engine
        .begin(first.clone(), config.clone(), scope(), http.clone())
        .unwrap();
    // A task/drain callback still owns this session after real-call counters end.
    let cleanup = call.session.clone();
    drop(call);
    assert_eq!(cleanup.episode.lock().unwrap().calls, 0);
    assert_eq!(cleanup.episode.lock().unwrap().streams, 0);
    assert!(
        engine
            .begin(next.clone(), config.clone(), scope(), http.clone())
            .err()
            .unwrap()
            .to_string()
            .contains("cover_owner_limit")
    );
    assert!(
        engine
            .begin(first.clone(), config.clone(), scope(), http.clone())
            .err()
            .unwrap()
            .to_string()
            .contains("cover_owner_draining")
    );
    let (release, wait) = tokio::sync::oneshot::channel();
    let callback_engine = engine.clone();
    let callback_owner = first.clone();
    let callback = tokio::spawn(async move {
        let cleanup = cleanup;
        wait.await.unwrap();
        callback_engine
            .registry
            .lock()
            .unwrap()
            .owners
            .get_mut(&callback_owner)
            .unwrap()
            .qualified(Err(range::Failure("cover_forbidden")));
        drop(cleanup);
    });
    assert!(
        engine
            .begin(next.clone(), config.clone(), scope(), http.clone())
            .is_err()
    );
    release.send(()).unwrap();
    callback.await.unwrap();
    let call = engine.begin(next.clone(), config, scope(), http).unwrap();
    assert!(!engine.registry.lock().unwrap().owners.contains_key(&first));
    assert!(engine.registry.lock().unwrap().owners.contains_key(&next));
    let logs = std::fs::read_to_string(log.path()).unwrap();
    let eviction = logs
        .lines()
        .find(|line| line.contains("Cover owner cache capacity reached"))
        .unwrap();
    assert!(eviction.contains("DEBUG") && eviction.contains("limit_owners=1"));
    assert!(!eviction.contains("first") && !eviction.contains("next"));
    assert!(!eviction.contains("api.example.com"));
    drop(call);
}

#[tokio::test]
async fn ordinary_eviction_forgets_negative_capability_without_resetting_process_budget() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let reads = Arc::new(AtomicUsize::new(0));
    let count = reads.clone();
    let app = axum::Router::new().fallback(move || {
        let count = count.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            axum::http::StatusCode::FORBIDDEN
        }
    });
    let (addr, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = crate::test_socks::Socks::start(
        std::collections::BTreeMap::from([("api.example.com".into(), addr)]),
        crate::test_socks::Fault::None,
    )
    .await;
    let http = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA)
    .discovery("https://api.example.com", None)
    .unwrap();
    let engine = Engine::new(Limits {
        max_owners: 1,
        max_requests_per_window: 3,
        ..Default::default()
    });
    let mut config = crate::cover::tests::example_config();
    config.padding = None;
    config.start_delay = toml::from_str("distribution='uniform'\nmin_ms=0\nmax_ms=0").unwrap();
    let config = Arc::new(config);
    for (name, expected) in [
        ("first", 1),
        ("first", 1),
        ("other", 2),
        ("first", 3),
        ("third", 3),
    ] {
        let mut call = engine
            .begin(owner(name), config.clone(), scope(), http.clone())
            .unwrap();
        call.response_headers();
        engine.wait_experiment_idle().await;
        drop(call);
        assert_eq!(
            reads.load(Ordering::SeqCst),
            expected,
            "retained failures stay cached; eviction permits ordinary qualification under the same process limits"
        );
    }
    assert_eq!(engine.registry.lock().unwrap().owners.len(), 1);
    assert_eq!(engine.metrics().range_requests, 3);
    engine.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn recency_preserves_retained_negative_and_fallback_state() {
    let engine = Engine::new(Limits {
        max_owners: 2,
        ..Default::default()
    });
    let mut config = crate::cover::tests::example_config();
    config.ranges_enabled = false;
    config.fallback_url = Some("https://api.example.com/fallback".into());
    let config = Arc::new(config);
    let http = local_client();
    for name in ["cold", "hot"] {
        drop(
            engine
                .begin(owner(name), config.clone(), scope(), http.clone())
                .unwrap(),
        );
        let mut registry = engine.registry.lock().unwrap();
        let state = registry.owners.get_mut(&owner(name)).unwrap();
        state.using_fallback = true;
        state.qualified(Err(range::Failure("cover_forbidden")));
    }
    drop(
        engine
            .begin(owner("hot"), config.clone(), scope(), http.clone())
            .unwrap(),
    );
    drop(
        engine
            .begin(owner("new"), config.clone(), scope(), http.clone())
            .unwrap(),
    );
    let mut registry = engine.registry.lock().unwrap();
    assert!(!registry.owners.contains_key(&owner("cold")));
    let state = registry.owners.get_mut(&owner("hot")).unwrap();
    assert!(matches!(
        state.capability,
        Capability::Unavailable("cover_forbidden")
    ));
    assert!(state.using_fallback);
    assert!(!state.begin_qualification());
    drop(registry);
    // Only ordinary insertion pressure, never reuse of a failed entry, evicts.
    drop(engine.begin(owner("cold"), config, scope(), http).unwrap());
    let registry = engine.registry.lock().unwrap();
    let recreated = &registry.owners[&owner("cold")];
    assert!(matches!(recreated.capability, Capability::Unknown));
    assert!(!recreated.using_fallback);
}

#[tokio::test]
async fn partial_qualification_preemption_and_shutdown_drain_keep_observed_bytes_charged() {
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for preempt in [false, true] {
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let app = axum::Router::new().fallback(move |headers: axum::http::HeaderMap| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                assert_eq!(headers["range"], "bytes=0-1023");
                assert!(!headers.contains_key("payment-signature"));
                let first = futures_util::stream::once(async {
                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"observed fragment"))
                });
                let body =
                    axum::body::Body::from_stream(first.chain(futures_util::stream::pending()));
                (
                    axum::http::StatusCode::PARTIAL_CONTENT,
                    [
                        ("content-range", "bytes 0-1023/4096"),
                        ("content-length", "1024"),
                    ],
                    body,
                )
            }
        });
        let (addr, server) =
            crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
        let socks = crate::test_socks::Socks::start(
            std::collections::BTreeMap::from([("api.example.com".into(), addr)]),
            crate::test_socks::Fault::None,
        )
        .await;
        let http = NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA)
        .discovery("https://api.example.com", None)
        .unwrap();
        let engine = Engine::new(Limits {
            max_active_streams: 1,
            max_cover_body_bytes_per_window: 1024,
            ..Default::default()
        });
        let mut config = crate::cover::tests::example_config();
        config.padding = None;
        config.qualification_range_bytes = 1024;
        config.start_delay = toml::from_str("distribution='uniform'\nmin_ms=0\nmax_ms=0").unwrap();
        let config = Arc::new(config);
        let owner = owner("partial");
        let mut call = engine
            .begin(owner.clone(), config.clone(), scope(), http.clone())
            .unwrap();
        call.response_headers();
        let observed = b"observed fragment".len() as u64;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let received = engine
                    .last_range_counter
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|n| n.load(Ordering::Relaxed));
                if received == Some(observed) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("client must observe partial body before cancellation");
        assert_eq!(engine.metrics().in_flight_ranges, 1);
        let next = if preempt {
            let next = engine
                .begin(owner.clone(), config.clone(), scope(), http.clone())
                .unwrap();
            tokio::time::timeout(Duration::from_secs(3), next.prioritize())
                .await
                .expect("API priority must not wait for remote EOF");
            Some(next)
        } else {
            None
        };
        tokio::time::timeout(Duration::from_secs(3), engine.stop_ranges())
            .await
            .expect("shutdown must drain owned range tasks without remote EOF");
        assert!(engine.tasks.is_empty());
        {
            let episode = call.session.episode.lock().unwrap();
            assert_eq!(episode.streams, 0);
            assert_eq!(episode.reserved, 0);
            assert_eq!(episode.received, observed);
            assert_eq!(episode.requests, 1);
        }
        let metrics = engine.metrics();
        assert_eq!(metrics.in_flight_ranges, 0);
        assert_eq!(metrics.range_requests, 1);
        assert_eq!(metrics.cover_body_bytes, observed);
        assert_eq!(metrics.qualified_ranges, 0);
        assert_eq!(metrics.range_outcomes.get("cancelled"), Some(&1));
        assert!(matches!(
            engine.registry.lock().unwrap().owners[&owner].capability,
            Capability::Unavailable("cover_qualification_cancelled")
        ));
        // The freed stream and unused reservation are reusable, while bytes
        // actually received remain charged for the process accounting window.
        let unused = engine
            .budget
            .reserve(Instant::now(), 1024 - observed, 0, true)
            .unwrap();
        drop(unused);
        assert!(matches!(
            engine
                .budget
                .reserve(Instant::now(), 1025 - observed, 0, true),
            Err(crate::cover::budget::Refusal::BodyLimit)
        ));
        assert!(
            engine
                .begin(owner, config, scope(), http)
                .err()
                .unwrap()
                .to_string()
                .contains("cover_shutdown")
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        drop(next);
        drop(call);
        server.abort();
    }
}

#[tokio::test]
async fn distinct_effective_scopes_isolate_sessions_and_drain_after_replacement() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let reads = Arc::new(AtomicUsize::new(0));
    let seen = reads.clone();
    let app = axum::Router::new().fallback(move |headers: axum::http::HeaderMap| {
        let seen = seen.clone();
        async move {
            let ordinal = seen.fetch_add(1, Ordering::SeqCst);
            if ordinal == 0 {
                return axum::http::StatusCode::FORBIDDEN.into_response();
            }
            let raw = headers["range"]
                .to_str()
                .unwrap()
                .strip_prefix("bytes=")
                .unwrap();
            let (a, b) = raw.split_once('-').unwrap();
            let (a, b) = (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap());
            (
                axum::http::StatusCode::PARTIAL_CONTENT,
                [
                    ("content-range", format!("bytes {a}-{b}/16384")),
                    ("content-length", format!("{}", b - a + 1)),
                    ("etag", "\"v1\"".into()),
                ],
                vec![b'a'; b - a + 1],
            )
                .into_response()
        }
    });
    use axum::response::IntoResponse;
    let (addr, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = crate::test_socks::Socks::start(
        std::collections::BTreeMap::from([("api.example.com".into(), addr)]),
        crate::test_socks::Fault::None,
    )
    .await;
    let ctx = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    let identity = IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
    let initial = ctx
        .http_participating(
            &identity,
            "https://api.example.com/v1",
            None,
            false,
            HttpPolicy::default(),
        )
        .unwrap();
    let candidate = ctx.http_candidate(&initial).unwrap();
    assert_eq!(initial.identity, candidate.identity);
    assert_ne!(initial.scope, candidate.scope);
    let owner_initial = Owner::from_transport(&initial);
    let owner_candidate = Owner::from_transport(&candidate);
    assert_ne!(owner_initial, owner_candidate);

    let engine = Engine::new(Limits::default());
    let mut config = crate::cover::tests::example_config();
    config.padding = None;
    config.start_delay = toml::from_str("distribution='uniform'\nmin_ms=0\nmax_ms=0").unwrap();
    config.request_gap = config.start_delay.clone();
    config.tail = toml::from_str("distribution='uniform'\nmin_ms=50\nmax_ms=50").unwrap();
    let config = Arc::new(config);

    // First scope fails qualification (403); retain an active call on it while promoting candidate.
    let mut call_initial = engine
        .begin(
            owner_initial.clone(),
            config.clone(),
            scope(),
            initial.client.clone(),
        )
        .unwrap();
    call_initial.response_headers();
    engine.wait_experiment_idle().await;
    assert!(matches!(
        engine.registry.lock().unwrap().owners[&owner_initial].capability,
        Capability::Unavailable("cover_forbidden")
    ));

    assert!(ctx.promote_http(&initial, &candidate));
    let mut call_candidate = engine
        .begin(
            owner_candidate.clone(),
            config.clone(),
            scope(),
            candidate.client.clone(),
        )
        .unwrap();
    assert!(
        !Arc::ptr_eq(&call_initial.session, &call_candidate.session),
        "new scope must not merge into old scope's session"
    );
    call_candidate.response_headers();
    tokio::time::timeout(Duration::from_secs(3), async {
        while engine.metrics().qualified_ranges == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("candidate scope must qualify independently of initial scope refusal");
    assert!(matches!(
        engine.registry.lock().unwrap().owners[&owner_candidate].capability,
        Capability::Available { .. }
    ));

    // Subsequent participating lookup reuses the promoted candidate scope and attaches to its session.
    let next = ctx
        .http_participating(
            &identity,
            "https://api.example.com/v1",
            None,
            false,
            HttpPolicy::default(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&next, &candidate));
    let call_next = engine
        .begin(
            Owner::from_transport(&next),
            config.clone(),
            scope(),
            next.client.clone(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&call_candidate.session, &call_next.session));

    // Old session finishes and drains after replacement without interfering with promoted scope.
    call_initial.complete();
    drop(call_initial);
    drop(call_next);
    drop(call_candidate);
    engine.wait_experiment_idle().await;
    let tokens = socks
        .records
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.password.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        tokens.len(),
        2,
        "each effective scope must use its own SOCKS isolation token"
    );
    engine.shutdown().await;
    server.abort();
}
