use super::*;
use crate::network::{HttpPolicy, IsolationId, Mode, NetworkContext, NetworkPolicy};

fn local_client() -> reqwest::Client {
    NetworkContext::new(Default::default())
        .unwrap()
        .discovery("https://api.example.com", None)
        .unwrap()
}

fn owner(name: &str) -> Owner {
    Owner {
        runtime: tokio::runtime::Handle::current().id(),
        identity: IsolationId::treasury(name),
        origin: "https://api.example.com".into(),
        transport: HttpPolicy::default(),
        public_only: false,
        timeout_ms: 1000,
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
