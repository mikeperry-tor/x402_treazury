//! Small synthetic registry reuse/invalidation and projection contracts.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn actual_builds_are_reused_across_unrelated_add_static_publication_and_restore() {
    if crate::test_log_capture::run_in_child(
        "discovery::tests::efficiency_tests::actual_builds_are_reused_across_unrelated_add_static_publication_and_restore",
    ) {
        return;
    }
    let builds = Arc::new(AtomicUsize::new(0));
    let observed = builds.clone();
    assert!(
        import::BUILD_OBSERVER
            .set(Box::new(move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
            }))
            .is_ok()
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registry.sqlite");
    let m = manager(Some(file.clone())).await;
    let mut doc = spec();
    doc["paths"]["/unsupported"] = json!({"get":{"parameters":[{"name":"session","in":"cookie"}]}});
    let first = register_spec(&m, "writer", "first", "first.json", doc)
        .await
        .unwrap();
    let id = registered_id(&m, &first);
    let built = m.inner.lock().unwrap().state.records[&id]
        .built
        .clone()
        .unwrap();
    let (tool_name, help) = {
        let snapshot = m.catalog.read();
        let tool = &snapshot.views["writer"][0];
        (tool.tool.name.clone(), tool.help.clone())
    };
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    register_spec(&m, "writer", "second", "second.json", spec())
        .await
        .unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert!(Arc::ptr_eq(
        &built,
        m.inner.lock().unwrap().state.records[&id]
            .built
            .as_ref()
            .unwrap()
    ));
    let base = m.static_snapshot().await.unwrap();
    m.replace_static(base.generation, base).await.unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    let current_help = m.catalog.read().views["writer"]
        .iter()
        .find(|tool| tool.tool.name == tool_name)
        .expect("original tool remains visible")
        .help
        .clone();
    assert!(Arc::ptr_eq(&help, &current_help));
    let result = m
        .existing_registration(
            "writer",
            import::RegistrationKey::Specification("https://api.example.com/first.json"),
        )
        .unwrap();
    assert_eq!(result["omitted_operations"][0]["path"], "/unsupported");
    drop(m);
    let restored = manager(Some(file)).await;
    assert_eq!(
        builds.load(Ordering::SeqCst),
        4,
        "one actual build per restored live record"
    );
    assert_eq!(restored.catalog.read().views["writer"].len(), 4);
    let result = restored
        .existing_registration(
            "writer",
            import::RegistrationKey::Specification("https://api.example.com/first.json"),
        )
        .unwrap();
    assert_eq!(result["omitted_operations"][0]["path"], "/unsupported");
}

#[tokio::test]
async fn generated_name_builds_once_under_the_final_name_and_keeps_failure_atomic() {
    if crate::test_log_capture::run_in_child(
        "discovery::tests::efficiency_tests::generated_name_builds_once_under_the_final_name_and_keeps_failure_atomic",
    ) {
        return;
    }
    let names = Arc::new(Mutex::new(Vec::new()));
    let observed = names.clone();
    assert!(
        import::BUILD_OBSERVER
            .set(Box::new(move |c| observed
                .lock()
                .unwrap()
                .push(c.name.clone())))
            .is_ok()
    );
    let m = manager(None).await;
    register_spec(&m, "writer", "api-example-com", "first.json", spec())
        .await
        .unwrap();
    let url = "https://api.example.com/second.json";
    let cell = OnceCell::new();
    cell.set(Fetched {
        completed: Instant::now(),
        result: Ok(Arc::new(Document::local(
            serde_json::to_vec(&spec()).unwrap(),
        ))),
    })
    .unwrap();
    m.fetches
        .lock()
        .unwrap()
        .insert(fetch_key(&m, url), Arc::new(cell));
    let added = m
        .invoke(
            "writer",
            "x402_treazury_providers_add_remote",
            json!({"spec_url":url}),
        )
        .await
        .unwrap();
    assert_eq!(added["name"], "api-example-com-2");
    assert_eq!(
        *names.lock().unwrap(),
        ["api-example-com", "api-example-com-2"]
    );
    let mut record = {
        let lock = m.inner.lock().unwrap();
        lock.state.records.values().next().unwrap().clone()
    };
    record.id = Uuid::new_v4().to_string();
    record.candidate.name = "api-example-com".into();
    record.candidate.spec_url = "https://api.example.com/third.json".into();
    record.candidate.selection.include_tools = vec!["dyn_api-example-com__read".into()];
    record.built = None;
    let before = m.catalog.read();
    assert!(
        m.commit("writer".into(), true, record).await.is_err(),
        "final name must reapply name-based selections before persistence"
    );
    assert!(Arc::ptr_eq(&before, &m.catalog.read()));
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
}

