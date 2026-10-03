use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
    routing::any,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use x402_treazury::{
    catalog::{Config, build_tools, build_tools_with_prices},
    pricing::PricingCache,
};

#[tokio::test]
async fn discovery_is_shared_one_shot_and_preserves_description_rules() {
    let counts = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    async fn handler(
        State(counts): State<Arc<Mutex<BTreeMap<String, usize>>>>,
        req: Request,
    ) -> impl IntoResponse {
        assert!(req.headers().get("payment-signature").is_none());
        assert!(req.headers().get("x-payment").is_none());
        assert_eq!(req.method(), "GET");
        let path = req.uri().path().to_owned();
        *counts.lock().unwrap().entry(path.clone()).or_default() += 1;
        tokio::time::sleep(Duration::from_millis(20)).await;
        if path == "/failed" {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("content-type", "text/plain".to_owned())],
                "",
            );
        }
        if path == "/malformed" {
            return (
                StatusCode::PAYMENT_REQUIRED,
                [("payment-required", "invalid".into())],
                "",
            );
        }
        if path == "/redirect" {
            return (
                StatusCode::FOUND,
                [("location", "/must-not-follow".into())],
                "",
            );
        }
        let challenge = if path == "/token" {
            json!({"accepts":[{"scheme":"upto","maxAmountRequired":"12345","asset":"OTHER","network":"eip155:8453"}]})
        } else {
            json!({"accepts":[{"scheme":"exact","amount":"14000","asset":"USDC","network":"eip155:8453"}]})
        };
        (
            StatusCode::PAYMENT_REQUIRED,
            [("payment-required", STANDARD.encode(challenge.to_string()))],
            "",
        )
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .fallback(any(handler))
        .with_state(counts.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = json!({"operations":[
        {"method":"GET","path":"/price","description":"Details"},
        {"method":"GET","path":"/failed"},
        {"method":"GET","path":"/malformed"},
        {"method":"GET","path":"/redirect"},
        {"method":"GET","path":"/token"},
        {"method":"GET","path":"/known","pricing":{"amount":"0.25","currency":"USD"}},
        {"method":"GET","path":"/items/{id}"},
        {"method":"POST","path":"/write"},
        {"method":"GET","path":"/excluded"}
    ]});
    let mut cfg = Config {
        exclude: vec!["/excluded".into()],
        help_url: Some(format!("{base}/help")),
        ..Default::default()
    };
    cfg.overrides = json!({"api_price":{"description":"Authored override"}});
    let tools = build_tools(&cfg, &root, "api").unwrap();
    let cache = PricingCache::default();
    let (a, b) = tokio::join!(
        cache.discover(&cfg, &root, &tools, &base),
        cache.discover(&cfg, &root, &tools, &base)
    );
    let a = a.unwrap();
    assert_eq!(a, b.unwrap());
    assert_eq!(a.len(), 2);
    assert!(a[&("GET".into(), "/price".into())].contains("$0.014"));
    assert!(a[&("GET".into(), "/token".into())].contains("12345 atomic"));
    let rendered = build_tools_with_prices(&cfg, &root, "api", &a).unwrap();
    assert_eq!(
        rendered
            .iter()
            .find(|t| t.name == "api_price")
            .unwrap()
            .description,
        "Authored override"
    );
    assert!(
        rendered
            .iter()
            .find(|t| t.path == "/known")
            .unwrap()
            .description
            .contains("$0.25")
    );
    cfg.probe_ttl_seconds = 0.000_001;
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert!(
        cache
            .discover(&cfg, &root, &tools, &base)
            .await
            .unwrap()
            .is_empty()
    );
    let snapshot = counts.lock().unwrap().clone();
    assert_eq!(snapshot.len(), 5);
    assert!(snapshot.values().all(|n| *n == 1));
    task.abort();
}

