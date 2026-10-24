#[path = "registry_tests.rs"]
mod registry_tests;
use super::*;
#[path = "coinbase_network_tests.rs"]
mod coinbase_network_tests;
use crate::mcp_wire::McpResponse;
use crate::{
    payment::{Payer, SpendPolicy},
    server::Server,
};
use std::collections::BTreeSet;
#[path = "curated_network_tests.rs"]
mod curated_network_tests;
#[path = "curated_tests.rs"]
mod curated_tests;
fn listeners(_persistent: bool) -> BTreeMap<String, ListenerConfig> {
    ["writer", "reader", "hidden"].into_iter().map(|id| (id.into(), serde_json::from_value(json!({"listen":"127.0.0.1:0","bearer_token_env":"TEST_TOKEN","sources":[],"wallet":"shared","source_management":id != "hidden"})).unwrap())).collect()
}
fn with_registry(
    mut listeners: BTreeMap<String, ListenerConfig>,
    file: Option<PathBuf>,
) -> BTreeMap<String, ListenerConfig> {
    if let Some(file) = &file {
        listeners.get_mut("reader").unwrap().source_registry_file =
            Some(file.with_file_name("reader-registry.sqlite"));
    }
    listeners.get_mut("writer").unwrap().source_registry_file = file;
    listeners
}

fn payer() -> PaidClient {
    PaidClient::new(
        Payer::new(
            &format!("{:064x}", 1),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    )
}

async fn manager_with(listeners: BTreeMap<String, ListenerConfig>) -> Arc<Manager> {
    let snapshot = CatalogSnapshot {
        instructions: Default::default(),
        generation: 0,
        views: listeners.keys().map(|k| (k.clone(), vec![])).collect(),
    };
    Manager::new(
        listeners,
        BTreeMap::from([("shared".into(), payer())]),
        Arc::new(CatalogState::new(snapshot)),
        vec![],
    )
    .await
    .unwrap()
}

async fn manager(file: Option<PathBuf>) -> Arc<Manager> {
    let persist = file.is_some();
    manager_with(with_registry(listeners(persist), file)).await
}

fn spec() -> Value {
    json!({"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],"paths":{"/read":{"get":{"tags":["read"],"description":"Read item","parameters":[{"in":"query","name":"q","schema":{"type":"string"}}]}},"/write":{"post":{"tags":["write"],"description":"Write item"}}}})
}

fn candidate(name: &str, lifetime: &str, visibility: &str) -> Value {
    json!({"name":name,"spec_url":"https://api.example.com/openapi.json","lifetime":lifetime,"visibility":visibility})
}

fn registered_id(m: &Manager, response: &Value) -> String {
    m.inner
        .lock()
        .unwrap()
        .state
        .records
        .values()
        .find(|record| {
            !record.removed
                && record.owner == "writer"
                && record.candidate.name == response["name"].as_str().unwrap()
        })
        .unwrap()
        .id
        .clone()
}

fn fetch_key(m: &Manager, url: &str) -> FetchKey {
    fetch_key_for(m, "writer", url)
}
fn fetch_key_for(m: &Manager, owner: &str, url: &str) -> FetchKey {
    let (client, authority) = m.document_client(owner).unwrap().pin_document_authority();
    FetchKey {
        url: url.into(),
        transport: Default::default(),
        timeout: import::read_timeout(None),
        max_spec_bytes: crate::limits::SPEC_BYTES,
        authority,
        policy: client.document_policy_key(),
    }
}

async fn seed(m: &Manager) {
    seed_for(m, "writer").await;
}
async fn seed_for(m: &Manager, owner: &str) {
    let cell = OnceCell::new();
    cell.set(Fetched {
        completed: Instant::now(),
        result: Ok(Arc::new(Document::local(
            serde_json::to_vec(&spec()).unwrap(),
        ))),
    })
    .unwrap();
    m.fetches.lock().unwrap().insert(
        fetch_key_for(m, owner, "https://api.example.com/openapi.json"),
        Arc::new(cell),
    );
}

fn server(m: &Arc<Manager>, id: &str) -> Server {
    let mut s = Server::new(vec![], payer(), "https://api.example.com".into(), None);
    s.catalog = m.catalog.clone();
    s.catalog_server = id.into();
    s.discovery = Some(m.clone());
    s
}

async fn listen(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    )
}

async fn rpc(base: &str, token: &str, method: &str, params: Value) -> Value {
    crate::network::discovery(base, Duration::from_secs(5))
        .unwrap()
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap()
        .mcp_json::<Value>()
        .await
        .unwrap()["result"]
        .clone()
}

async fn call(base: &str, name: &str, args: Value) -> Value {
    rpc(
        base,
        "test-token",
        "tools/call",
        json!({"name":name,"arguments":args}),
    )
    .await
}

async fn add(m: &Arc<Manager>, name: &str, _lifetime: &str, _visibility: &str) -> Value {
    seed(m).await;
    m.invoke(
        "writer",
        "x402_treazury_source_add",
        json!({"spec_url":"https://api.example.com/openapi.json","name":name}),
    )
    .await
    .unwrap()
}
#[test]
fn imported_and_saved_catalogs_share_post_preference_and_body_routing() {
    let c: Candidate = serde_json::from_value(candidate("demo", "process", "server")).unwrap();
    let mut doc = spec();
    doc["paths"]["/read"]["post"] = json!({"requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"q":{"type":"string"}}}}}}});
    let bytes = serde_json::to_vec(&doc).unwrap();
    let built = import::build(&c, &bytes).unwrap();
    let saved = import::saved_tools(&c, &bytes).unwrap();
    assert_eq!(
        serde_json::to_value(&built.tools).unwrap(),
        serde_json::to_value(saved).unwrap()
    );
    let read = built.tools.iter().find(|t| t.path == "/read").unwrap();
    assert_eq!(read.name, "dyn_demo__read");
    assert_eq!(read.method, "POST");
    assert_eq!(built.tools.iter().filter(|t| t.path == "/read").count(), 1);
    let args = json!({"q":"research"});
    let request = read.route(&built.base, args.as_object().unwrap()).unwrap();
    assert_eq!(request.body, Some(args));
    assert!(request.query.is_empty());
}

#[test]
fn importer_rejects_unsafe_and_expanding_documents() {
    let c: Candidate = serde_json::from_value(candidate("demo", "process", "server")).unwrap();
    assert!(import::build(&c, &serde_json::to_vec(&spec()).unwrap()).is_ok());
    for url in [
        "file:///etc/passwd",
        "http://api.example.com",
        "https://localhost/x",
        "https://127.0.0.1/x",
        "https://[::ffff:127.0.0.1]/x",
        "https://169.254.169.254/x",
        "https://user:secret@api.example.com",
        "https://metadata.internal",
        "https://0177.0.0.1",
    ] {
        assert!(import::endpoint(url).is_err(), "{url}");
    }
    for replacement in [
        json!({"$ref":"https://evil.example.com/schema"}),
        json!({"$ref":"#/missing"}),
        json!({"loop":{"$ref":"#/components"}}),
    ] {
        let mut doc = spec();
        doc["components"] = replacement;
        assert!(import::build(&c, &serde_json::to_vec(&doc).unwrap()).is_err());
    }
    let mut doc = spec();
    doc["paths"]["/read"]["get"]["servers"] = json!([{"url":"https://evil.example.com"}]);
    assert!(import::build(&c, &serde_json::to_vec(&doc).unwrap()).is_err());
    let mut doc = spec();
    doc["components"] = json!({"x0":{"type":"string"}});
    for n in 1..25 {
        doc["components"][format!("x{n}")] = json!({"allOf":[{"$ref":format!("#/components/x{}",n-1)},{"$ref":format!("#/components/x{}",n-1)}]});
    }
    assert!(
        import::build(&c, &serde_json::to_vec(&doc).unwrap())
            .unwrap_err()
            .to_string()
            .contains("complexity")
    );
}

#[test]
fn import_pipeline_preserves_origin_checks_and_validation_before_selection() {
    let mut c: Candidate = serde_json::from_value(candidate("demo", "process", "server")).unwrap();
    let build = |c: &Candidate, doc: &Value| import::build(c, &serde_json::to_vec(doc).unwrap());
    let mut doc = spec();
    doc["servers"][0]["url"] = json!("/v1");
    assert_eq!(build(&c, &doc).unwrap().base, "https://api.example.com/v1");
    c.base_url = Some("https://api.example.com/override".into());
    doc.as_object_mut().unwrap().remove("servers");
    assert_eq!(
        build(&c, &doc).unwrap().base,
        c.base_url.as_ref().unwrap().as_str()
    );
    c.base_url = Some("https://api.example.com/v1?secret=value".into());
    assert!(
        build(&c, &doc)
            .unwrap_err()
            .to_string()
            .contains("base_url must not contain query")
    );
    c.base_url = None;

    // Invalid operations are rejected even when selection would exclude them.
    c.selection.include = vec!["/write".into()];
    for (node, field, value, error) in [
        (
            "path",
            "servers",
            json!([{ "url": "https://other.example.com" }]),
            "per-path servers unsupported",
        ),
        (
            "operation",
            "servers",
            json!([{ "url": "https://other.example.com" }]),
            "per-operation servers unsupported",
        ),
        (
            "path",
            "$ref",
            json!("#/components/path"),
            "path references unsupported",
        ),
    ] {
        let mut bad = spec();
        bad["components"] = json!({"path":{"get":{}}});
        let target = if node == "path" {
            &mut bad["paths"]["/read"]
        } else {
            &mut bad["paths"]["/read"]["get"]
        };
        target[field] = value;
        assert!(
            build(&c, &bad).unwrap_err().to_string().contains(error),
            "{error}"
        );
    }
    // Selection still restricts the inventory without a per-source quota.
    assert_eq!(build(&c, &spec()).unwrap().tools.len(), 1);
    c.selection.include.clear();
    assert_eq!(build(&c, &spec()).unwrap().tools.len(), 2);

    let cross_origin = json!({"openapi":"3.0.3", "servers":[{"url":"https://api.example.com"}],
        "paths":{"https://other.example.com/read":{"get":{}}}});
    assert!(
        build(&c, &cross_origin)
            .unwrap_err()
            .to_string()
            .contains("cross_origin_operation_rejected")
    );
}

#[test]
fn registry_ownership_aliases_corruption_and_inspection_do_not_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("registry.sqlite");
    assert!(store::inspect(&file).is_err());
    assert!(!file.exists());
    let store = store::Store::open(&file, &[]).unwrap();
    assert!(store::Store::open(&file, &[]).is_err());
    drop(store);
    assert!(store::Store::open(&file, std::slice::from_ref(&file)).is_err());
    let alias = tmp.path().join("alias");
    std::fs::hard_link(&file, &alias).unwrap();
    assert!(store::Store::open(&alias, std::slice::from_ref(&file)).is_err());
    let bad = tmp.path().join("bad");
    std::fs::write(&bad, b"not a database").unwrap();
    assert!(store::Store::open(&bad, &[]).is_err());
    assert_eq!(std::fs::read(&bad).unwrap(), b"not a database");
}

#[test]
fn existing_registry_rows_cannot_silently_default_to_empty_state() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("registry.sqlite");
    let store = store::Store::open(&file, &[]).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute("INSERT INTO snapshot(id,json) VALUES(1,'{}')", [])
        .unwrap();
    drop(db);
    assert!(store::Store::open(&file, &[]).unwrap().load().is_err());
    assert!(store::inspect(&file).is_err());
}

#[test]
#[ignore = "subprocess helper, explicitly invoked by crash recovery test"]
fn registry_crash_child() {
    let file = PathBuf::from(std::env::var("TREAZURY_TEST_REGISTRY").unwrap());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let m = manager(Some(file)).await;
        seed(&m).await;
        m.crash_after_commit
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = m
            .invoke(
                "writer",
                "x402_treazury_source_add",
                json!({"spec_url":"https://api.example.com/openapi.json","name":"crash"}),
            )
            .await;
        panic!("crash hook was not reached");
    });
}

#[tokio::test]
async fn process_exit_between_sqlite_commit_and_catalog_publication_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("registry.sqlite");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .env("TREAZURY_TEST_REGISTRY", &file)
        .args([
            "--ignored",
            "--exact",
            "discovery::tests::registry_crash_child",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(43),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let m = manager(Some(file)).await;
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    let r = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":"https://api.example.com/openapi.json","name":"crash"}),
        )
        .await
        .unwrap();
    assert_eq!(r["status"], "available");
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
}

