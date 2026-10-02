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
use x402_treazure::{
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
