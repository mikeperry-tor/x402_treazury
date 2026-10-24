use super::*;
use crate::http_cache::{Entry, Metadata, Slot};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn warm_prefers_fresh_relay_catalog_without_endpoint_requests() {
    let dir = tempfile::tempdir().unwrap();
    let (relay, fixture, task) =
        crate::discovery_relay::tests::fixture(serde_json::json!({}), false, false).await;
    let mut config: MetaConfig = toml::from_str(
        r#"
version=1
[treasury]
state_dir='unused'
max_refund_shielding_fee_zec='0.001'
[wallets.w]
mode='static'
private_key_env='UNUSED'
max_api_payment_usdc='0.01'
[sources.api]
spec='http://127.0.0.1:1/spec'
base_url='http://127.0.0.1:1'
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
        assert_eq!(
            results[0].catalog_cache,
            if relay_catalog_fresh {
                "fresh_via_relay"
            } else {
                "fresh"
            }
        );
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.signed.load(Ordering::SeqCst), 0);
    }
    task.abort();
}

#[test]
fn default_metadata_fallback_warms_catalog_with_one_real_payment() {
    metadata_child(
        "warm",
        "default_metadata_fallback_warms_catalog_with_one_real_payment",
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
    let target = "https://api.example.com/spec";
    let document =
        json!({"operations":[{"method":"GET","path":"/read"},{"method":"GET","path":"/price"}]})
            .to_string();
    let relay = Router::new().route("/curl", post({
        let (calls, signed) = (calls.clone(), signed.clone());
        move |headers: HeaderMap, axum::Json(request): axum::Json<Value>| {
            let (calls, signed, document) = (calls.clone(), signed.clone(), document.clone());
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(request["url"], target);
                assert_eq!(request["method"], "GET");
                if let Some(signature) = headers.get("payment-signature") {
                    let payload: Value = serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
                    let expected: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
                    assert_eq!(crate::test_signatures::recover_exact(&payload), expected.address());
                    assert_eq!(payload["payload"]["authorization"]["value"], "10000");
                    signed.fetch_add(1, Ordering::SeqCst);
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
        socks_endpoint: Some(socks.address),
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
    if mode == "warm" {
        // Disabling metadata cannot purchase a catalog through sibling defaults.
        std::fs::write(
            &path,
            config.replace(
                "relays.GET=['relay.toml']",
                "relays.GET=['relay.toml']\nprovider_metadata=false",
            ),
        )
        .unwrap();
        assert!(
            super::super::cache_warm::warm_cache(&path, &[], false)
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        std::fs::write(&path, &config).unwrap();
        let summary = super::super::cache_warm::warm_cache(&path, &[], false)
            .await
            .unwrap();
        assert_eq!(summary.sources.len(), 1);
        assert_eq!(summary.sources[0].catalog_cache, "fresh_via_relay");
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
