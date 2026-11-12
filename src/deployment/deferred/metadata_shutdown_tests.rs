//! Detached dynamic enrichment retains its existing payer through deployment drain.
use super::*;
use axum::{
    Json, Router,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn shutdown_drains_signed_dynamic_pricing_and_closes_other_payers() {
    if crate::test_log_capture::run_in_child(
        "deployment::deferred::metadata_shutdown_tests::shutdown_drains_signed_dynamic_pricing_and_closes_other_payers",
    ) {
        return;
    }
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let signatures = Arc::new(AtomicUsize::new(0));
        let challenges = Arc::new(AtomicUsize::new(0));
        let (address, origin) = crate::test_tls::serve(Router::new()
            .route("/prices", get(|| async { StatusCode::FORBIDDEN }))
            .route("/relay", post({
                let entered = entered.clone();
                let release = release.clone();
                let signatures = signatures.clone();
                let challenges = challenges.clone();
                move |headers: reqwest::header::HeaderMap, Json(body): Json<Value>| {
                    let entered = entered.clone();
                    let release = release.clone();
                    let signatures = signatures.clone();
                    let challenges = challenges.clone();
                    async move {
                        assert_eq!(body["url"], "https://api.example.com/prices");
                        if let Some(signature) = headers.get("payment-signature") {
                            let payload: Value = serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
                            assert_eq!(payload["payload"]["authorization"]["value"], "100");
                            signatures.fetch_add(1, Ordering::SeqCst);
                            entered.notify_one();
                            release.notified().await;
                            let prices = json!({"x402Version":2,"resources":[{
                                "resource":"https://api.example.com/read","method":"GET",
                                "accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"123"}]
                            }]}).to_string();
                            return Json(json!({"url":"https://api.example.com/prices","method":"GET",
                                "statusCode":200,"headers":{},"body":prices,"redirectChain":[],"error":null})).into_response();
                        }
                        challenges.fetch_add(1, Ordering::SeqCst);
                        let challenge = json!({"x402Version":2,"resource":{"url":"https://relay.example.com/relay"},
                            "accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,
                            "amount":"100","payTo":"0x0000000000000000000000000000000000000003",
                            "maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                        (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))], "").into_response()
                    }
                }
            }))).await;
        let socks = crate::test_socks::Socks::start(BTreeMap::from([
            ("api.example.com".into(), address), ("relay.example.com".into(), address),
        ]), crate::test_socks::Fault::None).await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.toml");
        let root = Arc::new(json!({"paths":{"/read":{"get":{}}}}));
        std::fs::write(directory.path().join("api.json"), root.to_string()).unwrap();
        std::fs::write(&path, format!(r#"
version=1
[network]
mode='tor'
socks_endpoint='{socks}'
tor_isolation_retries=0
cover_traffic_enabled=false
[wallets.first]
mode='static'
private_key_env='FIRST'
[wallets.second]
mode='static'
private_key_env='SECOND'
[sources.api]
spec='api.json'
base_url='https://api.example.com'
[servers.first]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
wallet='first'
sources=['api']
provider_management_remote=true
[servers.second]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
wallet='second'
sources=['api']
provider_management_remote=true
"#, socks=socks.address)).unwrap();
        let relay = Arc::new(crate::discovery_relay::SearchRelay::fixture(
            "https://relay.example.com/relay".into(), Config::default(),
        ));
        let mut deployment = Deployment::load(&path).await.unwrap();
        deployment.search_relay = Some(relay.clone());
        let running = deployment.bind(&BTreeMap::from([
            ("FIRST".into(), format!("{:064x}", 1)),
            ("SECOND".into(), format!("{:064x}", 2)),
            ("TOKEN".into(), "fixture".into()),
        ])).await.unwrap();
        assert!(running.deferred.refresh_manager().is_some());
        let payer = running.deferred.wallets["first"].clone().with_document_relay(Some(relay));
        let other = running.deferred.wallets["second"].clone();
        let cfg = Config { pricing: Some(crate::pricing::advertised::PricingConfig {
            source: "https://api.example.com/prices".into(),
            format: crate::pricing::advertised::PricingFormat::X402Resources,
            key: None, unit: Some("call".into()), usd_as_usdc: false,
        }), ..Default::default() };
        let mut tools = catalog::build_tools(&cfg, &root, "dynamic").unwrap();
        // Use the real dynamic pricing acquisition and the same clone-shared
        // payer owned by the deployment's source manager.
        crate::pricing::discovery::attach_dynamic(&cfg, &root, "https://api.example.com", &mut tools, payer.clone());
        tokio::time::timeout(Duration::from_secs(5), entered.notified()).await.unwrap();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        stop.cancel();
        let admissions_closed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if other.run_owned(async {}).await.is_err() {
                    return true;
                }
                if serving.is_finished() {
                    return false;
                }
                tokio::task::yield_now().await;
            }
        }).await;
        let draining = !serving.is_finished();
        let late = payer.run_owned(async {}).await;
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), serving).await.unwrap().unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !tools[0].advertised_pricing.acquisition_finished() { tokio::task::yield_now().await; }
        }).await.unwrap();
        origin.abort();
        let _ = origin.await;
        assert!(admissions_closed.unwrap(), "shutdown must close all payer admission");
        assert!(draining, "signed metadata response must keep serving shutdown pending");
        assert!(late.is_err(), "late metadata fallback cannot enter a closed payer");
        assert_eq!(signatures.load(Ordering::SeqCst), 1);
        assert_eq!(challenges.load(Ordering::SeqCst), 1);
        assert_eq!(tools[0].advertised_pricing.amount(), Some(alloy_primitives::U256::from(123)));
    });
}
