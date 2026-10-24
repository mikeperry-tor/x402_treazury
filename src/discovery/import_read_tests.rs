use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn concurrent_imports_outlive_waiters_and_completed_cache_churn() {
    use tokio::sync::Semaphore;
    const IMPORTS: usize = 6;
    let m = manager(None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let arrived = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let app = axum::Router::new().fallback({
        let (calls, arrived, release) = (calls.clone(), arrived.clone(), release.clone());
        move || {
            let (calls, arrived, release) = (calls.clone(), arrived.clone(), release.clone());
            async move {
                if calls.fetch_add(1, Ordering::SeqCst) < IMPORTS {
                    arrived.add_permits(1);
                    release.acquire().await.unwrap().forget();
                }
                axum::Json(spec())
            }
        }
    });
    let (url, server) = listen(app).await;
    *m.fixture_endpoint.lock().unwrap() = Some(url);
    let mut adds = Vec::new();
    for n in 0..IMPORTS {
        let m = m.clone();
        adds.push(tokio::spawn(async move {
            m.invoke(
                "writer",
                "x402_treazury_source_add",
                json!({"spec_url":format!("https://api.example.com/import{n}"),"name":format!("import{n}")}),
            )
            .await
        }));
    }
    tokio::time::timeout(Duration::from_secs(5), arrived.acquire_many(IMPORTS as u32))
        .await
        .expect("all distinct same-listener imports must reach I/O together")
        .unwrap()
        .forget();
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    let active: Vec<_> = m
        .fetches
        .lock()
        .unwrap()
        .iter()
        .map(|(key, cell)| (key.clone(), cell.clone()))
        .collect();
    assert_eq!(active.len(), IMPORTS);
    adds.remove(0).abort();

    // More completed documents than the retention allowance cannot displace any
    // active acquisition, including the one whose original waiter disconnected.
    for n in 0..12 {
        m.invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":format!("https://api.example.com/churn{n}"),"name":format!("churn{n}")}),
        )
        .await
        .unwrap();
    }
    {
        let cache = m.fetches.lock().unwrap();
        for (key, cell) in &active {
            assert!(Arc::ptr_eq(&cache[key], cell));
            assert!(cell.get().is_none());
        }
    }
    let alias = tokio::spawn({
        let m = m.clone();
        async move {
            m.invoke(
                "writer",
                "x402_treazury_source_add",
                json!({"spec_url":"https://api.example.com/import0","name":"import0"}),
            )
            .await
        }
    });
    // Confirm the alias has joined the active cell before releasing origin I/O.
    tokio::time::timeout(Duration::from_secs(5), async {
        let key = fetch_key(&m, "https://api.example.com/import0");
        let (_, cell) = active
            .iter()
            .find(|(active_key, _)| *active_key == key)
            .unwrap();
        while Arc::strong_count(cell) < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), IMPORTS + 12);
    release.add_permits(IMPORTS);
    for add in adds {
        assert_eq!(add.await.unwrap().unwrap()["status"], "available");
    }
    assert_eq!(alias.await.unwrap().unwrap()["status"], "available");
    assert_eq!(calls.load(Ordering::SeqCst), IMPORTS + 12);
    assert_eq!(m.inner.lock().unwrap().state.records.len(), IMPORTS + 12);
    assert_eq!(m.catalog.read().views["writer"].len(), 2 * (IMPORTS + 12));
    let cache = m.fetches.lock().unwrap();
    assert!(cache.values().all(|cell| cell.get().is_some()));
    assert_eq!(cache.len(), 128 * 1024 * 1024 / crate::limits::SPEC_BYTES);
    server.abort();
}

