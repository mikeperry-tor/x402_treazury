//! One serving-owned publisher; accepted relay work drains before financial owners.
use super::*;
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use std::sync::Arc;

pub(super) struct Runtime {
    #[cfg(test)]
    failure: Option<tokio::sync::oneshot::Receiver<()>>,
    deployment: Deployment,
    wallets: BTreeMap<String, PaidClient>,
    catalog: Arc<crate::catalog_state::CatalogState>,
    manager: Option<Arc<crate::discovery::Manager>>,
    relays: Vec<Arc<crate::discovery_relay::Relay>>,
    pricing_gate: Arc<tokio::sync::Semaphore>,
}
enum Event {
    Catalog(String, Result<Box<Source>>),
    Pricing(String, Result<BTreeMap<String, ToolSpec>>),
}
impl Runtime {
    pub fn new(
        deployment: Deployment,
        wallets: BTreeMap<String, PaidClient>,
        catalog: Arc<crate::catalog_state::CatalogState>,
        manager: Option<Arc<crate::discovery::Manager>>,
    ) -> Self {
        let relays: Vec<_> = deployment
            .sources
            .values()
            .filter_map(|s| s.config.discovery_relay.clone())
            .chain(
                deployment
                    .pending_sources
                    .values()
                    .filter_map(|s| s.config.discovery_relay.clone()),
            )
            .collect();
        Self {
            #[cfg(test)]
            failure: None,
            deployment,
            wallets,
            catalog,
            manager,
            relays,
            pricing_gate: Arc::new(tokio::sync::Semaphore::new(crate::pricing::CONCURRENCY)),
        }
    }
    pub fn relays(&self) -> Vec<Arc<crate::discovery_relay::Relay>> {
        self.relays.clone()
    }
    fn pricing(&self, id: &str) -> futures_util::future::BoxFuture<'static, Event> {
        let id = id.to_owned();
        let source = self.deployment.sources[&id].clone();
        let selected = self
            .deployment
            .selected
            .values()
            .flatten()
            .filter(|(s, _)| s == &id)
            .map(|(_, t)| t.clone())
            .collect();
        let gate = self.pricing_gate.clone();
        async move {
            let _permit = gate.acquire_owned().await.expect("owned pricing gate");
            let result = startup::price_source(&id, &source, selected)
                .await
                .map(|(_, tools)| tools);
            Event::Pricing(id, result)
        }
        .boxed()
    }
    async fn publish(&self) -> Result<()> {
        let mut snapshot = crate::catalog_state::CatalogSnapshot::default();
        let old = match &self.manager {
            Some(manager) => manager.static_snapshot().await?,
            None => (*self.catalog.read()).clone(),
        };
        for (name, server) in self.deployment.build_servers(&self.wallets) {
            let mut tools = server.catalog.read().views["default"].clone();
            // Pricing and unrelated source publication preserve successful help.
            for tool in &mut tools {
                if let Some(previous) = old.views.get(&name).and_then(|tools| {
                    tools.iter().find(|old| {
                        old.tool.name == tool.tool.name
                            && old.base == tool.base
                            && old.tool.help_url == tool.tool.help_url
                    })
                }) {
                    tool.help = previous.help.clone();
                }
            }
            if self.deployment.config.source_management.is_some()
                || self.deployment.config.servers[&name].discover_on_demand
            {
                ensure!(
                    tools.iter().all(|tool| !tool.tool.name.starts_with("dyn_")
                        && !tool.tool.name.starts_with("x402_treazury_")),
                    "static tool uses reserved namespace"
                );
            }
            snapshot.views.insert(name.clone(), tools);
            snapshot.instructions.insert(name, server.instructions);
        }
        match &self.manager {
            Some(manager) => {
                manager.replace_static(old.generation, snapshot).await?;
            }
            None => {
                self.catalog.replace_static(old.generation, snapshot)?;
            }
        }
        Ok(())
    }
    fn accept_source(&mut self, id: &str, source: Source) -> Result<()> {
        self.deployment.sources.insert(id.into(), source);
        let prefix = self
            .deployment
            .unavailable_sources
            .remove(id)
            .expect("pending source prefix");
        let selected: Result<BTreeMap<_, _>> = self
            .deployment
            .config
            .servers
            .iter()
            .map(|(name, server)| {
                Ok((
                    name.clone(),
                    select_listener_tools(
                        name,
                        server,
                        &self.deployment.sources,
                        &self.deployment.unavailable_sources,
                    )?,
                ))
            })
            .collect();
        let selected = selected.and_then(|selected| {
            for (name, tools) in &selected {
                if self.deployment.config.source_management.is_some()
                    || self.deployment.config.servers[name].discover_on_demand
                {
                    ensure!(
                        tools.iter().all(|(_, tool)| !tool.name.starts_with("dyn_")
                            && !tool.name.starts_with("x402_treazury_")),
                        "static tool uses reserved namespace"
                    );
                }
            }
            validate_selected_cover(
                &self.deployment.wallet_resolution,
                &self.deployment.sources,
                &selected,
            )?;
            Ok(selected)
        });
        match selected {
            Ok(mut selected) => {
                for (listener, tools) in &mut selected {
                    for (source, tool) in tools {
                        if source != id
                            && let Some((_, previous)) = self.deployment.selected[listener]
                                .iter()
                                .find(|(old_source, old)| {
                                    old_source == source && old.name == tool.name
                                })
                        {
                            *tool = previous.clone();
                        }
                    }
                }
                self.deployment.selected = selected;
                Ok(())
            }
            Err(error) => {
                self.deployment.sources.remove(id);
                self.deployment
                    .unavailable_sources
                    .insert(id.into(), prefix);
                Err(error)
            }
        }
    }
    pub async fn run(mut self, stop: CancellationToken) -> Self {
        struct FailureGuard(Arc<crate::catalog_state::CatalogState>, bool);
        impl Drop for FailureGuard {
            fn drop(&mut self) {
                if self.1 {
                    self.0.mark_publication_failed();
                }
            }
        }
        let mut failure_guard = FailureGuard(self.catalog.clone(), true);
        #[cfg(test)]
        let failure_receiver = self.failure.take();
        #[cfg(test)]
        let failure = async move {
            match failure_receiver {
                Some(signal) => {
                    let _ = signal.await;
                }
                None => std::future::pending().await,
            }
        };
        #[cfg(not(test))]
        let failure = std::future::pending::<()>();
        tokio::pin!(failure);
        let mut work = FuturesUnordered::new();
        let gate = Arc::new(tokio::sync::Semaphore::new(
            self.deployment.config.startup.catalog_concurrency,
        ));
        for (id, pending) in &self.deployment.pending_sources {
            let id = id.clone();
            let pending = pending.clone();
            let gate = gate.clone();
            work.push(
                async move {
                    let result = pending.resume(&id, gate).await.map(Box::new);
                    Event::Catalog(id, result)
                }
                .boxed(),
            );
        }
        for id in self.deployment.sources.keys() {
            work.push(self.pricing(id));
        }
        loop {
            let event = tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                event = work.next(), if !work.is_empty() => event,
                _ = &mut failure => panic!("injected deferred worker failure"),
            };
            let Some(event) = event else { continue };
            match event {
                Event::Catalog(id, result) => {
                    self.deployment.pending_sources.remove(&id);
                    let result = result.and_then(|source| self.accept_source(&id, *source));
                    match result {
                        Ok(()) => {
                            work.push(self.pricing(&id));
                            tracing::info!(source = id, "Deferred catalog ready");
                        }
                        Err(_) => tracing::warn!(
                            source = id,
                            category = "catalog_unavailable",
                            "Deferred catalog failed; source remains unavailable"
                        ),
                    }
                }
                Event::Pricing(id, result) => match result {
                    Ok(tools) => {
                        for (source, tool) in self.deployment.selected.values_mut().flatten() {
                            if source == &id
                                && let Some(priced) = tools.get(&tool.name)
                            {
                                *tool = priced.clone();
                            }
                        }
                    }
                    Err(_) => tracing::warn!(
                        source = id,
                        category = "pricing_unavailable",
                        "Deferred pricing unavailable; catalog remains usable"
                    ),
                },
            }
            // Do not abandon an accepted publication worker on cancellation.
            if stop.is_cancelled() {
                break;
            }
            if self.publish().await.is_err() {
                tracing::warn!(
                    category = "catalog_publication_failed",
                    "Deferred publication stopped; existing snapshot retained"
                );
                self.catalog.mark_publication_failed();
                break;
            }
        }
        drop(work);
        failure_guard.1 = false;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use serde_json::json;
    use std::sync::atomic::Ordering;

    async fn prepared(
        stall: bool,
        invalid_first: bool,
        payer: Option<PaidClient>,
    ) -> (
        tempfile::TempDir,
        RunningDeployment,
        Arc<crate::catalog_state::CatalogState>,
        crate::discovery_relay::tests::Fixture,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let api = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", api.local_addr().unwrap());
        let api_task = tokio::spawn(async move {
            axum::serve(
                api,
                Router::new().route("/read", get(|| async { "healthy" })),
            )
            .await
            .unwrap();
        });
        std::fs::write(
            dir.path().join("spec.json"),
            r#"{"operations":[{"method":"GET","path":"/read"}]}"#,
        )
        .unwrap();
        let path = dir.path().join("deployment.toml");
        let (first, second) = if invalid_first {
            ("dyn", "late")
        } else {
            ("late", "dyn")
        };
        std::fs::write(
            &path,
            format!(
                r#"
version=1
[startup]
catalog_concurrency=1
[wallets.selected]
mode='static'
private_key_env='KEY'
[sources.healthy]
spec='spec.json'
base_url='{base}'
probe_pricing=false
[sources.a]
spec='spec.json'
prefix='{first}'
base_url='{base}'
probe_pricing=false
[sources.b]
spec='spec.json'
prefix='{second}'
base_url='{base}'
probe_pricing=false
[servers.one]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
wallet='selected'
sources=['healthy','a','b']
discover_on_demand=true
"#
            ),
        )
        .unwrap();
        let mut deployment = Deployment::load(&path).await.unwrap();
        // The unsigned origin fails locally before paid continuation is prepared.
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = format!("https://{}/spec", origin.local_addr().unwrap());
        drop(origin);
        let envelope = json!({"url":target,"method":"GET","statusCode":200,"headers":{},"body":r#"{"operations":[{"method":"GET","path":"/read"}]}"#,"redirectChain":[],"error":null});
        let (mut relay, fixture, relay_task) =
            crate::discovery_relay::tests::fixture(envelope, false, stall).await;
        if let Some(payer) = payer {
            crate::discovery_relay::tests::set_client(&mut relay, payer);
        }
        if stall {
            // Only the existing response-inactivity policy ends accepted paid work.
            crate::discovery_relay::tests::set_timeout(&mut relay, Duration::from_millis(200));
        }
        for id in ["a", "b"] {
            deployment
                .config
                .sources
                .get_mut(id)
                .unwrap()
                .provider
                .insert("spec".into(), toml::Value::String(target.clone()));
        }
        let (sources, unavailable, pending) = startup::load(
            &deployment.config,
            &path,
            true,
            BTreeMap::from([("a".into(), relay.clone()), ("b".into(), relay)]),
            true,
        )
        .await
        .unwrap();
        deployment.sources = sources;
        deployment.unavailable_sources = unavailable;
        deployment.pending_sources = pending;
        deployment.defer_discovery = true;
        for (name, server) in &deployment.config.servers {
            deployment.selected.insert(
                name.clone(),
                select_listener_tools(
                    name,
                    server,
                    &deployment.sources,
                    &deployment.unavailable_sources,
                )
                .unwrap(),
            );
        }
        deployment.prebind_listeners().await.unwrap();
        let env = BTreeMap::from([
            ("KEY".into(), format!("{:064x}", 2)),
            ("TOKEN".into(), "fixture".into()),
        ]);
        let running = deployment.bind(&env).await.unwrap();
        let catalog = running.deferred.as_ref().unwrap().catalog.clone();
        (dir, running, catalog, fixture, relay_task, api_task)
    }

    #[cfg(feature = "zcash")]
    #[tokio::test]
    async fn canonical_capacity_wakes_deferred_publication_in_same_serving_process() {
        use crate::rotation::{
            base::BaseRpc,
            manager::ManagedPool,
            store::{AllocationKind, Store, StoreHandle},
        };
        let state_dir = tempfile::tempdir().unwrap();
        let state_path = state_dir.path().join("state");
        let key = state_dir.path().join("key");
        let mut state = Store::create(&state_path, &key, 1, b"fixture").unwrap();
        let snapshot = state.snapshot().unwrap();
        let treasury_id = state.status().unwrap().treasury_id;
        let reserves =
            serde_json::from_value(json!({"small":{"funding_amount_usdc":"5"}})).unwrap();
        let pool = state
            .ensure_capacity_group("relay", &reserves, None, 0)
            .unwrap();
        // Existing allocation, not authority for the serving path to allocate.
        state
            .allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let rpc = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/",
                    axum::routing::post(crate::discovery_relay::readiness_tests::rpc),
                ),
            )
            .await
            .unwrap();
        });
        let manager = Arc::new(
            ManagedPool::new(
                store.clone(),
                pool,
                BaseRpc::new(&base, 12, 120).unwrap(),
                crate::payment::SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        );
        let (_dir, running, catalog, fixture, relay, api) =
            prepared(false, false, Some(PaidClient::managed(manager.clone()))).await;
        let relay_owner = running.deferred.as_ref().unwrap().relays()[0].clone();
        let waiting = crate::discovery_relay::tests::dependency_wait_signal(&relay_owner).await;
        drop(relay_owner);
        let initial = catalog.read();
        let healthy = initial.views["one"][0].clone();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        assert_eq!(
            healthy.invoke(&Default::default()).await.unwrap(),
            "healthy"
        );
        // The pending source cannot sign with an allocated but unobserved payer.
        tokio::time::timeout(Duration::from_secs(5), waiting.notified())
            .await
            .unwrap();
        assert_eq!(catalog.read().views["one"].len(), 1);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.call(|s| s.funding_jobs()).await.unwrap().len(), 1);
        manager.reconcile().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if catalog.read().views["one"].len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let published = catalog.read();
        let late = published.views["one"]
            .iter()
            .find(|t| t.tool.name.starts_with("late_"))
            .unwrap();
        assert_eq!(late.invoke(&Default::default()).await.unwrap(), "healthy");
        assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
        assert_eq!(initial.views["one"].len(), 1);
        assert_eq!(store.call(|s| s.snapshot()).await.unwrap(), snapshot);
        assert_eq!(store.call(|s| s.funding_jobs()).await.unwrap().len(), 1);
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(manager);
        drop(store);
        worker.await.unwrap();
        let reopened = Store::open(&state_path, &key, &treasury_id).unwrap();
        assert_eq!(reopened.funding_jobs().unwrap().len(), 1);
        rpc.abort();
        relay.abort();
        api.abort();
    }

    #[tokio::test]
    async fn invalid_late_source_does_not_stop_alias_publication_or_existing_calls() {
        for invalid_first in [false, true] {
            let (_dir, running, catalog, fixture, relay, api) =
                prepared(false, invalid_first, None).await;
            let captured = catalog.read();
            let healthy = captured.views["one"][0].clone();
            let stop = CancellationToken::new();
            let serving = tokio::spawn(running.serve(stop.clone()));
            assert_eq!(
                healthy.invoke(&Default::default()).await.unwrap(),
                "healthy"
            );
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let snapshot = catalog.read();
                    if snapshot.views["one"].len() == 2
                        && !snapshot.instructions["one"]
                            .as_ref()
                            .unwrap()
                            .contains("PENDING")
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let snapshot = catalog.read();
            assert!(
                snapshot.views["one"]
                    .iter()
                    .all(|tool| !tool.tool.name.starts_with("dyn_"))
            );
            assert!(
                snapshot.views["one"]
                    .iter()
                    .any(|tool| tool.tool.name.starts_with("late_"))
            );
            assert_eq!(captured.views["one"].len(), 1);
            let retained = snapshot.views["one"]
                .iter()
                .find(|tool| tool.tool.name == healthy.tool.name)
                .unwrap();
            assert!(Arc::ptr_eq(&healthy.help, &retained.help));
            assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
            assert!(!catalog.publication_failed());
            stop.cancel();
            tokio::time::timeout(Duration::from_secs(5), serving)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            relay.abort();
            api.abort();
        }
    }

    #[tokio::test]
    async fn available_calls_and_shutdown_do_not_abandon_accepted_deferred_payment() {
        let (_dir, running, catalog, fixture, relay, api) = prepared(true, false, None).await;
        let stop = CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        tokio::time::timeout(Duration::from_secs(5), async {
            while fixture.signed.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let snapshot = catalog.read();
        assert_eq!(
            snapshot.views["one"][0]
                .invoke(&Default::default())
                .await
                .unwrap(),
            "healthy"
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
        assert_eq!(catalog.read().views["one"].len(), 1);
        relay.abort();
        api.abort();
    }
    #[cfg(feature = "zcash")]
    #[tokio::test]
    async fn worker_panic_still_drains_relay_and_closes_store_before_returning() {
        let (dir, mut running, catalog, fixture, relay_task, api) =
            prepared(true, false, None).await;
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let store = crate::rotation::store::Store::create(&state, &key, 1, b"fixture").unwrap();
        let id = store.id().to_owned();
        running.store_only = Some(StoreOnlyOwner::new(store));
        let relay = running.deferred.as_ref().unwrap().relays[0].clone();
        let (fail, failed) = tokio::sync::oneshot::channel();
        running.deferred.as_mut().unwrap().failure = Some(failed);
        let stop = CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        tokio::time::timeout(Duration::from_secs(5), async {
            while fixture.signed.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!crate::discovery_relay::tests::is_idle(&relay));
        fail.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !catalog.publication_failed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        stop.cancel();
        let error = tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap()
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("deferred discovery worker failed"));
        assert!(
            crate::discovery_relay::tests::is_idle(&relay),
            "accepted relay owner must finish before serve returns"
        );
        let reopened = crate::rotation::store::Store::open(&state, &key, &id).unwrap();
        assert!(reopened.funding_jobs().unwrap().is_empty());
        assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
        relay_task.abort();
        api.abort();
    }
    #[tokio::test]
    async fn late_source_keeps_already_published_price_descriptions() {
        let (_dir, mut running, _catalog, _fixture, relay, api) = prepared(false, true, None).await;
        let runtime = running.deferred.as_mut().unwrap();
        let healthy = runtime
            .deployment
            .selected
            .get_mut("one")
            .unwrap()
            .iter_mut()
            .find(|(source, _)| source == "healthy")
            .unwrap();
        healthy.1.description = "recorded startup price fixture".into();
        let cfg = runtime.deployment.pending_sources["b"].config.clone();
        let source = Source::from_document(
            "b",
            cfg,
            Arc::new(json!({"operations":[{"method":"GET","path":"/read"}]})),
        )
        .unwrap();
        runtime.accept_source("b", source).unwrap();
        assert_eq!(
            runtime.deployment.selected["one"]
                .iter()
                .find(|(source, _)| source == "healthy")
                .unwrap()
                .1
                .description,
            "recorded startup price fixture"
        );
        drop(running);
        relay.abort();
        api.abort();
    }
}