#[tokio::test]
async fn production_public_transport_is_exercised_without_fixture_client_replacement() {
    let result = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "*")
        .args([
            "--ignored",
            "--exact",
            "discovery::tests::public_transport_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test]
#[ignore = "isolated immutable Tor policy; exercised by parent"]
async fn public_transport_child() {
    use crate::{
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
        test_socks::{Fault, Socks},
    };
    use axum::{
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use base64::Engine;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let requests = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let (u, s) = (requests.clone(), signed.clone());
    let large_catalog = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let large = large_catalog.clone();
    let (address,task)=crate::test_tls::serve(axum::Router::new()
        .route("/openapi.json",get(move || { let large = large.clone(); async move {
            let mut doc = spec();
            if large.load(Ordering::SeqCst) { for index in 0..511 { doc["paths"][format!("/extra-{index}")] = json!({"get":{}}); } }
            axum::Json(doc)
        }}))
        .route("/redirect",get(|| async {axum::response::Redirect::temporary("https://other.example.com/never")}))
        .route("/downgrade",get(|| async {axum::response::Redirect::temporary("http://127.0.0.1:1/never")}))
        .route("/read",get(move |headers:HeaderMap| {let (u,s)=(u.clone(),s.clone());async move {
            u.fetch_add(1,Ordering::SeqCst);
            if headers.contains_key("payment-signature") {s.fetch_add(1,Ordering::SeqCst);return "paid".into_response();}
            let challenge=json!({"x402Version":2,"resource":{"url":"https://api.example.com/read","description":"read","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"5000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (StatusCode::PAYMENT_REQUIRED,[("payment-required",base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
        }}))).await;
    let proxy = Socks::start(
        BTreeMap::from([
            ("spec.example.com".into(), address),
            ("api.example.com".into(), address),
        ]),
        Fault::None,
    )
    .await;
    let context = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(proxy.address),
        connect_timeout_seconds: Some(1),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    crate::network::install_test_context(context);
    let bytes = import::fetch_with(
        "https://spec.example.com/openapi.json",
        None,
        &PaidClient::unsigned(),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes.body).unwrap(),
        spec()
    );
    let payer = payer().public_destinations();
    let request = crate::catalog::RoutedRequest {
        method: "GET".into(),
        url: "https://api.example.com/read".into(),
        query: BTreeMap::new(),
        body: None,
    };
    assert_eq!(payer.execute(request).await.unwrap(), "paid");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(signed.load(Ordering::SeqCst), 1);
    for path in ["redirect", "downgrade"] {
        assert!(
            import::fetch_with(
                &format!("https://spec.example.com/{path}"),
                None,
                &PaidClient::unsigned()
            )
            .await
            .is_err()
        );
    }
    assert!(
        import::fetch_with(
            "https://127.0.0.1/openapi.json",
            None,
            &PaidClient::unsigned()
        )
        .await
        .is_err()
    );
    let ctx = crate::network::global();
    let signer: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
    let evm_id = IsolationId::evm(&signer.address().to_string()).unwrap();
    let evm_transport = ctx
        .http_participating(
            &evm_id,
            "https://api.example.com/read",
            None,
            true,
            Default::default(),
        )
        .unwrap();
    let discovery_id = IsolationId::discovery("https://spec.example.com").unwrap();
    let discovery_transport = ctx
        .http_participating(
            &discovery_id,
            "https://spec.example.com/openapi.json",
            import::read_timeout(None),
            true,
            Default::default(),
        )
        .unwrap();
    assert_eq!(discovery_transport.identity, discovery_id);
    assert_eq!(
        discovery_transport.selection(),
        crate::network::SelectionPolicy::Participating
    );
    assert_ne!(
        ctx.credentials(&discovery_transport.scope).1,
        ctx.credentials(&discovery_id).1
    );
    let ids = [
        discovery_transport.scope.clone(),
        evm_transport.scope.clone(),
    ];
    {
        let records = proxy.records.lock().unwrap();
        assert_eq!(
            records.len(),
            2,
            "redirect or private URL caused an extra connection"
        );
        for (record, id) in records.iter().zip(ids) {
            assert_eq!(record.address_type, 3);
            assert_eq!(
                (record.user.clone(), record.password.clone()),
                ctx.credentials(&id)
            );
        }
    }
    // Operator refresh uses the same public/Tor import path without loading keys.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let config = operator_config(dir.path(), &file);
    let mut config_value: toml::Value =
        toml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    config_value.as_table_mut().unwrap().insert(
        "network".into(),
        toml::Value::try_from(&ctx.policy).unwrap(),
    );
    std::fs::write(&config, toml::to_string(&config_value).unwrap()).unwrap();
    let m = manager(Some(file.clone())).await;
    let added = add(&m, "refresh", "", "").await;
    let id = registered_id(&m, &added);
    assert!(maintain_cli(&config, "writer", &id, true).await.is_err());
    drop(m);
    large_catalog.store(true, Ordering::SeqCst);
    maintain_cli(&config, "writer", &id, true).await.unwrap();
    let saved = store::Store::open(&file, &[]).unwrap().load().unwrap();
    assert_eq!(saved.records[&id].revision, 2);
    assert_eq!(
        serde_json::from_str::<Value>(&saved.records[&id].document).unwrap()["paths"]
            .as_object()
            .unwrap()
            .len(),
        513
    );
    // Refresh and runtime restoration use the same filtered inventory.
    config_value["servers"]["writer"]
        .as_table_mut()
        .unwrap()
        .insert("tags".into(), toml::Value::try_from(vec!["read"]).unwrap());
    std::fs::write(&config, toml::to_string(&config_value).unwrap()).unwrap();
    maintain_cli(&config, "writer", &id, true).await.unwrap();
    let mut filtered_listeners = listeners(true);
    filtered_listeners.get_mut("writer").unwrap().tags = vec!["read".into()];
    let restored = manager_with(with_registry(filtered_listeners, Some(file.clone()))).await;
    assert_eq!(restored.catalog.read().views["writer"].len(), 1);
    assert_eq!(
        restored.catalog.read().views["writer"][0].tool.name,
        "dyn_refresh__read"
    );
    assert_eq!(
        store::inspect(&file).unwrap()["sources"][0]["tools"]
            .as_array()
            .unwrap()
            .len(),
        513
    );
    drop(restored);

    // On-demand listeners also retain the complete inventory.
    config_value["servers"]["writer"]
        .as_table_mut()
        .unwrap()
        .remove("tags");
    config_value["servers"]["writer"]
        .as_table_mut()
        .unwrap()
        .insert("discover_on_demand".into(), toml::Value::Boolean(true));
    std::fs::write(&config, toml::to_string(&config_value).unwrap()).unwrap();
    maintain_cli(&config, "writer", &id, true).await.unwrap();
    assert_eq!(
        store::Store::open(&file, &[])
            .unwrap()
            .load()
            .unwrap()
            .records[&id]
            .revision,
        4
    );
    task.abort();
}

#[cfg(unix)]
#[test]
fn registry_protected_aliases_and_sidecars_preserve_targets() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for suffix in ["", ".owner.lock", "-wal", "-shm", "-journal"] {
        for hard in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let protected = tmp.path().join("protected");
            std::fs::write(&protected, b"private sentinel").unwrap();
            std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o600)).unwrap();
            let file = tmp.path().join("registry.sqlite");
            let alias = tmp.path().join(format!("registry.sqlite{suffix}"));
            if hard {
                std::fs::hard_link(&protected, &alias).unwrap();
            } else {
                symlink(&protected, &alias).unwrap();
            }
            assert!(store::Store::open(&file, std::slice::from_ref(&protected)).is_err());
            assert_eq!(std::fs::read(&protected).unwrap(), b"private sentinel");
        }
    }
    for suffix in [".owner.lock", "-wal", "-shm", "-journal"] {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("registry.sqlite");
        let missing = tmp.path().join("missing");
        symlink(
            &missing,
            tmp.path().join(format!("registry.sqlite{suffix}")),
        )
        .unwrap();
        assert!(store::Store::open(&file, &[]).is_err());
        assert!(!missing.exists());
        assert!(!file.exists());
    }
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("protected");
    std::fs::create_dir(&parent).unwrap();
    symlink(&parent, tmp.path().join("alias")).unwrap();
    assert!(
        store::Store::open(
            &tmp.path().join("alias/../protected/registry.sqlite"),
            std::slice::from_ref(&parent)
        )
        .is_err()
    );
    assert!(!parent.join("registry.sqlite").exists());
}

#[path = "../../tests/support/import_limits.rs"]
mod import_limits;
#[path = "../../tests/support/mcp_execution.rs"]
mod mcp_execution;

#[tokio::test]
async fn endpoint_local_add_is_duplicate_safe_and_tools_are_minimal() {
    use rmcp::ServerHandler;
    let m = manager(None).await;
    let args = json!({"spec_url":"https://api.example.com/openapi.json"});
    seed(&m).await;
    let results = futures_util::future::join_all(
        (0..8).map(|_| m.invoke("writer", "x402_treazury_source_add", args.clone())),
    )
    .await;
    let first = results[0].as_ref().unwrap();
    assert!(results.iter().all(|r| r.as_ref().unwrap() == first));
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    assert!(m.catalog.read().views["reader"].is_empty());
    let other = m
        .invoke("reader", "x402_treazury_source_add", args.clone())
        .await
        .unwrap();
    assert_eq!(first["name"], other["name"]);
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
    assert!(
        m.invoke("hidden", "x402_treazury_source_add", args)
            .await
            .is_err()
    );
    let s = server(&m, "writer");
    for name in [
        "providers_list",
        "sources_search",
        "source_details",
        "source_add",
        "raw_call",
        "tools_search",
        "tool_call",
    ] {
        assert!(s.get_tool(&format!("x402_treazury_{name}")).is_some());
    }
    for name in [
        "source_url_info",
        "raw_get",
        "raw_post",
        "source_preview",
        "sources_list",
        "source_update",
        "source_remove",
    ] {
        assert!(s.get_tool(&format!("x402_treazury_{name}")).is_none());
        assert!(
            m.invoke("writer", &format!("x402_treazury_{name}"), json!({}))
                .await
                .is_err()
        );
    }
    for extra in [
        "targets",
        "visibility",
        "lifetime",
        "idempotency_key",
        "candidate",
        "wallet",
        "selection",
    ] {
        let mut args = json!({"spec_url":"https://api.example.com/openapi.json"});
        args[extra] = json!("forbidden");
        assert!(
            m.invoke("writer", "x402_treazury_source_add", args)
                .await
                .is_err(),
            "{extra}"
        );
    }
}

#[tokio::test]
async fn tool_ids_survive_revision_and_process_while_cursors_expire() {
    let m = manager(None).await;
    add(&m, "demo", "", "").await;
    let search = m
        .invoke("writer", "x402_treazury_tools_search", json!({"limit":1}))
        .await
        .unwrap();
    let reference = search["items"][0]["tool_id"].as_str().unwrap();
    assert!(m.find_tool("writer", reference).is_ok());
    assert!(m.find_tool("reader", reference).is_err());
    assert!(
        m.invoke(
            "reader",
            "x402_treazury_tools_search",
            json!({"cursor":search["next_cursor"]})
        )
        .await
        .is_err()
    );
    let mut snapshot = (*m.catalog.read()).clone();
    for b in snapshot.views.get_mut("writer").unwrap() {
        b.source.as_mut().unwrap().1 += 1;
    }
    snapshot.generation += 1;
    m.catalog.publish(snapshot);
    assert!(m.find_tool("writer", reference).is_ok());
    assert!(
        m.invoke(
            "writer",
            "x402_treazury_tools_search",
            json!({"cursor":search["next_cursor"]})
        )
        .await
        .is_err()
    );
    let another = manager(None).await;
    add(&another, "demo", "", "").await;
    assert!(another.find_tool("writer", reference).is_ok());
}

#[tokio::test]
async fn registry_persistence_is_operator_controlled_and_revocation_is_local() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let m = manager(Some(file.clone())).await;
    let first = add(&m, "saved", "", "").await;
    assert_eq!(first["status"], "available");
    let tool_id = m.catalog.read().views["writer"][0].tool.name.clone();
    drop(m);
    let m = manager(Some(file.clone())).await;
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    // No seed or network fetch: repeated add returns persisted registration.
    let again = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":"https://api.example.com/openapi.json"}),
        )
        .await
        .unwrap();
    assert_eq!(again, first);
    assert!(m.find_tool("writer", &tool_id).is_ok());
    drop(m);
    let mut ls = listeners(true);
    ls.get_mut("writer").unwrap().source_management = false;
    let m = manager_with(with_registry(ls, Some(file))).await;
    assert!(m.catalog.read().views.values().all(Vec::is_empty));
    assert!(
        m.inner
            .lock()
            .unwrap()
            .state
            .records
            .values()
            .next()
            .unwrap()
            .disabled
            .is_some()
    );
    let temporary = manager(None).await;
    assert_eq!(
        add(&temporary, "temporary", "", "").await["status"],
        "available"
    );
}

#[tokio::test]
async fn failed_persistence_never_publishes_and_committed_add_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let m = manager(Some(file.clone())).await;
    seed(&m).await;
    m.fail_after_commit
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let args = json!({"spec_url":"https://api.example.com/openapi.json"});
    assert!(
        m.invoke("writer", "x402_treazury_source_add", args.clone())
            .await
            .is_err()
    );
    assert!(m.catalog.read().views["writer"].is_empty());
    drop(m);
    let m = manager(Some(file.clone())).await;
    assert_eq!(
        m.invoke("writer", "x402_treazury_source_add", args)
            .await
            .unwrap()["status"],
        "available"
    );
    // A failed SQLite transaction must not publish another registration.
    m.inner
        .lock()
        .unwrap()
        .stores
        .get("writer")
        .unwrap()
        .reject_writes();
    m.inner
        .lock()
        .unwrap()
        .stores
        .get("reader")
        .unwrap()
        .reject_writes();
    seed_for(&m, "reader").await;
    assert!(
        m.invoke(
            "reader",
            "x402_treazury_source_add",
            json!({"spec_url":"https://api.example.com/openapi.json"})
        )
        .await
        .is_err()
    );
    assert!(m.catalog.read().views["reader"].is_empty());
}

#[path = "../../tests/support/dynamic_scope.rs"]
mod dynamic_scope;

#[tokio::test]
async fn endpoint_wallet_override_and_filters_preserve_boundaries() {
    let mut ls = listeners(false);
    ls.get_mut("reader").unwrap().wallet = Some("other".into());
    ls.get_mut("reader").unwrap().tags = vec!["read".into()];
    let shared = payer();
    let other = payer();
    let snapshot = CatalogSnapshot {
        instructions: Default::default(),
        generation: 0,
        views: ls.keys().map(|id| (id.clone(), vec![])).collect(),
    };
    let m = Manager::new(
        ls,
        BTreeMap::from([
            ("shared".into(), shared.clone()),
            ("other".into(), other.clone()),
        ]),
        Arc::new(CatalogState::new(snapshot)),
        vec![],
    )
    .await
    .unwrap();
    add(&m, "first", "", "").await;
    // The reader has a distinct retained authority even though this fixture's
    // two public deterministic keys have the same address. Seed its exact key.
    seed_for(&m, "reader").await;
    assert_ne!(
        fetch_key_for(&m, "writer", "https://api.example.com/openapi.json"),
        fetch_key_for(&m, "reader", "https://api.example.com/openapi.json")
    );
    m.invoke(
        "reader",
        "x402_treazury_source_add",
        json!({"spec_url":"https://api.example.com/openapi.json"}),
    )
    .await
    .unwrap();
    let snap = m.catalog.read();
    assert_eq!(snap.views["writer"].len(), 2);
    assert_eq!(snap.views["reader"].len(), 1);
    assert!(snap.views["writer"][0].client.shares_profile_with(&shared));
    assert!(snap.views["reader"][0].client.shares_profile_with(&other));
    assert!(!snap.views["reader"][0].client.shares_profile_with(&shared));
}

#[tokio::test]
async fn real_http_directory_search_and_cached_client_workflow() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let (vendor, vendor_task)=listen(axum::Router::new().route("/services",axum::routing::get(move |axum::extract::Query(query):axum::extract::Query<BTreeMap<String,String>>| {
        count.fetch_add(1,Ordering::SeqCst);
        async move { axum::Json(json!({"query":query,"services":[{"spec_url":"https://api.example.com/openapi.json"}]})) }
    }))).await;
    let m = manager(None).await;
    *m.fixture_directory.lock().unwrap() = Some(vendor);
    assert!(m.catalog.read().views["writer"].is_empty());
    let (base, task) = listen(crate::server::http_app(
        server(&m, "writer"),
        "test-token".into(),
    ))
    .await;
    let denied = crate::network::discovery(&base,Duration::from_secs(5)).unwrap()
        .post(format!("{base}/mcp")).bearer_auth("wrong-token")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"x402_treazury_sources_search","arguments":{"q":"weather"}}}))
        .send().await.unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    let found = call(
        &base,
        "x402_treazury_sources_search",
        json!({"q":"weather"}),
    )
    .await;
    assert_ne!(found["isError"], true, "{found}");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    seed(&m).await;
    let added = call(
        &base,
        "x402_treazury_source_add",
        json!({"spec_url":"https://api.example.com/openapi.json","name":"demo"}),
    )
    .await;
    assert_eq!(added["structuredContent"]["tool_prefix"], "dyn_demo__");
    let signatures = call(
        &base,
        "x402_treazury_tools_search",
        json!({"source_name":"demo"}),
    )
    .await;
    let item = &signatures["structuredContent"]["items"][0];
    assert!(item["input_schema"].is_object());
    assert!(item["tool_id"].is_string());
    assert_eq!(
        call(
            &base,
            "x402_treazury_tool_call",
            json!({"tool_id":"stale","arguments":{}})
        )
        .await["isError"],
        true
    );
    task.abort();
    vendor_task.abort();
}

