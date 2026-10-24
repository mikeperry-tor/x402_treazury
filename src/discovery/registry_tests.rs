use super::*;

async fn open(configured: BTreeMap<String, ListenerConfig>) -> Result<Arc<Manager>> {
    let snapshot = CatalogSnapshot {
        views: configured
            .keys()
            .map(|owner| (owner.clone(), vec![]))
            .collect(),
        ..Default::default()
    };
    Manager::new(
        configured,
        BTreeMap::from([("shared".into(), payer())]),
        Arc::new(CatalogState::new(snapshot)),
        vec![],
    )
    .await
}

async fn register(manager: &Arc<Manager>, owner: &str) {
    seed_for(manager, owner).await;
    manager
        .invoke(
            owner,
            "x402_treazury_source_add",
            json!({
                "spec_url":"https://api.example.com/openapi.json", "name":"saved"
            }),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn endpoint_files_restore_independently_and_ephemeral_sibling_disappears() {
    let dir = tempfile::tempdir().unwrap();
    let writer = dir.path().join("writer.sqlite");
    let reader = dir.path().join("reader-registry.sqlite");
    let mut configured = with_registry(listeners(true), Some(writer.clone()));
    configured.get_mut("hidden").unwrap().source_management = true;
    let manager = open(configured.clone()).await.unwrap();
    for owner in ["writer", "reader", "hidden"] {
        register(&manager, owner).await;
    }
    assert_eq!(manager.inner.lock().unwrap().state.records.len(), 3);
    drop(manager);
    for (owner, file) in [("writer", &writer), ("reader", &reader)] {
        let state = store::Store::open(file, &[]).unwrap().load().unwrap();
        assert_eq!(state.records.len(), 1);
        assert!(state.records.values().all(|record| record.owner == owner));
    }
    let restored = open(configured).await.unwrap();
    assert_eq!(restored.inner.lock().unwrap().state.records.len(), 2);
    assert_eq!(restored.catalog.read().views["writer"].len(), 2);
    assert_eq!(restored.catalog.read().views["reader"].len(), 2);
    assert!(restored.catalog.read().views["hidden"].is_empty());
}

#[tokio::test]
async fn foreign_endpoint_file_is_rejected_without_changing_saved_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("writer.sqlite");
    let manager = manager(Some(file.clone())).await;
    register(&manager, "writer").await;
    drop(manager);
    let before = std::fs::read(&file).unwrap();
    let mut configured = listeners(true);
    configured.get_mut("reader").unwrap().source_registry_file = Some(file.clone());
    let error = open(configured).await.err().unwrap();
    assert!(error.to_string().contains("another endpoint"));
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert_eq!(
        store::Store::open(&file, &[])
            .unwrap()
            .load()
            .unwrap()
            .records
            .len(),
        1
    );
}

#[tokio::test]
async fn endpoints_cannot_open_the_same_registry_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shared.sqlite");
    let mut configured = listeners(true);
    for owner in ["writer", "reader"] {
        configured.get_mut(owner).unwrap().source_registry_file = Some(file.clone());
    }
    let error = open(configured).await.err().unwrap();
    assert!(error.to_string().contains("source registry already owned"));
    assert!(store::Store::open(&file, &[]).is_ok());
}

#[test]
fn registry_paths_cannot_alias_another_endpoints_sidecars() {
    let dir = tempfile::tempdir().unwrap();
    let path = operator_config(dir.path(), &dir.path().join("one.sqlite"));
    let mut config: crate::deployment::MetaConfig =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = dir.path().join("one.sqlite");
    for suffix in [".owner.lock", "-wal", "-shm", "-journal"] {
        let mut other = config.servers["writer"].clone();
        other.source_registry_file = Some(PathBuf::from(format!("{}{suffix}", base.display())));
        config.servers.insert("reader".into(), other);
        assert!(policy::validate_registry_path(&config, &path).is_err());
        assert!(!base.exists());
    }
}

#[tokio::test]
async fn operator_removal_updates_only_the_selected_endpoints_file() {
    let dir = tempfile::tempdir().unwrap();
    let writer = dir.path().join("writer.sqlite");
    let reader = dir.path().join("reader-registry.sqlite");
    let configured = with_registry(listeners(true), Some(writer.clone()));
    let manager = open(configured.clone()).await.unwrap();
    register(&manager, "writer").await;
    register(&manager, "reader").await;
    let id = manager
        .inner
        .lock()
        .unwrap()
        .state
        .records
        .values()
        .find(|record| record.owner == "writer")
        .unwrap()
        .id
        .clone();
    drop(manager);
    let before = std::fs::read(&reader).unwrap();
    let path = operator_config(dir.path(), &writer);
    let mut config: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["servers"].as_table_mut().unwrap().insert(
        "reader".into(),
        toml::Value::try_from(&configured["reader"]).unwrap(),
    );
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    maintain_cli(&path, "writer", &id, false).await.unwrap();
    assert_eq!(std::fs::read(&reader).unwrap(), before);
    assert!(
        store::Store::open(&writer, &[])
            .unwrap()
            .load()
            .unwrap()
            .records[&id]
            .removed
    );
    let restored = open(configured).await.unwrap();
    assert!(restored.catalog.read().views["writer"].is_empty());
    assert_eq!(restored.catalog.read().views["reader"].len(), 2);
}
