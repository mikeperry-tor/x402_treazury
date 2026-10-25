//! Endpoint operation authority is checked independently of tool advertising.
use super::*;
use crate::discovery::policy::{EndpointPolicy, RemoteDirectory};

async fn no_payer(
    local: bool,
    remote: bool,
    raw: bool,
    directories: Vec<RemoteDirectory>,
) -> Arc<Manager> {
    let mut configured = listeners(false);
    let owner = configured.get_mut("writer").unwrap();
    owner.provider_management_local = local;
    owner.provider_management_remote = remote;
    owner.raw_url_calls = raw;
    owner.remote_provider_directories = directories;
    Manager::new(
        configured,
        BTreeMap::new(),
        Arc::new(CatalogState::default()),
        vec![],
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn capability_matrix_checks_disabled_operations_before_payer_or_publication() {
    for mask in 0..8 {
        let (local, remote, raw) = (mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
        let m = no_payer(local, remote, raw, vec![RemoteDirectory::X402List]).await;
        let advertised = tools::listener_definitions(m.policy("writer"), false, None, None);
        let names: BTreeSet<_> = advertised.iter().map(|tool| tool.name.as_ref()).collect();
        for (tool, expected) in [
            (local::NAME, local),
            ("x402_treazury_providers_add_local", local),
            (directory::SEARCH, remote),
            ("x402_treazury_providers_add_remote", remote),
            (source_details::NAME, remote),
            (raw::NAME, raw),
            ("x402_treazury_tools_search", local || remote),
            ("x402_treazury_tool_call", local || remote),
        ] {
            assert_eq!(names.contains(tool), expected, "mask {mask}, {tool}");
        }
        let listing = m.invoke("writer", local::NAME, json!({})).await;
        assert_eq!(listing.is_ok(), local);
        for (enabled, tool, arguments, error) in [
            (
                local,
                "x402_treazury_providers_add_local",
                json!({"provider_id":"exa"}),
                "local_provider_management_disabled",
            ),
            (
                remote,
                "x402_treazury_providers_add_remote",
                json!({"spec_url":"https://api.example.com/openapi.json"}),
                "remote_provider_management_disabled",
            ),
        ] {
            if !enabled {
                assert_eq!(
                    m.invoke("writer", tool, arguments)
                        .await
                        .unwrap_err()
                        .to_string(),
                    error
                );
            }
        }
        if !remote {
            assert_eq!(
                m.invoke_search_remote("writer", json!({}).as_object().unwrap())
                    .await
                    .unwrap_err()
                    .to_string(),
                "remote_provider_search_disabled"
            );
            assert_eq!(
                m.invoke_source_details(
                    "writer",
                    json!({"base_url":"https://api.exa.ai"})
                        .as_object()
                        .unwrap()
                )
                .await
                .unwrap_err()
                .to_string(),
                "remote_provider_management_disabled"
            );
        }
        if !raw {
            assert_eq!(
                m.invoke_raw(
                    "writer",
                    json!({"method":"GET", "url":"https://api.example.com/"})
                        .as_object()
                        .unwrap()
                )
                .await
                .unwrap_err()
                .to_string(),
                "raw_url_calls_disabled"
            );
        }
        assert!(m.payers.is_empty());
        assert!(m.fetches.lock().unwrap().is_empty());
        assert!(m.inner.lock().unwrap().state.records.is_empty());
        assert_eq!(m.catalog.read().generation, 0);
    }
}

#[tokio::test]
async fn directory_subsets_control_search_and_id_schema_without_gating_url_details() {
    let keys = [
        RemoteDirectory::Coinbase,
        RemoteDirectory::CoinbaseCurated,
        RemoteDirectory::X402List,
    ];
    let base = crate::providers::all()
        .iter()
        .find_map(|provider| provider.config.base_url.as_deref())
        .unwrap();
    for mask in 0..8 {
        let selected: Vec<_> = keys
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, key)| *key)
            .collect();
        let has_x402 = selected.contains(&RemoteDirectory::X402List);
        let m = no_payer(false, true, false, selected.clone()).await;
        let advertised = tools::listener_definitions(m.policy("writer"), false, None, None);
        assert_eq!(
            advertised.iter().any(|tool| tool.name == directory::SEARCH),
            !selected.is_empty()
        );
        let details = advertised
            .iter()
            .find(|tool| tool.name == source_details::NAME)
            .unwrap();
        let schema = serde_json::to_value(&*details.input_schema).unwrap();
        assert_eq!(schema["properties"].get("x402_list_id").is_some(), has_x402);
        let error = m
            .invoke_source_details(
                "writer",
                json!({"x402_list_id":"weather-kit"}).as_object().unwrap(),
            )
            .await
            .unwrap_err();
        if has_x402 {
            assert!(
                error.to_string().contains("wallet unavailable"),
                "{error:#}"
            );
        } else {
            assert_eq!(error.to_string(), "x402_list_directory_disabled");
        }
        let details = m
            .invoke_source_details("writer", json!({"base_url":base}).as_object().unwrap())
            .await
            .unwrap();
        assert_eq!(details["url_discovery"]["status"], "bundled_provider_found");
        assert!(!details.to_string().contains("providers_add_local"));
        assert!(details.to_string().contains("providers_add_remote"));
        assert!(m.fetches.lock().unwrap().is_empty());
        assert!(m.inner.lock().unwrap().state.records.is_empty());
    }
}

#[test]
fn all_advertised_parameters_have_help_and_local_definitions_explain_preparation() {
    for mask in 0..8 {
        let policy = EndpointPolicy {
            local: mask & 1 != 0,
            remote: mask & 2 != 0,
            raw: mask & 4 != 0,
            directories: vec![RemoteDirectory::Coinbase, RemoteDirectory::X402List],
        };
        for tool in tools::listener_definitions(policy, false, None, None) {
            let description = tool.description.as_deref().unwrap();
            let schema = serde_json::to_value(&*tool.input_schema).unwrap();
            for (name, parameter) in schema["properties"].as_object().unwrap() {
                assert!(
                    !parameter["description"]
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .is_empty(),
                    "{} parameter {name} lacks help",
                    tool.name
                );
            }
            if tool.name == local::NAME || tool.name == "x402_treazury_providers_add_local" {
                for word in ["locally", "bundled", "reviewed", "filtered", "repaired"] {
                    assert!(description.contains(word), "{} lacks {word}", tool.name);
                }
            }
            for retired in [
                "source_add",
                "sources_search",
                "source_details",
                "local_only",
            ] {
                assert!(
                    !description.contains(retired),
                    "{} recommends retired {retired}",
                    tool.name
                );
            }
        }
    }
    let raw_only = EndpointPolicy {
        raw: true,
        ..Default::default()
    };
    assert_eq!(
        tools::listener_definitions(raw_only.clone(), false, None, None).len(),
        1
    );
    assert_eq!(
        tools::listener_definitions(raw_only, true, None, None).len(),
        3
    );
}

#[tokio::test]
async fn remote_only_exact_bundled_spec_import_retains_provider_settings() {
    let mut configured = listeners(false);
    configured
        .get_mut("writer")
        .unwrap()
        .provider_management_local = false;
    let m = manager_with(configured).await;
    let provider = AcceptedProvider::current("netintel").unwrap();
    let (client, authority) = m.source_client("writer").unwrap().pin_document_authority();
    let cell = OnceCell::new();
    cell.set(Fetched {
        completed: Instant::now(),
        result: Ok(Arc::new(Document::local(
            json!({
                "openapi":"3.0.3", "servers":[{"url":"https://broken.invalid"}],
                "paths":{"/dns/lookup":{"post":{"summary":"DNS lookup"}}}
            })
            .to_string()
            .into_bytes(),
        ))),
    })
    .unwrap();
    m.fetches.lock().unwrap().insert(
        FetchKey {
            authority,
            policy: client.document_policy_key(),
            url: provider.settings.spec.clone(),
            transport: provider.settings.transport(),
            timeout: import::read_timeout(Some(&provider)),
            max_spec_bytes: import::max_spec_bytes(Some(&provider)),
        },
        Arc::new(cell),
    );
    let added = m
        .invoke(
            "writer",
            "x402_treazury_providers_add_remote",
            json!({"spec_url":provider.settings.spec, "name":"matched"}),
        )
        .await
        .unwrap();
    assert_eq!(added["provider_id"], "netintel");
    let inner = m.inner.lock().unwrap();
    let accepted = inner.state.records.values().next().unwrap();
    assert_eq!(accepted.candidate.provider.as_ref().unwrap().id, "netintel");
    assert_eq!(
        m.catalog.read().views["writer"][0].base,
        "https://netintel.dev"
    );
}

#[tokio::test]
async fn local_only_restores_existing_generic_registration_with_original_payer() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("writer.sqlite");
    let m = manager(Some(file.clone())).await;
    let added = add(&m, "generic", "persistent", "server").await;
    let id = registered_id(&m, &added);
    drop(m);
    let mut configured = with_registry(listeners(true), Some(file));
    configured
        .get_mut("writer")
        .unwrap()
        .provider_management_remote = false;
    configured.get_mut("writer").unwrap().raw_url_calls = false;
    let restored = manager_with(configured).await;
    let inner = restored.inner.lock().unwrap();
    let record = &inner.state.records[&id];
    assert!(record.disabled.is_none());
    assert!(record.candidate.provider.is_none());
    assert_eq!(
        record.candidate.spec_url,
        "https://api.example.com/openapi.json"
    );
    assert_eq!(restored.catalog.read().views["writer"].len(), 2);
    assert!(restored.source_client("writer").is_ok());
}
