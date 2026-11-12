//! Accepted curated settings use the same import/publication/restore paths as generic sources.
use super::*;
use crate::catalog::{self, Config};
use crate::discovery::curated::AcceptedProvider;

#[test]
fn accepted_provider_keeps_advertised_lifetime_without_enabling_disk_cache() {
    let accepted = AcceptedProvider::new(
        "fixture",
        Config {
            catalog_cache_ttl_seconds: crate::catalog::CacheLifetime::Seconds(123),
            ..Default::default()
        },
    );
    assert_eq!(
        accepted.settings.catalog_cache_ttl_seconds,
        crate::catalog::CacheLifetime::Seconds(123)
    );
    assert!(!accepted.settings.http_cache_enabled);
    assert!(
        crate::http_cache::Slot::new(
            &accepted.settings,
            "https://api.example.com/prices",
            "catalog",
            1024
        )
        .is_none()
    );
}

fn authored() -> AcceptedProvider {
    AcceptedProvider::new("retired-provider-not-in-bundle", Config {
        spec: "accepted-schema.json".into(),
        prefix: Some("original".into()),
        base_url: Some("https://api.example.com".into()),
        include: vec!["/v1".into()],
        exclude_operations: vec!["GET /v1/skip".into()],
        help_url: Some("https://api.example.com/help".into()),
        instructions_text: Some("Use original_read and original_*; original_missing is unsupported. https://docs.example.com/original_read".into()),
        overrides: json!({"original_read":{"description":"Call original_read, then original_help; preserve original_missing. https://docs.example.com/original_read", "pricing_description":"Cost: $0.01/call."}}),
        response_mappings: serde_json::from_value(json!({"POST /v1/read":{"images":[{"pointer":"/image_url","encoding":"data_uri"}]}})).unwrap(),
        read_timeout_seconds: Some(93.0),
        allow_http1: true,
        allow_tls12: true,
        recommended_tools: vec!["original_help".into()],
        use_recommended_tools: true,
        ..Default::default()
    })
}

