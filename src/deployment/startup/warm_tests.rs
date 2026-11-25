use super::*;
use crate::http_cache::{Entry, Metadata, Slot};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn warm_bypasses_fresh_origin_and_relay_catalogs() {
    if crate::test_log_capture::run_in_child(
        "deployment::startup::warm_tests::warm_bypasses_fresh_origin_and_relay_catalogs",
    ) {
        return;
    }
    let (origin_address, origin) = crate::test_tls::serve(axum::Router::new().route(
        "/spec",
        axum::routing::get(|| async { axum::http::StatusCode::FORBIDDEN }),
    ))
    .await;
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), origin_address),
            ("loopback".into(), origin_address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        crate::network::NetworkContext::new(crate::network::NetworkPolicy {
            mode: crate::network::Mode::Tor,
            socks_endpoint: Some(socks.address.into()),
            tor_isolation_retries: Some(0),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let dir = tempfile::tempdir().unwrap();
    let (relay, fixture, task) =
        crate::discovery_relay::tests::fixture(serde_json::json!({"url":"https://api.example.com/spec","method":"GET","statusCode":200,"headers":{},"body":serde_json::json!({"openapi":"3.0.3","paths":{"/read":{"get":{}}}}).to_string(),"redirectChain":[],"error":null}), false, false).await;
    let mut config: MetaConfig = toml::from_str(
        r#"
version=1
[treasury]
state_dir='unused'
[wallets.w]
mode='static'
private_key_env='UNUSED'
max_api_payment_usdc='0.01'
[sources.api]
spec='https://api.example.com/spec'
base_url='https://api.example.com'
[servers.test]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='w'
sources=['api']
"#,
    )
    .unwrap();
    config.treasury.as_mut().unwrap().state_dir = dir.path().to_owned();
    let path = dir.path().join("deployment.toml");
    let mut cfg = crate::config::resolve(config.sources["api"].provider.clone(), &path)
        .await
        .unwrap()
        .settings;
    cfg.discovery_source = Some("api".into());
    cfg.relay_fallback = Some(relay.clone());
    cfg.http_cache_directory = Some(dir.path().join("http-cache"));
    let headers = reqwest::header::HeaderMap::from_iter([(
        reqwest::header::CACHE_CONTROL,
        "max-age=600".parse().unwrap(),
    )]);
    let metadata = Metadata::from_headers(&headers, Duration::ZERO, None).unwrap();
    let catalog = Entry {
        hints: None,
        metadata: metadata.clone(),
        data:
            br#"{"operations":[{"method":"GET","path":"/read"},{"method":"GET","path":"/read"}]}"#
                .to_vec(),
    };
    let direct_catalog = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
    let relay_catalog =
        crate::discovery_relay::slot(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
    direct_catalog.write(Some(catalog.clone())).await;
    relay_catalog.write(Some(catalog.clone())).await;
    let selected = ["api".to_owned()].into_iter().collect();
    let relays = BTreeMap::from([("api".to_owned(), relay)]);
    for relay_catalog_fresh in [true, false] {
        if !relay_catalog_fresh {
            let mut stale = catalog.clone();
            stale.metadata.expires = 0;
            relay_catalog.write(Some(stale)).await;
        }
        let results = warm(&config, &path, &selected, false, relays.clone())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].catalog_cache, Some("fresh_via_relay"));
        assert_eq!(
            fixture.calls.load(Ordering::SeqCst),
            if relay_catalog_fresh { 2 } else { 4 }
        );
        assert_eq!(
            fixture.signed.load(Ordering::SeqCst),
            if relay_catalog_fresh { 1 } else { 2 }
        );
    }
    task.abort();
    origin.abort();
}

#[test]
fn default_metadata_fallback_warms_catalog_with_one_real_payment() {
    metadata_child(
        "warm",
        "default_metadata_fallback_warms_catalog_with_one_real_payment",
    );
}

#[test]
fn cancelled_warming_drains_accepted_relay_before_closing_owner() {
    metadata_child(
        "cancel",
        "cancelled_warming_drains_accepted_relay_before_closing_owner",
    );
}

#[test]
fn default_metadata_fallback_publishes_catalog_but_does_not_relay_api_data() {
    metadata_child(
        "serve",
        "default_metadata_fallback_publishes_catalog_but_does_not_relay_api_data",
    );
}

fn metadata_child(mode: &str, name: &str) {
    const CHILD: &str = "TREAZURY_DEFAULT_METADATA_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(mode) {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(metadata_fixture(mode));
        return;
    }
    let exact = format!("deployment::startup::warm_tests::{name}");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .env(CHILD, mode)
        .env("METADATA_FIXTURE_KEY", format!("{:064x}", 1))
        .env("METADATA_FIXTURE_TOKEN", "fixture")
        .args(["--exact", &exact, "--nocapture"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn metadata_fixture(mode: &str) {
    use crate::{
        deployment::Deployment,
        mcp_wire::McpResponse,
        network::{Mode, NetworkContext, NetworkPolicy},
    };
    use axum::{
        Router,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::{Value, json};
    use std::sync::atomic::AtomicUsize;

    let catalogs = Arc::new(AtomicUsize::new(0));
    let apis = Arc::new(AtomicUsize::new(0));
    let endpoint_probes = Arc::new(AtomicUsize::new(0));
    let origin = Router::new()
        .route(
            "/spec",
            get({
                let seen = catalogs.clone();
                move |headers: HeaderMap| {
                    let seen = seen.clone();
                    async move {
                        assert!(!headers.contains_key("payment-signature"));
                        seen.fetch_add(1, Ordering::SeqCst);
                        StatusCode::FORBIDDEN
                    }
                }
            }),
        )
        .route(
            "/read",
            get({
                let seen = apis.clone();
                move |headers: HeaderMap| {
                    let seen = seen.clone();
                    async move {
                        assert!(!headers.contains_key("payment-signature"));
                        seen.fetch_add(1, Ordering::SeqCst);
                        StatusCode::FORBIDDEN
                    }
                }
            }),
        )
        .route(
            "/price",
            get({
                let seen = endpoint_probes.clone();
                move || {
                    let seen = seen.clone();
                    async move {
                        seen.fetch_add(1, Ordering::SeqCst);
                        StatusCode::FORBIDDEN
                    }
                }
            }),
        );
    let (origin_address, origin_task) = crate::test_tls::serve(origin).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let accepted = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let cancelling = mode == "cancel";
    let cancellation_log = if cancelling {
        let log = tempfile::NamedTempFile::new().unwrap();
        tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::INFO)
                .with_writer(log.reopen().unwrap())
                .finish(),
        )
        .unwrap();
        Some(log)
    } else {
        None
    };
    let target = "https://api.example.com/spec";
    let document =
        json!({"operations":[{"method":"GET","path":"/read"},{"method":"GET","path":"/price"}]})
            .to_string();
    let relay = Router::new().route("/curl", post({
        let (calls, signed) = (calls.clone(), signed.clone());
        let (accepted, release) = (accepted.clone(), release.clone());
        move |headers: HeaderMap, axum::Json(request): axum::Json<Value>| {
            let (calls, signed, document) = (calls.clone(), signed.clone(), document.clone());
            let (accepted, release) = (accepted.clone(), release.clone());
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(request["url"], target);
                assert_eq!(request["method"], "GET");
                if let Some(signature) = headers.get("payment-signature") {
                    let payload: Value = serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
                    let expected: crate::payment::signer::EvmSigner = format!("{:064x}", 1).parse().unwrap();
                    assert_eq!(crate::test_signatures::recover_exact(&payload), expected.address());
                    assert_eq!(payload["payload"]["authorization"]["value"], "10000");
                    signed.fetch_add(1, Ordering::SeqCst);
                    if cancelling { accepted.notify_one(); release.notified().await; }
                    return axum::Json(json!({"url":target,"method":"GET","statusCode":200,"headers":{},"body":document,"redirectChain":[],"error":null})).into_response();
                }
                let challenge = json!({"x402Version":2,"resource":{"url":"https://spec.example.com/curl","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"10000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))], "").into_response()
            }
        }
    }));
    let (relay_address, relay_task) = crate::test_tls::serve(relay).await;
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), origin_address),
            ("spec.example.com".into(), relay_address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    let network = NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address.into()),
        tor_isolation_retries: Some(0),
        cover_traffic_enabled: Some(false),
        ..Default::default()
    };
    crate::network::install_test_context(
        NetworkContext::new(network.clone())
            .unwrap()
            .with_test_root(crate::test_tls::CA),
    );
    let dir = tempfile::tempdir().unwrap();
    // Static identity preflight reads an existing authenticated treasury store.
    // The snapshot is synthetic and never opened by the SDK or funded.
    let state = dir.path().join("state");
    let key = state.join("wallet.key");
    let existing = crate::rotation::store::Store::create(&state, &key, 1, b"fixture").unwrap();
    let treasury_id = existing.id().to_owned();
    let before = serde_json::to_value(existing.status().unwrap()).unwrap();
    let (revision, snapshot) = existing.snapshot().unwrap();
    drop(existing);
    std::fs::write(
        dir.path().join("relay.json"),
        r#"{"operations":[{"method":"POST","path":"/curl"}]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("relay.toml"),
        "spec='relay.json'\nbase_url='https://spec.example.com'\nrelay_format='curl'\n",
    )
    .unwrap();
    let path = dir.path().join("deployment.toml");
    let config = format!(
        r#"
version=1
[network]
mode='tor'
socks_endpoint='{}'
tor_isolation_retries=0
cover_traffic_enabled=false
[relay_fallback]
relays.GET=['relay.toml']
[treasury]
state_dir='{}'
[wallets.w]
mode='static'
private_key_env='METADATA_FIXTURE_KEY'
max_api_payment_usdc='0.01'
[sources.api]
spec='{target}'
base_url='https://api.example.com'
[servers.one]
listen='127.0.0.1:0'
bearer_token_env='METADATA_FIXTURE_TOKEN'
wallet='w'
sources=['api']
"#,
        socks.address,
        state.display()
    );
    std::fs::write(&path, &config).unwrap();
    let env = std::env::vars().collect();
    // Strict inspection remains unsigned despite default metadata permission.
    assert!(Deployment::load(&path).await.is_err());
    assert_eq!(catalogs.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(signed.load(Ordering::SeqCst), 0);
    if cancelling {
        let warming_path = path.clone();
        let caller = tokio::spawn(async move {
            super::super::cache_warm::warm_cache(&warming_path, &[], false).await
        });
        accepted.notified().await;
        caller.abort();
        assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
        let log = cancellation_log.as_ref().unwrap();
        let started = "Catalog warming stopped; waiting for accepted relay work to finish";
        let finished = "Catalog warming stopped; accepted relay work has finished";
        tokio::time::timeout(Duration::from_secs(5), async {
            while !std::fs::read_to_string(log.path())
                .unwrap()
                .contains(started)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("supervisor must start draining after caller cancellation");
        assert!(
            !std::fs::read_to_string(log.path())
                .unwrap()
                .contains(finished),
            "accepted paid response must keep the supervisor drain pending"
        );
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !std::fs::read_to_string(log.path())
                .unwrap()
                .contains(finished)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("supervisor must finish draining after the accepted response completes");
        assert!(
            !std::fs::read_to_string(log.path())
                .unwrap()
                .contains("Catalog warming finished sources="),
            "cancelled warming must not announce completed warming"
        );
        assert_eq!(catalogs.load(Ordering::SeqCst), 2);
    } else if mode == "warm" {
        // Disabling metadata cannot purchase a catalog through sibling defaults.
        std::fs::write(
            &path,
            config.replace(
                "relays.GET=['relay.toml']",
                "relays.GET=['relay.toml']\nprovider_metadata=false",
            ),
        )
        .unwrap();
        let unavailable = super::super::cache_warm::warm_cache(&path, &[], false)
            .await
            .unwrap();
        assert_eq!(unavailable.outcome, "degraded");
        assert_eq!(unavailable.sources[0].status, "unavailable");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        std::fs::write(&path, &config).unwrap();
        let summary = super::super::cache_warm::warm_cache(&path, &[], false)
            .await
            .unwrap();
        assert_eq!(summary.sources.len(), 1);
        assert_eq!(summary.sources[0].catalog_cache, Some("fresh_via_relay"));
        assert_eq!(catalogs.load(Ordering::SeqCst), 3);
    } else {
        let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
            .await
            .unwrap();
        let running = deployment.bind(&env).await.unwrap();
        let address = running.addresses()[0].1;
        let stop = tokio_util::sync::CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        let url = format!("http://{address}/mcp");
        let local = NetworkContext::new(Default::default()).unwrap();
        let http = local
            .http(&crate::network::IsolationId::bootstrap(), &url, None)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let inventory: Value = http
                    .post(&url)
                    .bearer_auth("fixture")
                    .header("accept", "application/json, text/event-stream")
                    .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
                    .send()
                    .await
                    .unwrap()
                    .mcp_json()
                    .await
                    .unwrap();
                if inventory["result"]["tools"]
                    .as_array()
                    .is_some_and(|ts| ts.iter().any(|t| t["name"] == "api_read"))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(apis.load(Ordering::SeqCst), 0);
        assert_eq!(endpoint_probes.load(Ordering::SeqCst), 0);
        let output: Value = http.post(&url).bearer_auth("fixture").header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"api_read","arguments":{}}}))
            .send().await.unwrap().mcp_json().await.unwrap();
        assert_eq!(output["result"]["isError"], true);
        assert_eq!(apis.load(Ordering::SeqCst), 1);
        stop.cancel();
        serving.await.unwrap().unwrap();
        assert_eq!(catalogs.load(Ordering::SeqCst), 2);
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one unsigned challenge and one signed relay exchange"
    );
    assert_eq!(signed.load(Ordering::SeqCst), 1);
    assert_eq!(endpoint_probes.load(Ordering::SeqCst), 0);
    let reopened = crate::rotation::store::Store::open(&state, &key, &treasury_id).unwrap();
    let after = reopened.status().unwrap();
    assert!(
        after.pools.is_empty(),
        "metadata work must not allocate wallets"
    );
    assert!(after.funding_jobs.is_empty());
    assert!(after.treasury_operations.is_empty());
    assert!(!after.outgoing_pending);
    assert_eq!(serde_json::to_value(after).unwrap(), before);
    let (after_revision, after_snapshot) = reopened.snapshot().unwrap();
    assert_eq!(after_revision, revision);
    assert!(after_snapshot.as_slice() == snapshot.as_slice());
    drop(reopened);
    origin_task.abort();
    relay_task.abort();
}

