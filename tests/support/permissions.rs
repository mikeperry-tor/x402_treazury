//! AUTH cases: permission boundaries rather than only generated tool counts.
use super::*;

fn writers(persistent: bool) -> BTreeMap<String, ListenerConfig> {
    let mut ls = listeners(persistent);
    let mut second = ls["writer"].clone();
    second.source_management.as_mut().unwrap().allowed_targets =
        Some(vec!["writer".into(), "reader".into(), "second".into()]);
    ls.insert("second".into(), second);
    ls.get_mut("writer")
        .unwrap()
        .source_management
        .as_mut()
        .unwrap()
        .allowed_targets = Some(vec!["writer".into(), "reader".into(), "second".into()]);
    ls
}
#[tokio::test]
async fn target_grants_and_persistence_boundaries_are_explicit() {
    for (visibility, targets, process, persist, accept, allowed, ok) in [
        ("server", vec![], false, false, true, None, true),
        ("server", vec!["reader"], true, false, true, None, false),
        ("servers", vec![], true, false, true, None, false),
        ("servers", vec!["reader"], true, false, true, None, false),
        (
            "servers",
            vec!["writer", "reader", "reader"],
            false,
            false,
            true,
            Some(vec!["writer", "reader"]),
            true,
        ),
        (
            "servers",
            vec!["missing"],
            true,
            false,
            true,
            Some(vec!["missing"]),
            false,
        ),
        (
            "servers",
            vec!["reader"],
            true,
            false,
            false,
            Some(vec!["reader"]),
            false,
        ),
        (
            "process",
            vec![],
            false,
            false,
            true,
            Some(vec!["writer", "reader"]),
            false,
        ),
        (
            "process",
            vec![],
            true,
            false,
            true,
            Some(vec!["writer"]),
            false,
        ),
        (
            "process",
            vec![],
            true,
            false,
            true,
            Some(vec!["writer", "reader"]),
            true,
        ),
        (
            "process",
            vec!["writer"],
            true,
            false,
            true,
            Some(vec!["writer", "reader"]),
            false,
        ),
        ("server", vec![], false, true, true, None, false),
    ] {
        let mut ls = listeners(false);
        let g = ls
            .get_mut("writer")
            .unwrap()
            .source_management
            .as_mut()
            .unwrap();
        g.allow_process_scope = process;
        g.allowed_targets = allowed.map(|v| v.into_iter().map(str::to_owned).collect());
        ls.get_mut("reader")
            .unwrap()
            .source_management
            .as_mut()
            .unwrap()
            .accept_sources = accept;
        let m = manager_with(policy(None), ls).await;
        seed(&m).await;
        let mut c = candidate(
            "matrix",
            if persist { "persistent" } else { "process" },
            visibility,
        );
        c["targets"] = json!(targets);
        let result = m
            .invoke("writer", "treazure_source_preview", json!({"candidate":c}))
            .await;
        assert_eq!(result.is_ok(), ok, "{visibility} {targets:?}: {result:?}");
        assert_eq!(m.catalog.read().generation, 0);
        assert!(m.inner.lock().unwrap().state.records.is_empty());
        if let Ok(result) = result {
            let targets = result["targets"].as_array().unwrap();
            assert_eq!(
                targets.len(),
                targets
                    .iter()
                    .map(Value::to_string)
                    .collect::<BTreeSet<_>>()
                    .len()
            );
        }
    }
}
#[tokio::test]
async fn authenticated_listener_controls_authority_and_hidden_invocations() {
    let mut ls = writers(false);
    ls.get_mut("reader").unwrap().exclude_tags = vec!["write".into()];
    let m = manager_with(policy(None), ls).await;
    seed(&m).await;
    let (writer, w) = listen(crate::server::http_app(
        server(&m, "writer"),
        "writer-token".into(),
    ))
    .await;
    let (reader, r) = listen(crate::server::http_app(
        server(&m, "reader"),
        "reader-token".into(),
    ))
    .await;
    let (second, s) = listen(crate::server::http_app(
        server(&m, "second"),
        "second-token".into(),
    ))
    .await;
    let http = crate::network::discovery(&reader, Duration::from_secs(2)).unwrap();
    let response = http
        .post(format!("{reader}/mcp"))
        .bearer_auth("writer-token")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let call_at = |url: String, token: &'static str, name: &'static str, args: Value| async move {
        rpc(
            &url,
            token,
            "tools/call",
            json!({"name":name,"arguments":args}),
        )
        .await
    };
    let added = call_at(
        writer.clone(),
        "writer-token",
        "treazure_source_add",
        json!({"candidate":candidate("http-auth","process","process"),"idempotency_key":"one"}),
    )
    .await;
    assert_ne!(added["isError"], true, "{added}");
    let id = added["structuredContent"]["source_id"].clone();
    let generation = m.catalog.read().generation;
    let hidden = m.catalog.read().views["writer"]
        .iter()
        .find(|t| t.tool.method == "POST")
        .unwrap()
        .tool
        .name
        .clone();
    for (url, token) in [(&reader, "reader-token"), (&second, "second-token")] {
        for name in ["treazure_source_update", "treazure_source_remove"] {
            let result = call_at(
                url.clone(),
                token,
                name,
                json!({"source_id":id,"expected_revision":1,"idempotency_key":name}),
            )
            .await;
            assert_eq!(result["isError"], true, "{result}");
        }
    }
    let direct = rpc(
        &reader,
        "reader-token",
        "tools/call",
        json!({"name":hidden,"arguments":{}}),
    )
    .await;
    assert_eq!(direct["isError"], true);
    assert!(
        direct["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool")
    );
    let fallback = call_at(
        reader.clone(),
        "reader-token",
        "treazure_tool_call",
        json!({"tool_id":hidden,"expected_revision":1,"arguments":{}}),
    )
    .await;
    assert_eq!(fallback["isError"], true);
    assert!(
        fallback["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool")
    );
    for field in ["owner", "wallet", "private_key", "registry_file"] {
        let mut c = candidate("smuggle", "process", "server");
        c[field] = json!("second");
        let result = call_at(
            writer.clone(),
            "writer-token",
            "treazure_source_add",
            json!({"candidate":c,"idempotency_key":field}),
        )
        .await;
        assert_eq!(result["isError"], true, "{field}");
    }
    assert_eq!(m.catalog.read().generation, generation);
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
    let list = rpc(&reader, "reader-token", "tools/list", json!({})).await;
    assert!(
        !list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == hidden)
    );
    // Even deliberately shared bearer text never changes the listener principal.
    let (shared, task) = listen(crate::server::http_app(
        server(&m, "reader"),
        "writer-token".into(),
    ))
    .await;
    let denied=call_at(shared,"writer-token","treazure_source_add",json!({"candidate":candidate("shared-token","process","server"),"idempotency_key":"shared"})).await;
    assert_eq!(denied["isError"], true);
    assert_eq!(m.catalog.read().generation, generation);
    for task in [w, r, s, task] {
        task.abort();
    }
}
#[tokio::test]
async fn owner_receipts_and_expiring_previews_do_not_cross_principals() {
    let m = manager_with(policy(None), writers(false)).await;
    seed(&m).await;
    let preview = m
        .invoke(
            "writer",
            "treazure_source_preview",
            json!({"candidate":candidate("one","process","server"),"limit":1}),
        )
        .await
        .unwrap();
    let pid = preview["preview_id"].clone();
    assert!(
        m.invoke(
            "second",
            "treazure_source_preview",
            json!({"preview_id":pid})
        )
        .await
        .is_err()
    );
    assert!(m.invoke("second","treazure_source_add",json!({"candidate":candidate("one","process","server"),"preview_id":pid,"idempotency_key":"x"})).await.is_err());
    assert!(m.invoke("writer","treazure_source_add",json!({"candidate":candidate("changed","process","server"),"preview_id":pid,"idempotency_key":"x"})).await.is_err());
    m.previews
        .lock()
        .unwrap()
        .get_mut(pid.as_str().unwrap())
        .unwrap()
        .created = Instant::now() - Duration::from_secs(301);
    assert!(
        m.invoke(
            "writer",
            "treazure_source_preview",
            json!({"preview_id":pid})
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("expired")
    );
    let mut ids = vec![];
    for owner in ["writer", "second"] {
        let result=m.invoke(owner,"treazure_source_add",json!({"candidate":candidate("same","process","server"),"idempotency_key":"same-key"})).await.unwrap();
        assert_eq!(result["targets"], json!([owner]));
        ids.push(result["source_id"].clone());
    }
    assert_ne!(ids[0], ids[1]);
    assert_eq!(m.catalog.read().generation, 2);
}
#[tokio::test]
async fn restore_keeps_targets_fixed_and_disables_revoked_records() {
    for change in [
        "new-listener",
        "owner-removed",
        "targets-narrowed",
        "origins-narrowed",
        "persistence-revoked",
        "quota-reduced",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("sources.sqlite");
        let m = manager(Some(file.clone())).await;
        add(&m, "saved-a", "persistent", "process").await;
        add(&m, "saved-b", "persistent", "process").await;
        drop(m);
        let mut ls = listeners(true);
        let mut p = policy(Some(file.clone()));
        match change {
            "new-listener" => {
                ls.insert("new".into(), ls["reader"].clone());
            }
            "owner-removed" => {
                ls.remove("writer");
            }
            "targets-narrowed" => {
                ls.get_mut("writer")
                    .unwrap()
                    .source_management
                    .as_mut()
                    .unwrap()
                    .allowed_targets = Some(vec!["writer".into()])
            }
            "origins-narrowed" => p.allowed_origins = vec!["https://different.example.com".into()],
            "persistence-revoked" => {
                ls.get_mut("writer")
                    .unwrap()
                    .source_management
                    .as_mut()
                    .unwrap()
                    .allow_persistence = false
            }
            "quota-reduced" => p.max_sources = 1,
            _ => unreachable!(),
        }
        let m = manager_with(p, ls).await;
        assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
        if change == "new-listener" {
            assert!(m.catalog.read().views["new"].is_empty());
            assert_eq!(m.catalog.read().views["writer"].len(), 4);
        } else {
            assert!(
                m.catalog.read().views.values().all(Vec::is_empty),
                "{change}"
            );
            assert!(
                m.inner
                    .lock()
                    .unwrap()
                    .state
                    .records
                    .values()
                    .all(|r| r.disabled.is_some()),
                "{change}"
            );
        }
        drop(m);
        assert_eq!(
            store::inspect(&file).unwrap()["sources"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
#[tokio::test]
async fn quota_boundaries_never_publish_a_partial_multi_listener_view() {
    for quota in ["owner", "source", "listener"] {
        let mut p = policy(None);
        let mut ls = listeners(false);
        match quota {
            "owner" => {
                ls.get_mut("writer")
                    .unwrap()
                    .source_management
                    .as_mut()
                    .unwrap()
                    .max_owned_sources = 1
            }
            "source" => p.max_tools_per_source = 1,
            "listener" => {
                p.max_tools_per_server = 3;
                ls.get_mut("reader").unwrap().exclude_tags = vec!["write".into()];
            }
            _ => unreachable!(),
        }
        let m = manager_with(p, ls).await;
        seed(&m).await;
        let mut c = candidate("first", "process", "process");
        if quota == "source" {
            c["selection"] = json!({"tags":["read"]});
        }
        let a = m
            .invoke(
                "writer",
                "treazure_source_add",
                json!({"candidate":c,"idempotency_key":"first"}),
            )
            .await
            .unwrap();
        let before = m.catalog.read();
        let result = if quota == "source" {
            m.invoke("writer","treazure_source_update",json!({"source_id":a["source_id"],"expected_revision":1,"selection":{},"idempotency_key":"expand"})).await
        } else {
            m.invoke("writer","treazure_source_add",json!({"candidate":candidate("second","process","process"),"idempotency_key":"second"})).await
        };
        assert!(result.is_err(), "{quota}");
        assert_eq!(m.catalog.read().generation, before.generation);
        for (id, view) in &before.views {
            assert_eq!(m.catalog.read().views[id].len(), view.len());
        }
        assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
    }
}

#[tokio::test]
async fn filter_intersections_preserve_management_and_static_sources_are_immutable() {
    use rmcp::ServerHandler;
    for (selection, tags, exclude_tags, include_tools, exclude_tools, expected) in [
        (json!({}), vec![], vec![], vec![], vec![], 3),
        (
            json!({"tags":["read"]}),
            vec!["write"],
            vec![],
            vec![],
            vec![],
            0,
        ),
        (json!({}), vec!["read"], vec!["read"], vec![], vec![], 0),
        (
            json!({"include":["/plain"]}),
            vec!["read"],
            vec![],
            vec![],
            vec![],
            0,
        ),
        (
            json!({"include_tools":["*read"]}),
            vec![],
            vec![],
            vec!["dyn_*"],
            vec!["*read"],
            0,
        ),
        (
            json!({"exclude":["/write"]}),
            vec![],
            vec![],
            vec![],
            vec![],
            2,
        ),
    ] {
        let mut ls = listeners(false);
        let cfg = ls.get_mut("writer").unwrap();
        cfg.sources = vec!["immutable".into()];
        cfg.tags = tags.into_iter().map(str::to_owned).collect();
        cfg.exclude_tags = exclude_tags.into_iter().map(str::to_owned).collect();
        cfg.include_tools = include_tools.into_iter().map(str::to_owned).collect();
        cfg.exclude_tools = exclude_tools.into_iter().map(str::to_owned).collect();
        let m = manager_with(policy(None), ls).await;
        let mut doc = spec();
        doc["paths"]["/plain"] = json!({"get":{}});
        let cell = OnceCell::new();
        cell.set(Ok(Arc::new(serde_json::to_vec(&doc).unwrap())))
            .unwrap();
        m.fetches.lock().unwrap().insert(
            "https://api.example.com/openapi.json".into(),
            (Instant::now(), Arc::new(cell)),
        );
        let mut c = candidate("filtered", "process", "server");
        c["selection"] = selection;
        m.invoke(
            "writer",
            "treazure_source_add",
            json!({"candidate":c,"idempotency_key":"filtered"}),
        )
        .await
        .unwrap();
        assert_eq!(m.catalog.read().views["writer"].len(), expected);
        assert!(
            server(&m, "writer")
                .get_tool("treazure_source_add")
                .is_some()
        );
        assert!(
            server(&m, "reader")
                .get_tool("treazure_source_add")
                .is_none()
        );
        let generation = m.catalog.read().generation;
        assert!(m.invoke("writer","treazure_source_add",json!({"candidate":candidate("immutable","process","server"),"idempotency_key":"reserved"})).await.unwrap_err().to_string().contains("reserved"));
        for operation in ["treazure_source_remove", "treazure_source_update"] {
            assert!(m.invoke("writer",operation,json!({"source_id":"immutable","expected_revision":0,"idempotency_key":operation})).await.unwrap_err().to_string().contains("not_manageable"));
        }
        assert_eq!(m.catalog.read().generation, generation);
    }
}

#[tokio::test]
async fn cursors_bind_owner_process_and_valid_offset() {
    let m = manager_with(policy(None), writers(false)).await;
    add(&m, "pages", "process", "process").await;
    let first = m
        .invoke("writer", "treazure_tools_search", json!({"limit":1}))
        .await
        .unwrap();
    let cursor = first["next_cursor"].as_str().unwrap();
    assert!(
        m.invoke("second", "treazure_tools_search", json!({"cursor":cursor}))
            .await
            .is_err()
    );
    let prefix = cursor.rsplit_once(':').unwrap().0;
    for suffix in [
        "-1",
        "9999999999999999999999999999999999",
        "999",
        "not-a-number",
    ] {
        assert!(
            m.invoke(
                "writer",
                "treazure_tools_search",
                json!({"cursor":format!("{prefix}:{suffix}")})
            )
            .await
            .is_err()
        );
    }
    let other = manager_with(policy(None), writers(false)).await;
    add(&other, "pages", "process", "process").await;
    assert_eq!(m.catalog.read().generation, other.catalog.read().generation);
    assert!(
        other
            .invoke("writer", "treazure_tools_search", json!({"cursor":cursor}))
            .await
            .unwrap_err()
            .to_string()
            .contains("stale_cursor")
    );
}
