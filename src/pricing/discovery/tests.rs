use super::*;
use alloy_primitives::U256;
use axum::serve::Listener as _;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
    routing::any,
};
use futures_util::FutureExt;
use serde_json::json;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

fn config(source: &str) -> Config {
    Config {
        spec: "authored.json".into(),
        prefix: Some("fixture".into()),
        pricing: Some(PricingConfig {
            source: source.into(),
            format: PricingFormat::X402Discovery,
            key: None,
            unit: None,
            usd_as_usdc: false,
        }),
        ..Default::default()
    }
}
fn schema() -> Value {
    json!({"openapi":"3.0.3","paths":{"/read":{"get":{}}}})
}
fn document(url: &str, amount: u64) -> Value {
    json!({"x402Version":2,"items":[{"resource":url,"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":amount.to_string()}]}]})
}
fn tools(cfg: &Config, root: &Value) -> Vec<ToolSpec> {
    crate::catalog::build_tools(cfg, root, "fixture").unwrap()
}
async fn eventually(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn loaded(document: Value) -> crate::catalog::LoadedCatalog {
    crate::catalog::LoadedCatalog {
        document: Arc::new(document),
        hints: None,
        advertised_expires: Some(crate::http_cache::now() + 120),
    }
}

#[test]
fn inventory_preserves_raw_openapi_and_digest_identities() {
    let mut cfg = config("https://api.example.com/prices");
    cfg.operation_url_extension = Some("x-url".into());
    for root in [
        json!({"paths":{
            "/read":{"get":{"x-url":"https://other.example.com/read"},"post":{},
                "head":{},"GET":{},"parameters":[]},
            "/write":{"put":{},"patch":{},"delete":{}}
        }}),
        json!({"operations":[
            {"method":"GET","path":"/read","x-url":"https://other.example.com/read"},
            {"method":"head","path":"/read"},
            {"method":"CUSTOM","path":"/write"}
        ]}),
    ] {
        let projected = crate::catalog::operations(&root, None).unwrap();
        let identities = crate::catalog::operation_identities(&root).unwrap();
        assert_eq!(identities.len(), projected.len());
        let expected: Vec<_> = identities
            .iter()
            .zip(&projected)
            .map(|((method, path, original), operation)| {
                assert_eq!(*method, operation["method"].as_str().unwrap());
                assert_eq!(*path, operation["path"].as_str().unwrap());
                Operation {
                    method: method.to_ascii_uppercase(),
                    resource: resource(
                        "https://api.example.com",
                        original["x-url"].as_str().unwrap_or(path),
                    ),
                }
            })
            .collect();
        assert_eq!(
            inventory(&cfg, &root, "https://api.example.com").unwrap(),
            expected
        );
        assert_eq!(expected[0].resource, "https://other.example.com/read");
    }
    for root in [
        json!({}),
        json!({"paths":null}),
        json!({"paths":{"/read":[]}}),
        json!({"operations":{}}),
        json!({"operations":[null]}),
        json!({"operations":[{"method":1,"path":"/read"}]}),
        json!({"operations":[{"method":"GET","path":1}]}),
    ] {
        assert!(inventory(&cfg, &root, "https://api.example.com").is_err());
    }
}

#[test]
fn inventory_does_not_expand_request_schemas() {
    let cfg = config("https://api.example.com/prices");
    let mut definitions = serde_json::Map::new();
    for index in 0..70 {
        definitions.insert(
            index.to_string(),
            json!({"$ref":format!("#/defs/{}", index+1)}),
        );
    }
    definitions.insert("70".into(), json!({"type":"string"}));
    for root in [
        json!({"defs":definitions,"paths":{"/read":{
            "get":{"parameters":[{"in":"query","name":"q","schema":{"$ref":"#/defs/0"}}]},
            "post":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"#/defs/0"}}}}}
        }}}),
        json!({"defs":definitions,"operations":[
            {"method":"GET","path":"/read","params":[{"in":"query","name":"q","schema":{"$ref":"#/defs/0"}}]},
            {"method":"POST","path":"/read","body":{"schema":{"$ref":"#/defs/0"}}}
        ]}),
    ] {
        crate::catalog::expansion::shape(&root).unwrap();
        assert!(
            crate::catalog::operations(&root, None)
                .unwrap_err()
                .is::<crate::catalog::expansion::ComplexityLimit>()
        );
        let identities = inventory(&cfg, &root, "https://api.example.com").unwrap();
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].method, "GET");
        assert_eq!(identities[1].method, "POST");
        assert!(
            assignments(
                cfg.pricing.as_ref().unwrap(),
                &document("https://api.example.com/read", 1000),
                &identities
            )
            .unwrap()
            .is_empty(),
            "hidden or incompatible methods must still preserve ambiguity"
        );
    }
}

