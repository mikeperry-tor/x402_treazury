//! Runtime refresh publication uses staged cache bytes and endpoint-local failures.
use super::*;
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn unsupported_refresh_preserves_siblings_and_restores_availability_on_rollback() {
    let mut fixture = Fixture::configured(false, true).await;
    assert_eq!(fixture.runtime.catalog.read().views["affected"].len(), 2);
    let unsupported = json!({"paths":{"/read":{"get":{"parameters":[
        {"in":"cookie","name":"session","schema":{"type":"string"}}
    ]}}}});
    *fixture.document.lock().unwrap() = unsupported;
    let mut cfg = fixture.runtime.deployment.sources["api"].config.clone();
    cfg.catalog_cache_force_refresh = true;
    let acquire = || {
        refresh_source(
            "api",
            cfg.clone(),
            None,
            fixture.runtime.deployment.catalog_capacity.clone(),
        )
    };
    let rejected = acquire().await.unwrap();
    let accepted = acquire().await.unwrap();
    assert!(accepted.0.compatibility_unavailable());
    let old = fixture.runtime.catalog.read();
    let binding = fixture.runtime.refreshes["api"].clone();

    // Force failure at publication after selection has classified the replacement.
    let mut exhausted = (*old).clone();
    exhausted.generation = u64::MAX;
    fixture.runtime.catalog.publish(exhausted);
    assert!(binding.trigger());
    fixture
        .runtime
        .complete_refresh("api".into(), binding.clone(), Ok(rejected))
        .await;
    assert!(binding.check().is_ok());
    assert!(!binding.refreshing());
    assert!(
        !fixture
            .runtime
            .deployment
            .unavailable_sources
            .contains_key("api")
    );
    assert!(!fixture.runtime.deployment.sources["api"].compatibility_unavailable());
    assert_eq!(fixture.runtime.deployment.selected["affected"].len(), 2);
    fixture.runtime.catalog.publish((*old).clone());

    assert!(binding.trigger());
    fixture
        .runtime
        .complete_refresh("api".into(), binding.clone(), Ok(accepted))
        .await;
    assert!(binding.check().is_err());
    assert!(
        fixture
            .runtime
            .deployment
            .unavailable_sources
            .contains_key("api")
    );
    assert!(fixture.runtime.deployment.selection_errors.is_empty());
    let current = fixture.runtime.catalog.read();
    assert_eq!(current.views["affected"].len(), 1);
    assert_eq!(current.views["affected"][0].tool.name, "other_other");
    assert_eq!(
        current.views["affected"][0]
            .invoke_output(&Default::default())
            .await
            .unwrap()
            .text,
        "healthy"
    );
    fixture.assert_unrelated_usable().await;

    *fixture.document.lock().unwrap() = json!({"paths":{"/read":{"get":{}}}});
    let recovered = refresh_source(
        "api",
        cfg,
        None,
        fixture.runtime.deployment.catalog_capacity.clone(),
    )
    .await
    .unwrap();
    let binding = fixture.runtime.refreshes["api"].clone();
    assert!(binding.trigger());
    fixture
        .runtime
        .complete_refresh("api".into(), binding, Ok(recovered))
        .await;
    assert!(
        !fixture
            .runtime
            .deployment
            .unavailable_sources
            .contains_key("api")
    );
    assert!(fixture.runtime.deployment.selection_errors.is_empty());
    assert_eq!(fixture.runtime.catalog.read().views["affected"].len(), 2);
    fixture.origin.abort();
    let _ = fixture.origin.await;
}