#[tokio::test]
async fn accepted_import_retains_fetch_validation_and_publication_after_disconnect() {
    let manager = manager(None).await;
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new().fallback({
        let (arrived, release, calls) = (arrived.clone(), release.clone(), calls.clone());
        move || {
            let (arrived, release, calls) = (arrived.clone(), release.clone(), calls.clone());
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                arrived.notify_one();
                release.notified().await;
                axum::Json(spec())
            }
        }
    });
    let (url, server) = listen(app).await;
    *manager.fixture_endpoint.lock().unwrap() = Some(url);
    let add = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .invoke(
                    "writer",
                    "x402_treazury_source_add",
                    json!({"spec_url":"https://api.example.com/retained.json"}),
                )
                .await
        }
    });
    arrived.notified().await;
    add.abort();
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if manager.catalog.read().views["writer"].len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(manager.inner.lock().unwrap().state.records.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    manager
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":"https://api.example.com/retained.json"}),
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn import_cache_pins_actual_authority_and_preserves_compatible_aliases() {
    let m = manager(None).await;
    seed(&m).await;
    let old = fetch_key(&m, "https://api.example.com/openapi.json");
    let source = m.source_client("writer").unwrap();
    source.replace_payer(
        Payer::new(
            &format!("{:064x}", 2),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    );
    let new = fetch_key(&m, "https://api.example.com/openapi.json");
    assert_ne!(
        old, new,
        "payer replacement cannot reuse a previous authority's acquisition"
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new().fallback({
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                axum::Json(spec())
            }
        }
    });
    let (url, server) = listen(app).await;
    *m.fixture_endpoint.lock().unwrap() = Some(url);
    let (writer, reader) = tokio::join!(
        m.document("writer", "https://api.example.com/openapi.json"),
        m.document("reader", "https://api.example.com/openapi.json")
    );
    assert!(Arc::ptr_eq(&writer.unwrap(), &reader.unwrap()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(m.fetches.lock().unwrap().contains_key(&old));
    assert!(m.fetches.lock().unwrap().contains_key(&new));
    server.abort();
}

#[tokio::test]
async fn imports_reject_partial_documents_and_complete_invalid_json_without_publication() {
    use axum::response::IntoResponse;
    for content_range in [false, true] {
        let m = manager(None).await;
        let app = axum::Router::new().fallback(move || async move {
            let status = if content_range {
                axum::http::StatusCode::OK
            } else {
                axum::http::StatusCode::PARTIAL_CONTENT
            };
            let mut response = (status, spec().to_string()).into_response();
            if content_range {
                response
                    .headers_mut()
                    .insert("content-range", "bytes 0-99/200".parse().unwrap());
            }
            response
        });
        let (url, server) = listen(app).await;
        *m.fixture_endpoint.lock().unwrap() = Some(url);
        let error = m
            .invoke(
                "writer",
                "x402_treazury_source_add",
                json!({"spec_url":"https://api.example.com/partial.json"}),
            )
            .await
            .unwrap_err();
        assert!(!format!("{error:#}").is_empty());
        assert!(m.catalog.read().views["writer"].is_empty());
        assert!(m.inner.lock().unwrap().state.records.is_empty());
        server.abort();
    }
    let m = manager(None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new().fallback({
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                "complete invalid JSON"
            }
        }
    });
    let (url, server) = listen(app).await;
    *m.fixture_endpoint.lock().unwrap() = Some(url);
    for owner in ["writer", "reader"] {
        assert!(
            m.invoke(
                owner,
                "x402_treazury_source_add",
                json!({"spec_url":"https://api.example.com/invalid.json"})
            )
            .await
            .is_err()
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "complete content validation does not repurchase the shared document"
    );
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    server.abort();
}

#[test]
fn paid_import_cache_failures_and_cancelled_publication_in_isolated_process() {
    const CHILD: &str = "TREAZURY_IMPORT_READ_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(paid_import_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "discovery::tests::import_read_tests::paid_import_cache_failures_and_cancelled_publication_in_isolated_process", "--nocapture"])
        .env(CHILD, "1")
        .env("TREAZURY_IMPORT_CLI_SELECTED_KEY", format!("{:064x}", 19))
        .env_remove("TREAZURY_IMPORT_CLI_UNUSED_KEY").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[derive(Clone, Default)]
struct PaidImport {
    calls: Arc<Mutex<BTreeMap<String, Vec<&'static str>>>>,
    signers: Arc<Mutex<Vec<String>>>,
    signed: tokio_util::sync::CancellationToken,
    release: tokio_util::sync::CancellationToken,
}
async fn paid_import_origin(
    axum::extract::State(state): axum::extract::State<PaidImport>,
    axum::extract::Path(kind): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("authorization"));
    state
        .calls
        .lock()
        .unwrap()
        .entry(kind.clone())
        .or_default()
        .push("origin");
    if kind == "healthy" {
        axum::Json(spec()).into_response()
    } else {
        axum::http::StatusCode::FORBIDDEN.into_response()
    }
}
async fn paid_import_relay(
    axum::extract::State(state): axum::extract::State<PaidImport>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use base64::{Engine, engine::general_purpose::STANDARD};
    let request: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(request["method"], "GET");
    assert!(!headers.contains_key("authorization"));
    let target = request["url"].as_str().unwrap();
    let kind = target.rsplit('/').next().unwrap();
    let Some(signature) = headers.get("payment-signature") else {
        state
            .calls
            .lock()
            .unwrap()
            .entry(kind.into())
            .or_default()
            .push("challenge");
        let challenge = json!({"x402Version":2,"resource":{"url":format!("https://spec.example.com{}", uri.path())},"accepts":[{
            "scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1",
            "payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,
            "extra":{"name":"USD Coin","version":"2"}
        }]});
        return (
            axum::http::StatusCode::PAYMENT_REQUIRED,
            [(
                "payment-required",
                STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
            )],
            "",
        )
            .into_response();
    };
    let payment: Value =
        serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
    let signer = crate::test_signatures::recover_exact(&payment);
    state
        .signers
        .lock()
        .unwrap()
        .push(signer.to_string().to_ascii_lowercase());
    state
        .calls
        .lock()
        .unwrap()
        .entry(kind.into())
        .or_default()
        .push("signed");
    if kind == "held" {
        state.signed.cancel();
        state.release.cancelled().await;
    }
    if kind == "malformed" {
        return "PRIVATE_UPSTREAM_PROSE".into_response();
    }
    let document = if kind == "invalid-schema" {
        json!({"openapi":"3.0.3","paths":{},"servers":[{"url":"https://api.example.com"}]})
            .to_string()
    } else {
        spec().to_string()
    };
    axum::Json(json!({"url":target,"method":"GET","statusCode":200,"headers":{"content-type":"application/json"},"body":document,"redirectChain":[],"error":null})).into_response()
}
async fn paid_import_fixture() {
    use crate::network::{Mode, NetworkContext, NetworkPolicy};
    let state = PaidImport::default();
    let app = axum::Router::new()
        .route("/imports/{kind}", axum::routing::get(paid_import_origin))
        .route("/relay", axum::routing::post(paid_import_relay))
        .route("/curl", axum::routing::post(paid_import_relay))
        .with_state(state.clone());
    let (address, server) = crate::test_tls::serve(app).await;
    let proxy = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            tor_isolation_retries: Some(0),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let configured = listeners(false);
    let snapshot = CatalogSnapshot {
        instructions: Default::default(),
        generation: 0,
        views: configured.keys().map(|key| (key.clone(), vec![])).collect(),
    };
    let relay = Arc::new(crate::discovery_relay::SearchRelay::fixture(
        "https://spec.example.com/relay".into(),
        crate::catalog::Config::default(),
    ));
    let m = Manager::new_with_search(
        configured,
        BTreeMap::from([("shared".into(), payer())]),
        Arc::new(CatalogState::new(snapshot)),
        vec![],
        Some(relay),
    )
    .await
    .unwrap();
    let first = tokio::spawn({
        let m = m.clone();
        async move {
            m.invoke(
                "writer",
                "x402_treazury_source_add",
                json!({"spec_url":"https://api.example.com/imports/held","name":"retained"}),
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), state.signed.cancelled())
        .await
        .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    // The held paid response cannot block an independent complete origin import.
    m.invoke(
        "reader",
        "x402_treazury_source_add",
        json!({"spec_url":"https://api.example.com/imports/healthy","name":"healthy"}),
    )
    .await
    .unwrap();
    state.release.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if m.catalog.read().views["writer"].len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
    assert_eq!(
        state.calls.lock().unwrap()["held"],
        ["origin", "challenge", "signed"]
    );
    for kind in ["invalid-schema", "malformed"] {
        let url = format!("https://api.example.com/imports/{kind}");
        let (writer, reader) = tokio::join!(
            m.invoke(
                "writer",
                "x402_treazury_source_add",
                json!({"spec_url":url,"name":kind})
            ),
            m.invoke(
                "reader",
                "x402_treazury_source_add",
                json!({"spec_url":url,"name":kind})
            )
        );
        for error in [writer.unwrap_err(), reader.unwrap_err()] {
            let failure = SharedFailure::capture(&error, "source unavailable");
            assert!(failure.possible_payment(), "{error:#}");
            assert!(format!("{error:#}").contains("do not automatically repeat"));
            assert!(!format!("{error:#}").contains("PRIVATE_UPSTREAM_PROSE"));
        }
        assert_eq!(
            state.calls.lock().unwrap()[kind],
            ["origin", "challenge", "signed"]
        );
    }
    m.fail_after_commit.store(true, Ordering::SeqCst);
    let error = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":"https://api.example.com/imports/publication","name":"publication"}),
        )
        .await
        .unwrap_err();
    assert!(
        SharedFailure::capture(&error, "publication failed").possible_payment(),
        "{error:#}"
    );
    assert_eq!(
        state.calls.lock().unwrap()["publication"],
        ["origin", "challenge", "signed"]
    );
    m.fail_after_commit.store(false, Ordering::SeqCst);
    all_unsupported_refresh_avoids_authority().await;
    assert_eq!(state.calls.lock().unwrap()["unsupported"], ["origin"]);
    let signer: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
    assert_eq!(
        *state.signers.lock().unwrap(),
        vec![signer.address().to_string().to_ascii_lowercase(); 4]
    );
    supported_paid_cli_refresh(&state).await;
    assert_eq!(state.calls.lock().unwrap()["healthy"], ["origin"]);
    assert_eq!(
        m.inner.lock().unwrap().state.records.len(),
        2,
        "failed paid schemas/envelopes cannot publish a registration"
    );
    assert_eq!(
        m.payers.len(),
        1,
        "imports use the existing selected authority"
    );
    m.source_client("writer").unwrap().drain().await;
    server.abort();
}

async fn all_unsupported_refresh_avoids_authority() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry.sqlite");
    let url = format!(
        "https://api.example.com/imports/unsupported?exact={}",
        "x".repeat(9000)
    );
    let m = manager(Some(registry.clone())).await;
    let cell = OnceCell::new();
    cell.set(Fetched {
        completed: Instant::now(),
        result: Ok(Arc::new(Document::local(spec().to_string().into_bytes()))),
    })
    .unwrap();
    m.fetches
        .lock()
        .unwrap()
        .insert(fetch_key(&m, &url), Arc::new(cell));
    let added = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":url,"name":"unsupported"}),
        )
        .await
        .unwrap();
    let id = registered_id(&m, &added);
    drop(m);
    let config = dir.path().join("deployment.toml");
    let provider =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/provider.toml");
    let value = json!({"version":1,
        "network": fixture_network(),
        "wallets":{"shared":{"mode":"static","private_key_env":"IMPORT_UNSUPPORTED_MISSING_KEY"}},
        "relay_fallback":{"relays":{"GET":[provider]},"provider_metadata":true},
        "treasury":{"state_dir":dir.path().join("untouched-state"),"key_file":dir.path().join("untouched-key")},
        "servers":{"writer":{"listen":"127.0.0.1:0","auth":false,"source_management":true,"wallet":"shared","source_registry_file":registry}}
    });
    std::fs::write(&config, toml::to_string(&value).unwrap()).unwrap();
    let error = maintain_cli(&config, "writer", &id, true)
        .await
        .unwrap_err();
    let facts = SharedFailure::capture(&error, "refresh failed");
    assert_eq!(
        facts.status(),
        Some(403),
        "unsupported entries preserve the original HTTP failure: {error:#}"
    );
    assert!(!facts.possible_payment());
    assert!(!format!("{error:#}").contains("selected signing key unavailable"));
    assert!(!dir.path().join("untouched-state").exists());
    assert!(!dir.path().join("untouched-key").exists());
    let options = import::options("https://api.example.com/imports/supported", None).unwrap();
    let relay_policy: crate::discovery_relay::Policy =
        serde_json::from_value(value["relay_fallback"].clone()).unwrap();
    let relay = crate::discovery_relay::SearchRelay::load(&relay_policy, &config)
        .await
        .unwrap();
    assert!(
        crate::discovery_relay::document::prepare_fallback(&relay, &options)
            .unwrap()
            .is_some(),
        "the same configured adapter supports an ordinary short GET"
    );
    let mut options = options;
    options.url = url;
    assert!(
        !crate::discovery_relay::document::prepare_fallback(&relay, &options)
            .unwrap()
            .is_some()
    );
}

fn fixture_network() -> Value {
    serde_json::to_value(toml::Value::try_from(&crate::network::global().policy).unwrap()).unwrap()
}

async fn supported_paid_cli_refresh(state: &PaidImport) {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry.sqlite");
    let config = dir.path().join("deployment.toml");
    let provider = dir.path().join("relay.toml");
    let schema = dir.path().join("relay.json");
    std::fs::write(
        &schema,
        r#"{"operations":[{"method":"POST","path":"/curl"}]}"#,
    )
    .unwrap();
    std::fs::write(
        &provider,
        "spec='relay.json'\nrelay_format='curl'\nbase_url='https://spec.example.com'\n",
    )
    .unwrap();
    let url = "https://api.example.com/imports/cli-paid";
    let m = manager(Some(registry.clone())).await;
    let cell = OnceCell::new();
    let mut old = spec();
    old["paths"] = json!({"/before-refresh":{"get":{}}});
    cell.set(Fetched {
        completed: Instant::now(),
        result: Ok(Arc::new(Document::local(old.to_string().into_bytes()))),
    })
    .unwrap();
    m.fetches
        .lock()
        .unwrap()
        .insert(fetch_key(&m, url), Arc::new(cell));
    let added = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":url,"name":"cli-paid"}),
        )
        .await
        .unwrap();
    let id = registered_id(&m, &added);
    drop(m);
    let value = json!({"version":1,
        "network": fixture_network(),
        "wallets":{
            "shared":{"mode":"static","private_key_env":"TREAZURY_IMPORT_CLI_UNUSED_KEY"},
            "selected":{"mode":"static","private_key_env":"TREAZURY_IMPORT_CLI_SELECTED_KEY"},
            "unrelated":{"mode":"static","private_key_env":"TREAZURY_IMPORT_CLI_UNUSED_KEY"}},
        "relay_fallback":{"relays":{"GET":[provider]},"provider_metadata":true},
        "treasury":{"state_dir":dir.path().join("untouched-state"),"key_file":dir.path().join("untouched-key")},
        "servers":{"writer":{"listen":"127.0.0.1:0","auth":false,"source_management":true,"wallet":"selected","source_registry_file":registry}}
    });
    std::fs::write(&config, toml::to_string(&value).unwrap()).unwrap();
    maintain_cli(&config, "writer", &id, true).await.unwrap();
    let saved = store::Store::open(&registry, &[]).unwrap();
    let state_after = saved.load().unwrap();
    let refreshed = &state_after.records[&id];
    assert_eq!(refreshed.revision, 2);
    assert_eq!(
        serde_json::from_str::<Value>(&refreshed.document).unwrap(),
        spec()
    );
    assert_eq!(refreshed.hash, hash(refreshed.document.as_bytes()));
    assert!(refreshed.disabled.is_none());
    drop(saved);
    assert_eq!(
        state.calls.lock().unwrap()["cli-paid"],
        ["origin", "challenge", "signed"]
    );
    let signer: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 19).parse().unwrap();
    assert_eq!(
        state.signers.lock().unwrap().last().unwrap(),
        &signer.address().to_string().to_ascii_lowercase()
    );
    assert!(std::env::var_os("TREAZURY_IMPORT_CLI_UNUSED_KEY").is_none());
    assert!(!dir.path().join("untouched-state").exists());
    assert!(!dir.path().join("untouched-key").exists());
}