fn document() -> Value {
    json!({"openapi":"3.0.3","servers":[{"url":"https://broken.example.com/v1"}],"paths":{
        "/v1/read":{"get":{"summary":"Read query"},"post":{"summary":"Read body","requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"symbol":{"type":"string","description":"original_read","default":"https://docs.example.com/original_read"}}}}}}}},
        "/v1/skip":{"get":{"summary":"Skip"}}
    }})
}

fn curated_candidate(name: &str) -> Candidate {
    serde_json::from_value(json!({"name":name,"provider":authored(),"lifetime":"persistent"}))
        .unwrap()
}

#[test]
fn accepted_remote_advertisements_keep_acquisition_age_after_restore() {
    let mut candidate = curated_candidate("priced");
    let settings = &mut candidate.provider.as_mut().unwrap().settings;
    settings.spec = "https://api.example.com/openapi.json".into();
    settings.pricing = toml::from_str::<Config>(
        r#"
        [pricing]
        source = "catalog"
        format = "payment-info"
        unit = "call"
        usd_as_usdc = true
    "#,
    )
    .unwrap()
    .pricing;
    let mut root = document();
    root["paths"]["/v1/read"]["post"]["x-payment-info"] =
        json!({"price":{"currency":"USD","amount":"0.01"}});
    let document = root.to_string();
    for (lifetime, acquired, expected) in [
        (
            crate::catalog::CacheLifetime::Seconds(120),
            epoch(),
            Some(alloy_primitives::U256::from(10_000)),
        ),
        (crate::catalog::CacheLifetime::Seconds(120), 0, None),
        (
            crate::catalog::CacheLifetime::Indefinite,
            0,
            Some(alloy_primitives::U256::from(10_000)),
        ),
    ] {
        candidate
            .provider
            .as_mut()
            .unwrap()
            .settings
            .catalog_cache_ttl_seconds = lifetime;
        let mut record = Record {
            id: Uuid::new_v4().to_string(),
            owner: "writer".into(),
            candidate: candidate.clone(),
            targets: vec![],
            revision: 1,
            document: document.clone(),
            hash: hash(document.as_bytes()),
            format: FORMAT,
            accepted_at: acquired,
            updated_at: acquired,
            removed: false,
            disabled: None,
            built: None,
        };
        record.prepare().unwrap();
        let amount = |record: &Record| {
            record
                .built
                .as_ref()
                .unwrap()
                .tools
                .iter()
                .find(|t| t.path == "/v1/read")
                .unwrap()
                .advertised_pricing
                .amount()
        };
        assert_eq!(amount(&record), expected);
        let mut restored: Record =
            serde_json::from_value(serde_json::to_value(&record).unwrap()).unwrap();
        restored.prepare().unwrap();
        assert_eq!(
            amount(&restored),
            expected,
            "restart cannot renew acquired pricing"
        );
    }
}

#[tokio::test]
async fn external_advertisements_keep_parent_expiry_on_initial_load_and_restore() {
    if crate::test_log_capture::run_in_child(
        "discovery::tests::curated_tests::external_advertisements_keep_parent_expiry_on_initial_load_and_restore",
    ) {
        return;
    }
    use axum::{Json, Router, routing::get};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let (address, server) = crate::test_tls::serve(Router::new().route(
        "/prices", get({
            let entered = entered.clone();
            let release = release.clone();
            let calls = calls.clone();
            move |headers: axum::http::HeaderMap| {
                let entered = entered.clone();
                let release = release.clone();
                let calls = calls.clone();
                async move {
                    assert!(!headers.contains_key("payment-signature"));
                    assert!(!headers.contains_key("x-payment"));
                    calls.fetch_add(1, Ordering::SeqCst);
                    entered.notify_one();
                    release.notified().await;
                    Json(json!({"x402Version":2,"items":[{
                        "resource":"https://api.example.com/read","method":"GET",
                        "accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000"}]
                    }]}))
                }
            }
        }),
    )).await;
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        crate::network::NetworkContext::new(crate::network::NetworkPolicy {
            mode: crate::network::Mode::Tor,
            socks_endpoint: Some(socks.address.into()),
            tor_isolation_retries: Some(0),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let document = json!({"openapi":"3.0.3","paths":{"/read":{"get":{}}}}).to_string();
    let price = alloy_primitives::U256::from(1000);
    for (acquired, restore, lifetime, expected) in [
        (
            epoch(),
            false,
            catalog::CacheLifetime::Seconds(120),
            Some(price),
        ),
        (0, false, catalog::CacheLifetime::Seconds(120), None),
        (0, true, catalog::CacheLifetime::Seconds(120), None),
        (0, true, catalog::CacheLifetime::Indefinite, Some(price)),
    ] {
        let settings: Config = toml::from_str(
            "spec='https://api.example.com/spec'\nbase_url='https://api.example.com'\nprefix='fixture'\n[pricing]\nsource='https://api.example.com/prices'\nformat='x402-discovery'\n",
        ).unwrap();
        let mut candidate: Candidate = serde_json::from_value(json!({
            "name":"priced", "provider":AcceptedProvider::new("fixture", settings),
            "lifetime":"persistent",
        }))
        .unwrap();
        candidate
            .provider
            .as_mut()
            .unwrap()
            .settings
            .catalog_cache_ttl_seconds = lifetime;
        let mut record = Record {
            id: Uuid::new_v4().to_string(),
            owner: "writer".into(),
            candidate,
            targets: vec![],
            revision: 1,
            document: document.clone(),
            hash: hash(document.as_bytes()),
            format: FORMAT,
            accepted_at: acquired,
            updated_at: acquired,
            removed: false,
            disabled: None,
            built: None,
        };
        if restore {
            record = serde_json::from_value(serde_json::to_value(&record).unwrap()).unwrap();
        }
        record
            .prepare_pricing(PaidClient::unsigned())
            .unwrap()
            .unwrap()
            .send(())
            .unwrap();
        let tool = record.built.as_ref().unwrap().tools[0].clone();
        entered.notified().await;
        assert!(tool.advertised_pricing.amount().is_none());
        assert!(
            tool.route("https://api.example.com", &Default::default())
                .is_ok()
        );
        release.notify_one();
        // source() observes completed enrichment even when its expiry correctly
        // prevents amount()/description() from exposing an expired price.
        tokio::time::timeout(Duration::from_secs(5), async {
            while tool.advertised_pricing.source().as_deref()
                != Some("https://api.example.com/prices")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(tool.advertised_pricing.amount(), expected);
        assert_eq!(
            tool.advertised_pricing.external_description().is_some(),
            expected.is_some()
        );
        assert_eq!(record.updated_at, acquired);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    server.abort();
}

pub(super) async fn commit_curated(m: &Arc<Manager>, name: &str) -> String {
    let document = document().to_string();
    let mut candidate = curated_candidate(name);
    if m.listeners["writer"].source_registry_file.is_none() {
        candidate.lifetime = Lifetime::Process;
    }
    let id = Uuid::new_v4().to_string();
    m.commit(
        "writer".into(),
        false,
        Record {
            built: None,
            id: id.clone(),
            owner: "writer".into(),
            candidate,
            targets: vec![],
            revision: 1,
            hash: hash(document.as_bytes()),
            document,
            format: FORMAT,
            accepted_at: epoch(),
            updated_at: epoch(),
            removed: false,
            disabled: None,
        },
    )
    .await
    .unwrap();
    id
}

#[test]
fn canonical_curated_configuration_applies_before_dynamic_renaming() {
    let candidate = curated_candidate("custom");
    let built = import::build(&candidate, document().to_string().as_bytes()).unwrap();
    assert_eq!(built.base, "https://api.example.com");
    assert_eq!(
        built
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["dyn_custom__help", "dyn_custom__read"])
    );
    let read = built
        .tools
        .iter()
        .find(|tool| tool.name == "dyn_custom__read")
        .unwrap();
    assert_eq!(read.method, "POST");
    assert_eq!(read.path, "/v1/read");
    assert!(read.response_mapping.is_some());
    assert!(read.description.contains("dyn_custom__read"));
    assert!(read.description.contains("dyn_custom__help"));
    assert!(read.description.contains("original_missing"));
    assert!(
        read.description
            .contains("https://docs.example.com/original_read")
    );
    assert!(read.description.ends_with("Cost: $0.01/call."));
    let observed = Some(crate::output::ObservedPrice {
        amount: alloy_primitives::U256::from(5_000),
    });
    let rendered = read.render_description(observed);
    assert!(rendered.contains("Call dyn_custom__read, then dyn_custom__help"));
    assert!(rendered.contains("https://docs.example.com/original_read"));
    assert!(rendered.ends_with("Observed cost: ~$0.005/call."));
    assert!(!rendered.contains("Cost: $0.01/call."));
    let saved = import::saved_tools(&candidate, document().to_string().as_bytes()).unwrap();
    assert_eq!(
        saved
            .tools
            .iter()
            .find(|tool| tool.name == read.name)
            .unwrap()
            .render_description(observed),
        rendered
    );
    assert_eq!(
        read.input_schema["properties"]["body"]["properties"]["symbol"]["description"],
        "original_read"
    );
    assert_eq!(
        read.input_schema["properties"]["body"]["properties"]["symbol"]["default"],
        "https://docs.example.com/original_read"
    );
    let guidance = built.instructions.unwrap();
    assert!(guidance.contains("dyn_custom__read"));
    assert!(guidance.contains("dyn_custom__*"));
    assert!(guidance.contains("original_missing is unsupported"));
    assert!(guidance.contains("https://docs.example.com/original_read"));
    assert_eq!(
        import::saved_tools(&candidate, document().to_string().as_bytes())
            .unwrap()
            .tools
            .iter()
            .map(|tool| &tool.name)
            .collect::<Vec<_>>(),
        built
            .tools
            .iter()
            .map(|tool| &tool.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn curated_operation_urls_survive_restore_and_validate_actual_public_destinations() {
    let mut candidate = curated_candidate("direct");
    candidate
        .provider
        .as_mut()
        .unwrap()
        .settings
        .operation_url_extension = Some("x-direct-url".into());
    let mut document = document();
    document["paths"]["/v1/read"]["post"]["x-direct-url"] = json!("https://read.example.com/read");
    let bytes = serde_json::to_vec(&document).unwrap();
    let restored: Candidate =
        serde_json::from_slice(&serde_json::to_vec(&candidate).unwrap()).unwrap();
    for candidate in [&candidate, &restored] {
        let built = import::build(candidate, &bytes).unwrap();
        let tool = built
            .tools
            .iter()
            .find(|t| t.name == "dyn_direct__read")
            .unwrap();
        assert_eq!(tool.path, "/v1/read");
        assert!(tool.response_mapping.is_some());
        assert_eq!(
            tool.route(&built.base, &Default::default()).unwrap().url,
            "https://read.example.com/read"
        );
    }
    // A generic import cannot acquire cross-origin routing from vendor metadata.
    let generic: Candidate = serde_json::from_value(json!({
        "name":"generic", "spec_url":"https://api.example.com/openapi.json",
        "base_url":"https://api.example.com"
    }))
    .unwrap();
    let built = import::build(&generic, &bytes).unwrap();
    assert!(built.tools.iter().all(|t| t.request_url.is_none()));
    let mut unrelated = document.clone();
    unrelated["paths"]["https://other.example.com/unrelated"] = json!({"get":{}});
    let mut unfiltered = candidate.clone();
    unfiltered
        .provider
        .as_mut()
        .unwrap()
        .settings
        .include
        .clear();
    assert!(
        import::build(&unfiltered, &serde_json::to_vec(&unrelated).unwrap())
            .unwrap_err()
            .to_string()
            .contains("cross_origin_operation_rejected")
    );
    for url in [
        "https://127.0.0.1/private",
        "http://read.example.com/read",
        "https://user:secret@read.example.com/read",
    ] {
        document["paths"]["/v1/read"]["post"]["x-direct-url"] = json!(url);
        assert!(import::build(&candidate, &serde_json::to_vec(&document).unwrap()).is_err());
    }
}

#[test]
fn listener_projection_preserves_selection_and_validates_hidden_operations() {
    let candidate = curated_candidate("custom");
    let mut document = document();
    document["paths"]["/v1/read"]["post"]["tags"] = json!(["read"]);
    document["paths"]["/v1/read"]["get"]["tags"] = json!(["write"]);
    document["paths"]["/v1/other"] = json!({"get":{"tags":["read"]}});
    document["paths"]["/v1/blocked"] = json!({"get":{"tags":["read","blocked"]}});
    let bytes = serde_json::to_vec(&document).unwrap();
    for (filters, expected) in [
        (json!({}), vec!["blocked", "other", "read", "help"]),
        (json!({"tags":["write"]}), vec![]),
        (
            json!({"tags":["read"],"exclude_tags":["blocked"]}),
            vec!["other", "read"],
        ),
        (
            json!({"exclude_tags":["blocked"]}),
            vec!["other", "read", "help"],
        ),
        (
            json!({"include_tools":["dyn_custom__r*","dyn_custom__help"]}),
            vec!["read", "help"],
        ),
        (
            json!({"include_tools":["dyn_custom__*"],"exclude_tools":["*read","*help"]}),
            vec!["blocked", "other"],
        ),
        (json!({"exclude_tools":["*"]}), vec![]),
    ] {
        let mut listener = listeners(false).remove("writer").unwrap();
        let mut value = serde_json::to_value(&listener).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .extend(filters.as_object().unwrap().clone());
        listener = serde_json::from_value(value).unwrap();
        let built = import::build_visible(&candidate, &bytes, &listener).unwrap();
        assert_eq!(
            built
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|suffix| format!("dyn_custom__{suffix}"))
                .collect::<Vec<_>>(),
            "{filters}"
        );
        assert!(
            built
                .tools
                .iter()
                .filter(|tool| tool.name.ends_with("__read"))
                .all(|tool| tool.method == "POST")
        );
        assert!(
            built.instructions.is_some(),
            "projection leaves instruction composition to publication"
        );
    }
    let mut hidden = listeners(false).remove("writer").unwrap();
    hidden.exclude_tools = vec!["*".into()];
    document["paths"]["/v1/read"]["post"]["servers"] = json!([{"url":"https://other.example.com"}]);
    assert!(
        import::build_visible(&candidate, &serde_json::to_vec(&document).unwrap(), &hidden)
            .is_err()
    );
    assert_eq!(
        import::saved_tools(&candidate, &bytes).unwrap().tools.len(),
        4,
        "saved inspection stays unfiltered"
    );
}

#[test]
fn provider_acquisition_settings_apply_and_generic_imports_use_shared_defaults() {
    let mut provider = authored();
    provider.settings.max_spec_bytes = 8192;
    assert_eq!(
        import::read_timeout(Some(&provider)),
        Some(Duration::from_secs(93))
    );
    assert_eq!(import::max_spec_bytes(Some(&provider)), 8192);
    let options = import::options("https://api.example.com/spec", Some(&provider)).unwrap();
    assert_eq!(options.limit, 8192);
    provider.settings.read_timeout_seconds = None;
    assert_eq!(import::read_timeout(Some(&provider)), None);
    assert_eq!(import::read_timeout(None), None);
    assert_eq!(import::max_spec_bytes(None), crate::limits::SPEC_BYTES);

    let mut candidate = curated_candidate("bounds");
    let bytes = document().to_string().into_bytes();
    candidate.provider.as_mut().unwrap().settings.max_spec_bytes = bytes.len();
    import::build(&candidate, &bytes).unwrap();
    candidate.provider.as_mut().unwrap().settings.max_spec_bytes = bytes.len() - 1;
    assert!(
        import::build(&candidate, &bytes)
            .unwrap_err()
            .to_string()
            .contains("spec_too_large")
    );
}

#[test]
fn authored_cross_origin_help_uses_its_declared_documentation_origin() {
    let mut candidate = curated_candidate("origins");
    let provider = candidate.provider.as_mut().unwrap();
    provider.settings.help_url = Some("https://docs.example.com/help".into());
    let bytes = document().to_string().into_bytes();
    let built = import::build(&candidate, &bytes).unwrap();
    assert!(
        built
            .tools
            .iter()
            .any(|tool| tool.help_url.as_deref() == Some("https://docs.example.com/help"))
    );
    assert_eq!(
        import::help_origins(candidate.provider.as_ref()).unwrap(),
        vec!["https://docs.example.com"]
    );
    candidate.provider.as_mut().unwrap().settings.help_url = Some("https://127.0.0.1/help".into());
    assert!(import::build(&candidate, &bytes).is_err());
}

#[test]
fn every_local_bundled_schema_passes_dynamic_destination_and_import_validation() {
    for provider in crate::providers::all()
        .iter()
        .filter(|provider| provider.local_schema.is_some())
    {
        let accepted = AcceptedProvider::current(provider.id).unwrap();
        let candidate: Candidate =
            serde_json::from_value(json!({"name":"local","provider":accepted})).unwrap();
        let bytes = crate::discovery::curated::local_document(candidate.provider.as_ref().unwrap())
            .unwrap();
        import::build(&candidate, &bytes)
            .unwrap_or_else(|error| panic!("{}: {error:#}", provider.id));
    }
}

#[tokio::test]
async fn accepted_curated_catalog_restores_offline_without_bundle_reresolution() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("registry.sqlite");
    let m = manager(Some(file.clone())).await;
    let id = commit_curated(&m, "custom").await;
    let published = m.catalog.read();
    let names: Vec<_> = published.views["writer"]
        .iter()
        .map(|tool| tool.tool.name.clone())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(m.fetches.lock().unwrap().is_empty());
    drop(published);
    drop(m);
    let inspected = store::inspect(&file).unwrap();
    assert_eq!(inspected["sources"][0]["id"], id);
    assert_eq!(
        inspected["sources"][0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        names.iter().map(String::as_str).collect::<Vec<_>>()
    );
    let restored = manager(Some(file)).await;
    assert_eq!(
        restored.catalog.read().views["writer"]
            .iter()
            .map(|tool| tool.tool.name.clone())
            .collect::<Vec<_>>(),
        names
    );
    let saved = restored.inner.lock().unwrap().state.records[&id].clone();
    assert_eq!(
        saved.candidate.provider.unwrap().id,
        "retired-provider-not-in-bundle"
    );
    assert!(saved.disabled.is_none());
    assert!(restored.fetches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dynamic_guidance_recomposes_from_static_base_without_duplicates() {
    let m = manager(None).await;
    commit_curated(&m, "custom").await;
    let text = m.catalog.read().instructions["writer"].clone().unwrap();
    assert!(text.contains("dyn_custom__read"));
    let mut base = m.static_snapshot().await.unwrap();
    base.instructions
        .insert("writer".into(), Some("Static guidance".into()));
    let revision = m.replace_static(base.generation, base).await.unwrap();
    let text = m.catalog.read().instructions["writer"].clone().unwrap();
    assert!(text.starts_with("Static guidance"));
    assert_eq!(text.matches("[custom]").count(), 1);
    let base = m.static_snapshot().await.unwrap();
    m.replace_static(revision, base).await.unwrap();
    assert_eq!(
        m.catalog.read().instructions["writer"]
            .as_ref()
            .unwrap()
            .matches("[custom]")
            .count(),
        1
    );
}

#[tokio::test]
async fn explicit_listener_instruction_suppression_omits_provider_guidance() {
    let mut config = listeners(false);
    config.get_mut("writer").unwrap().instructions_text = Some(String::new());
    let m = manager_with(config).await;
    commit_curated(&m, "custom").await;
    assert!(
        m.catalog
            .read()
            .instructions
            .get("writer")
            .and_then(Option::as_deref)
            .unwrap_or_default()
            .is_empty()
    );
    assert_eq!(m.catalog.read().views["writer"].len(), 2);
}

#[tokio::test]
async fn operator_removal_clears_curated_guidance_and_saved_catalog() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("registry.sqlite");
    let deployment = operator_config(directory.path(), &file);
    let m = manager(Some(file.clone())).await;
    let id = commit_curated(&m, "custom").await;
    assert!(
        m.catalog.read().instructions["writer"]
            .as_ref()
            .unwrap()
            .contains("dyn_custom__read")
    );
    drop(m);
    maintain_cli(&deployment, "writer", &id, false)
        .await
        .unwrap();
    let restored = manager(Some(file.clone())).await;
    assert!(restored.catalog.read().views["writer"].is_empty());
    assert!(
        restored
            .catalog
            .read()
            .instructions
            .get("writer")
            .and_then(Option::as_deref)
            .unwrap_or_default()
            .is_empty()
    );
    let record = restored.inner.lock().unwrap().state.records[&id].clone();
    assert!(record.removed);
    assert!(record.document.is_empty());
    assert!(
        record.candidate.provider.is_none(),
        "removed records clear accepted catalog settings"
    );
    drop(restored);
    assert_eq!(
        store::inspect(&file).unwrap()["sources"][0]["tools"],
        json!([])
    );
}

async fn commit_curated_with(m: &Arc<Manager>, name: &str, provider: AcceptedProvider) -> String {
    let document = document().to_string();
    let candidate = serde_json::from_value(json!({
        "name": name, "provider": provider, "lifetime": "persistent"
    }))
    .unwrap();
    let id = Uuid::new_v4().to_string();
    m.commit(
        "writer".into(),
        false,
        Record {
            built: None,
            id: id.clone(),
            owner: "writer".into(),
            candidate,
            targets: vec![],
            revision: 1,
            hash: hash(document.as_bytes()),
            document,
            format: FORMAT,
            accepted_at: epoch(),
            updated_at: epoch(),
            removed: false,
            disabled: None,
        },
    )
    .await
    .unwrap();
    id
}

fn saved_curated_state(path: &std::path::Path) -> State {
    store::Store::open(path, &[]).unwrap().load().unwrap()
}

#[tokio::test]
async fn curated_cli_refresh_atomically_adopts_current_local_bundle_and_preserves_identity() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("registry.sqlite");
    let deployment = operator_config(directory.path(), &file);
    let m = manager(Some(file.clone())).await;
    let mut stale = authored();
    stale.id = "relayshield-phone-reputation".into();
    let id = commit_curated_with(&m, "custom", stale).await;
    assert_eq!(
        m.catalog.read().views["writer"]
            .iter()
            .map(|bound| bound.tool.name.as_str())
            .collect::<Vec<_>>(),
        ["dyn_custom__read", "dyn_custom__help"]
    );
    assert!(m.fetches.lock().unwrap().is_empty());
    drop(m);
    let before = saved_curated_state(&file).records[&id].clone();

    maintain_cli(&deployment, "writer", &id, true)
        .await
        .unwrap();

    let state = saved_curated_state(&file);
    assert_eq!(state.records.len(), 1);
    let refreshed = &state.records[&id];
    assert_eq!(refreshed.id, before.id);
    assert_eq!(refreshed.owner, before.owner);
    assert_eq!(refreshed.candidate.name, before.candidate.name);
    assert_eq!(refreshed.targets, before.targets);
    assert_eq!(refreshed.revision, before.revision + 1);
    assert_eq!(refreshed.accepted_at, before.accepted_at);
    assert!(!refreshed.removed);
    assert!(refreshed.disabled.is_none());
    let shipped = crate::providers::get("relayshield-phone-reputation").unwrap();
    let accepted = refreshed.candidate.provider.as_ref().unwrap();
    assert_eq!(accepted.id, "relayshield-phone-reputation");
    assert_eq!(accepted.settings.base_url, shipped.config.base_url);
    assert_eq!(accepted.settings.prefix, shipped.config.prefix);
    assert_eq!(accepted.settings.spec, shipped.config.spec);
    assert_eq!(refreshed.document, shipped.local_schema.unwrap());
    assert_ne!(refreshed.document, before.document);
    assert_eq!(refreshed.hash, hash(refreshed.document.as_bytes()));
    assert_ne!(refreshed.hash, before.hash);
    let inventory =
        import::saved_tools(&refreshed.candidate, refreshed.document.as_bytes()).unwrap();
    assert_eq!(
        inventory
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["dyn_custom__phone_reputation"]
    );
    let inspected = store::inspect(&file).unwrap();
    assert_eq!(inspected["sources"][0]["id"], id);
    assert_eq!(inspected["sources"][0]["revision"], refreshed.revision);
    assert_eq!(
        inspected["sources"][0]["tools"],
        json!([{"name":"dyn_custom__phone_reputation","method":"POST","path":"/v1/payg/phone-reputation"}])
    );
    let restored = manager(Some(file)).await;
    assert_eq!(
        restored.catalog.read().views["writer"]
            .iter()
            .map(|bound| bound.tool.name.as_str())
            .collect::<Vec<_>>(),
        inventory
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
    );
    assert!(restored.fetches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn curated_cli_refresh_missing_provider_preserves_entire_accepted_state() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("registry.sqlite");
    let deployment = operator_config(directory.path(), &file);
    let m = manager(Some(file.clone())).await;
    let id = commit_curated(&m, "custom").await;
    assert!(m.fetches.lock().unwrap().is_empty());
    drop(m);
    let before = serde_json::to_value(saved_curated_state(&file)).unwrap();
    let before_inventory = store::inspect(&file).unwrap();

    let error = maintain_cli(&deployment, "writer", &id, true)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("unknown_provider_id"));

    assert_eq!(
        serde_json::to_value(saved_curated_state(&file)).unwrap(),
        before
    );
    assert_eq!(store::inspect(&file).unwrap(), before_inventory);
    let restored = manager(Some(file)).await;
    assert_eq!(restored.catalog.read().views["writer"].len(), 2);
    assert!(
        restored.inner.lock().unwrap().state.records[&id]
            .disabled
            .is_none()
    );
    assert!(restored.fetches.lock().unwrap().is_empty());
}
