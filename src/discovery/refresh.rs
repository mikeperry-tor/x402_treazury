//! Source-local runtime refresh preserves the accepted registration contract.
use super::*;

impl Manager {
    pub(super) fn refresh_binding(&self, record: &Record) -> Arc<crate::catalog_refresh::Binding> {
        let key = (record.id.clone(), record.revision);
        let mut bindings = self.refresh_bindings.lock().unwrap();
        // Snapshots and accepted work own gates. Failed publication, removal and
        // obsolete captured tools leave no separate retention responsibility here.
        bindings.retain(|_, binding| binding.strong_count() > 0);
        if let Some(binding) = bindings.get(&key).and_then(Weak::upgrade) {
            return binding;
        }
        let manager = self.self_weak.clone();
        let identity = key.clone();
        let binding = crate::catalog_refresh::Binding::new(move |binding| {
            let Some(manager) = manager.upgrade() else {
                binding.retire();
                return;
            };
            let key = identity.clone();
            let tasks = manager.refresh_tasks.clone();
            tasks.spawn(async move {
                let result = manager.refresh_registered(&key.0, key.1).await;
                match result {
                    Ok(()) => binding.retire(),
                    Err(error) => {
                        let failure = SharedFailure::capture(&error, "source_refresh_failed");
                        let diagnostic =
                            crate::catalog::diagnostics::Failure::from_error(&error, "refresh");
                        tracing::warn!(
                            source = key.0,
                            http_status = failure.status(),
                            failure_stage = diagnostic.stage,
                            authorization_may_charge = failure.possible_payment(),
                            "Provider definitions could not be refreshed; existing tools remain available; the original call was not replayed: {}",
                            diagnostic.reason
                        );
                        binding.fail();
                    }
                }
            });
        });
        if let Some(built) = &record.built {
            binding.own_prices(&built.tools);
        }
        bindings.insert(key, Arc::downgrade(&binding));
        binding
    }

    /// Call after listener work drains: every final-outcome callback is now tracked.
    pub(crate) async fn drain_refreshes(&self) {
        self.refresh_tasks.close();
        self.refresh_tasks.wait().await;
    }

    async fn refresh_registered(self: &Arc<Self>, id: &str, revision: u64) -> Result<()> {
        let record = self
            .inner
            .lock()
            .unwrap()
            .state
            .records
            .get(id)
            .cloned()
            .context("source_removed")?;
        if record.removed || record.revision != revision {
            return Ok(());
        }
        let binding = self.refresh_binding(&record);
        let client = self.document_client(&record.owner)?;
        let owner = client.clone();
        let manager = self.clone();
        owner
            .run_owned(async move {
                manager
                    .refresh_registered_owned(record, client, binding)
                    .await
            })
            .await?
    }

