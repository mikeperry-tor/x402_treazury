use super::*;

#[tokio::test]
async fn registration_guidance_tracks_visible_tools_without_changing_registration() {
    let mut configured = listeners(false);
    configured.get_mut("writer").unwrap().exclude_tools = vec!["dyn_hidden__*".into()];
    let m = manager_with(configured).await;
    let visible = register_spec(&m, "writer", "visible", "visible.json", spec())
        .await
        .unwrap();
    assert_eq!(visible["tool_count"], 2);
    assert_eq!(visible["status"], "available");
    assert!(visible["next"].as_str().unwrap().contains("then invoke"));
    let hidden = register_spec(&m, "writer", "hidden", "hidden.json", spec())
        .await
        .unwrap();
    assert_eq!(hidden["tool_count"], 0);
    assert_eq!(hidden["status"], "available");
    let next = hidden["next"].as_str().unwrap();
    assert!(next.contains("no visible tools"));
    assert!(next.contains("operator"));
    assert!(next.contains("without source_name"));
    assert!(!next.contains("then invoke"));
    let before = m.inner.lock().unwrap().state.records.len();
    let repeated = m
        .invoke(
            "writer",
            "x402_treazury_providers_add_remote",
            json!({"spec_url":"https://api.example.com/hidden.json"}),
        )
        .await
        .unwrap();
    assert_eq!(repeated, hidden);
    assert_eq!(m.inner.lock().unwrap().state.records.len(), before);
    let scope = m
        .invoke(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"hidden"}),
        )
        .await
        .unwrap();
    assert_eq!(scope["available"], 0);
    let all = m
        .invoke("writer", "x402_treazury_tools_search", json!({}))
        .await
        .unwrap();
    assert_eq!(all["available"], 2);
    eprintln!(
        "Registration results: {}",
        json!({"visible":visible,"filtered":hidden})
    );
}

#[tokio::test]
async fn disabled_registration_guidance_requires_operator_repair() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry.sqlite");
    let m = manager(Some(registry.clone())).await;
    let original = add(&m, "broken", "", "").await;
    let id = registered_id(&m, &original);
    {
        let mut inner = m.inner.lock().unwrap();
        let mut state = inner.state.clone();
        state
            .records
            .get_mut(&id)
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
    let m = manager(Some(registry)).await;
    let args = json!({"spec_url":"https://api.example.com/openapi.json"});
    let disabled = m
        .invoke("writer", "x402_treazury_providers_add_remote", args.clone())
        .await
        .unwrap();
    assert_eq!(disabled["name"], original["name"]);
    assert_eq!(disabled["tool_prefix"], original["tool_prefix"]);
    assert_eq!(disabled["status"], "disabled");
    assert_eq!(disabled["tool_count"], 0);
    assert!(disabled["reason"].is_string());
    let next = disabled["next"].as_str().unwrap();
    assert!(next.contains("operator"));
    assert!(next.contains("refresh"));
    assert!(next.contains("same disabled registration"));
    assert!(!next.contains("then invoke"));
    assert_eq!(
        m.invoke("writer", "x402_treazury_providers_add_remote", args)
            .await
            .unwrap(),
        disabled
    );
    assert!(m.catalog.read().views.values().all(Vec::is_empty));
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
    eprintln!("Disabled registration result: {disabled}");
}