#[tokio::test]
async fn shared_managed_registration_does_not_allocate_or_fund() {
    use crate::rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{Store, StoreHandle},
    };
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &tmp.path().join("state"),
        &tmp.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        store,
        "shared",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let before = serde_json::to_value(store.status().unwrap()).unwrap();
    let (store, worker) = StoreHandle::spawn(store);
    let managed = PaidClient::managed(Arc::new(
        ManagedPool::new(
            store.clone(),
            pool,
            BaseRpc::new("http://127.0.0.1:1/rpc", 12).unwrap(),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    ));
    let registry = tmp.path().join("registry.sqlite");
    let ls = with_registry(listeners(true), Some(registry.clone()));
    let (url, task) = listen(
        axum::Router::new().route("/spec", axum::routing::get(|| async { axum::Json(spec()) })),
    )
    .await;
    let snapshot = CatalogSnapshot {
        instructions: Default::default(),
        generation: 0,
        views: ls.keys().map(|k| (k.clone(), vec![])).collect(),
    };
    let m = Manager::new(
        ls.clone(),
        BTreeMap::from([("shared".into(), managed.clone())]),
        Arc::new(CatalogState::new(snapshot)),
        vec![],
    )
    .await
    .unwrap();
    *m.fixture_endpoint.lock().unwrap() = Some(format!("{url}/spec"));
    add(&m, "one", "", "").await;
    let curated = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"provider_id":"relayshield-phone-reputation"}),
        )
        .await
        .unwrap();
    assert_eq!(curated["schema"]["kind"], "bundled");
    m.invoke(
        "reader",
        "x402_treazury_source_add",
        json!({"spec_url":"https://api.example.com/openapi.json"}),
    )
    .await
    .unwrap();
    let after = store
        .call(|s| Ok(serde_json::to_value(s.status()?)?))
        .await
        .unwrap();
    assert_eq!(before, after);
    drop(m);
    let reopened = Manager::new(
        ls.clone(),
        BTreeMap::from([("shared".into(), managed)]),
        Arc::new(CatalogState::new(CatalogSnapshot {
            instructions: Default::default(),
            generation: 0,
            views: ls.keys().map(|id| (id.clone(), vec![])).collect(),
        })),
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(
        reopened
            .inner
            .lock()
            .unwrap()
            .state
            .records
            .values()
            .filter(|r| !r.removed)
            .count(),
        3
    );
    assert_eq!(
        before,
        store
            .call(|s| Ok(serde_json::to_value(s.status()?)?))
            .await
            .unwrap()
    );
    drop(reopened);
    task.abort();
    drop(store);
    worker.await.unwrap();
}

fn operator_config(dir: &std::path::Path, file: &std::path::Path) -> PathBuf {
    let path = dir.join("deployment.toml");
    let config = json!({"version":1,"wallets":{"shared":{"mode":"static","private_key_env":"UNUSED_KEY"}},
        "servers":{"writer":{"listen":"127.0.0.1:8000","bearer_token_env":"UNUSED_TOKEN","source_management":true,"wallet":"shared","source_registry_file":file}}});
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    path
}
#[tokio::test]
async fn operator_removal_requires_exclusive_ownership_and_allows_readdition() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let config = operator_config(dir.path(), &file);
    let m = manager(Some(file.clone())).await;
    let first = add(&m, "saved", "", "").await;
    let id = registered_id(&m, &first);
    assert!(maintain_cli(&config, "writer", &id, false).await.is_err());
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    drop(m);
    assert!(maintain_cli(&config, "reader", &id, false).await.is_err());
    maintain_cli(&config, "writer", &id, false).await.unwrap();
    let m = manager(Some(file)).await;
    assert!(m.catalog.read().views["writer"].is_empty());
    assert!(m.inner.lock().unwrap().state.records[&id].removed);
    let second = add(&m, "saved", "", "").await;
    assert_ne!(id, registered_id(&m, &second));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)] // Stall only the blocking commit worker to cancel its waiter.