    async fn refresh_registered_owned(
        self: &Arc<Self>,
        mut record: Record,
        client: PaidClient,
        binding: Arc<crate::catalog_refresh::Binding>,
    ) -> Result<()> {
        // Deliberately bypass completed import retention: a hard trigger requests
        // new bytes, while the binding already coalesces this exact revision.
        let document = match record.candidate.remote_spec() {
            Some(url) => {
                self.fetch_document(url, record.candidate.provider.as_ref(), &client)
                    .await?
            }
            None => Document::local(curated::local_document(
                record
                    .candidate
                    .provider
                    .as_ref()
                    .context("provider_required")?,
            )?),
        };
        let result = async {
            let revision = record.revision;
            record.document = String::from_utf8((*document.body).clone())?;
            record.hash = hash(&document.body);
            record.format = FORMAT;
            record.disabled = None;
            record.built = None;
            advance_record(&mut record)?;
            let mut record = tokio::task::spawn_blocking(move || {
                record
                    .prepare()
                    .context(crate::catalog::LoadStage::Generation)?;
                Ok::<_, anyhow::Error>(record)
            })
            .await??;
            // Optional pricing enriches this generation without delaying publication.
            let expires = record.advertised_expires();
            let pricing = if let Some(provider) = &record.candidate.provider {
                let root = Arc::new(crate::catalog::parse_document(record.document.as_bytes())?);
                let built = Arc::get_mut(record.built.as_mut().context("source_not_prepared")?)
                    .context("source_preparation_shared")?;
                let mut settings = provider.settings.clone();
                settings.catalog_pricing_refresh = true;
                settings.catalog_pricing_expires = expires;
                crate::pricing::discovery::prepare_dynamic(
                    &settings,
                    &root,
                    &built.base,
                    &mut built.tools,
                    client,
                )
            } else {
                None
            };
            #[cfg(test)]
            if let Some(prepared) = self.refresh_prepared.lock().unwrap().take() {
                let _ = prepared.send(record.built.as_ref().unwrap().tools.clone());
            }
            let permit = crate::limits::acquire_owned_capacity(
                self.mutations.clone(),
                "mutation_queue_full",
                Some(16),
                "hardcoded",
            )
            .await
            .context("mutation_queue_closed")?;
            let manager = self.clone();
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let mut inner = manager.inner.lock().unwrap();
                let Some(current) = inner.state.records.get(&record.id) else {
                    return Ok(());
                };
                if current.removed || current.revision != revision {
                    return Ok(());
                }
                let mut state = inner.state.clone();
                let id = record.id.clone();
                let owner = record.owner.clone();
                state.records.insert(id.clone(), record);
                state.generation = state
                    .generation
                    .max(manager.catalog.read().generation)
                    .checked_add(1)
                    .context("generation exhausted")?;
                let snapshot = manager.snapshot(&state, &inner.base, state.generation)?;
                if let Some(store) = inner.stores.get_mut(&owner) {
                    let mut saved = state.clone();
                    saved.records.retain(|_, record| {
                        record.owner == owner && record.candidate.lifetime == Lifetime::Persistent
                    });
                    store.save(&saved)?;
                }
                inner.state = state;
                manager.catalog.publish_retiring(snapshot, Some(&binding));
                if let Some(pricing) = pricing {
                    let _ = pricing.send(());
                }
                manager
                    .refresh_bindings
                    .lock()
                    .unwrap()
                    .remove(&(id, revision));
                Ok(())
            })
            .await?
        }
        .await;
        // Keep any relay payment evidence when validation or publication fails.
        document.validate(|_, _, _| result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn manager(file: Option<PathBuf>) -> (Arc<Manager>, String) {
        let listener: ListenerConfig = serde_json::from_value(json!({
            "listen":"127.0.0.1:0", "sources":[], "wallet":"shared",
            "provider_management_remote":true,
        }))
        .unwrap();
        let mut listener = listener;
        listener.source_registry_file = file;
        let manager = Manager::new(
            BTreeMap::from([("owner".into(), listener)]),
            BTreeMap::from([("shared".into(), PaidClient::unsigned())]),
            Arc::new(CatalogState::new(CatalogSnapshot {
                views: BTreeMap::from([("owner".into(), vec![])]),
                ..Default::default()
            })),
            vec![],
        )
        .await
        .unwrap();
        let candidate: Candidate = serde_json::from_value(json!({
            "name":"saved", "spec_url":"https://api.example.com/openapi.json",
            "lifetime": if manager.inner.lock().unwrap().stores.is_empty() { "process" } else { "persistent" },
            "visibility":"server",
        })).unwrap();
        let document = json!({"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],
            "paths":{"/old":{"get":{}}}})
        .to_string();
        let id = Uuid::new_v4().to_string();
        manager
            .commit(
                "owner".into(),
                false,
                Record {
                    id: id.clone(),
                    owner: "owner".into(),
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
                    built: None,
                },
            )
            .await
            .unwrap();
        (manager, id)
    }

    #[tokio::test]
    async fn runtime_refresh_bypasses_import_retention_and_persists_only_new_revision() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sources.sqlite");
        let (manager, id) = manager(Some(file.clone())).await;
        let old = manager.catalog.read().views["owner"][0].clone();
        let before = manager.inner.lock().unwrap().state.records[&id].clone();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let (address, task) = crate::test_tls::serve(Router::new().route(
            "/spec",
            get(move || {
                seen.fetch_add(1, Ordering::SeqCst);
                async {
                    axum::Json(
                        json!({"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],
                "paths":{"/new":{"get":{}}}}),
                    )
                }
            }),
        ))
        .await;
        *manager.fixture_endpoint.lock().unwrap() = Some(format!("https://{address}/spec"));
        let same = manager.refresh_binding(&before);
        assert!(Arc::ptr_eq(old.refresh.as_ref().unwrap(), &same));
        let base = manager.static_snapshot().await.unwrap();
        assert!(
            manager
                .replace_static_retiring(base.generation + 1, base.clone(), Some(same.clone()))
                .await
                .is_err()
        );
        let mut invalid = base.clone();
        invalid.views.clear();
        assert!(
            manager
                .replace_static_retiring(base.generation, invalid, Some(same.clone()))
                .await
                .is_err()
        );
        assert!(
            same.check().is_ok(),
            "failed publication must preserve old admission"
        );
        let observed = same.clone();
        *manager.catalog.before_publish.lock().unwrap() = Some(Arc::new(move || {
            assert!(
                observed.check().is_err(),
                "old admission must end before visibility"
            );
        }));
        manager.refresh_registered(&id, 1).await.unwrap();
        manager.catalog.before_publish.lock().unwrap().take();
        assert!(same.check().is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let after = manager.inner.lock().unwrap().state.records[&id].clone();
        assert_eq!(after.revision, 2);
        assert_eq!(after.accepted_at, before.accepted_at);
        assert_eq!(
            serde_json::to_value(&after.candidate).unwrap(),
            serde_json::to_value(&before.candidate).unwrap()
        );
        let tools = &manager.catalog.read().views["owner"];
        assert_eq!(tools[0].tool.path, "/new");
        assert!(!Arc::ptr_eq(tools[0].refresh.as_ref().unwrap(), &same));
        manager.refresh_registered(&id, 1).await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "obsolete results cannot refresh the replacement"
        );
        // The result can arrive before retained work releases its manager owner.
        // Drain task destruction before testing exclusive restart ownership.
        manager.document_client("owner").unwrap().drain().await;
        drop(manager);
        let saved = store::Store::open(&file, &[]).unwrap().load().unwrap();
        assert_eq!(saved.records[&id].revision, 2);
        assert!(saved.records[&id].document.contains("/new"));
        task.abort();
        let _ = task.await;
    }

    fn configure_external_pricing(manager: &Manager, id: &str) {
        let mut inner = manager.inner.lock().unwrap();
        let mut state = inner.state.clone();
        let record = state.records.get_mut(id).unwrap();
        record.candidate.provider = Some(AcceptedProvider::new(
            "fixture",
            crate::catalog::Config {
                spec: "https://api.example.com/spec".into(),
                prefix: Some("fixture".into()),
                pricing: Some(crate::pricing::advertised::PricingConfig {
                    source: "https://api.example.com/prices".into(),
                    format: crate::pricing::advertised::PricingFormat::X402Resources,
                    key: None,
                    unit: Some("call".into()),
                    usd_as_usdc: false,
                }),
                ..Default::default()
            },
        ));
        record.built = None;
        record.prepare().unwrap();
        state.generation += 1;
        let snapshot = manager
            .snapshot(&state, &inner.base, state.generation)
            .unwrap();
        inner.state = state;
        manager.catalog.publish(snapshot);
    }

    #[test]
    fn repaired_dynamic_tools_publish_before_optional_pricing_and_fence_late_enrichment() {
        if crate::test_log_capture::run_in_child(
            "discovery::refresh::tests::repaired_dynamic_tools_publish_before_optional_pricing_and_fence_late_enrichment",
        ) {
            return;
        }
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            use axum::{Json, http::StatusCode};
            use crate::network::{self, NetworkContext, NetworkPolicy, Mode};
            let document = Arc::new(Mutex::new(json!({"openapi":"3.0.3", "servers":[{"url":"https://api.example.com"}], "paths":{"/new":{"get":{}}}})));
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let downloads = Arc::new(AtomicUsize::new(0));
            let prices = Arc::new(AtomicUsize::new(0));
            let app = Router::new()
                .route("/spec", get({
                    let document = document.clone();
                    let downloads = downloads.clone();
                    move || {
                        downloads.fetch_add(1, Ordering::SeqCst);
                        let document = document.lock().unwrap().clone();
                        async move { Json(document) }
                    }
                }))
                .route("/prices", get({
                    let prices = prices.clone();
                    let entered = entered.clone();
                    let release = release.clone();
                    move || {
                        let first = prices.fetch_add(1, Ordering::SeqCst) == 0;
                        let entered = entered.clone();
                        let release = release.clone();
                        async move {
                            if first {
                                entered.notify_one();
                                release.notified().await;
                            }
                            Json(json!({"x402Version":2,"resources":[{"resource":if first {"https://api.example.com/new"} else {"https://api.example.com/next"},"method":"GET","accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":if first {"123"} else {"789"}}]}]}))
                        }
                    }
                }))
                .route("/old", get(|| async { StatusCode::NOT_FOUND }))
                .route("/new", get(|| async { "repaired" }))
                .route("/next", get(|| async { "next" }));
            let (address, http) = crate::test_tls::serve(app).await;
            // Keep production public-destination checks; only SOCKS maps this name locally.
            let socks = crate::test_socks::Socks::start(BTreeMap::from([("api.example.com".into(),address)]), crate::test_socks::Fault::None).await;
            network::install_test_context(NetworkContext::new(NetworkPolicy {
                mode: Mode::Tor, socks_endpoint: Some(socks.address.into()),
                tor_isolation_retries: Some(0), cover_traffic_enabled: Some(false),
                ..Default::default()
            }).unwrap().with_test_root(crate::test_tls::CA));
            // Capture the proposed tools at the real mutation boundary. Keeping
            // them alive proves that dropping the publication sender, rather than
            // tool destruction or scheduling luck, prevents optional I/O.
            for refusal in ["removed", "newer", "snapshot", "save"] {
                let directory = tempfile::tempdir().unwrap();
                let (manager, id) = manager(Some(directory.path().join("sources.sqlite"))).await;
                configure_external_pricing(&manager, &id);
                let previous = manager.catalog.read();
                let held = manager.mutations.acquire_many(16).await.unwrap();
                let (prepared, ready) = tokio::sync::oneshot::channel();
                *manager.refresh_prepared.lock().unwrap() = Some(prepared);
                let owner = manager.clone();
                let identity = id.clone();
                let refresh = tokio::spawn(async move { owner.refresh_registered(&identity, 1).await });
                let proposed = tokio::time::timeout(Duration::from_secs(5), ready).await.unwrap().unwrap();
                assert!(proposed[0].advertised_pricing.has_acquisition_owner());
                {
                    let mut inner = manager.inner.lock().unwrap();
                    match refusal {
                        "removed" => inner.state.records.get_mut(&id).unwrap().removed = true,
                        "newer" => inner.state.records.get_mut(&id).unwrap().revision += 1,
                        "snapshot" => inner.base.views.clear(),
                        "save" => inner.stores["owner"].reject_writes(),
                        _ => unreachable!(),
                    }
                }
                drop(held);
                let result = refresh.await.unwrap();
                assert_eq!(result.is_err(), matches!(refusal, "snapshot" | "save"));
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !proposed[0].advertised_pricing.acquisition_finished() {
                        tokio::task::yield_now().await;
                    }
                }).await.unwrap();
                assert_eq!(prices.load(Ordering::SeqCst), 0, "{refusal}: unpublished pricing must perform no requests or enter relay fallback");
                assert!(Arc::ptr_eq(&previous, &manager.catalog.read()));
                manager.drain_refreshes().await;
            }
            for retire_before_completion in [false, true] {
                prices.store(0, Ordering::SeqCst);
                downloads.store(0, Ordering::SeqCst);
                *document.lock().unwrap() = json!({"openapi":"3.0.3", "servers":[{"url":"https://api.example.com"}], "paths":{"/new":{"get":{}}}});
                let (manager, id) = manager(None).await;
                configure_external_pricing(&manager, &id);
                let observed_prices = prices.clone();
                *manager.catalog.before_publish.lock().unwrap() = Some(Arc::new(move || {
                    assert_eq!(observed_prices.load(Ordering::SeqCst), 0);
                }));
                let old = manager.catalog.read().views["owner"][0].clone();
                let binding = old.refresh.as_ref().unwrap().clone();
                assert!(old.invoke_output(&Default::default()).await.is_err());
                entered.notified().await;
                // The callback must finish publication while optional pricing is still held.
                tokio::time::timeout(Duration::from_secs(5), refresh_finished(&binding)).await.unwrap();
                manager.catalog.before_publish.lock().unwrap().take();
                let current = manager.catalog.read().views["owner"][0].clone();
                assert_eq!(current.tool.path, "/new");
                assert_eq!(current.invoke_output(&Default::default()).await.unwrap().text, "repaired");
                assert!(current.tool.advertised_pricing.amount().is_none());
                assert!(binding.check().is_err());
                if retire_before_completion {
                    *document.lock().unwrap() = json!({"openapi":"3.0.3", "servers":[{"url":"https://api.example.com"}], "paths":{"/next":{"get":{}}}});
                    let next_binding = current.refresh.as_ref().unwrap();
                    assert!(next_binding.trigger());
                    tokio::time::timeout(Duration::from_secs(5), refresh_finished(next_binding)).await.unwrap();
                }
                release.notify_one();
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !current.tool.advertised_pricing.acquisition_finished() {
                        tokio::task::yield_now().await;
                    }
                }).await.unwrap();
                assert_eq!(current.tool.advertised_pricing.amount(), (!retire_before_completion).then_some(alloy_primitives::U256::from(123)));
                if retire_before_completion {
                    let next = manager.catalog.read().views["owner"][0].clone();
                    tokio::time::timeout(Duration::from_secs(5), async {
                        while !next.tool.advertised_pricing.acquisition_finished() { tokio::task::yield_now().await; }
                    }).await.unwrap();
                    assert_eq!(next.tool.path, "/next");
                    assert_eq!(next.tool.advertised_pricing.amount(), Some(alloy_primitives::U256::from(789)));
                    assert_eq!(next.invoke_output(&Default::default()).await.unwrap().text, "next");
                }
                assert_eq!(downloads.load(Ordering::SeqCst), if retire_before_completion {2} else {1});
                assert_eq!(prices.load(Ordering::SeqCst), if retire_before_completion {2} else {1});
                manager.drain_refreshes().await;
                manager.document_client("owner").unwrap().drain().await;
            }
            http.abort();
        });
    }

    #[tokio::test]
    async fn invalid_runtime_refresh_retains_complete_previous_publication() {
        let (manager, id) = manager(None).await;
        let before = manager.catalog.read();
        let document = manager.inner.lock().unwrap().state.records[&id]
            .document
            .clone();
        let (address, task) =
            crate::test_tls::serve(Router::new().route("/spec", get(|| async { "invalid json" })))
                .await;
        *manager.fixture_endpoint.lock().unwrap() = Some(format!("https://{address}/spec"));
        assert!(manager.refresh_registered(&id, 1).await.is_err());
        assert!(Arc::ptr_eq(&before, &manager.catalog.read()));
        {
            let inner = manager.inner.lock().unwrap();
            assert_eq!(inner.state.records[&id].revision, 1);
            assert_eq!(inner.state.records[&id].document, document);
        }
        task.abort();
        let _ = task.await;
    }

    async fn held_schema(
        manager: &Manager,
        body: &'static str,
    ) -> (
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/spec",
                get({
                    let entered = entered.clone();
                    let release = release.clone();
                    let hits = hits.clone();
                    move || {
                        let entered = entered.clone();
                        let release = release.clone();
                        let hits = hits.clone();
                        async move {
                            hits.fetch_add(1, Ordering::SeqCst);
                            entered.notify_one();
                            release.notified().await;
                            body
                        }
                    }
                }),
            )
            .route("/old", get(|| async { "current" }));
        let (address, task) = crate::test_tls::serve(app).await;
        *manager.fixture_endpoint.lock().unwrap() = Some(format!("https://{address}/spec"));
        (entered, release, hits, task)
    }

    async fn refresh_finished(binding: &crate::catalog_refresh::Binding) {
        // Observe the callback's terminal transition rather than relying on a
        // delay or the paid-client drain, which precedes callback completion.
        while binding.refreshing() {
            tokio::task::yield_now().await;
        }
    }

    fn server(manager: &Arc<Manager>) -> crate::server::Server {
        let mut server =
            crate::server::Server::new(vec![], PaidClient::unsigned(), String::new(), None);
        server.catalog = manager.catalog.clone();
        server.catalog_server = "owner".into();
        server.discovery = Some(manager.clone());
        server
    }

    #[tokio::test]
    async fn callback_coalesces_while_dynamic_calls_and_search_survive_refresh_failure() {
        let (manager, id) = manager(None).await;
        let old = manager.catalog.read().views["owner"][0].clone();
        let binding = old.refresh.as_ref().unwrap().clone();
        let (entered, release, hits, task) = held_schema(&manager, "invalid JSON").await;
        let mut snapshot = (*manager.catalog.read()).clone();
        // The dispatcher uses a local unsigned fixture client; production imported
        // tools retain their public-destination enforcement.
        snapshot.views.get_mut("owner").unwrap()[0].client = PaidClient::unsigned();
        snapshot.views.get_mut("owner").unwrap()[0].base = manager
            .fixture_endpoint
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .trim_end_matches("/spec")
            .to_owned();
        manager.catalog.publish(snapshot);
        let server = server(&manager);
        assert!(binding.trigger());
        entered.notified().await;
        assert!(!binding.trigger());
        let args = json!({"tool_id":old.tool.name,"arguments":{}});
        let output = server
            .invoke_output("x402_treazury_tool_call", args.as_object().unwrap())
            .await
            .unwrap();
        assert_eq!(output.text, "current");
        let search = server
            .invoke_output("x402_treazury_tools_search", &Default::default())
            .await
            .unwrap();
        let search: Value = serde_json::from_str(&search.text).unwrap();
        assert_eq!(search["items"].as_array().unwrap().len(), 1);
        assert!(search.get("hint").is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        release.notify_one();
        refresh_finished(&binding).await;
        let output = server
            .invoke_output("x402_treazury_tool_call", args.as_object().unwrap())
            .await
            .unwrap();
        assert_eq!(output.text, "current");
        assert!(binding.check().is_ok());
        let search = server
            .invoke_output("x402_treazury_tools_search", &Default::default())
            .await
            .unwrap();
        let search: Value = serde_json::from_str(&search.text).unwrap();
        assert!(
            search.get("hint").is_none(),
            "terminal failure must not promise an active refresh"
        );
        assert_eq!(manager.inner.lock().unwrap().state.records[&id].revision, 1);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "successful calls must not initiate another catalog acquisition"
        );
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn accepted_refresh_is_drained_through_publication() {
        let (manager, id) = manager(None).await;
        let binding = manager.catalog.read().views["owner"][0]
            .refresh
            .clone()
            .unwrap();
        let (entered, release, _, task) =
            held_schema(&manager, r#"{"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],"paths":{"/replacement":{"get":{}}}}"#).await;
        binding.trigger();
        entered.notified().await;
        // This handle is retained independently of any static publisher task.
        let drain_owner = manager.clone();
        let drain = tokio::spawn(async move { drain_owner.drain_refreshes().await });
        tokio::task::yield_now().await;
        assert!(!drain.is_finished());
        release.notify_one();
        drain.await.unwrap();
        assert_eq!(manager.inner.lock().unwrap().state.records[&id].revision, 2);
        assert!(!binding.refreshing());
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn callback_cannot_publish_over_removal_or_a_newer_revision() {
        for removed in [false, true] {
            let (manager, id) = manager(None).await;
            let old = manager.catalog.read().views["owner"][0].clone();
            let binding = old.refresh.as_ref().unwrap().clone();
            let (entered, release, hits, task) = held_schema(&manager,
                r#"{"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],"paths":{"/stale":{"get":{}}}}"#,
            ).await;
            assert!(binding.trigger());
            entered.notified().await;
            let generation = {
                let mut inner = manager.inner.lock().unwrap();
                let mut state = inner.state.clone();
                let record = state.records.get_mut(&id).unwrap();
                record.removed = removed;
                record.document = r#"{"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],"paths":{"/current":{"get":{}}}}"#.into();
                record.hash = hash(record.document.as_bytes());
                record.built = None;
                advance_record(record).unwrap();
                if !removed {
                    record.prepare().unwrap();
                }
                state.generation += 1;
                let generation = state.generation;
                let snapshot = manager.snapshot(&state, &inner.base, generation).unwrap();
                inner.state = state;
                manager.catalog.publish(snapshot);
                generation
            };
            release.notify_one();
            refresh_finished(&binding).await;
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            assert_eq!(manager.catalog.read().generation, generation);
            let snapshot = manager.catalog.read();
            if removed {
                assert!(snapshot.views["owner"].is_empty());
            } else {
                assert_eq!(snapshot.views["owner"][0].tool.path, "/current");
            }
            assert_eq!(manager.inner.lock().unwrap().state.records[&id].revision, 2);
            assert!(!binding.trigger());
            task.abort();
            let _ = task.await;
        }
    }

    #[tokio::test]
    async fn failed_snapshot_gates_have_no_independent_retention() {
        let (manager, id) = manager(None).await;
        let record = manager.inner.lock().unwrap().state.records[&id].clone();
        let mut proposed = record.clone();
        proposed.revision += 1;
        let gate = manager.refresh_binding(&proposed);
        let weak = Arc::downgrade(&gate);
        drop(gate);
        assert!(weak.upgrade().is_none());
        let current = manager.refresh_binding(&record);
        assert_eq!(manager.refresh_bindings.lock().unwrap().len(), 1);
        assert!(Arc::ptr_eq(
            &current,
            manager.catalog.read().views["owner"][0]
                .refresh
                .as_ref()
                .unwrap()
        ));
    }
}