#[tokio::test]
async fn refresh_newly_selected_uninitialized_wallet_retains_tools_and_publisher_progress() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deployment.toml");
    let api = directory.path().join("api.json");
    let other = directory.path().join("other.json");
    let original = json!({"paths":{"/read":{"get":{
        "x-payment-info":{"price":{"currency":"USD","amount":"0.014"}}
    }}}});
    std::fs::write(&api, original.to_string()).unwrap();
    std::fs::write(&other, json!({"paths":{"/other":{"get":{}}}}).to_string()).unwrap();
    std::fs::write(
        &path,
        r#"
version=1
[wallets.active]
mode='static'
private_key_env='ACTIVE'
[wallets.dormant]
mode='static'
private_key_env='ABSENT'
[sources.api]
spec='api.json'
base_url='https://example.com'
[sources.api.pricing]
source='catalog'
format='payment-info'
unit='call'
usd_as_usdc=true
[sources.other]
spec='other.json'
base_url='https://example.com'
wallet='active'
[servers.first]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='active'
sources=['api']
include_tools=['api_*']
[servers.second]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='dormant'
sources=['api','other']
include_tools=['api_new*','other_*']
"#,
    )
    .unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    // The omitted key must remain unnecessary, including after refresh failure.
    let wallets = deployment
        .static_wallets(
            &BTreeMap::from([("ACTIVE".into(), format!("{:064x}", 1))]),
            None,
            &BTreeSet::new(),
        )
        .unwrap();
    assert_eq!(
        wallets.keys().map(String::as_str).collect::<Vec<_>>(),
        ["active"]
    );
    let catalog = Arc::new(crate::catalog_state::CatalogState::new(
        crate::catalog_state::CatalogSnapshot {
            views: BTreeMap::from([("first".into(), Vec::new()), ("second".into(), Vec::new())]),
            ..Default::default()
        },
    ));
    let mut runtime = Runtime::new(deployment, wallets, catalog, None, false);
    runtime.publish().await.unwrap();
    let before = runtime.catalog.read();
    let old = &before.views["first"][0];
    let binding = old.refresh.as_ref().unwrap().clone();
    let price = old.tool.advertised_pricing.amount();
    assert!(price.is_some());
    let mut replacement = original.clone();
    replacement["paths"]["/new"] = json!({"get":{}});
    std::fs::write(&api, replacement.to_string()).unwrap();
    let acquired = refresh_source(
        "api",
        runtime.deployment.sources["api"].config.clone(),
        None,
        runtime.deployment.catalog_capacity.clone(),
    )
    .await
    .unwrap();
    assert!(binding.trigger());
    runtime
        .complete_refresh("api".into(), binding.clone(), Ok(acquired))
        .await;
    assert!(Arc::ptr_eq(&before, &runtime.catalog.read()));
    assert!(binding.check().is_ok());
    assert!(!binding.refreshing());
    assert_eq!(old.tool.advertised_pricing.amount(), price);
    assert!(Arc::ptr_eq(&runtime.refreshes["api"], &binding));
    assert_eq!(
        runtime.deployment.sources["api"].document.as_ref(),
        &original
    );
    assert_eq!(runtime.deployment.selected["second"].len(), 1);
    assert!(!runtime.wallets.contains_key("dormant"));

    // A later signal can succeed, and unrelated publication remains available.
    std::fs::write(&api, original.to_string()).unwrap();
    for (id, binding) in [
        ("other", runtime.refreshes["other"].clone()),
        ("api", binding),
    ] {
        let acquired = refresh_source(
            id,
            runtime.deployment.sources[id].config.clone(),
            None,
            runtime.deployment.catalog_capacity.clone(),
        )
        .await
        .unwrap();
        assert!(binding.trigger());
        runtime
            .complete_refresh(id.into(), binding.clone(), Ok(acquired))
            .await;
        assert!(binding.check().is_err());
    }
    let after = runtime.catalog.read();
    assert!(after.generation > before.generation);
    assert_eq!(after.views["first"][0].tool.path, "/read");
    assert_eq!(after.views["second"][0].tool.path, "/other");
    assert!(!runtime.wallets.contains_key("dormant"));
}

#[tokio::test]
async fn replacement_retires_old_admission_before_static_visibility_with_and_without_registry() {
    for registry in [false, true] {
        let mut fixture = Fixture::new().await;
        fixture
            .runtime
            .deployment
            .listeners
            .get_mut("affected")
            .unwrap()
            .include_tools
            .clear();
        fixture.runtime.deployment.refresh_selection(true).unwrap();
        fixture.runtime.publish().await.unwrap();
        if registry {
            let manager = crate::discovery::Manager::new(
                fixture.runtime.deployment.listeners.clone(),
                fixture.runtime.wallets.clone(),
                fixture.runtime.catalog.clone(),
                vec![],
            )
            .await
            .unwrap();
            fixture.runtime.manager = Some(manager);
        }
        let old = fixture.runtime.catalog.read().views["affected"][0].clone();
        let binding = old.refresh.as_ref().unwrap().clone();
        let observed = binding.clone();
        let publications = Arc::new(AtomicUsize::new(0));
        let seen = publications.clone();
        *fixture.runtime.catalog.before_publish.lock().unwrap() = Some(Arc::new(move || {
            assert!(
                observed.check().is_err(),
                "old admission must end before visibility"
            );
            seen.fetch_add(1, Ordering::SeqCst);
        }));
        let replacement = fixture.acquire().await;
        fixture
            .runtime
            .complete_refresh("api".into(), binding.clone(), Ok(replacement))
            .await;
        fixture
            .runtime
            .catalog
            .before_publish
            .lock()
            .unwrap()
            .take();
        assert!(publications.load(Ordering::SeqCst) > 0);
        assert!(binding.check().is_err());
        assert_eq!(
            fixture.runtime.catalog.read().views["affected"][0]
                .tool
                .path,
            "/new"
        );
        fixture.assert_unrelated_usable().await;
        fixture.origin.abort();
        let _ = fixture.origin.await;
    }
}