async fn accepted_add_commits_after_waiter_cancellation() {
    let m = manager(None).await;
    add(&m, "initial", "", "").await;
    let lock = m.inner.lock().unwrap();
    let mut record = lock.state.records.values().next().unwrap().clone();
    record.id = Uuid::new_v4().to_string();
    record.candidate.name = "second".into();
    record.candidate.spec_url = "https://api.example.com/second.json".into();
    let worker = m.clone();
    let task = tokio::spawn(async move { worker.commit("writer".into(), false, record).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while m.mutations.available_permits() == 16 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    drop(lock);
    tokio::time::timeout(Duration::from_secs(5), async {
        while m.catalog.read().views["writer"].len() != 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
}

#[tokio::test]
async fn boolean_config_and_shared_wallet_bindings_are_strict_and_offline() {
    for managed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deployment.toml");
        let wallet = if managed {
            "mode='zcash_rotation'\nmax_api_payment_usdc='0.05'\nmax_conversion_overhead_percent=5\n[wallets.agent_shared.reserves.small]\nfunding_amount_usdc='2.00'"
        } else {
            "mode='static'\nprivate_key_env='UNSET_TEST_KEY'"
        };
        std::fs::write(
            &path,
            format!(
                r#"version=1
[treasury]
state_dir='state'
[funding]
confidentiality='public'
[wallets.agent_shared]
{wallet}
[servers.research]
listen='127.0.0.1:0'
bearer_token_env='UNSET_RESEARCH_TOKEN'
source_management=true
wallet="agent_shared"
source_registry_file="registry.sqlite"
[servers.analysis]
listen='127.0.0.1:0'
bearer_token_env='UNSET_ANALYSIS_TOKEN'
source_management=true
wallet="agent_shared"
source_registry_file="registry.sqlite"
"#
            ),
        )
        .unwrap();
        let shown = crate::deployment::Deployment::show_config(&path)
            .await
            .unwrap();
        for endpoint in ["research", "analysis"] {
            assert_eq!(shown["servers"][endpoint]["source_management"], true);
            assert_eq!(shown["servers"][endpoint]["wallet"], "agent_shared");
        }
        assert!(!dir.path().join("registry.sqlite").exists());
        assert!(!dir.path().join("state").exists());
    }
    for value in [json!({"enabled":true}), json!("manage")] {
        assert!(
            serde_json::from_value::<ListenerConfig>(
                json!({"listen":"127.0.0.1:0","source_management":value})
            )
            .is_err()
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-created.sqlite");
    let config = operator_config(dir.path(), &file);
    crate::deployment::Deployment::show_config(&config)
        .await
        .unwrap();
    assert!(!file.exists());
    assert!(
        maintain_cli(&config, "writer", "missing", false)
            .await
            .is_err()
    );
    assert!(!file.exists());
}

#[tokio::test]
async fn saved_nonlocal_bindings_stay_disabled_and_duplicate_add_is_readable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let m = manager(Some(file.clone())).await;
    let added = add(&m, "old", "", "").await;
    let added_id = registered_id(&m, &added);
    {
        let mut inner = m.inner.lock().unwrap();
        let mut state = inner.state.clone();
        state
            .records
            .get_mut(&added_id)
            .unwrap()
            .targets
            .push("removed_endpoint".into());
        inner
            .stores
            .get_mut("writer")
            .unwrap()
            .save(&state)
            .unwrap();
    }
    drop(m);
    let m = manager(Some(file)).await;
    let result = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":"https://api.example.com/openapi.json"}),
        )
        .await
        .unwrap();
    assert_eq!(result["status"], "disabled");
    assert!(result["reason"].is_string());
    assert_eq!(result["tool_count"], 0);
    assert!(m.catalog.read().views.values().all(Vec::is_empty));
}

#[test]
fn embedded_directory_matches_reviewed_search_contract() {
    let cfg: crate::catalog::Config =
        toml::from_str(include_str!("../../providers/x402-list.toml")).unwrap();
    assert_eq!(cfg.base_url.as_deref(), Some(directory::BASE));
    let doc =
        serde_json::from_str(include_str!("../../tests/fixtures/x402_list_openapi.json")).unwrap();
    let expected = crate::catalog::build_tools(&cfg, &doc, "x402_list").unwrap();
    for path in ["/services", "/best", "/services/{slug}"] {
        let expected = expected.iter().find(|t| t.path == path).unwrap();
        assert_eq!(
            serde_json::to_value(directory::tool(path)).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[tokio::test]
async fn management_only_deployment_starts_and_lists_exactly_seven_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
version = 1
[wallets.shared]
mode = "static"
private_key_env = "TEST_KEY"
[servers.research]
listen = "127.0.0.1:0"
bearer_token_env = "TEST_TOKEN"
source_management = true
wallet = "shared"
# API filters must not hide management tools.
exclude_tools = ["*"]
"#,
    )
    .unwrap();
    let deployment = crate::deployment::Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    assert_eq!(inventory[0].management_tools.len(), 7);
    assert!(inventory[0].tools.is_empty());
    let running = deployment
        .bind(&BTreeMap::from([
            ("TEST_KEY".into(), format!("{:064x}", 1)),
            ("TEST_TOKEN".into(), "test-token".into()),
        ]))
        .await
        .unwrap();
    let address = running.addresses()[0].1;
    let stop = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    let listed = rpc(
        &format!("http://{address}"),
        "test-token",
        "tools/list",
        json!({}),
    )
    .await;
    let actual = listed["tools"].as_array().unwrap();
    assert_eq!(actual.len(), 7);
    for expected in &inventory[0].management_tools {
        assert!(actual.contains(&serde_json::to_value(expected).unwrap()));
    }
    assert!(
        actual
            .iter()
            .all(|t| t["name"].as_str().unwrap().starts_with("x402_treazury_"))
    );
    stop.cancel();
    task.await.unwrap().unwrap();
}

#[test]
fn automatic_directory_uses_endpoint_wallet_caps_and_tor() {
    if std::env::var_os("TREAZURY_DIRECTORY_CHILD").is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(directory_payment_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "discovery::tests::automatic_directory_uses_endpoint_wallet_caps_and_tor",
            "--nocapture",
        ])
        .env("TREAZURY_DIRECTORY_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn directory_payment_fixture() {
    use crate::{
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
        test_socks::{Fault, Socks},
    };
    use axum::{
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let requests = Arc::new(AtomicUsize::new(0));
    let signatures = Arc::new(Mutex::new(Vec::new()));
    let unsigned_faults = Arc::new(Mutex::new(std::collections::VecDeque::<StatusCode>::new()));
    let (count, signed, faults) = (
        requests.clone(),
        signatures.clone(),
        unsigned_faults.clone(),
    );
    let seller = move |headers: HeaderMap,
                       method: axum::http::Method,
                       uri: axum::http::Uri,
                       body: axum::body::Bytes| {
        let (count, signed, faults) = (count.clone(), signed.clone(), faults.clone());
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            if !headers.contains_key("payment-signature")
                && let Some(code) = faults.lock().unwrap().pop_front()
            {
                return (code, "transient exit rejection").into_response();
            }
            if uri.path() == "/raw/redirect" {
                return (
                    StatusCode::FOUND,
                    [("location", "https://api.example.com/api/v1/services")],
                )
                    .into_response();
            }
            if let Some(header) = headers.get("payment-signature") {
                let payload =
                    serde_json::from_slice(&STANDARD.decode(header.as_bytes()).unwrap()).unwrap();
                signed
                    .lock()
                    .unwrap()
                    .push(crate::test_signatures::recover_exact(&payload).to_string());
                match uri.path() {
                    "/raw/image" => {
                        return (
                            [("content-type", "image/png")],
                            b"\x89PNG\r\n\x1a\nfixture".to_vec(),
                        )
                            .into_response();
                    }
                    "/raw/bad-image" => {
                        return ([("content-type", "image/png")], "invalid image").into_response();
                    }
                    "/raw/large" => {
                        return (
                            [("content-type", "text/plain")],
                            "x".repeat(crate::limits::RESPONSE_BYTES + 1),
                        )
                            .into_response();
                    }
                    _ => {}
                }
                return axum::Json(json!({"services":[], "method":method.as_str(), "query":uri.query(), "body":if body.is_empty() { Value::Null } else { serde_json::from_slice::<Value>(&body).unwrap() }, "content_type":headers.get("content-type").map(|v| v.to_str().unwrap())})).into_response();
            }
            let challenge = json!({"x402Version":2,"resource":{"url":format!("https://api.example.com{}", uri.path()),"description":"search","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"5000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (
                StatusCode::PAYMENT_REQUIRED,
                [(
                    "payment-required",
                    STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
                )],
            )
                .into_response()
        }
    };
    let app = axum::Router::new()
        .route("/api/v1/services", axum::routing::any(seller.clone()))
        .route("/api/v1/best", get(seller.clone()))
        .route("/api/v1/services/{slug}", get(seller.clone()))
        .route("/raw/{kind}", get(seller));
    let (address, vendor_task) = crate::test_tls::serve(app).await;
    let proxy = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let mut ls = listeners(false);
    ls.get_mut("reader").unwrap().wallet = Some("split".into());
    let mut blocked = ls["writer"].clone();
    blocked.wallet = Some("capped".into());
    ls.insert("blocked".into(), blocked);
    let clients = [
        (1, "shared", "0.01"),
        (2, "split", "0.01"),
        (3, "capped", "0.001"),
    ]
    .into_iter()
    .map(|(key, name, cap)| {
        (
            name.into(),
            PaidClient::new(
                Payer::new(&format!("{key:064x}"), SpendPolicy::dollars(cap).unwrap()).unwrap(),
            ),
        )
    })
    .collect();
    let snapshot = CatalogSnapshot {
        instructions: Default::default(),
        views: ls.keys().map(|name| (name.clone(), vec![])).collect(),
        ..Default::default()
    };
    let manager = Manager::new(ls, clients, Arc::new(CatalogState::new(snapshot)), vec![])
        .await
        .unwrap();
    assert_eq!(manager.directory("writer").unwrap().base, directory::BASE);
    // Use the public hostname on the fixture certificate; retain public-destination checks.
    *manager.fixture_directory.lock().unwrap() = Some("https://api.example.com/api/v1".into());
    assert!(manager.directory("hidden").is_err());
    assert_eq!(tools::definitions(true).len(), 7);
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "initialization fetched a catalog or probed prices"
    );
    let calls = [
        (directory::SEARCH, json!({})),
        (
            directory::SEARCH,
            json!({"mode":"best","q":"weather","limit":3}),
        ),
        (directory::DETAILS, json!({"slug":"weather"})),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://api.example.com/api/v1/services?existing=yes","query":{"q":"a & b","tags":["one","two"],"enabled":true,"limit":2}}),
        ),
        (
            raw::NAME,
            json!({"method":"POST","url":"https://api.example.com/api/v1/services","query":{"q":"post"},"body":{"items":[1,"two"],"nested":{"ok":true}}}),
        ),
        (
            raw::NAME,
            json!({"method":"POST","url":"https://api.example.com/api/v1/services"}),
        ),
        (
            raw::NAME,
            json!({"method":"POST","url":"https://api.example.com/api/v1/services","body":null}),
        ),
        (
            raw::NAME,
            json!({"method":"PUT","url":"https://api.example.com/api/v1/services","body":{"replace":true}}),
        ),
        (
            raw::NAME,
            json!({"method":"PATCH","url":"https://api.example.com/api/v1/services","body":{"patch":true}}),
        ),
        (
            raw::NAME,
            json!({"method":"DELETE","url":"https://api.example.com/api/v1/services"}),
        ),
        (
            raw::NAME,
            json!({"method":"OPTIONS","url":"https://api.example.com/api/v1/services"}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://api.example.com/api/v1/services","body":{"get_body":true}}),
        ),
    ];
    for owner in ["writer", "reader"] {
        for (name, args) in &calls {
            let result = server(&manager, owner)
                .invoke(name, args.as_object().unwrap())
                .await
                .unwrap();
            assert!(result.contains("services"));
            let response: Value = serde_json::from_str(&result).unwrap();
            if *name == raw::NAME {
                assert_eq!(response["method"], args["method"]);
                assert_eq!(
                    response["body"],
                    args.get("body").cloned().unwrap_or(Value::Null)
                );
                if args.get("body").is_some() {
                    assert_eq!(response["content_type"], "application/json");
                } else {
                    assert!(response["content_type"].is_null());
                }
                if args.get("query").is_some() && args["method"] == "GET" {
                    assert_eq!(
                        response["query"],
                        "existing=yes&enabled=true&limit=2&q=a+%26+b&tags=one&tags=two"
                    );
                }
            }
        }
    }
    for (name, args) in &calls {
        assert!(
            server(&manager, "blocked")
                .invoke(name, args.as_object().unwrap())
                .await
                .is_err()
        );
    }
    assert_eq!(
        requests.load(Ordering::SeqCst),
        calls.len() * 5,
        "cap failure must not sign or retry"
    );
    let addresses: Vec<_> = (1..=3)
        .map(|key| {
            let signer: alloy_signer_local::PrivateKeySigner =
                format!("{key:064x}").parse().unwrap();
            signer.address().to_string()
        })
        .collect();
    assert_eq!(
        *signatures.lock().unwrap(),
        addresses[..2]
            .iter()
            .flat_map(|a| vec![a.clone(); calls.len()])
            .collect::<Vec<_>>()
    );
    {
        let records = proxy.records.lock().unwrap();
        assert_eq!(records.len(), 3);
        for (record, address) in records.iter().zip(&addresses) {
            assert_eq!(record.address_type, 3);
            let id = IsolationId::evm(address).unwrap();
            let transport = crate::network::global()
                .http_participating(
                    &id,
                    "https://api.example.com",
                    Some(Duration::from_secs(60)),
                    true,
                    Default::default(),
                )
                .unwrap();
            assert_eq!(
                (record.user.clone(), record.password.clone()),
                crate::network::global().credentials(&transport.scope)
            );
            assert_ne!(record.password, crate::network::global().credentials(&id).1);
        }
    }
    // Exercise real MCP typed output and ensure output failures never replay paid work.
    let (base, http_task) = listen(crate::server::http_app(
        server(&manager, "writer"),
        "test-token".into(),
    ))
    .await;
    let local = NetworkContext::new(Default::default())
        .unwrap()
        .http(
            &IsolationId::discovery(&base).unwrap(),
            &base,
            Duration::from_secs(5),
        )
        .unwrap();
    let invoke_raw = |url: String| {
        let request = local.post(format!("{base}/mcp"))
            .bearer_auth("test-token")
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":raw::NAME,"arguments":{"method":"GET","url":url}}}));
        async move {
            request
                .send()
                .await
                .unwrap()
                .mcp_json::<Value>()
                .await
                .unwrap()["result"]
                .clone()
        }
    };
    let before = requests.load(Ordering::SeqCst);
    let signed_before = signatures.lock().unwrap().len();
    let image = invoke_raw("https://api.example.com/raw/image".into()).await;
    assert_ne!(image["isError"], true, "{image}");
    assert!(
        image["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["type"] == "image" && c["mimeType"] == "image/png")
    );
    for path in ["bad-image", "large"] {
        let result = invoke_raw(format!("https://api.example.com/raw/{path}")).await;
        assert_eq!(result["isError"], true, "{result}");
    }
    assert_eq!(requests.load(Ordering::SeqCst), before + 6);
    assert_eq!(signatures.lock().unwrap().len(), signed_before + 3);
    let result = invoke_raw("https://api.example.com/raw/redirect".into()).await;
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(
        requests.load(Ordering::SeqCst),
        before + 7,
        "redirect was followed"
    );
    assert_eq!(signatures.lock().unwrap().len(), signed_before + 3);

    // Directory search and raw_post recover from eligible unsigned Tor status rejections
    // on fresh isolation scopes and promote the winning challenge scope.
    let writer_id = IsolationId::evm(&addresses[0]).unwrap();
    let pre_retry_scope = crate::network::global()
        .http_participating(
            &writer_id,
            "https://api.example.com",
            Some(Duration::from_secs(60)),
            true,
            Default::default(),
        )
        .unwrap()
        .scope
        .clone();
    unsigned_faults
        .lock()
        .unwrap()
        .push_back(StatusCode::SERVICE_UNAVAILABLE);
    let dir_res = server(&manager, "writer")
        .invoke(directory::SEARCH, json!({}).as_object().unwrap())
        .await
        .unwrap();
    assert!(dir_res.contains("services"));
    let post_dir_scope = crate::network::global()
        .http_participating(
            &writer_id,
            "https://api.example.com",
            Some(Duration::from_secs(60)),
            true,
            Default::default(),
        )
        .unwrap()
        .scope
        .clone();
    assert_ne!(
        pre_retry_scope, post_dir_scope,
        "directory search unsigned retry must promote the winning candidate scope"
    );
    unsigned_faults
        .lock()
        .unwrap()
        .push_back(StatusCode::FORBIDDEN);
    let raw_post_res = server(&manager, "writer")
        .invoke(
            raw::NAME,
            json!({"method":"POST","url":"https://api.example.com/api/v1/services","body":{"retry":true}})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(raw_post_res.contains("services"));
    let post_raw_scope = crate::network::global()
        .http_participating(
            &writer_id,
            "https://api.example.com",
            Some(Duration::from_secs(60)),
            true,
            Default::default(),
        )
        .unwrap()
        .scope
        .clone();
    assert_ne!(
        post_dir_scope, post_raw_scope,
        "raw_post unsigned retry must promote the new winning candidate scope"
    );
    assert_eq!(
        requests.load(Ordering::SeqCst),
        before + 7 + 6,
        "each call makes 1 failed unsigned + 1 winning 402 + 1 signed 200"
    );
    assert_eq!(signatures.lock().unwrap().len(), signed_before + 3 + 2);

    http_task.abort();
    vendor_task.abort();
}

#[tokio::test]
async fn directory_modes_and_details_dispatch_once_and_reject_mixed_arguments() {
    use axum::{
        extract::{Path, Query},
        routing::get,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    let requests = Arc::new(AtomicUsize::new(0));
    let (browse, best, details) = (requests.clone(), requests.clone(), requests.clone());
    let app = axum::Router::new()
        .route(
            "/services",
            get(move |Query(args): Query<BTreeMap<String, String>>| {
                browse.fetch_add(1, Ordering::SeqCst);
                async move { axum::Json(json!({"route":"browse","args":args})) }
            }),
        )
        .route(
            "/best",
            get(move |Query(args): Query<BTreeMap<String, String>>| {
                best.fetch_add(1, Ordering::SeqCst);
                async move { axum::Json(json!({"route":"best","args":args,"ranking_version":3})) }
            }),
        )
        .route(
            "/services/{slug}",
            get(move |Path(slug): Path<String>| {
                details.fetch_add(1, Ordering::SeqCst);
                async move {
                    axum::Json(
                        json!({"route":"details","slug":slug,"endpoints":[{"path":"/read"}]}),
                    )
                }
            }),
        );
    let (vendor, vendor_task) = listen(app).await;
    let m = manager(None).await;
    *m.fixture_directory.lock().unwrap() = Some(vendor);
    let (base, task) = listen(crate::server::http_app(
        server(&m, "writer"),
        "test-token".into(),
    ))
    .await;
    let invoke = |name, args| call(&base, name, args);
    let result = invoke(
        directory::SEARCH,
        json!({"q":"weather","page":2,"per_page":7}),
    )
    .await;
    let parsed: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["route"], "browse");
    assert_eq!(
        parsed["args"],
        json!({"q":"weather","page":"2","per_page":"7","network":"BSE"})
    );
    let result = invoke(directory::SEARCH, json!({"mode":"best","q":"weather","prefer":"cheapest","max_price_usd":0.02,"require_verified":true,"limit":3})).await;
    let parsed: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["route"], "best");
    assert_eq!(parsed["ranking_version"], 3);
    assert_eq!(
        parsed["args"],
        json!({"q":"weather","network":"BSE","prefer":"cheapest","max_price_usd":"0.02","require_verified":"true","limit":"3"})
    );
    let result = invoke(
        directory::DETAILS,
        json!({"slug":"vendor/weather?region=UK"}),
    )
    .await;
    let parsed: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["listing"]["slug"], "vendor/weather?region=UK");
    assert_eq!(parsed["listing"]["endpoints"][0]["path"], "/read");
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    for (name, args) in [
        (directory::SEARCH, json!({"network":"BSE"})),
        (directory::SEARCH, json!({"mode":"best","network":"SOL"})),
        (directory::SEARCH, json!({"mode":"best","page":1})),
        (
            directory::SEARCH,
            json!({"mode":"browse","prefer":"cheapest"}),
        ),
        (directory::SEARCH, json!({"max_price_usd":0.01})),
        (directory::SEARCH, json!({"mode":"best","limit":21})),
        (directory::SEARCH, json!({"mode":"best","max_price_usd":-1})),
        (
            directory::SEARCH,
            json!({"mode":"best","require_verified":"true"}),
        ),
        (directory::SEARCH, json!({"mode":"best","prefer":"invalid"})),
        (directory::SEARCH, json!({"mode":"invalid"})),
        (directory::SEARCH, json!({"q":null})),
        (directory::SEARCH, json!({"per_page":101})),
        (directory::DETAILS, json!({"slug":""})),
        (directory::DETAILS, json!({"slug":".."})),
        (directory::DETAILS, json!({"slug":"."})),
        (
            directory::DETAILS,
            json!({"source_id":"not-a-directory-slug"}),
        ),
        (directory::DETAILS, json!({"slug":"weather","mode":"best"})),
    ] {
        let result = invoke(name, args.clone()).await;
        assert_eq!(result["isError"], true, "{name} {args}: {result}");
    }
    assert_eq!(
        requests.load(Ordering::SeqCst),
        3,
        "invalid input made a provider request"
    );
    assert!(
        directory::request(
            directory::SEARCH,
            json!({"mode":"browse","q":"x"}).as_object().unwrap()
        )
        .unwrap()
        .1
        .get("mode")
        .is_none()
    );
    assert!(tools::definitions(false).is_empty());
    let schema = directory::search_schema();
    assert!(schema["properties"].get("network").is_none());
    for branch in schema["oneOf"].as_array().unwrap() {
        assert!(branch["properties"].get("network").is_none());
    }
    assert!(schema["properties"]["mode"].get("default").is_none());
    assert_eq!(schema["oneOf"][0]["properties"]["mode"]["const"], "browse");
    assert!(schema["oneOf"][0]["properties"].get("limit").is_none());
    assert_eq!(schema["oneOf"][1]["required"], json!(["mode"]));
    assert!(schema["oneOf"][1]["properties"].get("page").is_none());
    for path in ["/services", "/best"] {
        for (name, field) in directory::tool(path).input_schema["properties"]
            .as_object()
            .unwrap()
        {
            if name == "network" {
                continue;
            }
            let branch = if path == "/services" { 0 } else { 1 };
            let mut authored = schema["oneOf"][branch]["properties"][name].clone();
            assert!(
                authored["description"]
                    .as_str()
                    .is_some_and(|help| !help.is_empty())
            );
            authored["description"] = field["description"].clone();
            assert_eq!(authored, *field, "{path} {name}: branch constraint changed");
            let mut merged = schema["properties"][name].clone();
            if name == "per_page" {
                assert!(merged.get("maximum").is_none());
                assert_eq!(schema["oneOf"][2]["properties"]["per_page"]["default"], 25);
                assert!(
                    schema["oneOf"][2]["properties"]["per_page"]
                        .get("maximum")
                        .is_none()
                );
                continue;
            }
            assert!(
                merged["description"]
                    .as_str()
                    .is_some_and(|help| !help.is_empty())
            );
            merged["description"] = field["description"].clone();
            let mut expected = field.clone();
            expected.as_object_mut().unwrap().remove("default");
            if name == "max_price_usd" {
                assert_eq!(merged["type"], "number");
            }
            assert_eq!(merged, expected, "{path} {name}: schema constraint changed");
        }
    }
    task.abort();
    vendor_task.abort();
}

#[tokio::test]
async fn import_aliases_share_active_fetches_and_failed_fetches_recover() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    use std::sync::atomic::{AtomicUsize, Ordering};
    let m = manager(None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let app = axum::Router::new().fallback({
        let calls = calls.clone();
        let arrived = arrived.clone();
        let release = release.clone();
        move || {
            let calls = calls.clone();
            let arrived = arrived.clone();
            let release = release.clone();
            async move {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                arrived.notify_one();
                if call == 0 {
                    release.notified().await;
                    (axum::http::StatusCode::SERVICE_UNAVAILABLE, String::new())
                } else {
                    (axum::http::StatusCode::OK, spec().to_string())
                }
            }
        }
    });
    let (url, task) = listen(app).await;
    *m.fixture_endpoint.lock().unwrap() = Some(url);
    let mut aliases = tokio::task::JoinSet::new();
    for _ in 0..20 {
        let m = m.clone();
        aliases.spawn(async move { m.document("writer", "https://api.example.com/active").await });
    }
    arrived.notified().await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // A different URL on the same listener proceeds despite twenty coalesced waiters.
    assert!(
        m.document("writer", "https://api.example.com/other")
            .await
            .is_ok()
    );
    let original =
        m.fetches.lock().unwrap()[&fetch_key(&m, "https://api.example.com/active")].clone();
    // Churn completed retention beyond capacity; an active fetch is never evicted.
    for n in 0..12 {
        assert!(
            m.document("writer", &format!("https://api.example.com/churn{n}"))
                .await
                .is_ok()
        );
    }
    assert!(Arc::ptr_eq(
        &original,
        &m.fetches.lock().unwrap()[&fetch_key(&m, "https://api.example.com/active")]
    ));
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        logs.lines().any(|line| line.contains("DEBUG")
            && line.contains("Completed import cache capacity reached"))
    );
    assert!(logs.contains("limit_entries=4"));
    assert!(!logs.contains("https://api.example.com"));
    release.notify_one();
    while let Some(result) = aliases.join_next().await {
        assert!(result.unwrap().is_err());
    }
    let before = calls.load(Ordering::SeqCst);
    assert!(
        m.document("writer", "https://api.example.com/active")
            .await
            .is_err()
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "negative backoff prevents a retry storm"
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(
        m.document("writer", "https://api.example.com/active")
            .await
            .is_ok()
    );
    assert_eq!(calls.load(Ordering::SeqCst), before + 1);
    task.abort();
}

#[tokio::test]
async fn directory_http_calls_outlive_rmcp_drain_and_disconnected_waiters() {
    use axum::routing::get;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let arrived = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (seen, gate, count) = (arrived.clone(), release.clone(), calls.clone());
    let app = axum::Router::new().route(
        "/services",
        get(move || {
            let (seen, gate, count) = (seen.clone(), gate.clone(), count.clone());
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                seen.add_permits(1);
                gate.acquire().await.unwrap().forget();
                axum::Json(json!({"services":[]}))
            }
        }),
    );
    let (vendor, vendor_task) = listen(app).await;
    let m = manager(None).await;
    *m.fixture_directory.lock().unwrap() = Some(vendor);
    let server = server(&m, "writer");
    let work = server.work.clone();
    let (base, http_task) = listen(crate::server::http_app(server, "test-token".into())).await;
    for disconnect in [false, true] {
        let base = base.clone();
        let request = tokio::spawn(async move {
            crate::network::NetworkContext::new(Default::default()).unwrap()
                .discovery(&base, Duration::from_secs(3)).unwrap().post(format!("{base}/mcp"))
                .bearer_auth("test-token")
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":directory::SEARCH,"arguments":{"q":"fixture"}}}))
                .send().await.unwrap().mcp_json::<Value>().await.unwrap()["result"].clone()
        });
        tokio::time::timeout(Duration::from_secs(5), arrived.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        if disconnect {
            request.abort();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if std::fs::read_to_string(log.path())
                        .unwrap()
                        .contains("mcp_response_cancelled")
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        // Exceed both rmcp drain allowances. These are not live request deadlines.
        tokio::time::sleep(Duration::from_secs(6)).await;
        if !disconnect {
            assert!(!request.is_finished());
        }
        release.add_permits(1);
        if !disconnect {
            let result = tokio::time::timeout(Duration::from_secs(5), request)
                .await
                .unwrap()
                .unwrap();
            assert_ne!(result["isError"], true);
            assert_eq!(result["structuredContent"]["services"], json!([]));
            assert!(
                !std::fs::read_to_string(log.path())
                    .unwrap()
                    .contains("timed out draining")
            );
        } else {
            tokio::time::timeout(Duration::from_secs(5), work.drain())
                .await
                .unwrap();
            let logs = std::fs::read_to_string(log.path()).unwrap();
            assert!(
                logs.contains("timed out draining in-flight responses"),
                "{logs}"
            );
            // The application task survives the caller and rmcp's drain timeout.
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }
    http_task.abort();
    vendor_task.abort();
}

#[tokio::test]
async fn on_demand_management_keeps_seven_wrappers_and_scoped_dynamic_search() {
    let m = manager(None).await;
    add(&m, "demo", "", "").await;
    let mut writer = server(&m, "writer");
    writer.discover_on_demand = true;
    let mut reader = server(&m, "reader");
    reader.discover_on_demand = true;
    let (base, task) = listen(crate::server::http_app(writer, "test-token".into())).await;
    let (other, other_task) = listen(crate::server::http_app(reader, "test-token".into())).await;
    let listed = rpc(&base, "test-token", "tools/list", json!({})).await;
    assert_eq!(listed["tools"].as_array().unwrap().len(), 7);
    assert!(
        listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"].as_str().unwrap().starts_with("x402_treazury_"))
    );
    let found = call(&base, "x402_treazury_tools_search", json!({})).await;
    let items = found["structuredContent"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    for item in items {
        let bound = m
            .find_tool("writer", item["tool_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(item["input_schema"], bound.tool.input_schema);
        assert_eq!(item["description"], bound.tool.description);
    }
    let empty = call(&other, "x402_treazury_tools_search", json!({})).await;
    assert!(
        empty["structuredContent"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let denied = call(
        &other,
        "x402_treazury_tool_call",
        json!({"tool_id":items[0]["tool_id"],"arguments":{}}),
    )
    .await;
    assert_eq!(denied["isError"], true);
    let mut snapshot = (*m.catalog.read()).clone();
    let template = snapshot.views["writer"][0].clone();
    snapshot.views.insert(
        "writer".into(),
        (0..26)
            .map(|i| {
                let mut tool = template.clone();
                tool.tool.name = format!("fixture_{i}");
                tool
            })
            .collect(),
    );
    snapshot.generation += 1;
    m.catalog.publish(snapshot);
    let page = call(&base, "x402_treazury_tools_search", json!({})).await;
    assert_eq!(
        page["structuredContent"]["items"].as_array().unwrap().len(),
        25
    );
    let next = call(
        &base,
        "x402_treazury_tools_search",
        json!({"cursor":page["structuredContent"]["next_cursor"]}),
    )
    .await;
    assert_eq!(
        next["structuredContent"]["items"].as_array().unwrap().len(),
        1
    );
    assert!(next["structuredContent"]["next_cursor"].is_null());
    task.abort();
    other_task.abort();
}

#[tokio::test]
async fn raw_calls_require_endpoint_authority_and_validate_before_io() {
    use rmcp::ServerHandler;
    let manager = manager_with(listeners(false)).await;
    for on_demand in [false, true] {
        let mut disabled = server(&manager, "hidden");
        disabled.discover_on_demand = on_demand;
        let mut enabled = server(&manager, "writer");
        enabled.discover_on_demand = on_demand;
        assert!(disabled.get_tool(raw::NAME).is_none());
        assert!(enabled.get_tool(raw::NAME).is_some());
        let args = json!({"method":"GET","url":"https://api.example.com/read","body":{}});
        assert!(
            disabled
                .invoke_output(raw::NAME, args.as_object().unwrap())
                .await
                .is_err()
        );
        assert!(
            manager
                .invoke_raw("hidden", args.as_object().unwrap())
                .await
                .unwrap_err()
                .to_string()
                .contains("source_management_disabled")
        );
    }
    let standalone = Server::new(vec![], payer(), "https://api.example.com".into(), None);
    assert!(standalone.get_tool(raw::NAME).is_none());
    let enabled = server(&manager, "writer");
    for (name, args) in [
        (
            raw::NAME,
            json!({"method":"GET","url":"http://api.example.com/read"}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://127.0.0.1/read"}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://[::1]/read"}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://foo.local/read"}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://user:password@api.example.com/read"}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://api.example.com/read#fragment"}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://other.internal/read"}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://api.example.com/read","query":{"nested":{"a":1}}}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://api.example.com/read","query":{"null":null}}),
        ),
        (
            raw::NAME,
            json!({"method":"GET","url":"https://api.example.com/read","query":{"array":[{}]}}),
        ),
        (
            raw::NAME,
            json!({"method":"bad method","url":"https://api.example.com/read"}),
        ),
        (raw::NAME, json!({"url":"https://api.example.com/read"})),
        (
            raw::NAME,
            json!({"method":"POST","url":"https://api.example.com/read","body":{},"headers":{"Authorization":"secret"}}),
        ),
        (
            raw::NAME,
            json!({"method":"POST","url":"https://api.example.com/read","body":{},"wallet":"other"}),
        ),
        (
            raw::NAME,
            json!({"method":"bad method","url":"https://api.example.com/read","body":{}}),
        ),
    ] {
        let error = enabled
            .invoke_output(name, args.as_object().unwrap())
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(
            [
                "HTTPS",
                "non-public",
                "destination origin",
                "body",
                "query values",
                "missing field",
                "unknown field",
                "method"
            ]
            .iter()
            .any(|category| message.contains(category)),
            "unexpected failure (validation must precede I/O): {message}"
        );
        assert!(!error.to_string().contains("password"));
        assert!(!error.to_string().contains("secret"));
    }
    assert!(manager.inner.lock().unwrap().state.records.is_empty());
    assert!(manager.catalog.read().views["writer"].is_empty());
}

#[path = "source_details_document_tests.rs"]
mod source_details_document_tests;
#[path = "source_details_tests.rs"]
mod source_details_tests;

#[tokio::test]
async fn static_publication_composes_with_dynamic_commits_and_fences_stale_pricing() {
    let m = manager(None).await;
    add(&m, "first", "", "").await;
    let initial = m.catalog.read();
    let dynamic = initial.views["writer"][0].clone();
    dynamic.help.set("retained help".into()).unwrap();
    let mut base = m.static_snapshot().await.unwrap();
    let mut tool = dynamic.clone();
    tool.source = None;
    tool.tool.name = "static_read".into();
    tool.tool.description = "initial static description".into();
    base.views.get_mut("writer").unwrap().push(tool);
    assert_eq!(m.replace_static(0, base.clone()).await.unwrap(), 1);
    let captured = m.catalog.read();
    let generation = captured.generation;
    let mut priced = m.static_snapshot().await.unwrap();
    priced.views.get_mut("writer").unwrap()[0].tool.description = "startup price".into();
    let second_url = "https://api.example.com/second.json";
    let cell =
        m.fetches.lock().unwrap()[&fetch_key(&m, "https://api.example.com/openapi.json")].clone();
    m.fetches
        .lock()
        .unwrap()
        .insert(fetch_key(&m, second_url), cell);
    let (registered, published) = tokio::join!(
        m.invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":second_url,"name":"second"})
        ),
        m.replace_static(1, priced.clone()),
    );
    registered.unwrap();
    assert_eq!(published.unwrap(), 2);
    let current = m.catalog.read();
    assert_eq!(current.generation, generation + 2);
    assert_eq!(
        current.views["writer"]
            .iter()
            .filter(|b| b.source.is_some())
            .count(),
        4
    );
    assert_eq!(
        current.views["writer"]
            .iter()
            .find(|b| b.tool.name == "static_read")
            .unwrap()
            .tool
            .description,
        "startup price"
    );
    let unchanged = current.views["writer"]
        .iter()
        .find(|b| b.tool.name == dynamic.tool.name)
        .unwrap();
    assert!(Arc::ptr_eq(&unchanged.help, &dynamic.help));
    assert_eq!(
        unchanged.help.get().unwrap().as_ref().unwrap(),
        "retained help"
    );
    assert_eq!(
        captured.views["writer"]
            .iter()
            .find(|b| b.tool.name == "static_read")
            .unwrap()
            .tool
            .description,
        "initial static description"
    );
    // A later static removal cannot be undone by an earlier completed price.
    let mut removed = m.static_snapshot().await.unwrap();
    removed.views.get_mut("writer").unwrap().clear();
    assert_eq!(m.replace_static(2, removed).await.unwrap(), 3);
    let before = m.catalog.read();
    assert!(m.replace_static(1, priced).await.is_err());
    assert!(Arc::ptr_eq(&before, &m.catalog.read()));
    assert!(
        m.catalog.read().views["writer"]
            .iter()
            .all(|b| b.tool.name != "static_read")
    );
    // Duplicate registration remains concise after catalog publication changes.
    let duplicate = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":second_url,"name":"second"}),
        )
        .await
        .unwrap();
    assert_eq!(duplicate["status"], "available");
    assert!(duplicate.get("catalog_generation").is_none());
    assert_eq!(initial.views["writer"].len(), 2);
}

#[tokio::test]
async fn rejected_static_publications_preserve_revision_and_catalog() {
    for fault in [
        "listeners",
        "dynamic_source",
        "reserved",
        "duplicate",
        "static_overflow",
        "catalog_overflow",
    ] {
        let m = manager(None).await;
        add(&m, "first", "", "").await;
        let mut base = m.static_snapshot().await.unwrap();
        let mut bound = m.catalog.read().views["writer"][0].clone();
        bound.source = None;
        bound.tool.name = "static_read".into();
        base.views.get_mut("writer").unwrap().push(bound.clone());
        let mut expected = 0;
        match fault {
            "listeners" => {
                base.views.remove("hidden");
            }
            "dynamic_source" => {
                base.views.get_mut("writer").unwrap()[0].source = Some(("forged".into(), 1));
            }
            "reserved" => {
                base.views.get_mut("writer").unwrap()[0].tool.name = "dyn_reserved".into();
            }
            "duplicate" => {
                base.views.get_mut("writer").unwrap().push(bound);
            }
            "static_overflow" => {
                m.inner.lock().unwrap().base.generation = u64::MAX;
                expected = u64::MAX;
            }
            "catalog_overflow" => {
                let mut s = (*m.catalog.read()).clone();
                s.generation = u64::MAX;
                m.catalog.publish(s);
            }
            _ => unreachable!(),
        }
        let before = m.catalog.read();
        let static_before = m.static_snapshot().await.unwrap();
        assert!(m.replace_static(expected, base).await.is_err(), "{fault}");
        assert!(Arc::ptr_eq(&before, &m.catalog.read()), "{fault}");
        let after = m.static_snapshot().await.unwrap();
        assert_eq!(after.generation, static_before.generation);
        assert!(after.views.values().all(Vec::is_empty));
    }
}

#[tokio::test]
async fn saturated_static_publication_waits_without_warning_or_rejection() {
    let (_, logs) = crate::limits::tests::capture(async {
        let m = manager(None).await;
        let base = m.static_snapshot().await.unwrap();
        let held = m.mutations.clone().acquire_many_owned(16).await.unwrap();
        let publication = m.replace_static(0, base);
        tokio::pin!(publication);
        let polled = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(publication.as_mut(), cx))
        })
        .await;
        assert!(polled.is_pending());
        assert_eq!(m.catalog.read().generation, 0);
        drop(held);
        assert_eq!(publication.await.unwrap(), 1);
        assert_eq!(m.catalog.read().generation, 1);
    })
    .await;
    assert!(!logs.contains("WARN"), "{logs}");
    assert!(logs.contains("DEBUG"), "{logs}");
    assert!(
        logs.contains("mutation_queue_full") && logs.contains("limit=16"),
        "{logs}"
    );
    assert!(logs.contains("action=\"waiting\""), "{logs}");
    assert!(!logs.contains("operation rejected"), "{logs}");
}

#[tokio::test]
async fn accepted_static_publication_outlives_cancelled_waiter() {
    let m = manager(None).await;
    add(&m, "first", "", "").await;
    let mut base = m.static_snapshot().await.unwrap();
    let mut bound = m.catalog.read().views["writer"][0].clone();
    bound.source = None;
    bound.tool.name = "static_read".into();
    base.views.get_mut("writer").unwrap().push(bound);
    let (locked, acquired) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let held = m.clone();
    let lock_task = tokio::task::spawn_blocking(move || {
        let _guard = held.inner.lock().unwrap();
        locked.send(()).unwrap();
        released.recv().unwrap();
    });
    acquired.await.unwrap();
    let owner = m.clone();
    let task = tokio::spawn(async move { owner.replace_static(0, base).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while m.mutations.available_permits() == 16 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    release.send(()).unwrap();
    lock_task.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if m.static_snapshot().await.unwrap().generation == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("accepted publication orphaned");
    assert!(
        m.catalog.read().views["writer"]
            .iter()
            .any(|b| b.tool.name == "static_read")
    );
}

#[tokio::test]
async fn more_than_sixteen_sources_and_one_hundred_tools_are_accepted() {
    let m = manager(None).await;
    for index in 0..17 {
        let url = format!("https://api.example.com/catalog-{index}.json");
        let mut doc = spec();
        if index == 0 {
            doc["paths"] = json!({});
            for tool in 0..101 {
                doc["paths"][format!("/tool-{tool}")] = json!({"get":{}});
            }
        }
        let cell = OnceCell::new();
        cell.set(Fetched {
            completed: Instant::now(),
            result: Ok(Arc::new(Document::local(serde_json::to_vec(&doc).unwrap()))),
        })
        .unwrap();
        m.fetches
            .lock()
            .unwrap()
            .insert(fetch_key(&m, url.as_str()), Arc::new(cell));
        m.invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":url,"name":format!("source-{index}")}),
        )
        .await
        .unwrap();
    }
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 17);
    assert_eq!(m.catalog.read().views["writer"].len(), 101 + 16 * 2);
}

#[tokio::test]
async fn large_sources_survive_add_restart_and_static_publication_in_both_modes() {
    for on_demand in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("registry.sqlite");
        let configured_file = Some(file);
        let mut configured = listeners(true);
        configured.get_mut("writer").unwrap().discover_on_demand = on_demand;
        let configured = with_registry(configured, configured_file);
        let m = manager_with(configured.clone()).await;
        let mut doc = spec();
        doc["paths"] = (0..513).map(|i| (format!("/tool-{i}"), json!({"get":{"parameters":[{"name":"nested","in":"query","schema":{"type":"object","properties":{"value":{"type":"string"}}}}]}}))).collect::<serde_json::Map<_,_>>().into();
        register_spec(&m, "writer", "large", "large.json", doc)
            .await
            .unwrap();
        assert_eq!(m.catalog.read().views["writer"].len(), 513);
        let base = m.static_snapshot().await.unwrap();
        m.replace_static(base.generation, base).await.unwrap();
        assert_eq!(m.catalog.read().views["writer"].len(), 513);
        drop(m);
        let restored = manager_with(configured).await;
        assert_eq!(restored.catalog.read().views["writer"].len(), 513);
        let found = restored
            .invoke(
                "writer",
                "x402_treazury_tools_search",
                json!({"limit":1000}),
            )
            .await
            .unwrap();
        assert_eq!(found["items"].as_array().unwrap().len(), 513);
        assert!(
            restored
                .find_tool("writer", found["items"][0]["tool_id"].as_str().unwrap())
                .is_ok()
        );
    }
}

// Seed an imported document without live provider requests or wallet material.
async fn register_spec(
    m: &Arc<Manager>,
    owner: &str,
    name: &str,
    url_path: &str,
    doc: Value,
) -> Result<Value> {
    let url = format!("https://api.example.com/{url_path}");
    let cell = OnceCell::new();
    cell.set(Fetched {
        completed: Instant::now(),
        result: Ok(Arc::new(Document::local(serde_json::to_vec(&doc).unwrap()))),
    })
    .unwrap();
    m.fetches
        .lock()
        .unwrap()
        .insert(fetch_key_for(m, owner, url.as_str()), Arc::new(cell));
    m.invoke(
        owner,
        "x402_treazury_source_add",
        json!({"name":name,"spec_url":url}),
    )
    .await
}

#[tokio::test]
async fn named_sources_have_unambiguous_tools_and_name_only_search() {
    let m = manager(None).await;
    let mut doc = spec();
    doc["paths"] = json!({"/ip_risk":{"get":{}}});
    let added = register_spec(&m, "writer", "acme", "one.json", doc)
        .await
        .unwrap();
    let mut doc = spec();
    doc["paths"] = json!({"/risk":{"get":{}}});
    register_spec(&m, "writer", "acme_ip", "two.json", doc)
        .await
        .unwrap();
    let names: BTreeSet<_> = m.catalog.read().views["writer"]
        .iter()
        .map(|b| b.tool.name.clone())
        .collect();
    assert_eq!(
        names,
        ["dyn_acme__ip_risk", "dyn_acme_ip__risk"]
            .map(String::from)
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(added["tool_prefix"], "dyn_acme__");
    let result = m
        .search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"acme"}),
        )
        .unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["tool_id"], "dyn_acme__ip_risk");
    let reference = result["items"][0]["tool_id"].as_str().unwrap();
    assert_eq!(
        m.find_tool("writer", reference).unwrap().tool.name,
        "dyn_acme__ip_risk"
    );
    assert!(m.find_tool("reader", reference).is_err());
    for args in [
        json!({"source_name":"missing"}),
        json!({"source_id":"obsolete"}),
    ] {
        assert!(
            m.search_tools("writer", "x402_treazury_tools_search", args)
                .is_err()
        );
    }
    assert!(
        m.search_tools(
            "reader",
            "x402_treazury_tools_search",
            json!({"source_name":"acme"})
        )
        .is_err()
    );
    assert!(
        search_catalog(
            &m.catalog,
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"acme"})
        )
        .is_err()
    );
    // Rebuilding saved sources preserves the same visible names.
    m.restore().unwrap();
    let restored: BTreeSet<_> = m.catalog.read().views["writer"]
        .iter()
        .map(|b| b.tool.name.clone())
        .collect();
    assert_eq!(restored, names.into_iter().collect());
}

#[tokio::test]
async fn source_filters_use_readable_names_and_exclude_unmatched_tools() {
    let mut ls = listeners(false);
    ls.get_mut("writer").unwrap().include_tools = vec!["dyn_demo__read".into()];
    ls.get_mut("reader").unwrap().include_tools =
        vec!["dyn_00000000000000000000000000000001_*".into()];
    let m = manager_with(ls).await;
    register_spec(&m, "writer", "demo", "demo.json", spec())
        .await
        .unwrap();
    register_spec(&m, "reader", "demo", "demo.json", spec())
        .await
        .unwrap();
    assert_eq!(m.catalog.read().views["writer"].len(), 1);
    assert_eq!(
        m.catalog.read().views["writer"][0].tool.name,
        "dyn_demo__read"
    );
    assert!(m.catalog.read().views["reader"].is_empty());
    assert_eq!(
        m.listeners["reader"].include_tools,
        ["dyn_00000000000000000000000000000001_*"]
    );
}

#[tokio::test]
async fn source_names_are_verbatim_unique_and_race_safe() {
    let m = manager(None).await;
    for (i, name) in ["Acme-IP", "Acme_IP", "acme_ip", "acme_", "_", "-"]
        .iter()
        .enumerate()
    {
        let result = register_spec(&m, "writer", name, &format!("{i}.json"), spec())
            .await
            .unwrap();
        assert_eq!(result["name"], *name);
        assert_eq!(result["tool_prefix"], format!("dyn_{name}__"));
        let result = m
            .search_tools(
                "writer",
                "x402_treazury_tools_search",
                json!({"source_name":name}),
            )
            .unwrap();
        assert!(result["items"].as_array().unwrap().iter().all(|item| {
            item["tool_id"]
                .as_str()
                .unwrap()
                .starts_with(&format!("dyn_{name}__"))
        }));
    }
    assert!(
        register_spec(&m, "writer", "acme__ip", "invalid.json", spec())
            .await
            .is_err()
    );
    let (one, two) = tokio::join!(
        register_spec(&m, "writer", "racer", "race-one.json", spec()),
        register_spec(&m, "writer", "racer", "race-two.json", spec())
    );
    assert_ne!(one.is_ok(), two.is_ok());
    assert_eq!(
        one.err().or(two.err()).unwrap().to_string(),
        "source_name_reserved"
    );
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 7);
    // The same name is independently available to a different authenticated endpoint.
    register_spec(&m, "reader", "racer", "reader.json", spec())
        .await
        .unwrap();
}

#[tokio::test]
async fn restore_unsupported_names_disables_only_the_affected_source() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let m = manager(Some(file.clone())).await;
    register_spec(&m, "writer", "healthy", "healthy.json", spec())
        .await
        .unwrap();
    let unsupported = register_spec(&m, "writer", "legacy", "legacy.json", spec())
        .await
        .unwrap();
    let unsupported_id = registered_id(&m, &unsupported);
    register_spec(&m, "reader", "healthy", "reader.json", spec())
        .await
        .unwrap();
    {
        let mut inner = m.inner.lock().unwrap();
        let mut state = inner.state.clone();
        state
            .records
            .get_mut(&unsupported_id)
            .unwrap()
            .candidate
            .name = "old__name".into();
        state.records.retain(|_, record| record.owner == "writer");
        inner
            .stores
            .get_mut("writer")
            .unwrap()
            .save(&state)
            .unwrap();
    }
    drop(m);
    let m = manager(Some(file)).await;
    assert_eq!(
        m.inner.lock().unwrap().state.records[&unsupported_id]
            .disabled
            .as_deref(),
        Some("invalid_source_name")
    );
    assert!(
        m.catalog.read().views["writer"]
            .iter()
            .all(|b| b.tool.name.starts_with("dyn_healthy__"))
    );
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    assert_eq!(m.catalog.read().views["reader"].len(), 2);
}

#[tokio::test]
async fn source_name_search_preserves_request_boundary_and_cursor_scope() {
    let m = manager(None).await;
    register_spec(&m, "writer", "demo", "demo.json", spec())
        .await
        .unwrap();
    for mut args in [
        json!({"query":""}),
        json!({"source_name":"demo","query":""}),
    ] {
        let overhead = serde_json::to_vec(&args).unwrap().len();
        args["query"] = json!("x".repeat(65536 - overhead));
        assert_eq!(serde_json::to_vec(&args).unwrap().len(), 65536);
        m.invoke("writer", "x402_treazury_tools_search", args.clone())
            .await
            .unwrap();
        if args.get("source_name").is_none() {
            search_catalog(
                &m.catalog,
                "writer",
                "x402_treazury_tools_search",
                args.clone(),
            )
            .unwrap();
        }
        args["query"] = json!(format!("{}x", args["query"].as_str().unwrap()));
        assert!(
            m.invoke("writer", "x402_treazury_tools_search", args)
                .await
                .is_err()
        );
    }
    let first = m
        .search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"demo","limit":1}),
        )
        .unwrap();
    let second = m
        .search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"demo","limit":1,"cursor":first["next_cursor"]}),
        )
        .unwrap();
    assert_ne!(first["items"][0]["tool_id"], second["items"][0]["tool_id"]);
}

