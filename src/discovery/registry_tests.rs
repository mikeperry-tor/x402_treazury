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
            "x402_treazury_providers_add_remote",
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
    configured
        .get_mut("hidden")
        .unwrap()
        .provider_management_local = true;
    configured
        .get_mut("hidden")
        .unwrap()
        .provider_management_remote = true;
    configured.get_mut("hidden").unwrap().raw_url_calls = true;
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
    for capability in [
        "provider_management_local",
        "provider_management_remote",
        "raw_url_calls",
    ] {
        config["servers"]["writer"][capability] = toml::Value::Boolean(false);
    }
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

fn saved_record(number: u128, name: &str) -> Record {
    let document = spec().to_string();
    Record {
        id: Uuid::from_u128(number).to_string(),
        owner: "writer".into(),
        candidate: serde_json::from_value(candidate(name, "persistent", "server")).unwrap(),
        targets: vec!["writer".into()],
        revision: 1,
        hash: hash(document.as_bytes()),
        document,
        format: FORMAT,
        accepted_at: 1,
        updated_at: 1,
        removed: false,
        disabled: None,
    }
}

fn write_records(file: &std::path::Path, records: Vec<Record>) {
    let state = State {
        generation: 7,
        records: records
            .into_iter()
            .map(|record| (record.id.clone(), record))
            .collect(),
        ..Default::default()
    };
    store::Store::open(file, &[]).unwrap().save(&state).unwrap();
}

fn read_records(file: &std::path::Path) -> State {
    store::Store::open(file, &[]).unwrap().load().unwrap()
}

#[tokio::test]
async fn removal_by_saved_name_handles_disabled_registration_and_preserves_identity() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("writer.sqlite");
    let config = operator_config(dir.path(), &file);
    let mut record = saved_record(1, "Saved_Name");
    record.disabled = Some("catalog_format_changed".into());
    let id = record.id.clone();
    write_records(&file, vec![record]);
    let before = std::fs::read(&file).unwrap();
    let error = maintain_cli(&config, "writer", "saved_name", false)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("source_not_manageable"),
        "{error:#}"
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
    maintain_cli(&config, "writer", "Saved_Name", false)
        .await
        .unwrap();
    let state = read_records(&file);
    assert_eq!(state.records.len(), 1);
    let removed = &state.records[&id];
    assert_eq!(removed.candidate.name, "Saved_Name");
    assert_eq!(removed.revision, 2);
    assert!(removed.removed);
    assert!(removed.document.is_empty());
}

#[tokio::test]
async fn refresh_by_saved_name_keeps_id_and_replaces_accepted_bundled_contract() {
    for (local, remote, raw) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("writer.sqlite");
        let config = operator_config(dir.path(), &file);
        let mut preferences: toml::Value =
            toml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        preferences["servers"]["writer"]["provider_management_local"] = toml::Value::Boolean(local);
        preferences["servers"]["writer"]["provider_management_remote"] =
            toml::Value::Boolean(remote);
        preferences["servers"]["writer"]["raw_url_calls"] = toml::Value::Boolean(raw);
        std::fs::write(&config, toml::to_string(&preferences).unwrap()).unwrap();
        let provider = crate::providers::all()
            .iter()
            .find(|provider| provider.local_schema.is_some())
            .unwrap();
        let mut record = saved_record(1, "saved_local");
        let mut accepted = AcceptedProvider::current(provider.id).unwrap();
        accepted.settings.instructions_text = Some("Previously accepted instructions".into());
        record.candidate.provider = Some(accepted);
        record.candidate.spec_url.clear();
        record.disabled = Some("explicit_refresh_required".into());
        let id = record.id.clone();
        write_records(&file, vec![record]);
        if !local && !remote {
            let before = serde_json::to_value(read_records(&file)).unwrap();
            let before_file = std::fs::read(&file).unwrap();
            let error = maintain_cli(&config, "writer", "saved_local", true)
                .await
                .unwrap_err();
            assert!(format!("{error:#}").contains("provider_management_disabled"));
            assert_eq!(serde_json::to_value(read_records(&file)).unwrap(), before);
            assert_eq!(std::fs::read(&file).unwrap(), before_file);
            continue;
        }
        maintain_cli(&config, "writer", "saved_local", true)
            .await
            .unwrap();
        let state = read_records(&file);
        assert_eq!(state.records.len(), 1);
        let refreshed = &state.records[&id];
        assert_eq!(refreshed.candidate.name, "saved_local");
        assert_eq!(refreshed.revision, 2);
        assert!(refreshed.disabled.is_none());
        assert!(!refreshed.removed);
        assert_eq!(refreshed.document, provider.local_schema.unwrap());
        assert_eq!(refreshed.hash, hash(refreshed.document.as_bytes()));
        assert_eq!(
            refreshed
                .candidate
                .provider
                .as_ref()
                .unwrap()
                .settings
                .instructions_text,
            AcceptedProvider::current(provider.id)
                .unwrap()
                .settings
                .instructions_text
        );
    }
}