#[derive(Clone)]
struct Acquisition {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

struct Fixture {
    _directory: tempfile::TempDir,
    runtime: Runtime,
    document: Arc<Mutex<Value>>,
    downloads: Arc<AtomicUsize>,
    read_removed: Arc<std::sync::atomic::AtomicBool>,
    acquisition: Arc<Mutex<Option<Acquisition>>>,
    origin: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        Self::with_alias(false).await
    }
    async fn with_alias(alias: bool) -> Self {
        Self::configured(alias, false).await
    }
    async fn configured(alias: bool, sibling: bool) -> Self {
        let document = Arc::new(Mutex::new(
            json!({"paths":{"/read":{"get":{"operationId":"read"}}}}),
        ));
        let downloads = Arc::new(AtomicUsize::new(0));
        let read_removed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acquisition = Arc::new(Mutex::new(None::<Acquisition>));
        let (address, origin) = crate::test_tls::serve(
            Router::new()
                .route(
                    "/spec",
                    get({
                        let document = document.clone();
                        let downloads = downloads.clone();
                        let acquisition = acquisition.clone();
                        move |headers: axum::http::HeaderMap| {
                            let document = document.clone();
                            let downloads = downloads.clone();
                            let acquisition = acquisition.clone();
                            async move {
                                assert!(!headers.contains_key("payment-signature"));
                                assert!(!headers.contains_key("x-payment"));
                                assert!(!headers.contains_key("if-none-match"));
                                assert!(!headers.contains_key("if-modified-since"));
                                downloads.fetch_add(1, Ordering::SeqCst);
                                let document = document.lock().unwrap().clone();
                                let acquisition = acquisition.lock().unwrap().clone();
                                if let Some(acquisition) = acquisition {
                                    acquisition.entered.notify_one();
                                    acquisition.release.notified().await;
                                }
                                Json(document)
                            }
                        }
                    }),
                )
                .route("/other", get(|| async { "healthy" }))
                .route("/first", get(|| async { "first" }))
                .route("/second", get(|| async { "second" }))
                .route(
                    "/read",
                    get({
                        let removed = read_removed.clone();
                        move || {
                            let removed = removed.clone();
                            async move {
                                if removed.load(Ordering::SeqCst) {
                                    (axum::http::StatusCode::NOT_FOUND, "removed")
                                } else {
                                    (axum::http::StatusCode::OK, "current")
                                }
                            }
                        }
                    }),
                ),
        )
        .await;
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("state")).unwrap();
        std::fs::write(
            directory.path().join("other.json"),
            json!({"paths":{"/other":{"get":{}}}}).to_string(),
        )
        .unwrap();
        let path = directory.path().join("deployment.toml");
        let mut config = format!(
            r#"
version=1
[treasury]
state_dir='state'
[wallets.payer]
mode='static'
private_key_env='UNUSED'
[sources.api]
spec='https://{address}/spec'
base_url='https://{address}'
[sources.other]
spec='other.json'
base_url='https://{address}'
[servers.affected]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='payer'
sources=['api']
include_tools=['api_read']
[servers.unrelated]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='payer'
sources=['other']
"#
        );
        if sibling {
            config = config
                .replace("sources=['api']", "sources=['api','other']")
                .replace(
                    "include_tools=['api_read']",
                    "include_tools=['api_read','other_other']\nwait_for_catalogs=false",
                );
        }
        if alias {
            config = config.replace("include_tools=['api_read']", "include_tools=[]");
            config.push_str(&format!(
                r#"
[sources.alias]
spec='https://{address}/spec'
base_url='https://{address}'
[servers.alias]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='payer'
sources=['alias']
"#
            ));
        }
        std::fs::write(&path, config).unwrap();
        let deployment = Deployment::load(&path).await.unwrap();
        assert_eq!(deployment.listeners["affected"].wait_for_catalogs, !sibling);
        let catalog = Arc::new(crate::catalog_state::CatalogState::new(
            crate::catalog_state::CatalogSnapshot {
                views: deployment
                    .listeners
                    .keys()
                    .map(|name| (name.clone(), Vec::new()))
                    .collect(),
                ..Default::default()
            },
        ));
        let runtime = Runtime::new(
            deployment,
            BTreeMap::from([("payer".into(), PaidClient::unsigned())]),
            catalog,
            None,
            false,
        );
        runtime.publish().await.unwrap();
        Self {
            _directory: directory,
            runtime,
            document,
            downloads,
            read_removed,
            acquisition,
            origin,
        }
    }
    fn slot(&self) -> crate::http_cache::Slot {
        let cfg = &self.runtime.deployment.sources["api"].config;
        crate::http_cache::Slot::new(cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap()
    }
    async fn acquire(&self) -> RefreshedSource {
        *self.document.lock().unwrap() = json!({"paths":{"/new":{"get":{"operationId":"new"}}}});
        let mut cfg = self.runtime.deployment.sources["api"].config.clone();
        cfg.catalog_cache_force_refresh = true;
        refresh_source(
            "api",
            cfg,
            None,
            self.runtime.deployment.catalog_capacity.clone(),
        )
        .await
        .unwrap()
    }
    async fn assert_unrelated_usable(&self) {
        let snapshot = self.runtime.catalog.read();
        assert_eq!(snapshot.views["unrelated"].len(), 1);
        assert_eq!(
            snapshot.views["unrelated"][0]
                .invoke_output(&Default::default())
                .await
                .unwrap()
                .text,
            "healthy"
        );
    }
}