#[tokio::test]
async fn discovery_help_omits_cached_source_state_across_publication_and_restart() {
    use rmcp::ServerHandler;
    for on_demand in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("registry.sqlite");
        let config = operator_config(dir.path(), &file);
        let configured_file = Some(file);
        let mut configured = listeners(true);
        configured.get_mut("writer").unwrap().discover_on_demand = on_demand;
        configured.get_mut("writer").unwrap().exclude_tools = vec!["dyn_Filtered__*".into()];
        let configured = with_registry(configured, configured_file);
        let m = manager_with(configured.clone()).await;
        let mut base = m.static_snapshot().await.unwrap();
        let static_tools = crate::catalog::build_tools(
            &crate::catalog::Config {
                include_tools: vec!["static_read".into()],
                ..Default::default()
            },
            &spec(),
            "static",
        )
        .unwrap();
        for owner in ["writer", "hidden"] {
            base.views
                .get_mut(owner)
                .unwrap()
                .extend(crate::catalog_state::bind(
                    static_tools
                        .iter()
                        .map(|tool| (tool.clone(), payer(), "https://api.example.com".into()))
                        .collect(),
                ));
        }
        m.replace_static(base.generation, base).await.unwrap();
        // A global Manager must not divert static on-demand listeners into its
        // source-management permission checks.
        let mut static_endpoint = server(&m, "hidden");
        static_endpoint.discover_on_demand = true;
        let found = static_endpoint
            .invoke("x402_treazury_tools_search", &Default::default())
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&found).unwrap()["items"][0]["tool_id"],
            "static_read"
        );
        drop(static_endpoint);
        let mut endpoint = server(&m, "writer");
        endpoint.discover_on_demand = on_demand;
        endpoint.discovery_search_tool_instructions_text =
            Some("Writer capability guidance".into());
        let empty = endpoint
            .get_tool("x402_treazury_tools_search")
            .unwrap()
            .description
            .unwrap();
        assert!(!empty.contains("description snapshot"));
        assert!(!empty.contains("Registered sources"));
        let first = register_spec(&m, "writer", "Zed_", "zed.json", spec())
            .await
            .unwrap();
        let first_id = registered_id(&m, &first);
        register_spec(&m, "writer", "Alpha-IP", "alpha.json", spec())
            .await
            .unwrap();
        register_spec(&m, "reader", "Other", "other.json", spec())
            .await
            .unwrap();
        register_spec(&m, "writer", "Filtered", "filtered.json", spec())
            .await
            .unwrap();
        let description = endpoint
            .get_tool("x402_treazury_tools_search")
            .unwrap()
            .description
            .unwrap();
        assert_eq!(description, empty);
        assert!(!description.contains("Other"));
        assert!(!description.contains("Filtered"));
        assert!(description.ends_with("Writer capability guidance"));
        assert!(!empty.contains("Alpha-IP"));
        let base = m.static_snapshot().await.unwrap();
        m.replace_static(base.generation, base).await.unwrap();
        assert_eq!(
            endpoint
                .get_tool("x402_treazury_tools_search")
                .unwrap()
                .description
                .unwrap(),
            description
        );
        let (url, http_task) =
            listen(crate::server::http_app(endpoint.clone(), "fixture".into())).await;
        let listed = rpc(&url, "fixture", "tools/list", json!({})).await;
        let listed_search = listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "x402_treazury_tools_search")
            .unwrap();
        assert_eq!(listed_search["description"], description.as_ref());
        let all = endpoint
            .invoke("x402_treazury_tools_search", &Default::default())
            .await
            .unwrap();
        let all: Value = serde_json::from_str(&all).unwrap();
        let names: BTreeSet<_> = all["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["tool_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "static_read",
                "dyn_Alpha-IP__read",
                "dyn_Alpha-IP__write",
                "dyn_Zed___read",
                "dyn_Zed___write"
            ]
            .into_iter()
            .collect()
        );
        assert!(
            !endpoint
                .get_tool(directory::SEARCH)
                .unwrap()
                .description
                .unwrap()
                .contains("Registered sources with searchable tools")
        );
        let found = endpoint
            .invoke(
                "x402_treazury_tools_search",
                &serde_json::from_value(json!({"source_name":"Alpha-IP"})).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&found).unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        http_task.abort();
        let _ = http_task.await;
        drop(endpoint);
        drop(m);
        let restored = manager_with(configured.clone()).await;
        let result = restored
            .search_tools(
                "writer",
                "x402_treazury_tools_search",
                json!({"source_name":"Alpha-IP"}),
            )
            .unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
        assert!(
            !server(&restored, "writer")
                .get_tool("x402_treazury_tools_search")
                .unwrap()
                .description
                .unwrap()
                .contains("description snapshot")
        );
        drop(restored);
        maintain_cli(&config, "writer", &first_id, false)
            .await
            .unwrap();
        let removed = manager_with(configured).await;
        let description = server(&removed, "writer")
            .get_tool("x402_treazury_tools_search")
            .unwrap()
            .description
            .unwrap();
        assert!(!description.contains("description snapshot"));
        assert!(!description.contains("Zed_"));
        assert!(!description.contains("Filtered"));
        assert!(!description.contains("Other"));
    }
}

