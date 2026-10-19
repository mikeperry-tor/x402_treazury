use super::*;
use crate::mcp_wire::McpResponse;
use crate::{
    payment::{Payer, SpendPolicy},
    server::Server,
};
fn listeners(_persistent: bool) -> BTreeMap<String, ListenerConfig> {
    ["writer", "reader", "hidden"].into_iter().map(|id| (id.into(), serde_json::from_value(json!({"listen":"127.0.0.1:0","bearer_token_env":"TEST_TOKEN","sources":[],"source_management":id != "hidden"})).unwrap())).collect()
}
fn policy(file: Option<PathBuf>) -> policy::Policy {
    serde_json::from_value(json!({"wallet":"shared","registry_file":file})).unwrap()
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

async fn manager_with(
    policy: policy::Policy,
    listeners: BTreeMap<String, ListenerConfig>,
) -> Arc<Manager> {
    let snapshot = CatalogSnapshot {
        instructions: Default::default(),
        generation: 0,
        views: listeners.keys().map(|k| (k.clone(), vec![])).collect(),
    };
    Manager::new(
        policy,
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
    manager_with(policy(file), listeners(persist)).await
}

fn spec() -> Value {
    json!({"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],"paths":{"/read":{"get":{"tags":["read"],"description":"Read item","parameters":[{"in":"query","name":"q","schema":{"type":"string"}}]}},"/write":{"post":{"tags":["write"],"description":"Write item"}}}})
}

fn candidate(name: &str, lifetime: &str, visibility: &str) -> Value {
    json!({"name":name,"spec_url":"https://api.example.com/openapi.json","lifetime":lifetime,"visibility":visibility})
}

async fn seed(m: &Manager) {
    let cell = OnceCell::new();
    cell.set(Fetched {
        completed: Instant::now(),
        result: Ok(Arc::new(serde_json::to_vec(&spec()).unwrap())),
    })
    .unwrap();
    m.fetches.lock().unwrap().insert(
        "https://api.example.com/openapi.json".into(),
        Arc::new(cell),
    );
}

fn server(m: &Arc<Manager>, id: &str) -> Server {
    let mut s = Server::new(
        vec![],
        payer(),
        "https://api.example.com".into(),
        None,
        None,
    );
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
fn importer_rejects_unsafe_and_expanding_documents() {
    let p = policy(None);
    let c: Candidate = serde_json::from_value(candidate("demo", "process", "server")).unwrap();
    let id = Uuid::new_v4().to_string();
    assert!(import::build(&p, &c, &id, &serde_json::to_vec(&spec()).unwrap()).is_ok());
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
        assert!(import::endpoint(&p, url).is_err(), "{url}");
    }
    for replacement in [
        json!({"$ref":"https://evil.example.com/schema"}),
        json!({"$ref":"#/missing"}),
        json!({"loop":{"$ref":"#/components"}}),
    ] {
        let mut doc = spec();
        doc["components"] = replacement;
        assert!(import::build(&p, &c, &id, &serde_json::to_vec(&doc).unwrap()).is_err());
    }
    let mut doc = spec();
    doc["paths"]["/read"]["get"]["servers"] = json!([{"url":"https://evil.example.com"}]);
    assert!(import::build(&p, &c, &id, &serde_json::to_vec(&doc).unwrap()).is_err());
    let mut doc = spec();
    doc["components"] = json!({"x0":{"type":"string"}});
    for n in 1..25 {
        doc["components"][format!("x{n}")] = json!({"allOf":[{"$ref":format!("#/components/x{}",n-1)},{"$ref":format!("#/components/x{}",n-1)}]});
    }
    assert!(
        import::build(&p, &c, &id, &serde_json::to_vec(&doc).unwrap())
            .unwrap_err()
            .to_string()
            .contains("complexity")
    );
}

#[test]
fn import_pipeline_preserves_origin_checks_and_validation_before_selection() {
    let mut p = policy(None);
    let mut c: Candidate = serde_json::from_value(candidate("demo", "process", "server")).unwrap();
    let id = Uuid::new_v4().to_string();
    let build = |p: &policy::Policy, c: &Candidate, doc: &Value| {
        import::build(p, c, &id, &serde_json::to_vec(doc).unwrap())
    };
    let mut doc = spec();
    doc["servers"][0]["url"] = json!("/v1");
    assert_eq!(
        build(&p, &c, &doc).unwrap().base,
        "https://api.example.com/v1"
    );
    c.base_url = Some("https://api.example.com/override".into());
    doc.as_object_mut().unwrap().remove("servers");
    assert_eq!(
        build(&p, &c, &doc).unwrap().base,
        c.base_url.as_ref().unwrap().as_str()
    );
    c.base_url = Some("https://api.example.com/v1?secret=value".into());
    assert!(
        build(&p, &c, &doc)
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
            build(&p, &c, &bad).unwrap_err().to_string().contains(error),
            "{error}"
        );
    }
    // Selection still restricts the inventory without a per-source quota.
    assert_eq!(build(&p, &c, &spec()).unwrap().tools.len(), 1);
    c.selection.include.clear();
    assert_eq!(build(&p, &c, &spec()).unwrap().tools.len(), 2);

    let cross_origin = json!({"openapi":"3.0.3", "servers":[{"url":"https://api.example.com"}],
        "paths":{"https://other.example.com/read":{"get":{}}}});
    assert!(
        build(&p, &c, &cross_origin)
            .unwrap_err()
            .to_string()
            .contains("cross_origin_operation_rejected")
    );
    p.allowed_origins = vec![
        "https://api.example.com".into(),
        "https://other.example.com".into(),
    ];
    assert_eq!(build(&p, &c, &cross_origin).unwrap().tools.len(), 1);
    p.allowed_origins = vec!["https://api.example.com".into()];
    assert!(
        build(&p, &c, &cross_origin)
            .unwrap_err()
            .to_string()
            .contains("destination origin not authorized")
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
    assert_eq!(r["revision"], 1);
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
    let (address,task)=crate::test_tls::serve(axum::Router::new()
        .route("/openapi.json",get(|| async {axum::Json(spec())}))
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
    let p = policy(None);
    let bytes = import::fetch(&p, "https://spec.example.com/openapi.json")
        .await
        .unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), spec());
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
            import::fetch(&p, &format!("https://spec.example.com/{path}"))
                .await
                .is_err()
        );
    }
    assert!(
        import::fetch(&p, "https://127.0.0.1/openapi.json")
            .await
            .is_err()
    );
    let ctx = crate::network::global();
    let signer: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
    let ids = [
        IsolationId::discovery("https://spec.example.com").unwrap(),
        IsolationId::evm(&signer.address().to_string()).unwrap(),
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
    let id = added["source_id"].as_str().unwrap();
    assert!(maintain_cli(&config, "writer", id, true).await.is_err());
    drop(m);
    maintain_cli(&config, "writer", id, true).await.unwrap();
    let saved = store::Store::open(&file, &[]).unwrap().load().unwrap();
    assert_eq!(saved.records[id].revision, 2);
    assert_eq!(
        serde_json::from_str::<Value>(&saved.records[id].document).unwrap(),
        spec()
    );
    // A refused refresh leaves the saved revision untouched.
    config_value["source_management"]
        .as_table_mut()
        .unwrap()
        .insert("max_tools_per_server".into(), toml::Value::Integer(1));
    std::fs::write(&config, toml::to_string(&config_value).unwrap()).unwrap();
    assert!(maintain_cli(&config, "writer", id, true).await.is_err());
    assert_eq!(
        store::Store::open(&file, &[])
            .unwrap()
            .load()
            .unwrap()
            .records[id]
            .revision,
        2
    );
    // On-demand listeners retain the same saved tools despite this eager-list quota.
    config_value["servers"]["writer"]
        .as_table_mut()
        .unwrap()
        .insert("discover_on_demand".into(), toml::Value::Boolean(true));
    std::fs::write(&config, toml::to_string(&config_value).unwrap()).unwrap();
    maintain_cli(&config, "writer", id, true).await.unwrap();
    assert_eq!(
        store::Store::open(&file, &[])
            .unwrap()
            .load()
            .unwrap()
            .records[id]
            .revision,
        3
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
    assert_ne!(first["source_id"], other["source_id"]);
    assert!(
        m.invoke("hidden", "x402_treazury_source_add", args)
            .await
            .is_err()
    );
    let s = server(&m, "writer");
    for name in [
        "sources_search",
        "source_details",
        "source_url_info",
        "source_add",
        "raw_get",
        "raw_post",
        "tools_search",
        "tool_call",
    ] {
        assert!(s.get_tool(&format!("x402_treazury_{name}")).is_some());
    }
    for name in [
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
async fn references_and_cursors_bind_endpoint_revision_and_process() {
    let m = manager(None).await;
    add(&m, "demo", "", "").await;
    let search = m
        .invoke("writer", "x402_treazury_tools_search", json!({"limit":1}))
        .await
        .unwrap();
    let reference = search["items"][0]["tool_ref"].as_str().unwrap();
    assert!(m.find_reference("writer", reference).is_ok());
    assert!(m.find_reference("reader", reference).is_err());
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
    assert!(m.find_reference("writer", reference).is_err());
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
    assert!(another.find_reference("writer", reference).is_err());
}

#[tokio::test]
async fn registry_persistence_is_operator_controlled_and_revocation_is_local() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let m = manager(Some(file.clone())).await;
    let first = add(&m, "saved", "", "").await;
    assert_eq!(first["lifetime"], "persistent");
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
    assert_eq!(again["source_id"], first["source_id"]);
    drop(m);
    let mut ls = listeners(true);
    ls.get_mut("writer").unwrap().source_management = false;
    let m = manager_with(policy(Some(file)), ls).await;
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
        add(&temporary, "temporary", "", "").await["lifetime"],
        "process"
    );
}

#[tokio::test]
async fn eager_endpoint_quota_never_publishes_partial_sources() {
    let mut p = policy(None);
    p.max_tools_per_server = 1;
    let m = manager_with(p, listeners(false)).await;
    seed(&m).await;
    assert!(
        m.invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":"https://api.example.com/openapi.json"})
        )
        .await
        .is_err()
    );
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    assert!(m.catalog.read().views.values().all(Vec::is_empty));
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
            .unwrap()["revision"],
        1
    );
    // A failed SQLite transaction must not publish another registration.
    m.inner
        .lock()
        .unwrap()
        .store
        .as_ref()
        .unwrap()
        .reject_writes();
    seed(&m).await;
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
        policy(None),
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
        json!({"spec_url":"https://api.example.com/openapi.json"}),
    )
    .await;
    let id = added["structuredContent"]["source_id"].clone();
    assert!(id.is_string(), "{added}");
    let signatures = call(&base, "x402_treazury_tools_search", json!({"source_id":id})).await;
    let item = &signatures["structuredContent"]["items"][0];
    assert!(item["input_schema"].is_object());
    assert!(item["tool_ref"].is_string());
    assert_eq!(
        call(
            &base,
            "x402_treazury_tool_call",
            json!({"tool_ref":"stale","arguments":{}})
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
    let ls = listeners(true);
    let registry = tmp.path().join("registry.sqlite");
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
        policy(Some(registry.clone())),
        ls.clone(),
        BTreeMap::from([("shared".into(), managed.clone())]),
        Arc::new(CatalogState::new(snapshot)),
        vec![],
    )
    .await
    .unwrap();
    *m.fixture_endpoint.lock().unwrap() = Some(format!("{url}/spec"));
    add(&m, "one", "", "").await;
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
        policy(Some(registry)),
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
        2
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
        "source_management":{"wallet":"shared","registry_file":file},"servers":{"writer":{"listen":"127.0.0.1:8000","bearer_token_env":"UNUSED_TOKEN","source_management":true}}});
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
    let id = first["source_id"].as_str().unwrap();
    assert!(maintain_cli(&config, "writer", id, false).await.is_err());
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    drop(m);
    assert!(maintain_cli(&config, "reader", id, false).await.is_err());
    maintain_cli(&config, "writer", id, false).await.unwrap();
    let m = manager(Some(file)).await;
    assert!(m.catalog.read().views["writer"].is_empty());
    assert!(m.inner.lock().unwrap().state.records[id].removed);
    let second = add(&m, "saved", "", "").await;
    assert_ne!(first["source_id"], second["source_id"]);
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
    let task = tokio::spawn(async move {
        worker
            .commit("writer".into(), "cancel-test".into(), json!({}), record)
            .await
    });
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
async fn boolean_config_and_examples_are_strict_and_offline() {
    for example in [
        "examples/deployments/agent-sources.toml",
        "examples/deployments/agent-sources-managed.toml",
    ] {
        let shown = crate::deployment::Deployment::show_config(std::path::Path::new(example))
            .await
            .unwrap();
        assert_eq!(shown["servers"]["research"]["source_management"], true);
        assert_eq!(shown["source_management"]["scope"], "endpoint_local");
        assert_eq!(
            shown["source_management"]["wallet_bindings"]["analysis"],
            "agent_shared"
        );
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
    {
        let mut inner = m.inner.lock().unwrap();
        let mut state = inner.state.clone();
        state
            .records
            .get_mut(added["source_id"].as_str().unwrap())
            .unwrap()
            .targets
            .push("removed_endpoint".into());
        inner.store.as_mut().unwrap().save(&state).unwrap();
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
    assert!(result["disabled"].is_string());
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
    assert!(
        serde_json::from_value::<policy::Policy>(
            json!({"wallet":"shared","directory_tool":"custom"})
        )
        .is_err()
    );
}

#[tokio::test]
async fn management_only_deployment_starts_and_lists_exactly_eight_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
version = 1
[wallets.shared]
mode = "static"
private_key_env = "TEST_KEY"
[source_management]
wallet = "shared"
[servers.research]
listen = "127.0.0.1:0"
bearer_token_env = "TEST_TOKEN"
source_management = true
# API filters must not hide management tools.
exclude_tools = ["*"]
"#,
    )
    .unwrap();
    let deployment = crate::deployment::Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    assert_eq!(inventory[0].management_tools.len(), 8);
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
    assert_eq!(actual.len(), 8);
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
    let (count, signed) = (requests.clone(), signatures.clone());
    let seller = move |headers: HeaderMap,
                       method: axum::http::Method,
                       uri: axum::http::Uri,
                       body: axum::body::Bytes| {
        let (count, signed) = (count.clone(), signed.clone());
        async move {
            count.fetch_add(1, Ordering::SeqCst);
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
                        return ([("content-type", "text/plain")], "x".repeat(1024))
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
        .route("/api/v1/services", get(seller.clone()).post(seller.clone()))
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
    let mut policy = policy(None);
    policy.max_response_bytes = 512;
    let manager = Manager::new(
        policy,
        ls,
        clients,
        Arc::new(CatalogState::new(snapshot)),
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(manager.directory("writer").unwrap().base, directory::BASE);
    // Use the public hostname on the fixture certificate; retain public-destination checks.
    *manager.fixture_directory.lock().unwrap() = Some("https://api.example.com/api/v1".into());
    assert!(manager.directory("hidden").is_err());
    assert_eq!(tools::definitions(true).len(), 8);
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
            raw::GET,
            json!({"url":"https://api.example.com/api/v1/services?existing=yes","query":{"q":"a & b","tags":["one","two"],"enabled":true,"limit":2}}),
        ),
        (
            raw::POST,
            json!({"url":"https://api.example.com/api/v1/services","query":{"q":"post"},"body":{"items":[1,"two"],"nested":{"ok":true}}}),
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
            if *name == raw::GET {
                assert_eq!(response["method"], "GET");
                assert_eq!(
                    response["query"],
                    "existing=yes&enabled=true&limit=2&q=a+%26+b&tags=one&tags=two"
                );
                assert!(response["body"].is_null());
                assert!(response["content_type"].is_null());
            } else if *name == raw::POST {
                assert_eq!(response["method"], "POST");
                assert_eq!(response["query"], "q=post");
                assert_eq!(response["body"], args["body"]);
                assert_eq!(response["content_type"], "application/json");
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
            assert_eq!(
                (record.user.clone(), record.password.clone()),
                crate::network::global().credentials(&IsolationId::evm(address).unwrap())
            );
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
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":raw::GET,"arguments":{"url":url}}}));
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
    assert_eq!(parsed["slug"], "vendor/weather?region=UK");
    assert_eq!(parsed["endpoints"][0]["path"], "/read");
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
    assert_eq!(schema["properties"]["mode"]["default"], "browse");
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
            let mut merged = schema["properties"][name].clone();
            let original_description = field["description"].as_str().unwrap();
            assert!(
                merged["description"]
                    .as_str()
                    .unwrap()
                    .contains(original_description)
            );
            merged["description"] = field["description"].clone();
            assert_eq!(merged, *field, "{path} {name}: schema constraint changed");
        }
    }
    task.abort();
    vendor_task.abort();
}

#[tokio::test]
async fn import_aliases_share_active_capacity_and_failed_fetches_recover() {
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
        aliases.spawn(async move {
            m.document("writer", "https://api.example.com/active", false)
                .await
        });
    }
    arrived.notified().await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // A different URL still has capacity despite twenty coalesced waiters.
    assert!(
        m.document("reader", "https://api.example.com/other", false)
            .await
            .is_ok()
    );
    let original = m.fetches.lock().unwrap()["https://api.example.com/active"].clone();
    // Churn completed retention beyond capacity; an active fetch is never evicted.
    for n in 0..12 {
        assert!(
            m.document(
                "reader",
                &format!("https://api.example.com/churn{n}"),
                false
            )
            .await
            .is_ok()
        );
    }
    assert!(Arc::ptr_eq(
        &original,
        &m.fetches.lock().unwrap()["https://api.example.com/active"]
    ));
    release.notify_one();
    while let Some(result) = aliases.join_next().await {
        assert!(result.unwrap().is_err());
    }
    let before = calls.load(Ordering::SeqCst);
    assert!(
        m.document("writer", "https://api.example.com/active", false)
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
        m.document("writer", "https://api.example.com/active", false)
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
async fn on_demand_management_keeps_eight_wrappers_and_scoped_dynamic_search() {
    let m = manager(None).await;
    add(&m, "demo", "", "").await;
    let mut writer = server(&m, "writer");
    writer.discover_on_demand = true;
    let mut reader = server(&m, "reader");
    reader.discover_on_demand = true;
    let (base, task) = listen(crate::server::http_app(writer, "test-token".into())).await;
    let (other, other_task) = listen(crate::server::http_app(reader, "test-token".into())).await;
    let listed = rpc(&base, "test-token", "tools/list", json!({})).await;
    assert_eq!(listed["tools"].as_array().unwrap().len(), 8);
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
            .find_reference("writer", item["tool_ref"].as_str().unwrap())
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
        json!({"tool_ref":items[0]["tool_ref"],"arguments":{}}),
    )
    .await;
    assert_eq!(denied["isError"], true);
    let mut snapshot = (*m.catalog.read()).clone();
    let template = snapshot.views["writer"][0].clone();
    snapshot.views.insert(
        "writer".into(),
        (0..6)
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
        5
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
    let mut policy = policy(None);
    policy.allowed_origins = vec!["https://api.example.com".into()];
    let manager = manager_with(policy, listeners(false)).await;
    for on_demand in [false, true] {
        let mut disabled = server(&manager, "hidden");
        disabled.discover_on_demand = on_demand;
        let mut enabled = server(&manager, "writer");
        enabled.discover_on_demand = on_demand;
        for name in [raw::GET, raw::POST] {
            assert!(disabled.get_tool(name).is_none());
            assert!(enabled.get_tool(name).is_some());
            let args = json!({"url":"https://api.example.com/read","body":{}});
            assert!(
                disabled
                    .invoke_output(name, args.as_object().unwrap())
                    .await
                    .is_err()
            );
            assert!(
                manager
                    .invoke_raw("hidden", name, args.as_object().unwrap())
                    .await
                    .is_err()
            );
        }
    }
    let standalone = Server::new(
        vec![],
        payer(),
        "https://api.example.com".into(),
        None,
        None,
    );
    for name in [raw::GET, raw::POST] {
        assert!(standalone.get_tool(name).is_none());
    }
    let enabled = server(&manager, "writer");
    for (name, args) in [
        (raw::GET, json!({"url":"http://api.example.com/read"})),
        (raw::GET, json!({"url":"https://127.0.0.1/read"})),
        (raw::GET, json!({"url":"https://[::1]/read"})),
        (raw::GET, json!({"url":"https://foo.local/read"})),
        (
            raw::GET,
            json!({"url":"https://user:password@api.example.com/read"}),
        ),
        (
            raw::GET,
            json!({"url":"https://api.example.com/read#fragment"}),
        ),
        (raw::GET, json!({"url":"https://other.example.com/read"})),
        (
            raw::GET,
            json!({"url":"https://api.example.com/read","body":null}),
        ),
        (
            raw::GET,
            json!({"url":"https://api.example.com/read","query":{"nested":{"a":1}}}),
        ),
        (
            raw::GET,
            json!({"url":"https://api.example.com/read","query":{"null":null}}),
        ),
        (
            raw::GET,
            json!({"url":"https://api.example.com/read","query":{"array":[{}]}}),
        ),
        (raw::POST, json!({"url":"https://api.example.com/read"})),
        (
            raw::POST,
            json!({"url":"https://api.example.com/read","body":{},"headers":{"Authorization":"secret"}}),
        ),
        (
            raw::POST,
            json!({"url":"https://api.example.com/read","body":{},"wallet":"other"}),
        ),
        (
            raw::POST,
            json!({"url":"https://api.example.com/read","body":{},"method":"DELETE"}),
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
                "unknown field"
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

#[path = "url_info_tests.rs"]
mod url_info_tests;

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
    let cell = m.fetches.lock().unwrap()["https://api.example.com/openapi.json"].clone();
    m.fetches.lock().unwrap().insert(second_url.into(), cell);
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
    assert_eq!(unchanged.help.get().unwrap(), "retained help");
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
    // Duplicate registration reports the current catalog, not the older durable
    // registration generation or a stale static base.
    let duplicate = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({"spec_url":second_url,"name":"second"}),
        )
        .await
        .unwrap();
    assert_eq!(duplicate["catalog_generation"], before.generation);
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

#[test]
fn source_policy_removes_registration_and_source_tool_quotas() {
    assert_eq!(policy(None).max_tools_per_server, 512);
    for removed in ["max_sources", "max_tools_per_source"] {
        let mut value = json!({"wallet":"shared"});
        value[removed] = json!(1);
        let error = serde_json::from_value::<policy::Policy>(value)
            .err()
            .unwrap();
        assert!(error.to_string().contains(removed), "{error}");
    }
    let serialized = serde_json::to_value(policy(None)).unwrap();
    assert!(serialized.get("max_sources").is_none());
    assert!(serialized.get("max_tools_per_source").is_none());
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
            result: Ok(Arc::new(serde_json::to_vec(&doc).unwrap())),
        })
        .unwrap();
        m.fetches
            .lock()
            .unwrap()
            .insert(url.clone(), Arc::new(cell));
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
async fn on_demand_endpoint_bypasses_quota_on_add_restart_and_static_publication() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let mut p = policy(Some(file));
    p.max_tools_per_server = 1;
    let mut configured = listeners(true);
    configured.get_mut("writer").unwrap().discover_on_demand = true;
    let m = manager_with(p.clone(), configured.clone()).await;
    add(&m, "on-demand", "", "").await;
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    let base = m.static_snapshot().await.unwrap();
    m.replace_static(base.generation, base).await.unwrap();
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    drop(m);
    let restored = manager_with(p, configured).await;
    assert_eq!(restored.catalog.read().views["writer"].len(), 2);
    let found = restored
        .invoke("writer", "x402_treazury_tools_search", json!({}))
        .await
        .unwrap();
    assert_eq!(found["items"].as_array().unwrap().len(), 2);
}