#[tokio::test]
async fn concurrent_alias_refreshes_publish_independently_and_restart_reuses_cache_winner() {
    let mut fixture = Fixture::with_alias(true).await;
    let initial = fixture.runtime.catalog.read();
    let old_api = initial.views["affected"][0].clone();
    let old_alias = initial.views["alias"][0].clone();
    let api_binding = old_api.refresh.as_ref().unwrap().clone();
    let alias_binding = old_alias.refresh.as_ref().unwrap().clone();
    assert!(!Arc::ptr_eq(&api_binding, &alias_binding));
    let old_hints = old_api.tool.catalog_hints.as_ref().unwrap().clone();
    assert!(Arc::ptr_eq(
        &old_hints,
        old_alias.tool.catalog_hints.as_ref().unwrap()
    ));
    let old_price = old_hints.operation("GET", "/read");
    old_price.learn(
        alloy_primitives::U256::from(42),
        crate::pricing::runtime::received().unwrap(),
    );
    let downloads = fixture.downloads.load(Ordering::SeqCst);
    let mut acquisitions = Vec::new();
    for (id, binding, path) in [
        ("api", &api_binding, "/first"),
        ("alias", &alias_binding, "/second"),
    ] {
        let acquisition = Acquisition {
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        };
        *fixture.acquisition.lock().unwrap() = Some(acquisition.clone());
        *fixture.document.lock().unwrap() = json!({"paths":{(path):{"get":{}}}});
        assert!(binding.trigger());
        assert!(!binding.trigger(), "each source coalesces its own requests");
        let (requested, owner) = fixture.runtime.refresh_receive.recv().await.unwrap();
        assert_eq!(requested, id);
        assert!(Arc::ptr_eq(&owner, binding));
        let mut config = fixture.runtime.deployment.sources[id].config.clone();
        config.catalog_cache_force_refresh = true;
        let gate = fixture.runtime.deployment.catalog_capacity.clone();
        let refresh =
            tokio::spawn(async move { refresh_source(&requested, config, None, gate).await });
        acquisition.entered.notified().await;
        assert!(!refresh.is_finished());
        acquisitions.push((acquisition, refresh));
    }
    assert!(fixture.runtime.refresh_receive.try_recv().is_err());
    assert!(api_binding.refreshing());
    assert!(alias_binding.refreshing());
    assert_eq!(fixture.downloads.load(Ordering::SeqCst), downloads + 2);
    for old in [&old_api, &old_alias] {
        assert_eq!(
            old.invoke_output(&Default::default()).await.unwrap().text,
            "current"
        );
    }
    assert_eq!(old_price.get(), Some(alloy_primitives::U256::from(42)));
    fixture.assert_unrelated_usable().await;

    let (first, api_refresh) = acquisitions.remove(0);
    first.release.notify_one();
    let acquired = api_refresh.await.unwrap().unwrap();
    assert!(acquired.1.pending.is_some());
    fixture
        .runtime
        .complete_refresh("api".into(), api_binding.clone(), Ok(acquired))
        .await;
    let after_api = fixture.runtime.catalog.read();
    assert_eq!(after_api.views["affected"][0].tool.path, "/first");
    assert_eq!(after_api.views["alias"][0].tool.path, "/read");
    assert!(api_binding.check().is_err());
    assert!(!api_binding.refreshing());
    assert!(!api_binding.trigger());
    assert!(alias_binding.check().is_ok());
    assert!(alias_binding.refreshing());
    assert!(!alias_binding.trigger());
    assert_eq!(
        old_alias
            .invoke_output(&Default::default())
            .await
            .unwrap()
            .text,
        "current"
    );
    let first_hints = after_api.views["affected"][0]
        .tool
        .catalog_hints
        .as_ref()
        .unwrap()
        .clone();
    let first_price = first_hints.operation("GET", "/first");
    first_price.learn(
        alloy_primitives::U256::from(43),
        crate::pricing::runtime::received().unwrap(),
    );
    assert_eq!(first_price.get(), Some(alloy_primitives::U256::from(43)));
    old_price.learn(
        alloy_primitives::U256::from(99),
        crate::pricing::runtime::received().unwrap(),
    );
    assert!(old_price.get().is_none());
    fixture.assert_unrelated_usable().await;

    let (second, alias_refresh) = acquisitions.remove(0);
    assert!(!alias_refresh.is_finished());
    second.release.notify_one();
    let acquired = alias_refresh.await.unwrap().unwrap();
    fixture
        .runtime
        .complete_refresh("alias".into(), alias_binding.clone(), Ok(acquired))
        .await;
    let current = fixture.runtime.catalog.read();
    assert_eq!(current.views["affected"][0].tool.path, "/first");
    assert_eq!(current.views["alias"][0].tool.path, "/second");
    assert!(alias_binding.check().is_err());
    assert!(!alias_binding.refreshing());
    assert!(!alias_binding.trigger());
    for (endpoint, expected) in [("affected", "first"), ("alias", "second")] {
        let tool = &current.views[endpoint][0];
        assert!(tool.refresh.as_ref().unwrap().check().is_ok());
        assert_eq!(
            tool.invoke_output(&Default::default()).await.unwrap().text,
            expected
        );
    }
    // The later commit retires the shared cache generation, not the first alias's admission.
    first_price.learn(
        alloy_primitives::U256::from(100),
        crate::pricing::runtime::received().unwrap(),
    );
    assert!(first_price.get().is_none());
    assert!(old_price.get().is_none());
    assert_eq!(initial.views["affected"][0].tool.path, "/read");
    assert_eq!(after_api.views["alias"][0].tool.path, "/read");
    fixture.assert_unrelated_usable().await;
    assert!(fixture.runtime.refresh_receive.try_recv().is_err());

    *fixture.acquisition.lock().unwrap() = None;
    let cached = fixture.slot().read().await.unwrap();
    assert_ne!(
        cached.hints.as_ref().unwrap().generation,
        first_hints.generation
    );
    let cached_document: Value = serde_json::from_slice(&cached.data).unwrap();
    assert!(cached_document["paths"].get("/second").is_some());
    let restarted = Deployment::load(&fixture._directory.path().join("deployment.toml"))
        .await
        .unwrap();
    for id in ["api", "alias"] {
        assert_eq!(restarted.sources[id].tools[0].path, "/second");
        assert_eq!(
            restarted.sources[id].tools[0]
                .catalog_hints
                .as_ref()
                .unwrap()
                .generation,
            cached.hints.as_ref().unwrap().generation
        );
    }
    assert_eq!(fixture.downloads.load(Ordering::SeqCst), downloads + 2);
    fixture.origin.abort();
}

