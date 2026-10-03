use super::*;
use crate::{
    payment::{Payer, SpendPolicy},
    server::Server,
};

fn policy(file: Option<PathBuf>) -> policy::Policy {
    serde_json::from_value(json!({"wallet":"shared","registry_file":file})).unwrap()
}
fn listeners(persistent: bool) -> BTreeMap<String, ListenerConfig> {
    ["writer","reader","hidden"].into_iter().map(|id| {
        let enabled=id=="writer";
        (id.into(),serde_json::from_value(json!({"listen":"127.0.0.1:0","bearer_token_env":"TEST_TOKEN","sources":[],"source_management":{"enabled":enabled,"accept_sources":id!="hidden","allowed_targets":["writer","reader"],"allow_process_scope":enabled,"allow_persistence":enabled&&persistent}})).unwrap())
    }).collect()
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
async fn add(m: &Arc<Manager>, name: &str, lifetime: &str, visibility: &str) -> Value {
    seed(m).await;
    m.invoke(
        "writer",
        "treazure_source_add",
        json!({"candidate":candidate(name,lifetime,visibility),"idempotency_key":name}),
    )
    .await
    .unwrap()
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
#[tokio::test]
async fn scopes_permissions_idempotency_and_revision_guards() {
    let m = manager(None).await;
    let result = add(&m, "demo", "process", "process").await;
    assert_eq!(result["targets"], json!(["reader", "writer"]));
    assert_eq!(
        result["wallet_profiles"],
        json!({"reader":"shared","writer":"shared"})
    );
    let snap = m.catalog.read();
    assert_eq!(snap.views["writer"].len(), 2);
    assert_eq!(snap.views["reader"].len(), 2);
    assert!(snap.views["hidden"].is_empty());
    let a = json!({"candidate":candidate("demo","process","process"),"idempotency_key":"demo"});
    assert_eq!(
        m.invoke("writer", "treazure_source_add", a.clone())
            .await
            .unwrap(),
        result
    );
    let mut changed = a;
    changed["candidate"]["name"] = json!("other");
    assert!(
        m.invoke("writer", "treazure_source_add", changed)
            .await
            .unwrap_err()
            .to_string()
            .contains("idempotency_conflict")
    );
    let remove =
        json!({"source_id":result["source_id"],"expected_revision":1,"idempotency_key":"remove"});
    assert!(
        m.invoke("reader", "treazure_source_remove", remove.clone())
            .await
            .is_err()
    );
    assert!(
        m.invoke("hidden", "treazure_sources_list", json!({}))
            .await
            .is_err()
    );
    let visible = m
        .invoke("reader", "treazure_sources_list", json!({}))
        .await
        .unwrap();
    assert_eq!(visible["items"][0]["targets"], json!(["reader"]));
    assert_eq!(visible["items"][0]["can_manage"], false);
    let search = m
        .invoke("writer", "treazure_tools_search", json!({"limit":1}))
        .await
        .unwrap();
    let tool = search["items"][0]["tool_id"].as_str().unwrap();
    let fallback = server(&m, "writer")
        .invoke(
            "treazure_tool_call",
            json!({"tool_id":tool,"arguments":{},"expected_revision":0})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(fallback.to_string().contains("revision_conflict"));
    let removed = m
        .invoke("writer", "treazure_source_remove", remove.clone())
        .await
        .unwrap();
    assert_eq!(
        m.invoke("writer", "treazure_source_remove", remove)
            .await
            .unwrap(),
        removed
    );
    assert!(m.catalog.read().views["writer"].is_empty());
    assert_eq!(snap.views["writer"].len(), 2);
    assert!(
        m.invoke(
            "writer",
            "treazure_tools_search",
            json!({"cursor":search["next_cursor"]})
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("stale_cursor")
    );
    assert!(
        server(&m, "writer")
            .invoke(tool, &Default::default())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn preview_pagination_filters_and_atomic_quota_races() {
    let mut ls = listeners(false);
    ls.get_mut("reader").unwrap().exclude_tags = vec!["write".into()];
    let mut p = policy(None);
    p.max_sources = 1;
    let m = manager_with(p, ls).await;
    seed(&m).await;
    let preview = m
        .invoke(
            "writer",
            "treazure_source_preview",
            json!({"candidate":candidate("demo","process","process"),"limit":1}),
        )
        .await
        .unwrap();
    assert_eq!(preview["items"].as_array().unwrap().len(), 1);
    assert!(m.catalog.read().views["writer"].is_empty());
    let next = m
        .invoke(
            "writer",
            "treazure_source_preview",
            json!({"preview_id":preview["preview_id"],"cursor":preview["next_cursor"],"limit":1}),
        )
        .await
        .unwrap();
    assert_ne!(preview["items"], next["items"]);
    let args = json!({"candidate":candidate("demo","process","process"),"preview_id":preview["preview_id"],"idempotency_key":"same"});
    let (a, b) = tokio::join!(
        m.invoke("writer", "treazure_source_add", args.clone()),
        m.invoke("writer", "treazure_source_add", args)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(m.catalog.read().views["reader"].len(), 1);
    assert!(
        m.invoke(
            "writer",
            "treazure_source_add",
            json!({"candidate":candidate("second","process","process"),"idempotency_key":"second"})
        )
        .await
        .is_err()
    );
    assert_eq!(m.catalog.read().generation, 1);
    let id = preview["source_id"].clone();
    let u = json!({"source_id":id,"expected_revision":1,"idempotency_key":"update","selection":{"tags":["read"]}});
    let r = json!({"source_id":id,"expected_revision":1,"idempotency_key":"remove"});
    let (a, b) = tokio::join!(
        m.invoke("writer", "treazure_source_update", u),
        m.invoke("writer", "treazure_source_remove", r)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(m.catalog.read().generation, 2);
}
#[tokio::test]
async fn persistence_exact_bytes_replay_withdrawal_and_revocation() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("sources.sqlite");
    let m = manager(Some(file.clone())).await;
    let first = add(&m, "saved", "persistent", "process").await;
    add(&m, "temporary", "process", "server").await;
    let id = first["source_id"].clone();
    drop(m);
    let m = manager(Some(file.clone())).await;
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
    let args =
        json!({"candidate":candidate("saved","persistent","process"),"idempotency_key":"saved"});
    assert_eq!(
        m.invoke("writer", "treazure_source_add", args)
            .await
            .unwrap(),
        first
    ); // no seeded cache or external fetch
    m.invoke("writer","treazure_source_update",json!({"source_id":id,"expected_revision":1,"idempotency_key":"withdraw","lifetime":"process"})).await.unwrap();
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    drop(m);
    let m = manager(Some(file.clone())).await;
    assert!(m.catalog.read().views["writer"].is_empty());
    let second = add(&m, "another", "persistent", "process").await;
    drop(m);
    let mut ls = listeners(true);
    ls.get_mut("reader")
        .unwrap()
        .source_management
        .as_mut()
        .unwrap()
        .accept_sources = false;
    let m = manager_with(policy(Some(file.clone())), ls).await;
    assert!(m.catalog.read().views["writer"].is_empty());
    let list = m
        .invoke("writer", "treazure_sources_list", json!({}))
        .await
        .unwrap();
    assert!(list["items"][0]["disabled"].is_string());
    m.invoke(
        "writer",
        "treazure_source_remove",
        json!({"source_id":second["source_id"],"expected_revision":1,"idempotency_key":"remove"}),
    )
    .await
    .unwrap();
    drop(m);
    let report = store::inspect(&file).unwrap();
    assert!(
        report["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["removed"] == true)
    );
}
#[tokio::test]
async fn promotion_restores_fixed_targets_and_format_changes_disable() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("sources.sqlite");
    let m = manager(Some(file.clone())).await;
    let r = add(&m, "promote", "process", "server").await;
    m.invoke("writer","treazure_source_update",json!({"source_id":r["source_id"],"expected_revision":1,"idempotency_key":"promote","lifetime":"persistent","name":"renamed"})).await.unwrap();
    drop(m);
    let m = manager(Some(file.clone())).await;
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    assert!(m.catalog.read().views["reader"].is_empty());
    {
        let mut inner = m.inner.lock().unwrap();
        inner.state.records.values_mut().next().unwrap().format = 99;
        let state = inner.state.clone();
        inner.store.as_mut().unwrap().save(&state).unwrap();
    }
    drop(m);
    let m = manager(Some(file)).await;
    assert!(m.catalog.read().views["writer"].is_empty());
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
            .as_ref()
            .unwrap()
            .contains("format")
    );
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

#[tokio::test]
async fn durable_commit_is_recovered_and_failed_writes_do_not_publish() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("sources.sqlite");
    let m = manager(Some(file.clone())).await;
    seed(&m).await;
    m.fail_after_commit
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let args =
        json!({"candidate":candidate("saved","persistent","server"),"idempotency_key":"crash"});
    assert!(
        m.invoke("writer", "treazure_source_add", args.clone())
            .await
            .is_err()
    );
    assert!(m.catalog.read().views["writer"].is_empty());
    drop(m);
    let m = manager(Some(file.clone())).await;
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    let r = m
        .invoke("writer", "treazure_source_add", args)
        .await
        .unwrap();
    m.inner
        .lock()
        .unwrap()
        .store
        .as_ref()
        .unwrap()
        .reject_writes();
    assert!(m.invoke("writer","treazure_source_remove",json!({"source_id":r["source_id"],"expected_revision":1,"idempotency_key":"failed-write"})).await.is_err());
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
    drop(m);
    assert_eq!(
        manager(Some(file)).await.catalog.read().views["writer"].len(),
        2
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)] // Deliberately stall only the blocking commit worker.
async fn accepted_mutation_survives_cancelled_waiter() {
    let m = manager(None).await;
    let r = add(&m, "cancel", "process", "server").await;
    let id = r["source_id"].as_str().unwrap().to_owned();
    let lock = m.inner.lock().unwrap();
    let worker = m.clone();
    let task = tokio::spawn(async move {
        worker
            .commit(
                "writer".into(),
                "remove:cancel".into(),
                json!({"source_id":id}),
                Change::Remove(id, 1),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while m.mutations.available_permits() == 16 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    drop(lock);
    tokio::time::timeout(Duration::from_secs(3), async {
        while !m.catalog.read().views["writer"].is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(m.catalog.read().generation, 2);
}
#[tokio::test]
async fn config_is_strict_inspection_is_offline_and_paths_are_protected() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("deployment.toml");
    let text = r#"version=1
[wallets.shared]
mode="static"
private_key_env="NEVER_READ_ME"
[source_management]
wallet="shared"
registry_file="registry.sqlite"
[servers.writer]
listen="127.0.0.1:0"
bearer_token_env="NEVER_READ_TOKEN"
[servers.writer.source_management]
enabled=true
accept_sources=true
allow_persistence=true
"#;
    std::fs::write(&path, text).unwrap();
    let shown = crate::deployment::Deployment::show_config(&path)
        .await
        .unwrap();
    assert_eq!(
        shown["source_management"]["wallet_bindings"]["writer"],
        "shared"
    );
    assert!(!tmp.path().join("registry.sqlite").exists());
    for (from, to) in [
        ("wallet=\"shared\"", "wallet=\"unknown\""),
        (
            "enabled=true",
            "enabled=true\nallowed_targets=[\"missing\"]",
        ),
        ("accept_sources=true", "accept_sources=false"),
        (
            "registry_file=\"registry.sqlite\"",
            "registry_file=\"deployment.toml\"",
        ),
        ("registry_file=\"registry.sqlite\"", "max_sources=0"),
        ("wallet=\"shared\"", "wallet=\"shared\"\nunknown=1"),
    ] {
        std::fs::write(&path, text.replace(from, to)).unwrap();
        assert!(
            crate::deployment::Deployment::show_config(&path)
                .await
                .is_err(),
            "{to}"
        );
    }
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
#[tokio::test]
async fn real_http_cached_client_management_and_payment_fallback() {
    use axum::{
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use base64::Engine;
    let m = manager(None).await;
    seed(&m).await;
    let (writer, w) = listen(crate::server::http_app(
        server(&m, "writer"),
        "test-token".into(),
    ))
    .await;
    let (reader, r) = listen(crate::server::http_app(
        server(&m, "reader"),
        "test-token".into(),
    ))
    .await;
    let init=rpc(&writer,"test-token","initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"cached","version":"1"}})).await;
    assert_ne!(init["capabilities"]["tools"]["listChanged"], true);
    let initial = rpc(&writer, "test-token", "tools/list", json!({})).await;
    assert_eq!(initial["tools"].as_array().unwrap().len(), 7);
    let receiver = rpc(&reader, "test-token", "tools/list", json!({})).await;
    assert_eq!(receiver["tools"].as_array().unwrap().len(), 3);
    let unauthorized = crate::network::discovery(&writer, Duration::from_secs(5))
        .unwrap()
        .post(format!("{writer}/mcp"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    let preview = call(
        &writer,
        "treazure_source_preview",
        json!({"candidate":candidate("http","process","process")}),
    )
    .await;
    assert_ne!(preview["isError"], true, "{preview}");
    let added=call(&writer,"treazure_source_add",json!({"candidate":candidate("http","process","process"),"preview_id":preview["structuredContent"]["preview_id"],"idempotency_key":"http"})).await;
    assert_ne!(added["isError"], true, "{added}");
    let id = added["structuredContent"]["source_id"].clone();
    let search = call(&reader, "treazure_tools_search", json!({"query":"Read"})).await;
    let name = search["structuredContent"]["items"][0]["tool_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let blocked = call(
        &reader,
        "treazure_source_remove",
        json!({"source_id":id,"expected_revision":1,"idempotency_key":"wrong-owner"}),
    )
    .await;
    assert_eq!(blocked["isError"], true);
    // Explicit test-only fixture routing: import still validates public URLs. Production
    // has no field or flag that can turn off the public-destination guard.
    let price = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let unsigned = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let signed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (p, u, s) = (price.clone(), unsigned.clone(), signed.clone());
    let (vendor,v)=listen(axum::Router::new().route("/read",get(move |headers:HeaderMap| {let(p,u,s)=(p.clone(),u.clone(),s.clone());async move {
        if let Some(signature)=headers.get("payment-signature") {s.fetch_add(1,std::sync::atomic::Ordering::SeqCst);let value:Value=serde_json::from_slice(&base64::engine::general_purpose::STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();return axum::Json(json!({"paid":true,"payer":value["payload"]["authorization"]["from"]})).into_response();}
        u.fetch_add(1,std::sync::atomic::Ordering::SeqCst);let amount=p.load(std::sync::atomic::Ordering::SeqCst);if amount==0 {return "free".into_response()}
        let challenge=json!({"x402Version":2,"resource":{"url":"https://api.example.com/read","description":"read","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":amount.to_string(),"payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
        (StatusCode::PAYMENT_REQUIRED,[("payment-required",base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
    }}))).await;
    let mut snapshot = (*m.catalog.read()).clone();
    for view in snapshot.views.values_mut() {
        for bound in view {
            bound.base = vendor.clone();
            bound.client = m.payers["shared"].clone();
        }
    }
    m.catalog.publish(snapshot);
    let args = json!({"tool_id":name,"arguments":{},"expected_revision":1});
    let free = call(&reader, "treazure_tool_call", args.clone()).await;
    assert_eq!(free["content"][0]["text"], "free");
    price.store(20000, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        call(&reader, "treazure_tool_call", args.clone()).await["isError"],
        true
    );
    assert_eq!(signed.load(std::sync::atomic::Ordering::SeqCst), 0);
    price.store(5000, std::sync::atomic::Ordering::SeqCst);
    let paid = call(&writer, "treazure_tool_call", args.clone()).await;
    assert_ne!(paid["isError"], true, "{paid}");
    assert_eq!(signed.load(std::sync::atomic::Ordering::SeqCst), 1);
    let update=call(&writer,"treazure_source_update",json!({"source_id":id,"expected_revision":1,"idempotency_key":"update","selection":{"tags":["read"]}})).await;
    assert_ne!(update["isError"], true, "{update}");
    let before = unsigned.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        call(&reader, "treazure_tool_call", args).await["isError"],
        true
    );
    assert_eq!(unsigned.load(std::sync::atomic::Ordering::SeqCst), before);
    let removed = call(
        &writer,
        "treazure_source_remove",
        json!({"source_id":id,"expected_revision":2,"idempotency_key":"remove"}),
    )
    .await;
    assert_ne!(removed["isError"], true, "{removed}");
    assert!(
        call(&reader, "treazure_tools_search", json!({})).await["structuredContent"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    w.abort();
    r.abort();
    v.abort();
}

#[tokio::test]
async fn bounded_fetch_deadlines_redirects_coalescing_and_explicit_refresh() {
    use axum::{response::Redirect, routing::get};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let app = axum::Router::new()
        .route(
            "/spec",
            get(move || {
                let h = h.clone();
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    axum::Json(spec())
                }
            }),
        )
        .route("/redirect", get(|| async { Redirect::temporary("/spec") }))
        .route("/large", get(|| async { "x".repeat(4096) }))
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                axum::Json(spec())
            }),
        );
    let (url, task) = listen(app).await;
    let mut p = policy(None);
    p.max_spec_bytes = 1024;
    p.fetch_timeout_seconds = 1;
    for route in ["large", "redirect", "slow"] {
        assert!(
            import::fixture_fetch(&p, &format!("{url}/{route}"))
                .await
                .is_err(),
            "{route}"
        );
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let mut ls = listeners(false);
    ls.get_mut("reader")
        .unwrap()
        .source_management
        .as_mut()
        .unwrap()
        .enabled = true;
    let m = manager_with(policy(None), ls).await;
    *m.fixture_endpoint.lock().unwrap() = Some(format!("{url}/spec"));
    let a = json!({"candidate":candidate("one","process","server")});
    let (a, b) = tokio::join!(
        m.invoke("writer", "treazure_source_preview", a.clone()),
        m.invoke("reader", "treazure_source_preview", a)
    );
    assert!(a.is_ok() && b.is_ok());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let added = m
        .invoke(
            "writer",
            "treazure_source_add",
            json!({"candidate":candidate("one","process","server"),"idempotency_key":"add"}),
        )
        .await
        .unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    m.invoke("writer","treazure_source_update",json!({"source_id":added["source_id"],"expected_revision":1,"idempotency_key":"refresh","refresh_spec":true})).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    task.abort();
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
    let ls = listeners(false);
    let snapshot = CatalogSnapshot {
        generation: 0,
        views: ls.keys().map(|k| (k.clone(), vec![])).collect(),
    };
    let m = Manager::new(
        policy(None),
        ls,
        BTreeMap::from([("shared".into(), managed)]),
        Arc::new(CatalogState::new(snapshot)),
        vec![],
    )
    .await
    .unwrap();
    let a = add(&m, "one", "process", "process").await;
    add(&m, "two", "process", "process").await;
    m.invoke(
        "writer",
        "treazure_source_remove",
        json!({"source_id":a["source_id"],"expected_revision":1,"idempotency_key":"remove"}),
    )
    .await
    .unwrap();
    let after = store
        .call(|s| Ok(serde_json::to_value(s.status()?)?))
        .await
        .unwrap();
    assert_eq!(before, after);
    drop(m);
    drop(store);
    worker.await.unwrap();
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
    let file = PathBuf::from(std::env::var("TREAZURE_TEST_REGISTRY").unwrap());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let m=manager(Some(file)).await;seed(&m).await;
        m.crash_after_commit.store(true,std::sync::atomic::Ordering::SeqCst);
        let _=m.invoke("writer","treazure_source_add",json!({"candidate":candidate("crash","persistent","server"),"idempotency_key":"crash"})).await;
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
        .env("TREAZURE_TEST_REGISTRY", &file)
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
            "treazure_source_add",
            json!({"candidate":candidate("crash","persistent","server"),"idempotency_key":"crash"}),
        )
        .await
        .unwrap();
    assert_eq!(r["revision"], 1);
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
}
#[tokio::test]
async fn actual_mcp_pagination_and_cross_query_cursors_reject_stale_views() {
    let mut p = policy(None);
    p.max_tools_per_source = 150;
    let m = manager_with(p, listeners(false)).await;
    let mut doc = spec();
    doc["paths"] = json!({});
    for n in 0..105 {
        doc["paths"][format!("/item{n}")] = json!({"get":{"description":"item"}});
    }
    let cell = OnceCell::new();
    cell.set(Ok(Arc::new(serde_json::to_vec(&doc).unwrap())))
        .unwrap();
    m.fetches.lock().unwrap().insert(
        "https://api.example.com/openapi.json".into(),
        (Instant::now(), Arc::new(cell)),
    );
    let added = m
        .invoke(
            "writer",
            "treazure_source_add",
            json!({"candidate":candidate("pages","process","server"),"idempotency_key":"pages"}),
        )
        .await
        .unwrap();
    let (base, task) = listen(crate::server::http_app(
        server(&m, "writer"),
        "test-token".into(),
    ))
    .await;
    let first = rpc(&base, "test-token", "tools/list", json!({})).await;
    assert_eq!(first["tools"].as_array().unwrap().len(), 100);
    let second = rpc(
        &base,
        "test-token",
        "tools/list",
        json!({"cursor":first["nextCursor"]}),
    )
    .await;
    assert_eq!(second["tools"].as_array().unwrap().len(), 12);
    let search = m
        .invoke("writer", "treazure_tools_search", json!({"limit":1}))
        .await
        .unwrap();
    assert!(
        m.invoke(
            "writer",
            "treazure_tools_search",
            json!({"query":"different","cursor":search["next_cursor"]})
        )
        .await
        .is_err()
    );
    m.invoke(
        "writer",
        "treazure_source_remove",
        json!({"source_id":added["source_id"],"expected_revision":1,"idempotency_key":"remove"}),
    )
    .await
    .unwrap();
    assert!(
        rpc(
            &base,
            "test-token",
            "tools/list",
            json!({"cursor":first["nextCursor"]})
        )
        .await
        .is_null()
    );
    task.abort();
}

#[tokio::test]
async fn explicit_wallet_override_and_parallel_add_quota_are_enforced() {
    let mut ls = listeners(false);
    ls.get_mut("reader")
        .unwrap()
        .source_management
        .as_mut()
        .unwrap()
        .wallet = Some("reader_wallet".into());
    let snapshot = CatalogSnapshot {
        generation: 0,
        views: ls.keys().map(|id| (id.clone(), vec![])).collect(),
    };
    let mut p = policy(None);
    p.max_sources = 1;
    let shared = payer();
    let other = payer();
    let m = Manager::new(
        p,
        ls,
        BTreeMap::from([
            ("shared".into(), shared.clone()),
            ("reader_wallet".into(), other.clone()),
        ]),
        Arc::new(CatalogState::new(snapshot)),
        vec![],
    )
    .await
    .unwrap();
    seed(&m).await;
    let (a, b) = tokio::join!(
        m.invoke(
            "writer",
            "treazure_source_add",
            json!({"candidate":candidate("a","process","process"),"idempotency_key":"a"})
        ),
        m.invoke(
            "writer",
            "treazure_source_add",
            json!({"candidate":candidate("b","process","process"),"idempotency_key":"b"})
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(m.catalog.read().generation, 1);
    let snapshot = m.catalog.read();
    assert!(
        snapshot.views["writer"][0]
            .client
            .shares_profile_with(&shared)
    );
    assert!(
        snapshot.views["reader"][0]
            .client
            .shares_profile_with(&other)
    );
    assert!(
        !snapshot.views["reader"][0]
            .client
            .shares_profile_with(&shared)
    );
}

// The real import and public-only payer are exercised in a fresh process because
// all production outbound paths share one immutable process network policy.
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
    drop(records);
    task.abort();
}

#[path = "../../tests/support/permissions.rs"]
mod permissions;