#[tokio::test]
async fn bm25_search_scopes_counts_ranked_pages_and_cursor_generation() {
    let m = manager(None).await;
    let document = json!({"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],"paths":{
        "/reviews":{"get":{"description":"List public reviews for a Yelp business","parameters":[{"in":"query","name":"businessAlias","schema":{"type":"string"}}]}},
        "/businesses":{"get":{"description":"Look up a Yelp business"}}
    }});
    let added = register_spec(&m, "writer", "social", "social.json", document)
        .await
        .unwrap();
    let arguments = json!({"source_name":"social","query":"yelp reviews","limit":1});
    let first = m
        .search_tools("writer", "x402_treazury_tools_search", arguments.clone())
        .unwrap();
    assert_eq!(first["items"][0]["tool_id"], "dyn_social__reviews");
    assert_eq!(first["available"], 2);
    assert_eq!(first["total"], 2);
    assert!(first.get("hint").is_none());
    let second = m.search_tools("writer", "x402_treazury_tools_search", json!({
        "source_name":"social","query":"yelp reviews","limit":1,"cursor":first["next_cursor"]
    })).unwrap();
    assert_eq!(second["items"][0]["tool_id"], "dyn_social__businesses");
    assert_eq!(second["available"], 2);
    assert_eq!(second["total"], 2);
    assert!(second["next_cursor"].is_null());
    let parameter = m
        .search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"social","query":"businessAlias"}),
        )
        .unwrap();
    assert_eq!(parameter["total"], 1);
    assert_eq!(parameter["items"][0]["tool_id"], "dyn_social__reviews");
    let enumeration = m
        .search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"social"}),
        )
        .unwrap();
    assert_eq!(enumeration["total"], 2);
    assert_eq!(enumeration["available"], 2);
    assert_eq!(enumeration["items"][0]["tool_id"], "dyn_social__businesses");
    for query in ["", " \t\n"] {
        let blank = m
            .search_tools(
                "writer",
                "x402_treazury_tools_search",
                json!({"source_name":"social","query":query}),
            )
            .unwrap();
        assert_eq!(blank["items"], enumeration["items"]);
    }
    // Excluded sources and other endpoints cannot change the selected corpus,
    // counts or ranking, even when their descriptions have many query terms.
    let mut noise = spec();
    noise["paths"] =
        json!({"/noise":{"get":{"description":"yelp reviews yelp reviews yelp reviews"}}});
    register_spec(&m, "writer", "noise", "noise.json", noise)
        .await
        .unwrap();
    let mut foreign = spec();
    foreign["paths"] = json!({"/private":{"get":{"description":"foreignonlyword yelp reviews"}}});
    register_spec(&m, "reader", "foreign", "foreign.json", foreign)
        .await
        .unwrap();
    let scoped = m
        .search_tools("writer", "x402_treazury_tools_search", arguments.clone())
        .unwrap();
    assert_eq!(scoped["items"], first["items"]);
    assert_eq!(scoped["available"], 2);
    assert_eq!(scoped["total"], 2);
    let mut endpoint = server(&m, "writer");
    endpoint.discover_on_demand = true;
    let scoped_http_path: Value = serde_json::from_str(
        &endpoint
            .invoke(
                "x402_treazury_tools_search",
                &serde_json::from_value(arguments.clone()).unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(scoped_http_path, scoped);
    let all = m
        .search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"query":"yelp reviews"}),
        )
        .unwrap();
    assert_eq!(all["available"], 3);
    assert_eq!(all["total"], 3);
    assert_eq!(
        search_catalog(
            &m.catalog,
            "writer",
            "x402_treazury_tools_search",
            json!({"query":"yelp reviews"})
        )
        .unwrap(),
        all
    );
    for query in ["nonexistentzzzz", "foreignonlyword", "***"] {
        let missed = m
            .search_tools(
                "writer",
                "x402_treazury_tools_search",
                json!({"query":query}),
            )
            .unwrap();
        assert_eq!(missed["available"], 3);
        assert_eq!(missed["total"], 0);
        assert!(missed["items"].as_array().unwrap().is_empty());
        assert!(missed["hint"].as_str().unwrap().contains("omit query"));
    }
    assert!(
        m.search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_id":"missing","query":"reviews"})
        )
        .is_err()
    );
    let empty_endpoint = search_catalog(
        &m.catalog,
        "hidden",
        "x402_treazury_tools_search",
        json!({"query":"reviews"}),
    )
    .unwrap();
    assert_eq!(empty_endpoint["available"], 0);
    assert_eq!(empty_endpoint["total"], 0);
    assert!(empty_endpoint.get("hint").is_none());
    for args in [
        json!({"source_name":"social","query":"reviews","cursor":scoped["next_cursor"]}),
        json!({"source_name":"noise","query":"yelp reviews","cursor":scoped["next_cursor"]}),
    ] {
        assert!(
            m.search_tools("writer", "x402_treazury_tools_search", args)
                .is_err()
        );
    }
    let reference = scoped["items"][0]["tool_id"].as_str().unwrap();
    assert_eq!(
        m.find_tool("writer", reference)
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .0,
        registered_id(&m, &added).as_str()
    );
    assert!(m.find_tool("reader", reference).is_err());
    let base = m.static_snapshot().await.unwrap();
    m.replace_static(base.generation, base).await.unwrap();
    let mut stale = arguments;
    stale["cursor"] = scoped["next_cursor"].clone();
    assert!(
        m.search_tools("writer", "x402_treazury_tools_search", stale)
            .is_err()
    );
    // A ranking refresh does not redirect or revoke an unchanged captured tool.
    assert_eq!(
        m.find_tool("writer", reference).unwrap().tool.name,
        "dyn_social__reviews"
    );
}