#[tokio::test]
async fn failed_live_publication_retains_previous_disk_bytes_and_generation() {
    let mut fixture = Fixture::new().await;
    let slot = fixture.slot();
    let previous = slot.read().await.unwrap();
    let previous_generation = previous.hints.as_ref().unwrap().generation.clone();
    let acquired = fixture.acquire().await;
    assert!(
        acquired.1.pending.is_some(),
        "refresh must retain cache publication ownership"
    );
    assert_eq!(slot.read().await.unwrap().data, previous.data);
    // A concurrently changed listener-view contract makes static publication reject.
    let mut snapshot = (*fixture.runtime.catalog.read()).clone();
    snapshot.views.insert("injected".into(), Vec::new());
    fixture.runtime.catalog.publish(snapshot);
    let binding = fixture.runtime.refreshes["api"].clone();
    assert!(binding.trigger());
    fixture
        .runtime
        .complete_refresh("api".into(), binding.clone(), Ok(acquired))
        .await;
    assert!(binding.check().is_ok());
    assert!(!binding.refreshing());
    let retained = slot.read().await.unwrap();
    assert_eq!(retained.data, previous.data);
    assert_eq!(retained.hints.unwrap().generation, previous_generation);
    assert_eq!(
        fixture.runtime.deployment.sources["api"].tools[0].path,
        "/read"
    );
    assert_eq!(fixture.downloads.load(Ordering::SeqCst), 2);
    fixture.assert_unrelated_usable().await;
    fixture.origin.abort();
}