#[test]
fn original_inventory_prevents_hidden_twins_and_ambiguous_templates() {
    let cfg = config("https://api.example.com/pricing");
    let root = json!({"openapi":"3.0.3","paths":{"/read":{"get":{},"post":{}}}});
    assert_eq!(
        tools(&cfg, &root).len(),
        1,
        "POST preference suppresses the GET tool"
    );
    let ops = inventory(&cfg, &root, "https://api.example.com/gateway").unwrap();
    let record = document("https://api.example.com/gateway/read", 1000);
    assert!(
        assignments(cfg.pricing.as_ref().unwrap(), &record, &ops)
            .unwrap()
            .is_empty()
    );
    let mut explicit = record;
    explicit["items"][0]["method"] = json!("POST");
    let prices = assignments(cfg.pricing.as_ref().unwrap(), &explicit, &ops).unwrap();
    assert_eq!(prices.len(), 1);
    assert_eq!(ops[*prices.keys().next().unwrap()].method, "POST");
    explicit["items"][0]["method"] = json!(17);
    assert!(
        assignments(cfg.pricing.as_ref().unwrap(), &explicit, &ops)
            .unwrap()
            .is_empty()
    );
    let ambiguous = vec![
        Operation {
            method: "GET".into(),
            resource: "https://api.example.com/company/{cnpj}".into(),
        },
        Operation {
            method: "GET".into(),
            resource: "https://api.example.com/company/{id}".into(),
        },
    ];
    assert!(
        assignments(
            cfg.pricing.as_ref().unwrap(),
            &document("https://api.example.com/company/123", 1000),
            &ambiguous
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn matching_preserves_origins_base_paths_and_explicit_operation_urls() {
    let mut cfg = config("https://api.example.com/pricing");
    cfg.operation_url_extension = Some("x-url".into());
    let root = json!({"openapi":"3.0.3","paths":{"/company/{cnpj}":{"get":{"x-url":"https://other.example.com/api/company/{cnpj}"}}}});
    let ops = inventory(&cfg, &root, "https://api.example.com/gateway").unwrap();
    assert_eq!(
        assignments(
            cfg.pricing.as_ref().unwrap(),
            &document("https://other.example.com/api/company/123", 1000),
            &ops
        )
        .unwrap()
        .len(),
        1
    );
    for url in [
        "https://api.example.com/api/company/123",
        "https://other.example.com/company/123",
        "https://other.example.com/api/company/123/extra",
        "https://other.example.com/api/company/123?x=1",
    ] {
        assert!(
            assignments(cfg.pricing.as_ref().unwrap(), &document(url, 1000), &ops)
                .unwrap()
                .is_empty(),
            "{url}"
        );
    }
    assert!(template_matches(
        "https://other.example.com/api/company/{cnpj}",
        "https://other.example.com/api/company/{cnpj}"
    ));
}

#[test]
fn halowerk_resources_require_methods_and_use_base_usdc_accepts() {
    let mut cfg = config("https://api.example.com/prices");
    cfg.pricing.as_mut().unwrap().format = PricingFormat::X402Resources;
    let ops = vec![Operation {
        method: "POST".into(),
        resource: "https://api.example.com/v1/geocode".into(),
    }];
    let mut record = document("https://api.example.com/v1/geocode", 4000);
    let item = record["items"][0].clone();
    record.as_object_mut().unwrap().remove("items");
    record["resources"] = json!([item]);
    assert!(
        assignments(cfg.pricing.as_ref().unwrap(), &record, &ops)
            .unwrap()
            .is_empty()
    );
    record["resources"][0]["method"] = json!("POST");
    record["resources"][0]["price_usdc"] = json!("0.004");
    assert_eq!(
        assignments(cfg.pricing.as_ref().unwrap(), &record, &ops).unwrap()[&0].selection_amount,
        Some(U256::from(4000))
    );
    record["resources"][0]["accepts"][0]["network"] = json!("eip155:1");
    assert!(
        assignments(cfg.pricing.as_ref().unwrap(), &record, &ops).unwrap()[&0]
            .selection_amount
            .is_none()
    );
}

#[tokio::test]
async fn pending_failure_replacement_and_last_owner_drop_do_not_block_tools() {
    let cfg = config("https://api.example.com/pricing");
    let root = Arc::new(schema());
    let mut first = tools(&cfg, &root);
    let (send, receive) = tokio::sync::oneshot::channel();
    start(
        &cfg,
        &root,
        "https://api.example.com",
        &mut first,
        async move { receive.await.unwrap() },
    );
    assert!(first[0].advertised_pricing.amount().is_none());
    assert_eq!(
        first[0]
            .route("https://api.example.com", &Default::default())
            .unwrap()
            .method,
        "GET"
    );
    let mut replacement = tools(&cfg, &root);
    start(
        &cfg,
        &root,
        "https://api.example.com",
        &mut replacement,
        async { Ok(loaded(document("https://api.example.com/read", 2000))) },
    );
    eventually(|| replacement[0].advertised_pricing.amount() == Some(U256::from(2000))).await;
    send.send(Ok(loaded(document("https://api.example.com/read", 1000))))
        .unwrap();
    eventually(|| first[0].advertised_pricing.amount() == Some(U256::from(1000))).await;
    assert_eq!(
        replacement[0].advertised_pricing.amount(),
        Some(U256::from(2000))
    );

    struct Ended(Arc<Notify>);
    impl Drop for Ended {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }
    let ended = Arc::new(Notify::new());
    let finished = ended.clone();
    let began = Arc::new(Notify::new());
    let started = began.clone();
    let mut pending = tools(&cfg, &root);
    start(
        &cfg,
        &root,
        "https://api.example.com",
        &mut pending,
        async move {
            let _guard = Ended(finished);
            started.notify_one();
            std::future::pending().await
        },
    );
    began.notified().await;
    let clone = pending.clone();
    drop(pending);
    assert!(ended.notified().now_or_never().is_none());
    drop(clone);
    tokio::time::timeout(Duration::from_secs(5), ended.notified())
        .await
        .unwrap();
    let mut failed = tools(&cfg, &root);
    start(&cfg, &root, "https://api.example.com", &mut failed, async {
        Err(anyhow::anyhow!("fixture unavailable"))
    });
    tokio::task::yield_now().await;
    assert!(failed[0].advertised_pricing.amount().is_none());
    assert!(
        failed[0]
            .route("https://api.example.com", &Default::default())
            .is_ok()
    );
}

#[derive(Clone)]
struct Origin {
    calls: Arc<AtomicUsize>,
    price: Arc<Mutex<u64>>,
    release: Arc<Notify>,
    arrived: Arc<Notify>,
    delay: bool,
}
async fn fixture(delay: bool) -> (String, Origin, tokio::task::JoinHandle<()>) {
    let state = Origin {
        calls: Arc::default(),
        price: Arc::new(Mutex::new(1000)),
        release: Arc::default(),
        arrived: Arc::default(),
        delay,
    };
    let listener = crate::test_tls::listener().await;
    let base = format!("https://{}", listener.local_addr().unwrap());
    let origin = base.clone();
    let app = Router::new()
        .fallback(any(
            move |State(state): State<Origin>, headers: HeaderMap, uri: Uri| {
                let base = origin.clone();
                async move {
                    assert!(!headers.contains_key("payment-signature"));
                    assert!(!headers.contains_key("x-payment"));
                    state.calls.fetch_add(1, Ordering::SeqCst);
                    state.arrived.notify_one();
                    if state.delay {
                        state.release.notified().await;
                    }
                    match uri.path() {
                        "/error" => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                        "/requires-payment" => StatusCode::PAYMENT_REQUIRED.into_response(),
                        _ => axum::Json(document(
                            &format!("{base}/read"),
                            *state.price.lock().unwrap(),
                        ))
                        .into_response(),
                    }
                }
            },
        ))
        .with_state(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, state, task)
}

#[tokio::test]
async fn static_external_acquisition_is_optional_cached_and_refreshes_independently() {
    let (base, state, task) = fixture(true).await;
    let root = Arc::new(schema());
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&format!("{base}/prices"));
    cfg.http_cache_directory = Some(dir.path().join("cache"));
    cfg.catalog_cache_ttl_seconds = 120;
    let mut first = tools(&cfg, &root);
    attach_static(&cfg, &root, &base, &mut first, None);
    state.arrived.notified().await;
    assert!(first[0].advertised_pricing.amount().is_none());
    assert!(first[0].route(&base, &Default::default()).is_ok());
    state.release.notify_one();
    eventually(|| first[0].advertised_pricing.amount().is_some()).await;
    assert_eq!(first[0].advertised_pricing.amount(), Some(U256::from(1000)));
    let mut cached = tools(&cfg, &root);
    attach_static(&cfg, &root, &base, &mut cached, None);
    eventually(|| cached[0].advertised_pricing.amount().is_some()).await;
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    let slot = crate::http_cache::Slot::new(
        &cfg,
        &cfg.pricing.as_ref().unwrap().source,
        "catalog",
        cfg.max_spec_bytes,
    )
    .unwrap();
    let mut entry = slot.read().await.unwrap();
    entry.metadata.expires = 0;
    slot.write(Some(entry)).await;
    *state.price.lock().unwrap() = 3000;
    let mut refreshed = tools(&cfg, &root);
    attach_static(&cfg, &root, &base, &mut refreshed, None);
    state.arrived.notified().await;
    assert!(refreshed[0].advertised_pricing.amount().is_none());
    state.release.notify_one();
    eventually(|| refreshed[0].advertised_pricing.amount() == Some(U256::from(3000))).await;
    assert_eq!(first[0].advertised_pricing.amount(), Some(U256::from(1000)));
    assert_eq!(state.calls.load(Ordering::SeqCst), 2);
    task.abort();
}

#[test]
fn static_external_402_uses_configured_paid_catalog_relay() {
    if crate::test_log_capture::run_in_child(
        "pricing::discovery::tests::static_external_402_uses_configured_paid_catalog_relay",
    ) {
        return;
    }
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(static_paid_relay(false, None));
}

#[test]
fn accepted_paid_catalog_relay_finishes_after_source_drop() {
    if crate::test_log_capture::run_in_child(
        "pricing::discovery::tests::accepted_paid_catalog_relay_finishes_after_source_drop",
    ) {
        return;
    }
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(static_paid_relay(true, None));
}

#[test]
fn optional_relay_failure_preserves_tools_and_required_catalog_diagnostics() {
    if crate::test_log_capture::run_in_child(
        "pricing::discovery::tests::optional_relay_failure_preserves_tools_and_required_catalog_diagnostics",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(log.reopen().unwrap())
            .finish(),
    )
    .unwrap();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(static_paid_relay(false, Some(log.path())));
}

async fn static_paid_relay(stall: bool, failure_log: Option<&std::path::Path>) {
    // Relay targets retain the catalog reader's public-destination policy.
    // Only the fixture SOCKS server maps this public name to localhost.
    let (local_base, _, origin) = fixture(false).await;
    let local_url = reqwest::Url::parse(&local_base).unwrap();
    let address = std::net::SocketAddr::new(
        local_url.host_str().unwrap().parse().unwrap(),
        local_url.port().unwrap(),
    );
    let base = "https://api.example.com";
    let url = format!(
        "{base}/{}",
        if stall { "error" } else { "requires-payment" }
    );
    let target_body = if failure_log.is_some() {
        "PRIVATE_UPSTREAM_DETAIL".to_owned()
    } else {
        document(&format!("{base}/read"), 4000).to_string()
    };
    let body = json!({"url":url,"method":"GET","statusCode":if failure_log.is_some() {503} else {200},"headers":{},"body":target_body,"redirectChain":[],"error":null});
    let (mut relay, seen, server) =
        crate::discovery_relay::tests::fixture(body, false, stall).await;
    if stall {
        crate::discovery_relay::tests::set_timeout(&mut relay, Duration::from_millis(100));
    }
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("loopback".into(), address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    network::install_test_context(
        network::NetworkContext::new(network::NetworkPolicy {
            mode: network::Mode::Tor,
            socks_endpoint: Some(socks.address.into()),
            tor_isolation_retries: Some(0),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let mut cfg = config(&url);
    cfg.relay_fallback = Some(relay.clone());
    let root = Arc::new(schema());
    let mut selected = tools(&cfg, &root);
    attach_static(&cfg, &root, base, &mut selected, None);
    if stall {
        eventually(|| seen.signed.load(Ordering::SeqCst) == 1).await;
        drop(selected);
        tokio::time::timeout(Duration::from_secs(5), relay.drain())
            .await
            .unwrap();
        assert!(crate::discovery_relay::tests::is_idle(&relay));
    } else if let Some(path) = failure_log {
        eventually(|| {
            std::fs::read_to_string(path)
                .unwrap()
                .contains("Advertised prices could not be loaded")
        })
        .await;
        relay.drain().await;
        assert!(selected[0].advertised_pricing.amount().is_none());
        assert!(selected[0].route(base, &Default::default()).is_ok());
        let logs = std::fs::read_to_string(path).unwrap();
        assert!(
            logs.contains("INFO") && logs.contains("authorization_may_charge=true"),
            "{logs}"
        );
        assert!(
            logs.contains("a relay payment authorization may still charge"),
            "{logs}"
        );
        assert!(!logs.contains("the catalog is unavailable"), "{logs}");
        // The same failed relay remains a required-catalog failure when called
        // by that consumer. Its retained failure grants no automatic replay.
        let mut required = cfg.clone();
        required.spec = cfg.pricing.as_ref().unwrap().source.clone();
        let error = crate::catalog::load_json_discovery_bounded(&required, network::global(), None)
            .await
            .unwrap_err();
        assert!(
            crate::discovery_relay::document::SharedFailure::capture(&error, "fixture")
                .possible_payment()
        );
        let logs = std::fs::read_to_string(path).unwrap();
        assert!(
            logs.lines().any(|line| line.contains("WARN")
                && line.contains("the catalog is unavailable")
                && line.contains("possible_submission=true")),
            "{logs}"
        );
        assert!(!logs.contains("PRIVATE_UPSTREAM_DETAIL"), "{logs}");
    } else {
        eventually(|| selected[0].advertised_pricing.amount() == Some(U256::from(4000))).await;
        relay.drain().await;
    }
    assert_eq!(seen.signed.load(Ordering::SeqCst), 1);
    assert_eq!(seen.calls.load(Ordering::SeqCst), 2);
    server.abort();
    origin.abort();
}

#[test]
fn dynamic_metadata_uses_its_management_payer_and_public_destination_policy() {
    if crate::test_log_capture::run_in_child(
        "pricing::discovery::tests::dynamic_metadata_uses_its_management_payer_and_public_destination_policy",
    ) {
        return;
    }
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        use base64::{Engine,engine::general_purpose::STANDARD};
        let seen=Arc::new(Mutex::new(Vec::<String>::new()));let captured=seen.clone();
        let origins=Arc::new(AtomicUsize::new(0));let calls=origins.clone();
        let app=Router::new().fallback(any(move |headers:HeaderMap,uri:Uri| {let seen=captured.clone();let origins=calls.clone();async move {
            if uri.path()=="/prices" {origins.fetch_add(1,Ordering::SeqCst);return StatusCode::SERVICE_UNAVAILABLE.into_response();}
            assert_eq!(uri.path(),"/relay");
            if let Some(signature)=headers.get("payment-signature") {
                let payload:Value=serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
                seen.lock().unwrap().push(payload["payload"]["authorization"]["from"].as_str().unwrap().to_ascii_lowercase());
                return axum::Json(json!({"url":"https://api.example.com/prices","method":"GET","statusCode":200,"headers":{},"body":document("https://api.example.com/read",6000).to_string(),"redirectChain":[],"error":null})).into_response();
            }
            let challenge=json!({"x402Version":2,"resource":{"url":"https://spec.example.com/relay"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))],"").into_response()
        }}));
        let (address,server)=crate::test_tls::serve(app).await;
        let socks=crate::test_socks::Socks::start(BTreeMap::from([("api.example.com".into(),address),("spec.example.com".into(),address)]),crate::test_socks::Fault::None).await;
        network::install_test_context(network::NetworkContext::new(network::NetworkPolicy {mode:network::Mode::Tor,socks_endpoint:Some(socks.address.into()),tor_isolation_retries:Some(0),cover_traffic_enabled:Some(false),..Default::default()}).unwrap().with_test_root(crate::test_tls::CA));
        let payer=crate::payment::Payer::new("0000000000000000000000000000000000000000000000000000000000000007",crate::payment::SpendPolicy::dollars("0.01").unwrap()).unwrap();
        let expected=payer.address.to_ascii_lowercase();
        let relay=Arc::new(crate::discovery_relay::SearchRelay::fixture("https://spec.example.com/relay".into(),Config::default()));
        let client=crate::payment::PaidClient::new(payer).with_document_relay(Some(relay));
        let cfg=config("https://api.example.com/prices");let root=Arc::new(schema());let mut selected=tools(&cfg,&root);
        attach_dynamic(&cfg,&root,"https://api.example.com",&mut selected,client.clone());
        eventually(||selected[0].advertised_pricing.amount()==Some(U256::from(6000))).await;
        assert_eq!(&*seen.lock().unwrap(),&[expected]);assert_eq!(origins.load(Ordering::SeqCst),1);
        let private=config("https://127.0.0.1/prices");
        assert!(load_dynamic(private.clone(),private.pricing.unwrap(),client.clone()).await.is_err());
        assert_eq!(seen.lock().unwrap().len(),1);assert_eq!(origins.load(Ordering::SeqCst),1);
        client.drain().await;server.abort();
    });
}

#[test]
fn indexed_candidates_preserve_literal_placeholder_and_url_boundaries() {
    let cfg = config("https://api.example.com/prices");
    let ops = vec![
        Operation {
            method: "GET".into(),
            resource: "https://api.example.com:443/api/{id}?a=1&b=2".into(),
        },
        Operation {
            method: "POST".into(),
            resource: "https://api.example.com/api/fixed?a=1&b=2".into(),
        },
    ];
    let index = matching::Index::new(&ops);
    for url in [
        "https://api.example.com/api/{id}?a=1&b=2",
        "https://api.example.com/api/%7Bid%7D?a=1&b=2",
        "https://api.example.com/api/%31?a=1&b=2",
    ] {
        assert_eq!(index.candidates(url, Some("gEt")), vec![0]);
    }
    for url in [
        "https://api.example.com/api/{other}?a=1&b=2",
        "https://api.example.com/api/%7bid%7d?a=1&b=2",
        "https://api.example.com/api/%7Bother%7D?a=1&b=2",
        "https://api.example.com/api/?a=1&b=2",
        "https://api.example.com/api/123?b=2&a=1",
        "https://api.example.com/api/123?a=1&b=2#",
        "https://api.example.com:444/api/123?a=1&b=2",
        "https://other.example.com/api/123?a=1&b=2",
        "https://api.example.com/api/123/extra?a=1&b=2",
    ] {
        assert!(index.candidates(url, Some("GET")).is_empty(), "{url}");
    }
    assert_eq!(
        index
            .candidates("https://api.example.com/api/fixed?a=1&b=2", None)
            .len(),
        2
    );
    let duplicate = vec![ops[0].clone(), ops[0].clone()];
    assert_eq!(
        matching::Index::new(&duplicate)
            .candidates("https://api.example.com/api/123?a=1&b=2", Some("GET"))
            .len(),
        2
    );
    let mut record = document("https://api.example.com/api/123?a=1&b=2", 1000);
    let duplicate = record["items"][0].clone();
    record["items"].as_array_mut().unwrap().push(duplicate);
    assert!(
        assignments(cfg.pricing.as_ref().unwrap(), &record, &ops)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn indexed_inventory_parses_once_and_narrows_literal_and_template_families() {
    let mut ops = Vec::new();
    for family in 0..64 {
        for suffix in ["literal", "{id}"] {
            ops.push(Operation {
                method: "GET".into(),
                resource: format!(
                    "https://api.example.com/a_very_long_shared_prefix/family{family}/{suffix}"
                ),
            });
        }
    }
    let index = matching::Index::new(&ops);
    assert_eq!(index.parses.get(), ops.len());
    for family in 0..64 {
        let value = format!("https://api.example.com/a_very_long_shared_prefix/family{family}/123");
        assert_eq!(index.candidates(&value, Some("GET")), vec![family * 2 + 1]);
        let literal =
            format!("https://api.example.com/a_very_long_shared_prefix/family{family}/literal");
        assert_eq!(
            index.candidates(&literal, Some("GET")).len(),
            2,
            "literal/template overlap remains ambiguous"
        );
    }
    assert_eq!(
        index.parses.get(),
        ops.len() + 128,
        "each resource is parsed once"
    );
    assert_eq!(
        index.visits.get(),
        128,
        "each lookup checks only its anchored template family"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn absent_api_outputs_do_not_start_metadata_acquisition() {
    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let cfg = config("https://api.example.com/prices");
    let root = Arc::new(schema());
    let polled = Arc::new(AtomicUsize::new(0));
    let seen = polled.clone();
    let dropped = Arc::new(AtomicUsize::new(0));
    let ownership = Dropped(dropped.clone());
    start(
        &cfg,
        &root,
        "https://api.example.com",
        &mut [],
        async move {
            let _ownership = ownership;
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(loaded(document("https://api.example.com/read", 1000)))
        },
    );
    // An aborted spawn still retains its future until the executor runs it.
    // With no consumers, acquisition must instead be discarded synchronously.
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    tokio::task::yield_now().await;
    assert_eq!(polled.load(Ordering::SeqCst), 0);
}