#[tokio::test]
async fn warm_generation_failure_retains_complete_previous_snapshot() {
    use axum::serve::Listener as _;
    use axum::{Router, routing::get};
    let dir = tempfile::tempdir().unwrap();
    let listener = crate::test_tls::listener().await;
    let base = format!("https://{}", listener.local_addr().unwrap());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_calls = calls.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/spec",
                get(move || {
                    let calls = calls.clone();
                    async move {
                        let document = match calls.fetch_add(1, Ordering::SeqCst) {
                            0 => serde_json::json!({"invalid":"replacement"}),
                            1 => serde_json::json!({"paths":{}}),
                            _ => {
                            serde_json::json!({"openapi":"3.0.3","paths":{
                                "/unsupported":{"post":{"parameters":[{"name":"session","in":"cookie","schema":{"type":"string"}}]}}
                            }})
                            }
                        };
                        axum::Json(document)
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let config: MetaConfig = toml::from_str(&format!(
        r#"
version=1
[treasury]
state_dir='{}'
[wallets.w]
mode='static'
private_key_env='UNUSED'
max_api_payment_usdc='0.01'
[sources.api]
spec='{base}/spec'
base_url='{base}'
[sources.api.pricing]
source='{base}/never-prices'
format='x402-discovery'
[servers.test]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='w'
sources=['api']
"#,
        dir.path().display()
    ))
    .unwrap();
    let path = dir.path().join("deployment.toml");
    let downloads = Downloads {
        cache_directory: Some(dir.path().join("http-cache")),
        ..Default::default()
    };
    let cfg = downloads
        .configure("api", &config.sources["api"], &path, false)
        .await
        .unwrap();
    let slot = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
    let previous = br#"{"operations":[{"name":"read","method":"GET","path":"/read"}]}"#.to_vec();
    // The existing snapshot is a complete generation accepted under this configuration.
    assert!(
        Source::from_document(
            "api",
            cfg.clone(),
            Arc::new(catalog::parse_document(&previous).unwrap())
        )
        .is_ok()
    );
    let hints = slot
        .write(Some(Entry {
            hints: None,
            metadata: Metadata::from_catalog_headers(
                &Default::default(),
                Duration::ZERO,
                None,
                cfg.catalog_cache_ttl_seconds,
            )
            .unwrap(),
            data: previous.clone(),
        }))
        .await
        .unwrap();
    let operation = hints.operation("GET", "/read");
    let price = alloy_primitives::U256::from(99);
    operation.learn(price, crate::pricing::runtime::received().unwrap());
    slot.persist_price(&hints.generation, &operation.operation, price, 1)
        .await;
    let selected = ["api".to_owned()].into_iter().collect();
    for attempt in 0..3 {
        let result = warm(&config, &path, &selected, false, Default::default())
            .await
            .unwrap();
        assert_eq!(result[0].status, "unavailable", "attempt {attempt}");
        if attempt == 1 {
            assert_eq!(result[0].pricing.as_ref().unwrap().status, "not_attempted");
        }
        if attempt == 2 {
            assert!(!result[0].omissions.is_empty());
            assert!(result[0].failure.is_none());
        }
        let retained = slot.read().await.unwrap();
        assert_eq!(retained.data, previous);
        assert_eq!(
            retained.hints.as_ref().unwrap().generation,
            hints.generation
        );
        assert_eq!(operation.get(), Some(price));
        assert_eq!(
            retained.hints.unwrap().operation("GET", "/read").get(),
            Some(price)
        );
    }
    assert_eq!(observed_calls.load(Ordering::SeqCst), 3);
    server.abort();
}

#[tokio::test]
async fn warm_refreshes_catalog_and_external_prices_unconditionally_while_serving_reuses_both() {
    use axum::serve::Listener as _;
    use axum::{Json, Router, response::IntoResponse, routing::get};
    use serde_json::json;
    let directory = tempfile::tempdir().unwrap();
    let listener = crate::test_tls::listener().await;
    let base = format!("https://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let handler_calls = calls.clone();
    let handler_base = base.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(get(move |request: axum::extract::Request| {
            let calls = handler_calls.clone();
            let base = handler_base.clone();
            async move {
                assert!(!request.headers().contains_key("if-none-match"));
                assert!(!request.headers().contains_key("if-modified-since"));
                assert!(!request.headers().contains_key("payment-signature"));
                assert!(!request.headers().contains_key("x-payment"));
                assert!(!request.headers().contains_key("authorization"));
                let path = request.uri().path();
                *calls.lock().unwrap().entry(path.to_owned()).or_default() += 1;
                let document = match path {
                    "/spec" => json!({"paths":{"/read":{"get":{"operationId":"read"}}}}),
                    "/prices" => json!({"x402Version":2,"items":[{
                        "resource":format!("{base}/read"),"method":"GET",
                        "accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000"}]
                    }]}),
                    other => panic!("unexpected request: {other}"),
                };
                ([("etag", "\"unchanged\"")], Json(document)).into_response()
            }
        }))).await.unwrap();
    });
    let config: MetaConfig = toml::from_str(&format!(
        r#"
version=1
[treasury]
state_dir='{}'
[wallets.w]
mode='static'
private_key_env='UNUSED'
[sources.api]
spec='{base}/spec'
base_url='{base}'
[sources.api.pricing]
source='{base}/prices'
format='x402-discovery'
[servers.test]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='w'
sources=['api']
"#,
        directory.path().display()
    ))
    .unwrap();
    let path = directory.path().join("deployment.toml");
    let selected = ["api".to_owned()].into_iter().collect();
    for expected in 1..=2 {
        warm(&config, &path, &selected, false, Default::default())
            .await
            .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            BTreeMap::from([
                ("/spec".to_owned(), expected),
                ("/prices".to_owned(), expected),
            ])
        );
    }
    let downloads = Downloads {
        purpose: CatalogPurpose::Serving,
        cache_directory: Some(directory.path().join("http-cache")),
        ..Default::default()
    };
    let cfg = downloads
        .configure("api", &config.sources["api"], &path, false)
        .await;
    assert!(!cfg.as_ref().unwrap().catalog_cache_force_refresh);
    let SourceLoad::Ready(source) = load_one("api", &config.sources["api"], cfg, &downloads)
        .await
        .unwrap()
    else {
        panic!("cached source must be immediately ready");
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while source.tools[0].advertised_pricing.amount()
            != Some(alloy_primitives::U256::from(1000))
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        BTreeMap::from([("/spec".to_owned(), 2), ("/prices".to_owned(), 2),])
    );
    server.abort();
}

