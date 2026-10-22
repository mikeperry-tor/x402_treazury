use super::*;
use tokio::net::TcpListener;

fn tor_context(endpoint: std::net::SocketAddr) -> NetworkContext {
    NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(endpoint),
        ..Default::default()
    })
    .unwrap()
}

#[tokio::test]
async fn direct_startup_needs_no_socks_observation() {
    let context = NetworkContext::new(NetworkPolicy::default()).unwrap();
    context.wait_for_startup_socks().await;
    assert!(!context.startup_socks.initialized());
    assert_eq!(context.http_cache_len(), 0);
}

#[tokio::test]
async fn unavailable_socks_coalesces_waiters_and_resumes_once_without_a_monitor() {
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = reserved.local_addr().unwrap();
    drop(reserved);
    let context = tor_context(endpoint);
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let waiters = async {
        tokio::join!(
            context.wait_for_startup_socks(),
            context.wait_for_startup_socks()
        );
    };
    tokio::pin!(waiters);
    // Keep both owners alive across multiple failed observations.
    assert!(
        tokio::time::timeout(RETRY + Duration::from_millis(100), &mut waiters)
            .await
            .is_err()
    );
    let text = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(text.matches("ERROR").count(), 1, "{text}");
    assert!(text.contains("tor_socks_unreachable"), "{text}");
    assert!(text.contains(&endpoint.to_string()), "{text}");
    assert!(text.contains("ConnectionRefused"), "{text}");
    assert!(!context.startup_socks.initialized());

    let listener = TcpListener::bind(endpoint).await.unwrap();
    tokio::time::timeout(RETRY * 2, &mut waiters)
        .await
        .expect("late SOCKS listener must release both startup owners");
    let (_connection, _) = listener.accept().await.unwrap();
    assert!(context.startup_socks.initialized());
    context.wait_for_startup_socks().await;
    assert!(
        tokio::time::timeout(RETRY + Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "completed startup observation must leave no monitor"
    );
    let text = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(text.matches("ERROR").count(), 1, "{text}");
    assert_eq!(text.matches("INFO").count(), 1, "{text}");
    assert!(text.contains("endpoint is reachable"), "{text}");
    assert!(
        text.contains("Tor bootstrap and destination connectivity are not yet verified"),
        "{text}"
    );
    assert!(!text.contains("<torS0X>") && !text.contains("tor-isolation"));
}

#[tokio::test]
async fn cancelled_startup_observation_can_be_requested_again() {
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = reserved.local_addr().unwrap();
    drop(reserved);
    let context = tor_context(endpoint);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), context.wait_for_startup_socks())
            .await
            .is_err()
    );
    assert!(!context.startup_socks.initialized());
    let listener = TcpListener::bind(endpoint).await.unwrap();
    assert!(
        tokio::time::timeout(RETRY + Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "dropping the startup owner must cancel its probe loop"
    );
    tokio::time::timeout(Duration::from_secs(1), context.wait_for_startup_socks())
        .await
        .expect("a later startup owner must be able to initialize the cell");
    let (_connection, _) = listener.accept().await.unwrap();
    assert!(context.startup_socks.initialized());
}

#[tokio::test]
async fn fresh_catalog_and_pricing_disk_hits_do_not_wait_for_absent_socks() {
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = reserved.local_addr().unwrap();
    drop(reserved);
    let context = tor_context(endpoint);
    let directory = tempfile::tempdir().unwrap();
    let base = "https://api.example.com";
    let cfg = crate::catalog::Config {
        spec: format!("{base}/openapi.json"),
        http_cache_directory: Some(directory.path().join("cache")),
        http_cache_direct_warm_target: Some(context.policy.clone()),
        wait_for_startup_socks: true,
        ..Default::default()
    };
    let document = serde_json::json!({
        "openapi":"3.0.0", "info":{"title":"fixture", "version":"1"},
        "paths":{"/price":{"get":{"operationId":"price", "responses":{"200":{"description":"ok"}}}}}
    });
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("cache-control", "max-age=600".parse().unwrap());
    let metadata =
        crate::http_cache::Metadata::from_headers(&headers, Duration::ZERO, None).unwrap();
    for (url, kind, limit, data) in [
        (
            cfg.spec.clone(),
            "catalog",
            cfg.max_spec_bytes,
            serde_json::to_vec(&document).unwrap(),
        ),
        (
            format!("{base}/price"),
            "pricing",
            64 * 1024,
            b"Cost: $0.001 USDC".to_vec(),
        ),
    ] {
        crate::http_cache::Slot::new(&cfg, &url, kind, limit)
            .unwrap()
            .write(Some(crate::http_cache::Entry {
                metadata: metadata.clone(),
                data,
            }))
            .await;
    }
    let loaded = tokio::time::timeout(
        Duration::from_secs(2),
        crate::catalog::load_json_discovery_with(&cfg, &context),
    )
    .await
    .expect("fresh catalog must remain available while SOCKS is absent")
    .unwrap();
    assert_eq!(loaded, document);
    let tools = crate::catalog::build_tools(&cfg, &loaded, "fixture").unwrap();
    assert_eq!(tools.len(), 1);
    let cache = crate::pricing::PricingCache::default();
    let prices = tokio::time::timeout(
        Duration::from_secs(2),
        cache.discover_observed_with(&context, &cfg, &loaded, &tools, base),
    )
    .await
    .expect("fresh pricing must remain available while SOCKS is absent")
    .unwrap();
    assert_eq!(prices.evidence.available_prices, 1);
    assert_eq!(prices.evidence.cache[&crate::pricing::CachePath::Disk], 1);
    assert!(!context.startup_socks.initialized());
    assert_eq!(context.http_cache_len(), 0);
}