#[tokio::test]
async fn repaired_tools_publish_while_pricing_is_pending_and_keep_generation_scoped_observations() {
    for (pricing_succeeds, retire_before_completion) in
        [(true, false), (false, false), (true, true)]
    {
        let mut fixture = Fixture::new().await;
        fixture
            .runtime
            .deployment
            .listeners
            .get_mut("affected")
            .unwrap()
            .include_tools
            .clear();
        fixture.runtime.deployment.refresh_selection(true).unwrap();
        fixture.runtime.publish().await.unwrap();
        let old = fixture.runtime.catalog.read().views["affected"][0].clone();
        let binding = old.refresh.as_ref().unwrap().clone();
        let old_price = old
            .tool
            .catalog_hints
            .as_ref()
            .unwrap()
            .operation("GET", "/read");
        old_price.learn(
            alloy_primitives::U256::from(42),
            crate::pricing::runtime::received().unwrap(),
        );
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let resource = format!(
            "{}/first",
            fixture.runtime.deployment.sources["api"].base_url
        );
        let (address, pricing) = crate::test_tls::serve(Router::new().route("/pricing", get({
            let entered = entered.clone();
            let release = release.clone();
            move || {
                let entered = entered.clone();
                let release = release.clone();
                let resource = resource.clone();
                async move {
                    entered.notify_one();
                    release.notified().await;
                    if pricing_succeeds {
                        Json(json!({"x402Version":2,"resources":[{"resource":resource,"method":"GET","accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"123"}]}]}))
                    } else {
                        Json(json!({"invalid":"pricing document"}))
                    }
                }
            }
        }))).await;
        *fixture.document.lock().unwrap() =
            json!({"paths":{"/first":{"get":{"operationId":"first"}}}});
        fixture.read_removed.store(true, Ordering::SeqCst);
        let mut config = fixture.runtime.deployment.sources["api"].config.clone();
        config.catalog_cache_force_refresh = true;
        config.pricing = Some(crate::pricing::advertised::PricingConfig {
            source: format!("https://{address}/pricing"),
            format: crate::pricing::advertised::PricingFormat::X402Resources,
            key: None,
            unit: Some("call".into()),
            usd_as_usdc: false,
        });
        let gate = fixture.runtime.deployment.catalog_capacity.clone();
        assert!(old.invoke_output(&Default::default()).await.is_err());
        assert!(binding.refreshing());
        let replacement = refresh_source("api", config, None, gate).await.unwrap();
        // Prepared pricing does not start before publication. The accepted
        // definitions must publish before the held pricing response completes.
        fixture
            .runtime
            .complete_refresh("api".into(), binding.clone(), Ok(replacement))
            .await;
        entered.notified().await;
        let current = fixture.runtime.catalog.read().views["affected"][0].clone();
        assert_eq!(current.tool.path, "/first");
        assert_eq!(
            current
                .invoke_output(&Default::default())
                .await
                .unwrap()
                .text,
            "first"
        );
        assert!(binding.check().is_err());
        assert!(!binding.trigger());
        assert!(current.tool.advertised_pricing.amount().is_none());
        let hints = current.tool.catalog_hints.as_ref().unwrap().clone();
        let observed = hints.operation("GET", "/first");
        observed.learn(
            alloy_primitives::U256::from(456),
            crate::pricing::runtime::received().unwrap(),
        );
        old_price.learn(
            alloy_primitives::U256::from(999),
            crate::pricing::runtime::received().unwrap(),
        );
        assert!(old_price.get().is_none());
        assert_eq!(observed.get(), Some(alloy_primitives::U256::from(456)));
        fixture.assert_unrelated_usable().await;

        if retire_before_completion {
            *fixture.document.lock().unwrap() =
                json!({"paths":{"/second":{"get":{"operationId":"second"}}}});
            let mut config = fixture.runtime.deployment.sources["api"].config.clone();
            config.pricing = None;
            let replacement = refresh_source(
                "api",
                config,
                None,
                fixture.runtime.deployment.catalog_capacity.clone(),
            )
            .await
            .unwrap();
            fixture
                .runtime
                .complete_refresh(
                    "api".into(),
                    current.refresh.as_ref().unwrap().clone(),
                    Ok(replacement),
                )
                .await;
        }
        release.notify_one();
        // Test watchdog only; wait for the actual enrichment task, including rejection.
        tokio::time::timeout(Duration::from_secs(5), async {
            while !current.tool.advertised_pricing.acquisition_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if retire_before_completion {
            assert!(current.tool.advertised_pricing.amount().is_none());
            assert!(observed.get().is_none());
            let next = fixture.runtime.catalog.read().views["affected"][0].clone();
            assert_eq!(next.tool.path, "/second");
            assert!(next.tool.advertised_pricing.amount().is_none());
            assert_eq!(
                next.invoke_output(&Default::default()).await.unwrap().text,
                "second"
            );
        } else {
            assert_eq!(
                current.tool.advertised_pricing.amount(),
                pricing_succeeds.then_some(alloy_primitives::U256::from(123))
            );
            assert_eq!(observed.get(), Some(alloy_primitives::U256::from(456)));
            assert!(
                current
                    .description()
                    .contains("Observed cost: ~$0.000456/call.")
            );
            // Completion must neither rotate the observed cache nor replace its cells.
            let cached = fixture.slot().read().await.unwrap();
            assert_eq!(cached.hints.unwrap().generation, hints.generation);
            assert_eq!(
                current
                    .invoke_output(&Default::default())
                    .await
                    .unwrap()
                    .text,
                "first"
            );
        }
        assert_eq!(
            fixture.downloads.load(Ordering::SeqCst),
            if retire_before_completion { 3 } else { 2 }
        );
        pricing.abort();
        fixture.origin.abort();
    }
}

#[tokio::test]
async fn refresh_missing_exact_selector_reports_waiting_endpoint_failure_after_publication() {
    if crate::test_log_capture::run_in_child(
        "deployment::deferred::refresh_tests::refresh_missing_exact_selector_reports_waiting_endpoint_failure_after_publication",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::ERROR)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let mut fixture = Fixture::new().await;
    let slot = fixture.slot();
    let previous = slot.read().await.unwrap();
    let acquired = fixture.acquire().await;
    let binding = fixture.runtime.refreshes["api"].clone();
    assert!(binding.trigger());
    fixture
        .runtime
        .complete_refresh("api".into(), binding, Ok(acquired))
        .await;
    let snapshot = fixture.runtime.catalog.read();
    assert!(snapshot.views["affected"].is_empty());
    assert!(
        fixture
            .runtime
            .deployment
            .selection_errors
            .contains_key("affected")
    );
    assert!(
        !fixture
            .runtime
            .deployment
            .selection_errors
            .contains_key("unrelated")
    );
    fixture.assert_unrelated_usable().await;
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        logs.lines().any(|line| line.contains("ERROR")
            && line.contains("server=\"affected\"")
            && line.contains("failure_stage=\"selection\"")),
        "{logs}"
    );
    assert!(
        !logs
            .lines()
            .any(|line| line.contains("server=\"unrelated\"")),
        "{logs}"
    );
    let published = slot.read().await.unwrap();
    assert_ne!(published.data, previous.data);
    assert_ne!(
        published.hints.unwrap().generation,
        previous.hints.unwrap().generation
    );
    assert_eq!(fixture.downloads.load(Ordering::SeqCst), 2);
    fixture.origin.abort();
}