#[tokio::test]
async fn remove_all_tombstones_active_and_disabled_sources_and_preserves_sibling_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("writer.sqlite");
    let other_file = dir.path().join("reader.sqlite");
    let config = operator_config(dir.path(), &file);
    let active = saved_record(1, "active");
    let mut disabled = saved_record(2, "disabled");
    disabled.disabled = Some("invalid_catalog".into());
    let mut tombstone = saved_record(3, "removed");
    tombstone.removed = true;
    tombstone.document.clear();
    tombstone.revision = 5;
    write_records(&file, vec![active, disabled, tombstone.clone()]);
    let mut sibling = saved_record(4, "reader_source");
    sibling.owner = "reader".into();
    sibling.targets = vec!["reader".into()];
    write_records(&other_file, vec![sibling]);
    let mut value: toml::Value =
        toml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let mut listener = value["servers"]["writer"].clone();
    listener["source_registry_file"] = toml::Value::try_from(&other_file).unwrap();
    listener["listen"] = toml::Value::String("127.0.0.1:0".into());
    value["servers"]
        .as_table_mut()
        .unwrap()
        .insert("reader".into(), listener);
    std::fs::write(&config, toml::to_string(&value).unwrap()).unwrap();
    let sibling_before = std::fs::read(&other_file).unwrap();
    remove_all_cli(&config, "writer").await.unwrap();
    assert_eq!(std::fs::read(&other_file).unwrap(), sibling_before);
    let state = read_records(&file);
    assert_eq!(state.generation, 8);
    assert_eq!(state.records.len(), 3);
    assert!(state.records.values().all(|record| record.removed));
    for number in [1, 2] {
        let removed = &state.records[&Uuid::from_u128(number).to_string()];
        assert_eq!(removed.revision, 2);
        assert!(removed.document.is_empty());
        assert!(removed.candidate.provider.is_none());
    }
    assert_eq!(
        serde_json::to_value(&state.records[&tombstone.id]).unwrap(),
        serde_json::to_value(&tombstone).unwrap()
    );
    let after = std::fs::read(&file).unwrap();
    remove_all_cli(&config, "writer").await.unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), after);
    assert_eq!(read_records(&file).generation, 8);
}

#[tokio::test]
async fn remove_all_revision_overflow_cannot_persist_partial_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("writer.sqlite");
    let config = operator_config(dir.path(), &file);
    let first = saved_record(1, "first");
    let mut exhausted = saved_record(2, "exhausted");
    exhausted.revision = u64::MAX;
    write_records(&file, vec![first, exhausted]);
    let before = std::fs::read(&file).unwrap();
    let error = remove_all_cli(&config, "writer").await.unwrap_err();
    assert!(
        error.to_string().contains("revision exhausted"),
        "{error:#}"
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert!(
        read_records(&file)
            .records
            .values()
            .all(|record| !record.removed)
    );
}

#[tokio::test]
async fn saved_names_must_be_unambiguous_and_live_ids_take_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("writer.sqlite");
    let config = operator_config(dir.path(), &file);
    let first = saved_record(1, "duplicate");
    let second = saved_record(2, "duplicate");
    let collision = saved_record(3, &first.id);
    let first_id = first.id.clone();
    let collision_id = collision.id.clone();
    write_records(&file, vec![first, second, collision]);
    let before = std::fs::read(&file).unwrap();
    let error = maintain_cli(&config, "writer", "duplicate", false)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ambiguous"), "{error:#}");
    assert_eq!(std::fs::read(&file).unwrap(), before);
    maintain_cli(&config, "writer", &first_id, false)
        .await
        .unwrap();
    let state = read_records(&file);
    assert!(state.records[&first_id].removed);
    assert!(!state.records[&collision_id].removed);
    assert_eq!(state.records[&collision_id].revision, 1);
    let before_repeat = std::fs::read(&file).unwrap();
    let error = maintain_cli(&config, "writer", &first_id, false)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("source_not_manageable"),
        "{error:#}"
    );
    assert_eq!(std::fs::read(&file).unwrap(), before_repeat);
    assert!(!read_records(&file).records[&collision_id].removed);
}