#[test]
fn compact_tag_projection_matches_operation_inventory_without_schema_rebuilds() {
    let c: Candidate = serde_json::from_value(candidate("tags", "process", "server")).unwrap();
    let mut doc = spec();
    doc["paths"]["/read"]["GET"] = json!({"tags":["ignored-uppercase"]});
    doc["paths"]["/untagged"] = json!({"get":{}});
    doc["paths"]["/replace"] = json!({"put":{"tags":["read"]}});
    doc["paths"]["/amend"] = json!({"patch":{"tags":["write"]}});
    doc["paths"]["/remove"] = json!({"delete":{"tags":["read","write"]}});
    let bytes = serde_json::to_vec(&doc).unwrap();
    let built = import::build(&c, &bytes).unwrap();
    assert_eq!(
        built
            .tools
            .iter()
            .map(|tool| tool.method.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["GET", "POST", "PUT", "PATCH", "DELETE"])
    );
    let operations = crate::catalog::operations(&doc, None).unwrap();
    for (include, exclude) in [
        (vec![], vec![]),
        (vec!["read"], vec![]),
        (vec![], vec!["write"]),
        (vec!["read"], vec!["read"]),
        (vec!["READ"], vec![]),
        (vec!["ignored-uppercase"], vec![]),
    ] {
        let mut listener = listeners(false).remove("writer").unwrap();
        listener.tags = include.into_iter().map(str::to_owned).collect();
        listener.exclude_tags = exclude.into_iter().map(str::to_owned).collect();
        let expected: Vec<_> = built
            .tools
            .iter()
            .filter(|tool| {
                crate::catalog::matches_operation_tags(
                    &operations,
                    tool,
                    &listener.tags,
                    &listener.exclude_tags,
                )
            })
            .map(|tool| tool.name.as_str())
            .collect();
        let actual: Vec<_> = import::visible_tools(&built, &listener)
            .unwrap()
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn changed_serialized_inputs_rebuild_and_never_persist_derived_results() {
    let m = manager(None).await;
    let response = register_spec(&m, "writer", "original", "original.json", spec())
        .await
        .unwrap();
    let id = registered_id(&m, &response);
    let lock = m.inner.lock().unwrap();
    let record = &lock.state.records[&id];
    let original = record.built.clone().unwrap();
    let saved = serde_json::to_value(record).unwrap();
    assert!(saved.get("built").is_none());
    drop(lock);
    for field in ["name", "base_url", "selection", "document"] {
        let mut saved = saved.clone();
        match field {
            "name" => saved["candidate"]["name"] = json!("changed"),
            "base_url" => saved["candidate"]["base_url"] = json!("https://other.example.com"),
            "selection" => saved["candidate"]["selection"]["include"] = json!(["/read"]),
            "document" => {
                let mut doc = spec();
                doc["paths"]["/extra"] = json!({"get":{}});
                saved["document"] = json!(doc.to_string());
            }
            _ => unreachable!(),
        }
        let mut record: Record = serde_json::from_value(saved).unwrap();
        assert!(record.built.is_none());
        record.prepare().unwrap();
        assert!(!Arc::ptr_eq(&original, record.built.as_ref().unwrap()));
        match field {
            "name" => assert!(
                record
                    .built
                    .as_ref()
                    .unwrap()
                    .tools
                    .iter()
                    .all(|tool| tool.name.starts_with("dyn_changed__"))
            ),
            "base_url" => assert_eq!(
                record.built.as_ref().unwrap().base,
                "https://other.example.com"
            ),
            "selection" => assert_eq!(record.built.as_ref().unwrap().tools.len(), 1),
            "document" => assert_eq!(record.built.as_ref().unwrap().tools.len(), 3),
            _ => unreachable!(),
        }
    }
}