#[tokio::test]
async fn probes_are_bounded_optional_and_validate_get_only() {
    let cfg = Config {
        probe_pricing: false,
        ..Default::default()
    };
    let root = json!({"operations":[{"method":"GET","path":"/a"}]});
    let tools = build_tools(&cfg, &root, "test").unwrap();
    let cache = PricingCache::default();
    assert!(
        cache
            .discover(&cfg, &root, &tools, "http://127.0.0.1:1")
            .await
            .unwrap()
            .is_empty()
    );
    let cfg = Config {
        probe_max_endpoints: 0,
        ..Default::default()
    };
    assert!(
        cache
            .discover(&cfg, &root, &tools, "http://127.0.0.1:1")
            .await
            .unwrap()
            .is_empty()
    );
    let cfg = Config {
        probe_methods: vec!["POST".into()],
        ..Default::default()
    };
    assert!(
        cache
            .discover(&cfg, &root, &tools, "http://127.0.0.1:1")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn observed_global_and_source_bounds_caps_and_full_url_cache_keys() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (sources, bound, expected_active) in [(1, 1, 1), (1, 2, 2), (2, 3, 4)] {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let permits = Arc::new(tokio::sync::Semaphore::new(0));
        let (arrived, mut arrivals) = tokio::sync::mpsc::unbounded_channel();
        let (a, m, p) = (active.clone(), maximum.clone(), permits.clone());
        let app = Router::new().fallback(any(move |req: Request| {
            let (a, m, p, arrived) = (a.clone(), m.clone(), p.clone(), arrived.clone());
            async move {
                assert!(!req.headers().contains_key("payment-signature"));
                let n = a.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(n, Ordering::SeqCst);
                arrived.send(req.uri().path().to_owned()).unwrap();
                p.acquire().await.unwrap().forget();
                a.fetch_sub(1, Ordering::SeqCst);
                (
                    StatusCode::PAYMENT_REQUIRED,
                    [(
                        "payment-required",
                        STANDARD
                            .encode(json!({"accepts":[{"amount":"1","asset":"USDC"}]}).to_string()),
                    )],
                )
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let vendor = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let cache = Arc::new(PricingCache::default());
        let root =
            json!({"paths":{"/d":{"get":{}},"/c":{"get":{}},"/a":{"get":{}},"/b":{"get":{}}}});
        let cfg = Config {
            probe_concurrency: bound,
            probe_max_endpoints: 3,
            probe_timeout: 5.0,
            ..Default::default()
        };
        let tools = build_tools(&cfg, &root, "t").unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        // Duplicate discoverers must share the same per-URL attempt.
        for source in 0..sources {
            for _ in 0..2 {
                let (cache, cfg, root, tools, base) = (
                    cache.clone(),
                    cfg.clone(),
                    root.clone(),
                    tools.clone(),
                    format!("{base}/source{source}"),
                );
                tasks.spawn(
                    async move { cache.discover(&cfg, &root, &tools, &base).await.unwrap() },
                );
            }
        }
        let mut paths = Vec::new();
        for _ in 0..expected_active {
            paths.push(
                tokio::time::timeout(Duration::from_secs(5), arrivals.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), arrivals.recv())
                .await
                .is_err()
        );
        assert_eq!(active.load(Ordering::SeqCst), expected_active);
        permits.add_permits(100);
        while let Some(result) = tasks.join_next().await {
            assert_eq!(result.unwrap().len(), 3);
        }
        while let Ok(path) = arrivals.try_recv() {
            paths.push(path);
        }
        paths.sort();
        let mut expected: Vec<_> = (0..sources)
            .flat_map(|source| ["a", "b", "c"].map(|p| format!("/source{source}/{p}")))
            .collect();
        expected.sort();
        assert_eq!(paths, expected);
        assert_eq!(maximum.load(Ordering::SeqCst), expected_active);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        vendor.abort();
    }
}

#[tokio::test]
async fn cancelled_initialization_retries_but_completed_timeout_is_cached() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let (h, e) = (hits.clone(), entered.clone());
    let app = Router::new().fallback(any(move |request: Request| {
        let (h, e) = (h.clone(), e.clone());
        async move {
            h.fetch_add(1, Ordering::SeqCst);
            if request.uri().path() == "/legacy" {
                return (
                    StatusCode::PAYMENT_REQUIRED,
                    [
                        ("payment-required", "malformed".to_owned()),
                        (
                            "x-payment-required",
                            STANDARD.encode(
                                json!({"accepts":[{"maxAmountRequired":"7","asset":"USDC"}]})
                                    .to_string(),
                            ),
                        ),
                    ],
                )
                    .into_response();
            }
            e.notify_one();
            std::future::pending::<()>().await;
            "unreachable".into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let vendor = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let cache = Arc::new(PricingCache::default());
    let root = json!({"paths":{"/timeout":{"get":{}}}});
    let cfg = Config {
        probe_timeout: 0.1,
        ..Default::default()
    };
    let tools = build_tools(&cfg, &root, "t").unwrap();
    // Repeated interrupted initializers would exhaust the four permits if leaked.
    for _ in 0..5 {
        let (cache, cfg, root, tools, base) = (
            cache.clone(),
            Config {
                probe_timeout: 5.0,
                ..cfg.clone()
            },
            root.clone(),
            tools.clone(),
            base.clone(),
        );
        let task = tokio::spawn(async move { cache.discover(&cfg, &root, &tools, &base).await });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    assert!(
        cache
            .discover(&cfg, &root, &tools, &base)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(hits.load(Ordering::SeqCst), 6);
    assert!(
        cache
            .discover(&cfg, &root, &tools, &base)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(hits.load(Ordering::SeqCst), 6);
    let legacy = json!({"paths":{"/legacy":{"get":{}}}});
    let tools = build_tools(&cfg, &legacy, "legacy").unwrap();
    let lines = cache.discover(&cfg, &legacy, &tools, &base).await.unwrap();
    assert!(lines[&("GET".into(), "/legacy".into())].contains("$0.000007"));
    assert_eq!(hits.load(Ordering::SeqCst), 7);
    vendor.abort();
}

#[tokio::test]
async fn batches_wait_for_slowest_probe_and_cancel_without_losing_completed_cache() {
    let slow = Arc::new(tokio::sync::Semaphore::new(0));
    let (arrived, mut arrivals) = tokio::sync::mpsc::unbounded_channel();
    let gate = slow.clone();
    let app = Router::new().fallback(any(move |request: Request| {
        let (gate, arrived) = (gate.clone(), arrived.clone());
        async move {
            let path = request.uri().path().to_owned();
            arrived.send(path.clone()).unwrap();
            if path == "/b" {
                gate.acquire().await.unwrap().forget();
            }
            (
                StatusCode::PAYMENT_REQUIRED,
                [(
                    "payment-required",
                    STANDARD.encode(json!({"accepts":[{"amount":"1","asset":"USDC"}]}).to_string()),
                )],
            )
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let vendor = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let cache = Arc::new(PricingCache::default());
    let cfg = Config {
        probe_concurrency: 2,
        probe_timeout: 5.0,
        ..Default::default()
    };
    let root = json!({"paths":{"/a":{"get":{}},"/b":{"get":{}},"/c":{"get":{}}}});
    let tools = build_tools(&cfg, &root, "t").unwrap();
    let launch = || {
        let (cache, cfg, root, tools, base) = (
            cache.clone(),
            cfg.clone(),
            root.clone(),
            tools.clone(),
            base.clone(),
        );
        tokio::spawn(async move { cache.discover(&cfg, &root, &tools, &base).await.unwrap() })
    };
    let first = launch();
    let mut paths = Vec::new();
    for _ in 0..2 {
        paths.push(
            tokio::time::timeout(Duration::from_secs(5), arrivals.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    paths.sort();
    assert_eq!(paths, ["/a", "/b"]);
    // Coalescing with /a establishes that its result has completed and been cached.
    let completed = tokio::time::timeout(
        Duration::from_secs(5),
        cache.discover(&cfg, &root, &tools[..1], &base),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(completed.len(), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), arrivals.recv())
            .await
            .is_err(),
        "next batch started before the slow member completed"
    );
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let restarted = launch();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), arrivals.recv())
            .await
            .unwrap()
            .unwrap(),
        "/b",
        "completed /a must stay cached; unfinished /b must retry after cancellation"
    );
    // Release both server handlers: the cancelled client's request may still be present.
    slow.add_permits(2);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), arrivals.recv())
            .await
            .unwrap()
            .unwrap(),
        "/c"
    );
    let lines = tokio::time::timeout(Duration::from_secs(5), restarted)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[&("GET".into(), "/a".into())],
        completed[&("GET".into(), "/a".into())]
    );
    assert!(arrivals.try_recv().is_err(), "unexpected extra probe");
    vendor.abort();
}
