//! Sleeping logical probes retain their cache owner without reserving request capacity.
use super::*;
use axum::{
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn delayed_probe_releases_single_source_slot_for_queued_endpoint() {
    let delayed = Arc::new(AtomicUsize::new(0));
    let healthy = Arc::new(AtomicUsize::new(0));
    let (entered, observed) = tokio::sync::oneshot::channel();
    let entered = Arc::new(std::sync::Mutex::new(Some(entered)));
    let app = axum::Router::new().fallback({
        let delayed = delayed.clone();
        let healthy = healthy.clone();
        move |uri: Uri, headers: HeaderMap| {
            let delayed = delayed.clone();
            let healthy = healthy.clone();
            let entered = entered.clone();
            async move {
                assert!(!headers.contains_key("payment-signature"));
                assert!(!headers.contains_key("x-payment"));
                if uri.path() == "/a_delayed" {
                    delayed.fetch_add(1, Ordering::SeqCst);
                    if let Some(entered) = entered.lock().unwrap().take() {
                        entered.send(()).unwrap();
                    }
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [("retry-after", "18446744073709551615")],
                    )
                        .into_response()
                } else {
                    healthy.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK.into_response()
                }
            }
        }
    });
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = origin.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(origin, app).await.unwrap() });
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([("pricing.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    let context = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
        mode: crate::network::Mode::Tor,
        socks_endpoint: Some(socks.address),
        ..Default::default()
    })
    .unwrap();
    let cfg = Config {
        probe_concurrency: 1,
        ..Default::default()
    };
    let root = serde_json::json!({"paths": {"/a_delayed": {"get": {}}, "/z_healthy": {"get": {}}}});
    let tools = crate::catalog::build_tools(&cfg, &root, "fixture").unwrap();
    let cache = PricingCache::default();
    let discovery =
        cache.discover_observed_with(&context, &cfg, &root, &tools, "http://pricing.example.com");
    tokio::pin!(discovery);
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = &mut discovery => panic!("delayed probe completed before cancellation"),
            _ = async {
                observed.await.unwrap();
                while healthy.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
            } => {}
        }
    })
    .await
    .unwrap();
    assert_eq!(delayed.load(Ordering::SeqCst), 1);
    assert_eq!(healthy.load(Ordering::SeqCst), 1);
    assert!(futures_util::poll!(&mut discovery).is_pending());
    // Returning drops the scoped initializer; no detached delayed attempt survives it.
    server.abort();
}

#[tokio::test]
async fn active_pricing_requests_still_obey_source_and_global_capacity() {
    for (per_source, expected) in [(2, 4), (CONCURRENCY, CONCURRENCY)] {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let arrived = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));
        let app = axum::Router::new().fallback({
            let active = active.clone();
            let peak = peak.clone();
            let arrived = arrived.clone();
            let release = release.clone();
            move || {
                let active = active.clone();
                let peak = peak.clone();
                let arrived = arrived.clone();
                let release = release.clone();
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    arrived.fetch_add(1, Ordering::SeqCst);
                    release.acquire().await.unwrap().forget();
                    active.fetch_sub(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }
        });
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = origin.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(origin, app).await.unwrap() });
        let cfg = Config {
            probe_concurrency: per_source,
            ..Default::default()
        };
        let mut roots = Vec::new();
        let mut tools = Vec::new();
        for source in ["a", "b"] {
            let operations: Vec<_> = (0..CONCURRENCY)
                .map(|n| serde_json::json!({"method":"GET", "path":format!("/{source}{n}")}))
                .collect();
            let root = serde_json::json!({"operations":operations});
            tools.push(crate::catalog::build_tools(&cfg, &root, source).unwrap());
            roots.push(root);
        }
        let context = crate::network::NetworkContext::new(Default::default()).unwrap();
        let cache = PricingCache::default();
        let base = format!("http://{address}");
        let discovery = futures_util::future::join(
            cache.discover_observed_with(&context, &cfg, &roots[0], &tools[0], &base),
            cache.discover_observed_with(&context, &cfg, &roots[1], &tools[1], &base),
        );
        tokio::pin!(discovery);
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                _ = &mut discovery => panic!("unreleased requests completed"),
                _ = async {
                    while arrived.load(Ordering::SeqCst) < expected { tokio::task::yield_now().await; }
                } => {}
            }
        }).await.unwrap();
        tokio::select! {
            _ = &mut discovery => panic!("unreleased requests completed"),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
        assert_eq!(arrived.load(Ordering::SeqCst), expected);
        assert_eq!(peak.load(Ordering::SeqCst), expected);
        release.add_permits(2 * CONCURRENCY);
        let (a, b) = tokio::time::timeout(Duration::from_secs(5), &mut discovery)
            .await
            .unwrap();
        assert_eq!(a.unwrap().evidence.observed, CONCURRENCY);
        assert_eq!(b.unwrap().evidence.observed, CONCURRENCY);
        assert!(peak.load(Ordering::SeqCst) <= expected);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        server.abort();
    }
}
