//! One serving-owned publisher; accepted relay work drains before financial owners.
#[cfg(test)]
mod readiness_tests;
#[cfg(test)]
mod startup_socks_tests;
#[cfg(test)]
mod tor_catalog_capacity_tests;
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
    can_allocate: bool,
    readiness: BTreeMap<String, tokio::sync::oneshot::Sender<Result<()>>>,
}
enum Event {
    OperatorStatus(String, Option<bool>, tokio::sync::watch::Receiver<bool>),
    Catalog(String, Result<Box<Source>>),
}
fn status_change(
    id: String,
    mut receiver: tokio::sync::watch::Receiver<bool>,
) -> futures_util::future::BoxFuture<'static, Event> {
    async move {
        let value = receiver
            .changed()
            .await
            .ok()
            .map(|()| *receiver.borrow_and_update());
        Event::OperatorStatus(id, value, receiver)
    }
    .boxed()
}
impl Runtime {
    pub fn new(
        deployment: Deployment,
        wallets: BTreeMap<String, PaidClient>,
        catalog: Arc<crate::catalog_state::CatalogState>,
        manager: Option<Arc<crate::discovery::Manager>>,
        can_allocate: bool,
    ) -> Self {
        let relays: Vec<_> = deployment
            .sources
            .values()
            .filter_map(|s| s.config.relay_fallback.clone())
            .chain(
                deployment
                    .pending_sources
                    .values()
                    .filter_map(|s| s.config.relay_fallback.clone()),
            )
            .collect();
        Self {
            can_allocate,
            readiness: BTreeMap::new(),
            #[cfg(test)]
            failure: None,
            deployment,
            wallets,
            catalog,
            manager,
            relays,
        }
    }
    pub fn relays(&self) -> Vec<Arc<crate::discovery_relay::Relay>> {
        self.relays.clone()
    }
    pub fn catalog_readiness(
        &mut self,
    ) -> BTreeMap<String, tokio::sync::oneshot::Receiver<Result<()>>> {
        self.deployment
            .config
            .servers
            .iter()
            .filter(|(_, config)| config.wait_for_catalogs)
            .map(|(name, _)| {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                self.readiness.insert(name.clone(), sender);
                (name.clone(), receiver)
            })
            .collect()
    }
    // Derive completion from the catalog owner rather than tracking a second
    // source ledger. Call only after the corresponding snapshot is published.
    fn signal_readiness(&mut self) {
        for name in self.readiness.keys().cloned().collect::<Vec<_>>() {
            let sources = &self.deployment.config.servers[&name].sources;
            let failed = sources.iter().find(|id| {
                !self.deployment.pending_sources.contains_key(*id)
                    && !self.deployment.sources.contains_key(*id)
                    && !self.deployment.config.sources[*id].optional_startup
            });
            let result = if let Some(id) = failed {
                Some(Err(anyhow::anyhow!("required catalog unavailable: {id}")))
            } else if sources
                .iter()
                .all(|id| !self.deployment.pending_sources.contains_key(id))
            {
                Some(Ok(()))
            } else {
                None
            };
            if let Some(result) = result {
                let _ = self.readiness.remove(&name).unwrap().send(result);
            }
        }
    }
    async fn publish(&self) -> Result<()> {
        let mut snapshot = crate::catalog_state::CatalogSnapshot::default();
        let old = match &self.manager {
            Some(manager) => manager.static_snapshot().await?,
            None => (*self.catalog.read()).clone(),
        };
        for (name, server) in self.deployment.build_servers(&self.wallets) {
            let mut tools = server.catalog.read().views["default"].clone();
            // Unrelated source publication preserves successful help.
            for tool in &mut tools {
                if let Some(previous) = old.views.get(&name).and_then(|tools| {
                    tools.iter().find(|old| {
                        old.tool.name == tool.tool.name
                            && old.base == tool.base
                            && old.tool.help_url == tool.tool.help_url
                            && old.help.compatible(&tool.help)
                    })
                }) {
                    tool.help = previous.help.clone();
                }
            }
            if crate::discovery::policy::enabled(&self.deployment.config)
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
                    listener_tool_inventory(
                        name,
                        server,
                        &self.deployment.sources,
                        &self.deployment.unavailable_sources,
                        true,
                    )?,
                ))
            })
            .collect();
        let selected = selected.and_then(|inventories| {
            let (selected, conflicts): (BTreeMap<_, _>, BTreeMap<_, _>) = inventories
                .into_iter()
                .map(|(name, inventory)| {
                    ((name.clone(), inventory.tools), (name, inventory.conflicts))
                })
                .unzip();
            for (name, tools) in &selected {
                if crate::discovery::policy::enabled(&self.deployment.config)
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
            Ok((selected, conflicts))
        });
        match selected {
            Ok((selected, conflicts)) => {
                self.deployment.selected = selected;
                self.deployment.conflicting_tools = conflicts;
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
        self.signal_readiness();
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
            let can_allocate = self.can_allocate;
            let (status, changed) = tokio::sync::watch::channel(false);
            let status_id = id.clone();
            work.push(status_change(status_id, changed));
            work.push(
                async move {
                    let result = pending
                        .resume(&id, gate, can_allocate, status)
                        .await
                        .map(Box::new);
                    Event::Catalog(id, result)
                }
                .boxed(),
            );
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
                Event::OperatorStatus(id, required, receiver) => {
                    let Some(required) = required else {
                        continue;
                    };
                    let Some(pending) = self.deployment.pending_sources.get_mut(&id) else {
                        continue;
                    };
                    pending.operator_required = required;
                    work.push(status_change(id, receiver));
                }
                Event::Catalog(id, result) => {
                    self.deployment.pending_sources.remove(&id);
                    let result = result.and_then(|source| {
                        self.accept_source(&id, *source)
                            .context(crate::catalog::LoadStage::Selection)
                    });
                    match result {
                        Ok(()) => {
                            tracing::info!(source = id, "Catalog ready");
                        }
                        Err(error) => {
                            let failure = startup::catalog_failure(&error, "fetch_parse");
                            tracing::warn!(
                                source = id,
                                category = "catalog_unavailable",
                                failure_stage = failure.stage,
                                reason = failure.reason,
                                http_status = failure.http_status,
                                "Catalog could not be made available; this source's tools are unavailable; correct the cause and restart to retry"
                            );
                        }
                    }
                    selection_diagnostics::report_completed(
                        &self.deployment.config,
                        &self.deployment.sources,
                        &self.deployment.pending_sources,
                        Some(&id),
                    );
                }
            }
            // Do not abandon an accepted publication worker on cancellation.
            if stop.is_cancelled() {
                break;
            }
            if let Err(error) = self.publish().await {
                let failure = startup::catalog_failure(&error, "publication");
                tracing::warn!(
                    category = "catalog_publication_failed",
                    failure_stage = failure.stage,
                    reason = failure.reason,
                    "Catalog publication stopped; existing tool inventory retained; endpoints still waiting for catalogs cannot start"
                );
                self.catalog.mark_publication_failed();
                break;
            }
            self.signal_readiness();
        }
        drop(work);
        for (_, sender) in std::mem::take(&mut self.readiness) {
            let _ = sender.send(Err(anyhow::anyhow!(
                "catalog publication stopped before endpoint readiness"
            )));
        }
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

    #[tokio::test]
    async fn conflicting_catalogs_withdraw_all_owners_in_either_completion_order() {
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", origin.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                origin,
                Router::new().fallback(|uri: axum::http::Uri| async move { uri.path().to_owned() }),
            )
            .await
            .unwrap();
        });
        let mut final_views = Vec::new();
        for order in [["a", "b", "c"], ["c", "b", "a"]] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("deployment.toml");
            let mut config =
                String::from("version=1\n[wallets.payer]\nmode='static'\nprivate_key_env='KEY'\n");
            for id in ["a", "b", "c"] {
                std::fs::write(
                    dir.path().join(format!("{id}.json")),
                    json!({"operations":[
                        {"method":"GET","path":"/shared"},
                        {"method":"GET","path":format!("/unique_{id}")}
                    ]})
                    .to_string(),
                )
                .unwrap();
                config.push_str(&format!(
                    "[sources.{id}]\nspec='{id}.json'\nbase_url='{base}/{id}'\n"
                ));
            }
            for (name, sources) in [
                ("combined", "['a','b','c']"),
                ("isolated", "['a']"),
                ("exact", "['a','b','c']"),
            ] {
                config.push_str(&format!("[servers.{name}]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='payer'\nsources={sources}\n"));
            }
            std::fs::write(&path, config).unwrap();
            let mut deployment = Deployment::load(&path).await.unwrap();
            let completed: BTreeMap<_, _> = std::mem::take(&mut deployment.sources)
                .into_iter()
                .map(|(id, source)| {
                    let mut cfg = source.config;
                    cfg.prefix = Some("common".into());
                    let source = Source::from_document(&id, cfg, source.document).unwrap();
                    (id, source)
                })
                .collect();
            let shared = completed["a"]
                .tools
                .iter()
                .find(|tool| tool.path == "/shared")
                .unwrap()
                .name
                .clone();
            deployment
                .config
                .servers
                .get_mut("exact")
                .unwrap()
                .include_tools = vec![shared.clone()];
            assert!(
                select_listener_tools(
                    "combined",
                    &deployment.config.servers["combined"],
                    &completed,
                    &BTreeMap::new()
                )
                .is_err()
            );
            deployment.unavailable_sources = completed
                .keys()
                .map(|id| (id.clone(), "common".into()))
                .collect();
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
            let catalog = Arc::new(crate::catalog_state::CatalogState::new(
                crate::catalog_state::CatalogSnapshot {
                    views: deployment
                        .config
                        .servers
                        .keys()
                        .map(|name| (name.clone(), Vec::new()))
                        .collect(),
                    ..Default::default()
                },
            ));
            let payer = PaidClient::new(
                Payer::new(&format!("{:064x}", 2), SpendPolicy::dollars("1").unwrap()).unwrap(),
            );
            let mut runtime = Runtime::new(
                deployment,
                BTreeMap::from([("payer".into(), payer)]),
                catalog.clone(),
                None,
                false,
            );
            runtime
                .accept_source(order[0], completed[order[0]].clone())
                .unwrap();
            runtime.publish().await.unwrap();
            let captured = catalog.read().views["combined"]
                .iter()
                .find(|bound| bound.tool.name == shared)
                .unwrap()
                .clone();
            for id in &order[1..] {
                runtime.accept_source(id, completed[*id].clone()).unwrap();
                runtime.publish().await.unwrap();
                assert!(
                    !catalog.read().views["combined"]
                        .iter()
                        .any(|bound| bound.tool.name == shared)
                );
            }
            let snapshot = catalog.read();
            assert_eq!(snapshot.views["combined"].len(), 3);
            assert!(snapshot.views["exact"].is_empty());
            assert!(
                snapshot.views["isolated"]
                    .iter()
                    .any(|bound| bound.tool.name == shared)
            );
            assert!(
                snapshot.instructions["combined"]
                    .as_ref()
                    .unwrap()
                    .contains(&format!("UNAVAILABLE ambiguous tool names: {shared}"))
            );
            assert!(
                snapshot.instructions["exact"]
                    .as_ref()
                    .unwrap()
                    .contains(&shared)
            );
            assert!(
                !snapshot.instructions["isolated"]
                    .as_deref()
                    .unwrap_or_default()
                    .contains("ambiguous")
            );
            assert_eq!(
                captured.invoke(&serde_json::Map::new()).await.unwrap(),
                format!("/{}/shared", order[0])
            );
            final_views.push(
                snapshot
                    .views
                    .iter()
                    .map(|(name, tools)| {
                        (
                            name.clone(),
                            tools
                                .iter()
                                .map(|bound| (bound.tool.name.clone(), bound.base.clone()))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>(),
            );
        }
        assert_eq!(final_views[0], final_views[1]);
        task.abort();
    }

    struct DeferredFixtureDns;
    impl reqwest::dns::Resolve for DeferredFixtureDns {
        fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
            let allowed = name.as_str() == "spec.example.com";
            Box::pin(async move {
                if !allowed {
                    return Err(
                        std::io::Error::other("unexpected deferred fixture DNS name").into(),
                    );
                }
                let local: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
                Ok(Box::new([local].into_iter()) as reqwest::dns::Addrs)
            })
        }
    }

    // These tests exercise serving publication and accepted financial work. Keep
    // their public logical target and loopback DNS mapping private to a child;
    // neither the production target checks nor the parent network policy change.
    async fn public_target_fixture(test: &str) -> bool {
        const CHILD: &str = "TREAZURY_DEFERRED_PUBLIC_TARGET_CHILD";
        if std::env::var(CHILD).ok().as_deref() == Some(test) {
            crate::network::install_test_context(
                crate::network::NetworkContext::new(Default::default())
                    .unwrap()
                    .with_test_dns(Arc::new(DeferredFixtureDns)),
            );
            return false;
        }
        let exact = format!("deployment::deferred::tests::{test}");
        let output = tokio::process::Command::new(std::env::current_exe().unwrap())
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
            .env(CHILD, test)
            .args(["--exact", &exact, "--nocapture"])
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        true
    }

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
[sources.a]
spec='spec.json'
prefix='{first}'
base_url='{base}'
[sources.b]
spec='spec.json'
prefix='{second}'
base_url='{base}'
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
        let target = format!(
            "https://spec.example.com:{}/spec",
            origin.local_addr().unwrap().port()
        );
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
            false,
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

    #[tokio::test]
    async fn ordinary_serving_binds_before_slow_unsigned_origin_without_relay() {
        let dir = tempfile::tempdir().unwrap();
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", origin.local_addr().unwrap());
        let (arrived, observed) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel();
        let arrived = Arc::new(tokio::sync::Mutex::new(Some(arrived)));
        let wait = Arc::new(tokio::sync::Mutex::new(Some(wait)));
        let api = tokio::spawn(async move {
            axum::serve(
                origin,
                Router::new()
                    .route("/read", get(|| async { "wrong-method" }).post(|| async { "healthy" }))
                    .route(
                        "/spec",
                        get(move || {
                            let arrived = arrived.clone();
                            let wait = wait.clone();
                            async move {
                                arrived.lock().await.take().unwrap().send(()).unwrap();
                                wait.lock().await.take().unwrap().await.unwrap();
                                axum::Json(json!({"operations":[{"method":"GET","path":"/read"},{"method":"POST","path":"/read"}]}))
                            }
                        }),
                    ),
            )
            .await
            .unwrap();
        });
        std::fs::write(
            dir.path().join("spec.json"),
            r#"{"operations":[{"method":"GET","path":"/read"},{"method":"POST","path":"/read"}]}"#,
        )
        .unwrap();
        let path = dir.path().join("deployment.toml");
        std::fs::write(
            &path,
            format!(
                r#"
version=1
[wallets.selected]
mode='static'
private_key_env='KEY'
[sources.healthy]
spec='spec.json'
base_url='{base}'
instructions_text='Inherited healthy guidance'
[sources.late]
spec='{base}/spec'
base_url='{base}'
[servers.one]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
wallet='selected'
sources=['healthy','late']
wait_for_catalogs=false
discover_on_demand=true
instructions_text=''
discovery_search_tool_instructions_text='Pending platform guidance'
"#
            ),
        )
        .unwrap();
        let env = BTreeMap::from([
            ("KEY".into(), format!("{:064x}", 2)),
            ("TOKEN".into(), "fixture".into()),
        ]);
        let deployment = tokio::time::timeout(
            Duration::from_secs(5),
            Deployment::load_for_serving_with_relay(&path, &env, Default::default()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(deployment.sources.len(), 1);
        assert_eq!(deployment.pending_sources.len(), 1);
        let preview = deployment.build_servers(&BTreeMap::from([(
            "selected".into(),
            PaidClient::new(
                Payer::new(&format!("{:064x}", 2), SpendPolicy::dollars("1").unwrap()).unwrap(),
            ),
        )]));
        let server = preview["one"].clone();
        use rmcp::ServerHandler;
        assert!(server.get_info().instructions.unwrap().contains("PENDING"));
        assert!(
            !server
                .get_info()
                .instructions
                .unwrap()
                .contains("Inherited healthy guidance")
        );
        assert!(
            server
                .get_tool("x402_treazury_tools_search")
                .unwrap()
                .description
                .unwrap()
                .contains("Pending platform guidance")
        );
        let running = deployment.bind(&env).await.unwrap();
        assert_ne!(running.addresses()[0].1.port(), 0);
        let catalog = running.deferred.as_ref().unwrap().catalog.clone();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        tokio::time::timeout(Duration::from_secs(5), observed)
            .await
            .unwrap()
            .unwrap();
        let initial = catalog.read();
        assert_eq!(initial.views["one"].len(), 1);
        assert_eq!(initial.views["one"][0].tool.name, "healthy_read");
        assert!(
            initial.instructions["one"]
                .as_ref()
                .unwrap()
                .contains("PENDING")
        );
        assert_eq!(
            initial.views["one"][0]
                .invoke(&Default::default())
                .await
                .unwrap(),
            "healthy"
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while catalog.read().views["one"].len() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !catalog.read().instructions["one"]
                .as_ref()
                .is_some_and(|s| s.contains("PENDING"))
        );
        for binding in &catalog.read().views["one"] {
            assert!(!binding.tool.name.ends_with("_get") && !binding.tool.name.ends_with("_post"));
            assert_eq!(
                binding.invoke(&Default::default()).await.unwrap(),
                "healthy"
            );
        }
        let mut server = server;
        server.catalog = catalog.clone();
        server.catalog_server = "one".into();
        assert!(
            !server
                .get_info()
                .instructions
                .unwrap()
                .contains("Inherited healthy guidance")
        );
        assert!(
            server
                .get_tool("x402_treazury_tools_search")
                .unwrap()
                .description
                .unwrap()
                .contains("Pending platform guidance")
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        api.abort();
    }

    #[tokio::test]
    async fn required_remote_failure_does_not_probe_prices_or_block_ordinary_serving() {
        use axum::http::{HeaderMap, StatusCode};
        use std::sync::atomic::AtomicUsize;
        let dir = tempfile::tempdir().unwrap();
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", origin.local_addr().unwrap());
        let probes = Arc::new(AtomicUsize::new(0));
        let catalogs = Arc::new(AtomicUsize::new(0));
        let api = Router::new()
            .route(
                "/read",
                get(|headers: HeaderMap| async move {
                    assert!(!headers.contains_key("payment-signature"));
                    assert!(!headers.contains_key("x-payment"));
                    "healthy"
                }),
            )
            .route(
                "/probe",
                get({
                    let probes = probes.clone();
                    move || {
                        let probes = probes.clone();
                        async move {
                            probes.fetch_add(1, Ordering::SeqCst);
                            StatusCode::PAYMENT_REQUIRED
                        }
                    }
                }),
            )
            .route(
                "/spec",
                get({
                    let catalogs = catalogs.clone();
                    move || {
                        let catalogs = catalogs.clone();
                        async move {
                            catalogs.fetch_add(1, Ordering::SeqCst);
                            StatusCode::SERVICE_UNAVAILABLE
                        }
                    }
                }),
            );
        let api = tokio::spawn(async move { axum::serve(origin, api).await.unwrap() });
        for (file, route) in [("healthy.json", "/read"), ("priced.json", "/probe")] {
            std::fs::write(
                dir.path().join(file),
                json!({"operations":[{
                    "method":"GET", "path":route,
                    "pricing":{"amount":"0.014","currency":"USDC"}
                }]})
                .to_string(),
            )
            .unwrap();
        }
        let path = dir.path().join("deployment.toml");
        std::fs::write(
            &path,
            format!(
                r#"
version=1
[wallets.selected]
mode='static'
private_key_env='KEY'
[sources.healthy]
spec='healthy.json'
base_url='{base}'
[sources.priced]
spec='priced.json'
base_url='{base}'
[sources.failed]
spec='{base}/spec'
base_url='{base}'
[servers.one]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
wallet='selected'
sources=['healthy','priced','failed']
wait_for_catalogs=false
"#
            ),
        )
        .unwrap();
        let env = BTreeMap::from([
            ("KEY".into(), format!("{:064x}", 2)),
            ("TOKEN".into(), "fixture".into()),
        ]);
        let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
            .await
            .unwrap();
        assert!(!deployment.config.sources["failed"].optional_startup);
        assert_eq!(deployment.sources.len(), 2);
        assert_eq!(deployment.pending_sources.len(), 1);
        let running = deployment.bind(&env).await.unwrap();
        assert_ne!(running.addresses()[0].1.port(), 0);
        let catalog = running.deferred.as_ref().unwrap().catalog.clone();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        let initial = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = catalog.read();
                if snapshot.instructions["one"].as_ref().is_some_and(|s| {
                    s.contains("Source failed: UNAVAILABLE") && !s.contains("PENDING")
                }) {
                    break snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!catalog.publication_failed());
        assert_eq!(initial.views["one"].len(), 2);
        assert!(
            initial.views["one"].iter().any(
                |t| t.tool.name.starts_with("priced_") && t.tool.description.contains("$0.014")
            )
        );
        assert_eq!(probes.load(Ordering::SeqCst), 0);
        let healthy = initial.views["one"]
            .iter()
            .find(|t| t.tool.name.starts_with("healthy_"))
            .unwrap()
            .clone();
        assert_eq!(
            healthy.invoke(&Default::default()).await.unwrap(),
            "healthy"
        );
        assert_eq!(catalogs.load(Ordering::SeqCst), 1);
        let error = Deployment::load(&path)
            .await
            .err()
            .expect("strict inspection must reject required remote failure");
        assert!(
            format!("{error:#}").contains("source failed: loading spec"),
            "{error:#}"
        );
        assert_eq!(catalogs.load(Ordering::SeqCst), 2);
        assert_eq!(probes.load(Ordering::SeqCst), 0);
        assert!(!catalog.publication_failed());
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        api.abort();
    }

    #[tokio::test]
    async fn canonical_capacity_wakes_deferred_publication_in_same_serving_process() {
        if public_target_fixture(
            "canonical_capacity_wakes_deferred_publication_in_same_serving_process",
        )
        .await
        {
            return;
        }
        use crate::rotation::{
            base::BaseRpc,
            manager::ManagedPool,
            store::{AllocationKind, Store, StoreHandle},
        };
        for existing in [true, false] {
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
            if existing {
                state
                    .allocate_capacity_kind(&pool, AllocationKind::Small, true)
                    .unwrap()
                    .unwrap();
            }
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
                    pool.clone(),
                    BaseRpc::new(&base, 12).unwrap(),
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
            assert_eq!(
                store.call(|s| s.funding_jobs()).await.unwrap().len(),
                usize::from(existing)
            );
            if !existing {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !catalog.read().instructions["one"]
                        .as_ref()
                        .unwrap()
                        .contains("OPERATOR_REQUIRED")
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                // Explicit fixture action supplies the missing allocation. The
                // diagnostic waiter neither allocates nor becomes terminal.
                store
                    .call(move |s| s.allocate_capacity_kind(&pool, AllocationKind::Small, true))
                    .await
                    .unwrap()
                    .unwrap();
                tokio::time::timeout(Duration::from_secs(5), async {
                    while catalog.read().instructions["one"]
                        .as_ref()
                        .unwrap()
                        .contains("OPERATOR_REQUIRED")
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(catalog.read().views["one"].len(), 1);
                assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
                assert!(
                    catalog.read().instructions["one"]
                        .as_ref()
                        .unwrap()
                        .contains("PENDING")
                );
            } else {
                assert!(
                    !catalog.read().instructions["one"]
                        .as_ref()
                        .unwrap()
                        .contains("OPERATOR_REQUIRED")
                );
            }
            store
                .call(|s| {
                    for job in s.funding_jobs()? {
                        s.record_funding_balance(
                            &job.id,
                            &job.operation_id,
                            &job.wallet_id,
                            &job.recipient,
                            alloy_primitives::U256::from(5_000_000),
                        )?;
                    }
                    Ok(())
                })
                .await
                .unwrap();
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
            assert!(
                !published.instructions["one"]
                    .as_ref()
                    .is_some_and(|s| s.contains("OPERATOR_REQUIRED"))
            );
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
    }

    #[tokio::test]
    async fn invalid_late_source_does_not_stop_alias_publication_or_existing_calls() {
        if public_target_fixture(
            "invalid_late_source_does_not_stop_alias_publication_or_existing_calls",
        )
        .await
        {
            return;
        }
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
        if public_target_fixture(
            "available_calls_and_shutdown_do_not_abandon_accepted_deferred_payment",
        )
        .await
        {
            return;
        }
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
    #[tokio::test]
    async fn worker_panic_still_drains_relay_and_closes_store_before_returning() {
        if public_target_fixture(
            "worker_panic_still_drains_relay_and_closes_store_before_returning",
        )
        .await
        {
            return;
        }
        let (dir, mut running, catalog, fixture, relay_task, api) =
            prepared(true, false, None).await;
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let store = crate::rotation::store::Store::create(&state, &key, 1, b"fixture").unwrap();
        let id = store.id().to_owned();
        running.runtime = ManagedRuntime::StoreOnly(StoreOnlyOwner::new(store));
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
        if public_target_fixture("late_source_keeps_already_published_price_descriptions").await {
            return;
        }
        let (_dir, mut running, _catalog, _fixture, relay, api) = prepared(false, true, None).await;
        let runtime = running.deferred.as_mut().unwrap();
        let authored = Source::from_document(
            "healthy",
            runtime.deployment.sources["healthy"].config.clone(),
            Arc::new(json!({"operations":[{"method":"GET","path":"/read",
                "pricing":{"amount":"0.014","currency":"USDC"}}]})),
        )
        .unwrap();
        runtime
            .deployment
            .sources
            .insert("healthy".into(), authored);
        let inventory = listener_tool_inventory(
            "one",
            &runtime.deployment.config.servers["one"],
            &runtime.deployment.sources,
            &runtime.deployment.unavailable_sources,
            true,
        )
        .unwrap();
        runtime
            .deployment
            .selected
            .insert("one".into(), inventory.tools);
        runtime
            .deployment
            .conflicting_tools
            .insert("one".into(), inventory.conflicts);
        runtime.publish().await.unwrap();
        let initial = runtime.catalog.read();
        let description = runtime.deployment.selected["one"]
            .iter()
            .find(|(source, _)| source == "healthy")
            .unwrap()
            .1
            .description
            .clone();
        assert!(description.contains("$0.014"));
        let cfg = runtime.deployment.pending_sources["b"].config.clone();
        let source = Source::from_document(
            "b",
            cfg,
            Arc::new(json!({"operations":[{"method":"GET","path":"/read"}]})),
        )
        .unwrap();
        runtime.accept_source("b", source).unwrap();
        runtime.publish().await.unwrap();
        assert!(
            initial.views["one"]
                .iter()
                .any(|tool| tool.tool.description == description)
        );
        assert!(
            runtime.catalog.read().views["one"]
                .iter()
                .any(|tool| tool.tool.description == description)
        );
        assert_eq!(
            runtime.deployment.selected["one"]
                .iter()
                .find(|(source, _)| source == "healthy")
                .unwrap()
                .1
                .description,
            description
        );
        drop(running);
        relay.abort();
        api.abort();
    }
}