#[tokio::test]
async fn automatic_source_names_are_readable_collision_safe_and_duplicate_safe() {
    let mut configured = listeners(false);
    configured
        .get_mut("writer")
        .unwrap()
        .sources
        .push("api-example-com".into());
    let m = manager_with(configured).await;
    for path in ["first.json", "second.json"] {
        let cell = OnceCell::new();
        cell.set(Fetched {
            completed: Instant::now(),
            result: Ok(Arc::new(Document::local(
                serde_json::to_vec(&spec()).unwrap(),
            ))),
        })
        .unwrap();
        m.fetches.lock().unwrap().insert(
            fetch_key(&m, &format!("https://api.example.com/{path}")),
            Arc::new(cell),
        );
    }
    let args = |path| json!({"spec_url":format!("https://api.example.com/{path}")});
    let results = futures_util::future::join_all((0..8).map(|index| {
        let m = m.clone();
        let args = args(if index % 2 == 0 {
            "first.json"
        } else {
            "second.json"
        });
        async move {
            m.invoke("writer", "x402_treazury_source_add", args)
                .await
                .unwrap()
        }
    }))
    .await;
    let names: BTreeSet<_> = results
        .iter()
        .map(|result| result["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["api-example-com-2", "api-example-com-3"])
    );
    assert!(
        names
            .iter()
            .all(|name| name.contains("api") && !name.contains('.'))
    );
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
    for result in &results {
        let fields: BTreeSet<_> = result
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            fields,
            ["name", "tool_prefix", "tool_count", "status", "next"]
                .into_iter()
                .collect()
        );
        assert_eq!(result["status"], "available");
    }
    assert!(
        results
            .iter()
            .step_by(2)
            .all(|result| result == &results[0])
    );
    assert!(
        results
            .iter()
            .skip(1)
            .step_by(2)
            .all(|result| result == &results[1])
    );
}

