use super::*;
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
    cell.set(Ok(Arc::new(serde_json::to_vec(&spec()).unwrap())))
        .unwrap();
    m.fetches.lock().unwrap().insert(
        "https://api.example.com/openapi.json".into(),
        (Instant::now(), Arc::new(cell)),
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
        .json::<Value>()
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
    // Tool-count limits apply to the selected inventory, after document validation.
    p.max_tools_per_source = 1;
    assert_eq!(build(&p, &c, &spec()).unwrap().tools.len(), 1);
    c.selection.include.clear();
    assert!(
        build(&p, &c, &spec())
            .unwrap_err()
            .to_string()
            .contains("source_tool_limit")
    );

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
    for name in ["sources_search", "source_add", "tools_search", "tool_call"] {
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
async fn quota_races_never_publish_partial_sources() {
    let mut p = policy(None);
    p.max_sources = 1;
    let m = manager_with(p, listeners(false)).await;
    seed(&m).await;
    let args = json!({"spec_url":"https://api.example.com/openapi.json"});
    let (a, b) = tokio::join!(
        m.invoke("writer", "x402_treazury_source_add", args.clone()),
        m.invoke("reader", "x402_treazury_source_add", args)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
    assert_eq!(
        m.catalog.read().views.values().map(Vec::len).sum::<usize>(),
        2
    );
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
    let cfg = crate::catalog::Config::default();
    let tools=crate::catalog::build_tools(&cfg,&json!({"paths":{"/services":{"get":{"parameters":[{"in":"query","name":"q","schema":{"type":"string"}}]}}}}),"x402_list").unwrap();
    let mut snap = (*m.catalog.read()).clone();
    snap.views.get_mut("writer").unwrap().push(BoundTool {
        tool: tools[0].clone(),
        client: payer(),
        base: vendor,
        source: None,
        help: Arc::new(OnceCell::new()),
    });
    m.catalog.publish(snap);
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
    let pool = store.ensure_pool("shared", "5").unwrap();
    let before = serde_json::to_value(store.status().unwrap()).unwrap();
    let (store, worker) = StoreHandle::spawn(store);
    let managed = PaidClient::managed(Arc::new(
        ManagedPool::new(
            store.clone(),
            pool,
            BaseRpc::new("http://127.0.0.1:1/rpc", 12, 120).unwrap(),
            "5",
            SpendPolicy::dollars("0.01").unwrap(),
            1,
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