fn warming_config(state: &Path, sources: &str) -> MetaConfig {
    toml::from_str(&format!(
        r#"
version=1
[treasury]
state_dir='{}'
[wallets.w]
mode='static'
private_key_env='UNUSED'
{sources}
[servers.test]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='w'
sources=[]
"#,
        state.display()
    ))
    .unwrap()
}

#[tokio::test]
async fn warming_reports_remote_depletion_through_exclusions_without_replacing_cache() {
    let document = Arc::new(std::sync::RwLock::new(serde_json::json!({"paths":{
        "/keep":{"get":{}}, "/removed":{"get":{}}
    }})));
    let served = document.clone();
    let (address, server) = crate::test_tls::serve(axum::Router::new().route(
        "/spec",
        axum::routing::get(move || {
            let value = served.read().unwrap().clone();
            async move { axum::Json(value) }
        }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    let config = warming_config(
        dir.path(),
        &format!(
            "[sources.api]\nspec='https://{address}/spec'\nbase_url='https://{address}'\nexclude_tools=['api_keep']\n"
        ),
    );
    let selected = ["api".to_owned()].into_iter().collect();
    let first = warm(&config, &path, &selected, false, Default::default())
        .await
        .unwrap();
    assert_eq!(first[0].status, "ready");
    let downloads = Downloads {
        purpose: CatalogPurpose::Warming,
        cache_directory: Some(dir.path().join("http-cache")),
        ..Default::default()
    };
    let cfg = downloads
        .configure("api", &config.sources["api"], &path, false)
        .await
        .unwrap();
    let slot = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
    let accepted = slot.read().await.unwrap();
    *document.write().unwrap() = serde_json::json!({"paths":{"/keep":{"get":{}}}});
    let results = warm(&config, &path, &selected, false, Default::default())
        .await
        .unwrap();
    assert_eq!(results[0].status, "unavailable");
    assert!(results[0].degraded());
    assert!(results[0].failure.is_none());
    assert_eq!(
        results[0].selection_notices,
        [catalog::SelectionNotice {
            setting: "tool_filters",
            selector: "*".into()
        }]
    );
    let retained = slot.read().await.unwrap();
    assert_eq!(retained.data, accepted.data);
    assert_eq!(
        retained.hints.unwrap().generation,
        accepted.hints.as_ref().unwrap().generation
    );
    let mut strict = cfg;
    strict.allow_empty_selection = false;
    assert!(catalog::build_catalog(&strict, &document.read().unwrap(), "api").is_err());
    server.abort();
}

#[tokio::test]
async fn warming_accepts_empty_filters_but_rejects_invalid_syntax_before_io() {
    use axum::{Json, Router, routing::get};
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let requests = calls.clone();
    let (address, server) = crate::test_tls::serve(Router::new().route(
        "/spec",
        get(move || {
            requests.fetch_add(1, Ordering::SeqCst);
            async { Json(serde_json::json!({"paths":{"/read":{"get":{}}}})) }
        }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    let config = |settings: &str| {
        format!(
            "version=1\n[treasury]\nstate_dir='{}'\n[wallets.w]\nmode='static'\nprivate_key_env='UNUSED'\n[sources.api]\nspec='https://{address}/spec'\nbase_url='https://{address}'\n{settings}\n[servers.one]\nlisten='127.0.0.1:0'\nauth=false\nwallet='w'\nsources=['api']\n",
            dir.path().display()
        )
    };
    for settings in [
        "include_tools=['']",
        "[sources.api.overrides.api_read]\npricing_description=17",
        "[sources.api.overrides.api_read]\npricing_desciption='quoted price'",
    ] {
        std::fs::write(&path, config(settings)).unwrap();
        assert!(
            super::super::cache_warm::warm_cache(&path, &[], false)
                .await
                .is_err(),
            "{settings}"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for settings in [
        "use_recommended_tools=true",
        "include_tools=['api_read']\nexclude_tools=['api_read']",
        "include_tools=['unknown_missing']",
    ] {
        std::fs::write(&path, config(settings)).unwrap();
        let result = super::super::cache_warm::warm_cache(&path, &[], false)
            .await
            .unwrap();
        assert_eq!(result.outcome, "degraded");
        assert_eq!(result.sources[0].status, "unavailable");
        assert!(!result.sources[0].selection_notices.is_empty());
        assert!(result.sources[0].failure.is_none());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    server.abort();
}

#[tokio::test]
async fn warming_drains_gated_sibling_and_accepts_partial_catalog() {
    use axum::{Json, Router, http::StatusCode, routing::get};
    let dir = tempfile::tempdir().unwrap();
    let failed = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let router = Router::new()
        .route("/failed", get({let failed=failed.clone(); move || {let failed=failed.clone(); async move {
            failed.notify_one(); StatusCode::SERVICE_UNAVAILABLE
        }}}))
        .route("/healthy", get({let release=release.clone(); move || {let release=release.clone(); async move {
            release.notified().await;
            Json(serde_json::json!({"paths":{
                "/read":{"get":{}},
                "/unsupported":{"post":{"parameters":[{"name":"session","in":"cookie","schema":{"type":"string"}}]}}
            }}))
        }}}));
    let (address, server) = crate::test_tls::serve(router).await;
    let base = format!("https://{address}");
    let config = warming_config(
        dir.path(),
        &format!(
            r#"
[sources.failed]
spec='{base}/failed'
base_url='{base}'
[sources.healthy]
spec='{base}/healthy'
base_url='{base}'
"#
        ),
    );
    let path = dir.path().join("deployment.toml");
    let selected = config.sources.keys().cloned().collect();
    let warming =
        tokio::spawn(
            async move { warm(&config, &path, &selected, false, Default::default()).await },
        );
    failed.notified().await;
    assert!(
        !warming.is_finished(),
        "one failure must not drop the gated sibling"
    );
    release.notify_one();
    let results = warming.await.unwrap().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].source, "failed");
    assert_eq!(results[0].status, "unavailable");
    assert_eq!(results[0].failure.as_ref().unwrap().http_status, Some(503));
    assert_eq!(results[1].status, "partial");
    assert_eq!(results[1].omissions.len(), 1);
    assert_eq!(results[1].catalog_cache, Some("fresh"));
    let cfg = catalog::Config {
        spec: format!("{base}/healthy"),
        base_url: Some(base),
        prefix: Some("healthy".into()),
        http_cache_directory: Some(dir.path().join("http-cache")),
        ..Default::default()
    };
    let cached = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes)
        .unwrap()
        .read()
        .await
        .unwrap();
    let built = catalog::build_catalog(
        &cfg,
        &catalog::parse_document(&cached.data).unwrap(),
        "healthy",
    )
    .unwrap();
    assert_eq!(built.tools.len(), 1);
    assert!(
        built.complete().is_err(),
        "strict inspection still rejects partial catalogs"
    );
    server.abort();
}

#[tokio::test]
async fn warming_distinguishes_local_failures_from_omissions_and_drains_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let healthy_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = healthy_calls.clone();
    let (address, server) = crate::test_tls::serve(
        axum::Router::new()
            .route(
                "/healthy",
                axum::routing::get(move || {
                    let observed = observed.clone();
                    async move {
                        observed.fetch_add(1, Ordering::SeqCst);
                        axum::Json(serde_json::json!({"paths":{"/read":{"get":{}}}}))
                    }
                }),
            )
            .fallback(|| async { axum::http::StatusCode::BAD_GATEWAY }),
    )
    .await;
    let base = format!("https://{address}");
    let config = warming_config(
        dir.path(),
        &format!(
            r#"
[sources.one]
spec='{base}/one'
base_url='{base}'
[sources.two]
spec='{base}/two'
base_url='{base}'
"#
        ),
    );
    let path = dir.path().join("deployment.toml");
    let selected = config.sources.keys().cloned().collect();
    let results = warm(&config, &path, &selected, false, Default::default())
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .all(|r| r.status == "unavailable" && r.degraded())
    );
    let json = serde_json::to_value(super::super::cache_warm::CacheWarmSummary {
        direct: false,
        outcome: "degraded",
        sources: results,
    })
    .unwrap();
    assert_eq!(json["sources"][0]["failure"]["reason"], "http_status");
    assert!(json["sources"][0].get("catalog_cache").is_none());
    std::fs::write(dir.path().join("invalid.json"), "not JSON").unwrap();
    let local = warming_config(
        dir.path(),
        &format!(
            r#"
[sources.local]
spec='invalid.json'
base_url='{base}'
[sources.remote]
spec='{base}/healthy'
base_url='{base}'
"#
        ),
    );
    let selected = local.sources.keys().cloned().collect();
    let unsupported = serde_json::json!({"post":{"parameters":[{"name":"session","in":"cookie","schema":{"type":"string"}}]}});
    for (attempt, (document, expected_status)) in [
        ("not JSON".to_owned(), None),
        (
            serde_json::json!({"paths":{"/unsupported":unsupported.clone()}}).to_string(),
            Some("unavailable"),
        ),
        (
            serde_json::json!({"paths":{"/read":{"get":{}},"/unsupported":unsupported}})
                .to_string(),
            Some("partial"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(dir.path().join("invalid.json"), document).unwrap();
        let result = warm(&local, &path, &selected, false, Default::default()).await;
        match expected_status {
            None => {
                result
                    .err()
                    .expect("malformed local input must remain fatal");
            }
            Some(status) => {
                let sources = result.unwrap();
                assert_eq!(sources[0].source, "local");
                assert_eq!(sources[0].status, status);
                assert_eq!(sources[0].omissions.len(), 1);
                assert!(sources[0].failure.is_none());
                assert!(sources[0].degraded());
                assert_eq!(sources[1].source, "remote");
                assert_eq!(sources[1].status, "ready");
            }
        }
        assert_eq!(
            healthy_calls.load(Ordering::SeqCst),
            attempt + 1,
            "local failure or omission must drain the healthy remote source"
        );
        let cfg = catalog::Config {
            spec: format!("{base}/healthy"),
            base_url: Some(base.clone()),
            prefix: Some("remote".into()),
            http_cache_directory: Some(dir.path().join("http-cache")),
            ..Default::default()
        };
        assert!(
            Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes)
                .unwrap()
                .read()
                .await
                .is_some(),
            "healthy remote acceptance must complete despite local failure or omissions"
        );
    }
    let unknown = ["missing".to_owned()].into_iter().collect();
    assert!(
        warm(&local, &path, &unknown, false, Default::default())
            .await
            .is_err()
    );
    server.abort();
}

#[tokio::test]
async fn warming_keeps_api_ready_when_pricing_fails_or_cannot_be_stored() {
    use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let prices = calls.clone();
    let router = Router::new()
        .route(
            "/spec",
            get(|| async { Json(serde_json::json!({"paths":{"/read":{"get":{}}}})) }),
        )
        .route(
            "/prices",
            get(move || {
                let calls = prices.clone();
                async move {
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                    (
                        [("vary", "*")],
                        Json(serde_json::json!({"x402Version":2,"items":[]})),
                    )
                        .into_response()
                }
            }),
        );
    let (address, server) = crate::test_tls::serve(router).await;
    let base = format!("https://{address}");
    let config = warming_config(
        dir.path(),
        &format!(
            r#"
[sources.api]
spec='{base}/spec'
base_url='{base}'
[sources.api.pricing]
source='{base}/prices'
format='x402-discovery'
"#
        ),
    );
    let path = dir.path().join("deployment.toml");
    let selected = config.sources.keys().cloned().collect();
    let failed = warm(&config, &path, &selected, false, Default::default())
        .await
        .unwrap();
    assert_eq!(failed[0].status, "ready");
    let pricing = failed[0].pricing.as_ref().unwrap();
    assert_eq!(pricing.status, "unavailable");
    assert_eq!(pricing.failure.as_ref().unwrap().http_status, Some(502));
    assert!(failed[0].degraded());
    let unstored = warm(&config, &path, &selected, false, Default::default())
        .await
        .unwrap();
    assert_eq!(unstored[0].status, "ready");
    let pricing = unstored[0].pricing.as_ref().unwrap();
    assert_eq!(pricing.status, "ready");
    assert_eq!(pricing.catalog_cache, Some("not_stored"));
    assert!(pricing.failure.is_none());
    assert!(unstored[0].degraded());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    server.abort();
}

#[test]
fn warming_does_not_classify_local_pricing_failures_as_optional_outages() {
    let local = anyhow::anyhow!("private failure").context(catalog::LocalCatalogFailure);
    assert!(!warm_remote_failure(&local));
    let unavailable = anyhow::anyhow!("remote response unavailable");
    assert!(warm_remote_failure(&unavailable));
}

#[tokio::test]
async fn warming_reports_info_progress_and_only_completed_outcomes() {
    if crate::test_log_capture::run_in_child(
        "deployment::startup::warm_tests::warming_reports_info_progress_and_only_completed_outcomes",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_env_filter("warn,x402_treazury=info")
            .with_writer(log.reopen().unwrap())
            .finish(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    let mut config = warming_config(
        dir.path(),
        "[sources.api]\nspec='private-catalog.json'\nbase_url='https://api.example.com'\n",
    );
    config.servers.get_mut("test").unwrap().sources = vec!["api".into()];
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let healthy = serde_json::json!({"paths":{"/read":{"get":{
        "description":"private catalog content"
    }}}});
    let mut partial = healthy.clone();
    partial["paths"]["/unsupported"] = serde_json::json!({"get":{
        "parameters":[{"name":"session","in":"cookie","schema":{"type":"string"}}]
    }});
    for (document, expected) in [
        (healthy.to_string(), Some(("ok", "ready"))),
        (partial.to_string(), Some(("degraded", "partial"))),
        ("not JSON".to_owned(), None),
    ] {
        std::fs::write(dir.path().join("private-catalog.json"), document).unwrap();
        let offset = std::fs::read_to_string(log.path()).unwrap().len();
        let result = super::super::cache_warm::warm_cache(&path, &[], false).await;
        if expected.is_some() {
            assert!(result.is_ok(), "{}", result.as_ref().err().unwrap());
        }
        let text = std::fs::read_to_string(log.path()).unwrap();
        let text = &text[offset..];
        assert!(
            text.contains("INFO") && text.contains("Warming selected catalogs"),
            "{text}"
        );
        if let Some((outcome, status)) = expected {
            let summary = result.unwrap();
            assert_eq!(summary.outcome, outcome);
            assert_eq!(summary.sources.len(), 1);
            assert_eq!(summary.sources[0].status, status);
            assert!(text.contains("Catalog warming source finished"), "{text}");
            assert!(text.contains("completed=1 total=1"), "{text}");
            let completion = text
                .lines()
                .find(|line| line.contains("Catalog warming finished sources="))
                .unwrap();
            assert!(
                completion.contains("INFO")
                    && completion.contains(&format!("outcome=\"{outcome}\"")),
                "{text}"
            );
            assert_eq!(text.contains("ERROR"), outcome == "degraded", "{text}");
        } else {
            assert!(result.is_err());
            assert!(
                !text.contains("Catalog warming finished sources="),
                "{text}"
            );
        }
        for private in [
            "private-catalog",
            "private catalog content",
            "api.example.com",
        ] {
            assert!(!text.contains(private), "{text}");
        }
        assert!(!text.contains("DEBUG"), "{text}");
    }
}