#[tokio::test]
async fn legacy_registry_bookkeeping_does_not_block_restore_or_registration() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let m = manager(Some(file.clone())).await;
    add(&m, "saved", "", "").await;
    let mut saved = serde_json::to_value(&m.inner.lock().unwrap().state).unwrap();
    // A count of historical registrations must not make accepted state unreloadable.
    let mut tombstone = saved["records"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    tombstone["removed"] = json!(true);
    tombstone["document"] = json!("");
    for index in 1..=10001 {
        let id = Uuid::from_u128(index).to_string();
        tombstone["id"] = json!(id);
        saved["records"][&id] = tombstone.clone();
    }
    // Old receipts have no authority and are ignored without imposing their old capacity.
    saved["receipts"] = Value::Object(
        (0..10001)
            .map(|index| (format!("legacy-{index}"), Value::Null))
            .collect(),
    );
    drop(m);
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute(
        "UPDATE snapshot SET json=?1 WHERE id=1",
        [serde_json::to_string(&saved).unwrap()],
    )
    .unwrap();
    drop(db);
    let m = manager(Some(file.clone())).await;
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    register_spec(&m, "writer", "new", "new.json", spec())
        .await
        .unwrap();
    assert_eq!(m.catalog.read().views["writer"].len(), 4);
    drop(m);
    let db = rusqlite::Connection::open(&file).unwrap();
    let stored: String = db
        .query_row("SELECT json FROM snapshot WHERE id=1", [], |row| row.get(0))
        .unwrap();
    assert!(
        serde_json::from_str::<Value>(&stored)
            .unwrap()
            .get("receipts")
            .is_none()
    );
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap()["records"]
            .as_object()
            .unwrap()
            .len(),
        10003
    );
    drop(db);
    let restored = manager(Some(file)).await;
    assert_eq!(restored.catalog.read().views["writer"].len(), 4);
    assert_eq!(restored.inner.lock().unwrap().state.records.len(), 10003);
}

#[tokio::test]
async fn simplified_tool_wrappers_reject_old_fields_and_filtered_names() {
    let mut configured = listeners(false);
    configured.get_mut("writer").unwrap().include_tools = vec!["dyn_demo__read".into()];
    let m = manager_with(configured).await;
    add(&m, "demo", "", "").await;
    let found = m
        .invoke("writer", "x402_treazury_tools_search", json!({}))
        .await
        .unwrap();
    let item = &found["items"][0];
    assert_eq!(
        item.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        ["tool_id", "description", "input_schema"]
            .into_iter()
            .collect()
    );
    assert!(m.find_tool("writer", "dyn_demo__write").is_err());
    assert!(
        m.find_tool("reader", item["tool_id"].as_str().unwrap())
            .is_err()
    );
    assert!(
        m.invoke(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_id":"obsolete"})
        )
        .await
        .is_err()
    );
    assert!(
        server(&m, "writer")
            .invoke(
                "x402_treazury_tool_call",
                json!({"tool_ref":item["tool_id"],"arguments":{}})
                    .as_object()
                    .unwrap()
            )
            .await
            .is_err()
    );
}

#[path = "provider_registration_tests.rs"]
mod provider_registration_tests;

#[tokio::test]
async fn uniform_inventory_pages_preserve_definitions_and_snapshot_scope() {
    let m = manager(None).await;
    add(&m, "demo", "", "").await;
    let template = m.catalog.read().views["writer"][0].clone();
    for owner in ["hidden", "writer"] {
        let (base, task) = listen(crate::server::http_app(
            server(&m, owner),
            "test-token".into(),
        ))
        .await;
        for count in [0, 25, 50, 51] {
            let mut snapshot = (*m.catalog.read()).clone();
            snapshot.generation += 1;
            snapshot.views.insert(owner.into(), (0..count).map(|i| {
                let mut bound = template.clone();
                bound.source = None;
                bound.tool.name = format!("tool_{i}");
                bound.tool.description = "Complete description 雪".repeat(100);
                bound.tool.input_schema = json!({"type":"object","properties":{"nested":{"type":"array","items":{"type":"object","properties":{"value":{"type":"string"}}}}}});
                bound
            }).collect());
            m.catalog.publish(snapshot);
            let mut expected: Vec<Value> =
                tools::listener_definitions(owner == "writer", false, None, None)
                    .into_iter()
                    .map(|t| serde_json::to_value(t).unwrap())
                    .collect();
            expected.extend(m.catalog.read().views[owner].iter().map(|b| json!({"name":b.tool.name,"description":b.tool.description,"inputSchema":b.tool.input_schema})));
            let mut actual = Vec::new();
            let mut params = json!({});
            loop {
                let result = rpc(&base, "test-token", "tools/list", params).await;
                let page = result["tools"].as_array().unwrap();
                assert!(page.len() <= 25);
                actual.extend(page.iter().cloned());
                if result["nextCursor"].is_null() {
                    break;
                }
                assert_eq!(page.len(), 25);
                params = json!({"cursor":result["nextCursor"]});
            }
            assert_eq!(actual, expected);
            let searched = if owner == "hidden" {
                search_catalog(&m.catalog, owner, "x402_treazury_tools_search", json!({})).unwrap()
            } else {
                m.invoke(owner, "x402_treazury_tools_search", json!({}))
                    .await
                    .unwrap()
            };
            assert_eq!(searched["items"].as_array().unwrap().len(), count.min(25));
            for limit in [101, u32::MAX as u64 + 1, u64::MAX] {
                let result = search_catalog_query(
                    &m.catalog,
                    owner,
                    "x402_treazury_tools_search",
                    serde_json::from_value(json!({"limit":limit})).unwrap(),
                )
                .unwrap();
                assert_eq!(result["items"].as_array().unwrap().len(), count);
            }
            for limit in [json!(0), json!(-1), json!(1.5)] {
                assert!(
                    search_catalog(
                        &m.catalog,
                        owner,
                        "x402_treazury_tools_search",
                        json!({"limit":limit})
                    )
                    .is_err()
                );
            }
        }
        let first = rpc(&base, "test-token", "tools/list", json!({})).await;
        let cursor = first["nextCursor"].clone();
        let other = if owner == "hidden" {
            "writer"
        } else {
            "hidden"
        };
        let (other_base, other_task) = listen(crate::server::http_app(
            server(&m, other),
            "test-token".into(),
        ))
        .await;
        let invalid = |url: String, cursor: Value| async move {
            let response: Value = crate::network::discovery(&url, Duration::from_secs(5)).unwrap().post(format!("{url}/mcp"))
                .bearer_auth("test-token").header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"cursor":cursor}}))
                .send().await.unwrap().mcp_json().await.unwrap();
            assert!(
                response["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("stale or invalid cursor")
            );
        };
        invalid(other_base, cursor.clone()).await;
        invalid(base.clone(), json!("invalid")).await;
        let mut snapshot = (*m.catalog.read()).clone();
        snapshot.generation += 1;
        m.catalog.publish(snapshot);
        invalid(base, cursor).await;
        task.abort();
        other_task.abort();
    }
}

#[path = "registration_guidance_tests.rs"]
mod registration_guidance_tests;

#[path = "import_read_tests.rs"]
mod import_read_tests;

#[path = "directory_relay_tests.rs"]
mod directory_relay_tests;
