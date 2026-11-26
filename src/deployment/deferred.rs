//! One serving-owned publisher; accepted relay work drains before financial owners.
#[cfg(test)]
use axum::serve::Listener as _;
#[cfg(test)]
mod metadata_shutdown_tests;
#[cfg(test)]
mod readiness_tests;
#[cfg(test)]
mod refresh_tests;
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
    refresh_send:
        tokio::sync::mpsc::UnboundedSender<(String, Arc<crate::catalog_refresh::Binding>)>,
    refresh_receive:
        tokio::sync::mpsc::UnboundedReceiver<(String, Arc<crate::catalog_refresh::Binding>)>,
    refreshes: BTreeMap<String, Arc<crate::catalog_refresh::Binding>>,
    deployment: Deployment,
    wallets: BTreeMap<String, PaidClient>,
    catalog: Arc<crate::catalog_state::CatalogState>,
    manager: Option<Arc<crate::discovery::Manager>>,
    relays: Vec<Arc<crate::discovery_relay::Relay>>,
    can_allocate: bool,
    readiness: BTreeMap<String, tokio::sync::oneshot::Sender<Result<()>>>,
    closed_endpoints: BTreeSet<String>,
    startup_ready: Option<tokio::sync::oneshot::Sender<readiness::FundingPools>>,
}
type RefreshedSource = (
    Box<Source>,
    catalog::LoadedCatalog,
    Option<tokio::sync::oneshot::Sender<()>>,
);
enum Event {
    Refresh(
        String,
        Arc<crate::catalog_refresh::Binding>,
        Result<RefreshedSource>,
    ),
    Dependency(
        String,
        Option<crate::discovery_relay::CatalogDependency>,
        tokio::sync::watch::Receiver<crate::discovery_relay::CatalogDependency>,
        CancellationToken,
    ),
    Catalog(String, Result<Box<Source>>),
    Cancelled,
}
fn status_change(
    id: String,
    mut receiver: tokio::sync::watch::Receiver<crate::discovery_relay::CatalogDependency>,
    cancel: CancellationToken,
) -> futures_util::future::BoxFuture<'static, Event> {
    async move {
        let value = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            changed = receiver.changed() => changed.ok().map(|()| receiver.borrow_and_update().clone()),
        };
        Event::Dependency(id, value, receiver, cancel)
    }
    .boxed()
}

fn refresh_binding(
    id: &str,
    sender: &tokio::sync::mpsc::UnboundedSender<(String, Arc<crate::catalog_refresh::Binding>)>,
) -> Arc<crate::catalog_refresh::Binding> {
    let sender = sender.clone();
    let id = id.to_owned();
    crate::catalog_refresh::Binding::new(move |binding| {
        if let Err(error) = sender.send((id.clone(), binding)) {
            error.0.1.fail();
        }
    })
}

async fn refresh_source(
    id: &str,
    config: catalog::Config,
    rules: Option<Vec<SendToolsRule>>,
    gate: Arc<crate::network::DiscoveryCapacity>,
) -> Result<RefreshedSource> {
    let document =
        catalog::load_json_discovery_bounded(&config, crate::network::global(), Some(gate.clone()))
            .await?;
    let mut source =
        Source::from_document(id, config, document.clone())?.with_send_tools(id, rules)?;
    let pricing = crate::pricing::discovery::prepare_static(
        &source.config,
        &source.document,
        &source.base_url,
        &mut source.tools,
        Some(gate),
    );
    Ok((Box::new(source), document, pricing))
}

fn refresh_failed(id: &str, binding: &crate::catalog_refresh::Binding, error: &anyhow::Error) {
    binding.fail();
    let failure =
        crate::discovery_relay::document::SharedFailure::capture(error, "catalog_refresh_failed");
    let diagnostic = startup::catalog_failure(error, "refresh");
    tracing::warn!(
        source = id,
        failure_stage = diagnostic.stage,
        reason = diagnostic.reason,
        http_status = failure.status(),
        authorization_may_charge = failure.possible_payment(),
        "Provider definitions could not be refreshed; existing tools remain available; the original API call was not replayed"
    );
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
        let (refresh_send, refresh_receive) = tokio::sync::mpsc::unbounded_channel();
        let refreshes = deployment
            .config
            .sources
            .keys()
            .map(|id| (id.clone(), refresh_binding(id, &refresh_send)))
            .collect();
        Self {
            refresh_send,
            refresh_receive,
            refreshes,
            can_allocate,
            readiness: BTreeMap::new(),
            closed_endpoints: BTreeSet::new(),
            startup_ready: None,
            #[cfg(test)]
            failure: None,
            deployment,
            wallets,
            catalog,
            manager,
            relays,
        }
    }
    pub(super) fn refresh_manager(&self) -> Option<Arc<crate::discovery::Manager>> {
        self.manager.clone()
    }
    pub fn relays(&self) -> Vec<Arc<crate::discovery_relay::Relay>> {
        self.relays.clone()
    }
    pub fn catalog_readiness(
        &mut self,
    ) -> BTreeMap<String, tokio::sync::oneshot::Receiver<Result<()>>> {
        self.deployment
            .listeners
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
            let result = self.deployment.endpoint_readiness(&name);
            if let Some(result) = result {
                if result.is_err() {
                    self.closed_endpoints.insert(name.clone());
                }
                let _ = self.readiness.remove(&name).unwrap().send(result);
            }
        }
        if self.startup_ready.is_some() {
            let mut complete = true;
            for name in self.deployment.listeners.keys() {
                match self.deployment.endpoint_readiness(name) {
                    Some(Ok(())) => {}
                    Some(Err(_)) => {
                        self.startup_ready.take();
                        return;
                    }
                    None => complete = false,
                }
            }
            if complete {
                let pools = readiness::funding_pools(&self.deployment, &self.wallets);
                let _ = self.startup_ready.take().unwrap().send(pools);
            }
        }
    }
    fn cancel_orphaned_pending_sources(
        &mut self,
        pending_cancels: &mut BTreeMap<String, CancellationToken>,
    ) {
        if self.closed_endpoints.is_empty() {
            return;
        }
        let orphaned: Vec<String> = self
            .deployment
            .pending_sources
            .keys()
            .filter(|id| {
                let mut referenced = false;
                for (endpoint, config) in &self.deployment.listeners {
                    if config.sources.iter().any(|source| source == *id) {
                        referenced = true;
                        if !self.closed_endpoints.contains(endpoint) {
                            return false;
                        }
                    }
                }
                referenced
            })
            .cloned()
            .collect();
        for id in orphaned {
            if let Some(cancel) = pending_cancels.remove(&id) {
                cancel.cancel();
            }
            self.deployment.pending_sources.remove(&id);
        }
    }
    pub(super) fn startup_readiness(
        &mut self,
    ) -> tokio::sync::oneshot::Receiver<readiness::FundingPools> {
        let (send, receive) = tokio::sync::oneshot::channel();
        self.startup_ready = Some(send);
        receive
    }
    pub(super) async fn publish(&self) -> Result<()> {
        self.publish_retiring(None).await
    }
    async fn publish_retiring(
        &self,
        retiring: Option<Arc<crate::catalog_refresh::Binding>>,
    ) -> Result<()> {
        for (name, tools) in &self.deployment.selected {
            for (source, _) in tools {
                let wallet = &self.deployment.wallet_resolution.bindings[name][source].wallet;
                ensure!(
                    self.wallets.contains_key(wallet),
                    anyhow::Error::new(catalog::diagnostics::Problem(
                        "The catalog selects a wallet that was not initialized at startup; restart to initialize the newly selected wallet"
                    ))
                    .context(catalog::LoadStage::Publication)
                );
            }
        }
        let mut snapshot = crate::catalog_state::CatalogSnapshot::default();
        let old = match &self.manager {
            Some(manager) => manager.static_snapshot().await?,
            None => (*self.catalog.read()).clone(),
        };
        for (id, source) in &self.deployment.sources {
            self.refreshes[id].own_prices(&source.tools);
        }
        for (name, server) in self.deployment.build_servers(&self.wallets) {
            let mut tools = server.catalog.read().views["default"].clone();
            for tool in &mut tools {
                if let Some((id, _)) = self.deployment.selected[&name]
                    .iter()
                    .find(|(_, selected)| selected.name == tool.tool.name)
                {
                    tool.refresh = self.refreshes.get(id).cloned();
                }
            }
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
            snapshot.views.insert(name.clone(), tools);
            snapshot.instructions.insert(name, server.instructions);
        }
        match &self.manager {
            Some(manager) => {
                manager
                    .replace_static_retiring(old.generation, snapshot, retiring)
                    .await?;
            }
            None => {
                self.catalog.replace_static_retiring(
                    old.generation,
                    snapshot,
                    retiring.as_deref(),
                )?;
            }
        }
        Ok(())
    }
    fn accept_source(
        &mut self,
        id: &str,
        source: Source,
    ) -> Result<BTreeMap<String, selection_diagnostics::Selection>> {
        let prefix = self
            .deployment
            .unavailable_sources
            .get(id)
            .expect("pending source prefix")
            .clone();
        self.deployment.sources.insert(id.into(), source);
        match self.deployment.refresh_selection(true) {
            Ok(diagnostics) => Ok(diagnostics),
            Err(error) => {
                self.deployment.sources.remove(id);
                self.deployment
                    .unavailable_sources
                    .insert(id.into(), prefix);
                Err(error)
            }
        }
    }
    // Report only published transitions. Failed publication retains old tools,
    // so selection preparation cannot announce their withdrawal.
    fn report_selection_failures(&self, previous: &BTreeMap<String, &'static str>, runtime: bool) {
        for (name, reason) in &self.deployment.selection_errors {
            if (runtime || !self.deployment.listeners[name].wait_for_catalogs)
                && previous.get(name) != Some(reason)
            {
                tracing::error!(
                    server = name,
                    failure_stage = "selection",
                    reason,
                    "MCP endpoint tools are unavailable because its tool selection is invalid; correct the configuration and restart"
                );
            }
        }
    }
    async fn complete_refresh(
        &mut self,
        id: String,
        binding: Arc<crate::catalog_refresh::Binding>,
        result: Result<RefreshedSource>,
    ) {
        let (source, document, pricing) = match result {
            Ok(acquired) => acquired,
            Err(error) => {
                refresh_failed(&id, &binding, &error);
                return;
            }
        };
        let previous_errors = self.deployment.selection_errors.clone();
        let previous_conflicts = self.deployment.published_conflicts();
        let old = self
            .deployment
            .sources
            .insert(id.clone(), *source)
            .expect("existing refresh source");
        let publication = async {
            let diagnostics = self.deployment.refresh_selection(true)?;
            self.refreshes
                .insert(id.clone(), refresh_binding(&id, &self.refresh_send));
            self.publish_retiring(Some(binding.clone())).await?;
            Ok(diagnostics)
        }
        .await;
        let diagnostics = match publication {
            Ok(diagnostics) => diagnostics,
            Err(error) => {
                self.deployment.sources.insert(id.clone(), old);
                self.refreshes.insert(id.clone(), binding.clone());
                let _ = self.deployment.refresh_selection(true);
                refresh_failed(&id, &binding, &error);
                return;
            }
        };
        // Published clones already own enrichment; let it perform I/O only after
        // the old generation has lost its right to publish shared metadata.
        if let Some(pricing) = pricing {
            let _ = pricing.send(());
        }
        self.report_selection_failures(&previous_errors, true);
        self.deployment
            .report_completed_diagnostics(diagnostics, &previous_conflicts, Some(&id));
        self.deployment.sources[&id].report_degradation(&id);
        self.signal_readiness();
        self.attach_observed_hints(&id, document).await;
        tracing::info!(
            source = id,
            "Provider definitions refreshed; the original API call was not replayed"
        );
    }
    // The complete definitions are already published. Cache/hint failure cannot
    // undo that success or reopen execution through the expired generation.
    async fn attach_observed_hints(&mut self, id: &str, mut document: catalog::LoadedCatalog) {
        if self.deployment.sources[id].tools.is_empty() {
            return;
        }
        document.commit().await;
        let Some(hints) = document.hints else {
            return;
        };
        let source = self
            .deployment
            .sources
            .get_mut(id)
            .expect("published source");
        hints.slot.record_configuration(&source.config).await;
        for tool in &mut source.tools {
            tool.catalog_hints = Some(hints.clone());
        }
        self.refreshes[id].own_prices(&source.tools);
        for tools in self.deployment.selected.values_mut() {
            for (source, tool) in tools {
                if source == id {
                    tool.catalog_hints = Some(hints.clone());
                }
            }
        }
        if let Err(error) = self.publish().await {
            let diagnostic = startup::catalog_failure(&error, "hint_publication");
            tracing::warn!(
                source = id,
                reason = diagnostic.reason,
                "Provider definitions refreshed, but shared price observations could not be attached; API tools remain usable"
            );
        }
    }
    fn report_waiting_catalogs(&self, gate: &crate::network::DiscoveryCapacity) {
        if self.readiness.is_empty() {
            return;
        }
        let mut waiting_sources = BTreeSet::new();
        for endpoint in self.readiness.keys() {
            for source in &self.deployment.listeners[endpoint].sources {
                if let Some(entry) = self.deployment.pending_sources.get(source) {
                    let omitted_optional = self.deployment.config.sources[source]
                        .allow_catalog_failure
                        && entry.is_relay_blocked();
                    if !omitted_optional {
                        waiting_sources.insert(source.as_str());
                    }
                }
            }
        }
        if waiting_sources.is_empty() {
            return;
        }
        let mut downloading = Vec::new();
        let mut pending = Vec::new();
        let mut awaiting_relay_funds = Vec::new();
        let mut relay_wallets = BTreeSet::new();
        let uses_tor = crate::network::global().policy.mode == crate::network::Mode::Tor;
        for source in waiting_sources {
            let entry = &self.deployment.pending_sources[source];
            if entry.dependency.waiting_for_relay_funds {
                awaiting_relay_funds.push(source);
                if let Some(wallet) = entry.dependency.relay_wallet.as_deref() {
                    relay_wallets.insert(wallet);
                }
            } else if startup::source_downloading(&entry.config, Some(gate), uses_tor) {
                downloading.push(source);
            } else {
                pending.push(source);
            }
        }
        let endpoints = startup::format_bounded_list(self.readiness.keys().map(String::as_str));
        let downloading = startup::format_bounded_list(downloading);
        let pending = startup::format_bounded_list(pending);
        let awaiting_relay_funds = startup::format_bounded_list(awaiting_relay_funds);
        let relay_wallets = startup::format_bounded_list(relay_wallets);
        if awaiting_relay_funds.is_some() {
            tracing::info!(
                category = "catalog_startup_waiting",
                endpoints = endpoints.as_deref(),
                downloading = downloading.as_deref(),
                pending = pending.as_deref(),
                awaiting_relay_funds = awaiting_relay_funds.as_deref(),
                relay_wallets = relay_wallets.as_deref(),
                "Still waiting for catalogs before starting endpoints; some catalogs need paid relay wallet funding"
            );
        } else {
            tracing::info!(
                category = "catalog_startup_waiting",
                endpoints = endpoints.as_deref(),
                downloading = downloading.as_deref(),
                pending = pending.as_deref(),
                "Still waiting for catalogs before starting endpoints"
            );
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
        self.report_selection_failures(&BTreeMap::new(), false);
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
        let mut pending_cancels: BTreeMap<String, CancellationToken> = BTreeMap::new();
        self.cancel_orphaned_pending_sources(&mut pending_cancels);
        let mut reported_relay_blocks: BTreeSet<(Option<String>, &'static str)> = BTreeSet::new();
        let mut work = FuturesUnordered::new();
        let gate = self.deployment.catalog_capacity.clone();
        for (id, pending) in &self.deployment.pending_sources {
            let id = id.clone();
            let pending = pending.clone();
            let gate = gate.clone();
            let source_cancel = stop.child_token();
            pending_cancels.insert(id.clone(), source_cancel.clone());
            let (status, changed) = tokio::sync::watch::channel(Default::default());
            let readiness = crate::discovery_relay::CatalogReadiness {
                status: Some(status),
                can_allocate: self.can_allocate,
                log_blocks: false,
            };
            let status_id = id.clone();
            work.push(status_change(status_id, changed, source_cancel.clone()));
            work.push(
                async move {
                    tokio::select! {
                        biased;
                        _ = source_cancel.cancelled() => Event::Cancelled,
                        result = pending.resume(&id, gate, readiness) => {
                            Event::Catalog(id, result.map(Box::new))
                        }
                    }
                }
                .boxed(),
            );
        }
        let heartbeat = tokio::time::sleep(startup::CATALOG_PROGRESS_INTERVAL);
        tokio::pin!(heartbeat);
        loop {
            let event = tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                event = work.next(), if !work.is_empty() => event,
                _ = &mut heartbeat, if !self.readiness.is_empty() => {
                    self.report_waiting_catalogs(&gate);
                    heartbeat
                        .as_mut()
                        .reset(tokio::time::Instant::now() + startup::CATALOG_PROGRESS_INTERVAL);
                    continue;
                }
                Some((id, binding)) = self.refresh_receive.recv() => {
                    if !self.refreshes.get(&id).is_some_and(|current| Arc::ptr_eq(current, &binding)) { continue; }
                    let Some(source) = self.deployment.sources.get(&id) else { binding.retire(); continue; };
                    let mut config = source.config.clone();
                    config.catalog_cache_force_refresh = true;
                    let rules = self.deployment.config.sources[&id].send_tools.clone();
                    let refresh_id = id.clone();
                    let gate = gate.clone();
                    work.push(async move {
                        let result = refresh_source(&refresh_id, config, rules, gate).await;
                        Event::Refresh(refresh_id, binding, result)
                    }.boxed());
                    continue;
                },
                _ = &mut failure => panic!("injected deferred worker failure"),
            };
            let Some(event) = event else { continue };
            let mut completed_source = None;
            let mut settled_source = None;
            let previous_selection_errors = self.deployment.selection_errors.clone();
            let previous_conflicts = self.deployment.published_conflicts();
            let mut selection = Ok(BTreeMap::new());
            match event {
                Event::Cancelled => continue,
                Event::Refresh(id, binding, result) => {
                    self.complete_refresh(id, binding, result).await;
                    continue;
                }
                Event::Dependency(id, dependency, receiver, cancel) => {
                    let Some(dependency) = dependency else {
                        continue;
                    };
                    let Some(pending) = self.deployment.pending_sources.get_mut(&id) else {
                        continue;
                    };
                    let previous = std::mem::replace(&mut pending.dependency, dependency.clone());
                    let was_blocked = previous.is_relay_blocked();
                    let now_blocked = dependency.is_relay_blocked();
                    let instructions_changed = previous.operator_required
                        != dependency.operator_required
                        || previous.relay_funding_blocked_reason
                            != dependency.relay_funding_blocked_reason;
                    if let Some((category, reason)) = dependency.block_category_and_reason() {
                        if reported_relay_blocks.insert((dependency.relay_wallet.clone(), category))
                        {
                            dependency.warn_wallet_block();
                        }
                        if !was_blocked && now_blocked {
                            let allow_failure =
                                self.deployment.config.sources[&id].allow_catalog_failure;
                            let closes_waiting = !allow_failure
                                && self.deployment.listeners.iter().any(|(endpoint, config)| {
                                    !self.closed_endpoints.contains(endpoint)
                                        && config.wait_for_catalogs
                                        && config.sources.iter().any(|source| source == &id)
                                });
                            let keeps_running = allow_failure
                                || self.deployment.listeners.iter().any(|(endpoint, config)| {
                                    !self.closed_endpoints.contains(endpoint)
                                        && !config.wait_for_catalogs
                                        && config.sources.iter().any(|source| source == &id)
                                });
                            let impact = match (keeps_running, closes_waiting) {
                                (true, true) => {
                                    "waiting listeners that require this source cannot start; non-waiting listeners omit it until relay funding succeeds"
                                }
                                (true, false) => {
                                    "this source is omitted until relay funding succeeds"
                                }
                                (false, _) => {
                                    "waiting listeners that require this source cannot start"
                                }
                            };
                            tracing::warn!(
                                source = id,
                                category = "catalog_unavailable",
                                relay_wallet = dependency.relay_wallet.as_deref(),
                                reason,
                                "The provider catalog fell back to the paid discovery relay, but relay wallet funding is currently unavailable; {impact}"
                            );
                            selection = self.deployment.refresh_selection(true);
                            settled_source = Some(id.clone());
                        }
                    } else if !dependency.waiting_for_relay_funds {
                        reported_relay_blocks
                            .retain(|(wallet, _)| wallet != &dependency.relay_wallet);
                    }
                    work.push(status_change(id, receiver, cancel));
                    if !instructions_changed && settled_source.is_none() {
                        continue;
                    }
                }
                Event::Catalog(id, result) => {
                    let Some(removed_pending) = self.deployment.pending_sources.remove(&id) else {
                        pending_cancels.remove(&id);
                        continue;
                    };
                    if result.is_ok() && removed_pending.dependency.waiting_for_relay_funds {
                        reported_relay_blocks.retain(|(wallet, _)| {
                            wallet != &removed_pending.dependency.relay_wallet
                        });
                    }
                    heartbeat
                        .as_mut()
                        .reset(tokio::time::Instant::now() + startup::CATALOG_PROGRESS_INTERVAL);
                    pending_cancels.remove(&id);
                    completed_source = Some(id.clone());
                    let result = result.and_then(|source| {
                        self.accept_source(&id, *source)
                            .context(crate::catalog::LoadStage::Selection)
                    });
                    match result {
                        Ok(diagnostics) => selection = Ok(diagnostics),
                        Err(error) => {
                            // The removed pending owner no longer defers exact names.
                            // Recompute after acquisition failure or admission rollback.
                            selection = self.deployment.refresh_selection(true);
                            let failure = startup::catalog_failure(&error, "fetch_parse");
                            tracing::warn!(
                                source = id,
                                category = "catalog_unavailable",
                                failure_stage = failure.stage,
                                reason = failure.reason,
                                http_status = failure.http_status,
                                "The provider catalog could not load, so this source's tools are unavailable. {}. Restart to retry.",
                                failure.reason
                            );
                        }
                    }
                }
            }
            // Do not abandon an accepted publication worker on cancellation.
            if stop.is_cancelled() {
                break;
            }
            let publication = match selection {
                Ok(diagnostics) => self.publish().await.map(|()| diagnostics),
                Err(error) => Err(error.context(catalog::LoadStage::Selection)),
            };
            let diagnostics = match publication {
                Ok(diagnostics) => diagnostics,
                Err(error) => {
                    let failure = startup::catalog_failure(&error, "publication");
                    tracing::warn!(
                        category = "catalog_publication_failed",
                        failure_stage = failure.stage,
                        reason = failure.reason,
                        "Catalog publication stopped. Existing tool inventories remain available, and listeners waiting for catalogs cannot start. {}.",
                        failure.reason
                    );
                    self.catalog.mark_publication_failed();
                    break;
                }
            };
            self.report_selection_failures(&previous_selection_errors, false);
            if let Some(id) = completed_source {
                self.deployment.report_completed_diagnostics(
                    diagnostics,
                    &previous_conflicts,
                    Some(&id),
                );
                if let Some(source) = self.deployment.sources.get(&id) {
                    tracing::info!(source = id, category = "catalog_ready", "Catalog ready");
                    source.report_degradation(&id);
                }
            } else if let Some(id) = settled_source {
                self.deployment.report_completed_diagnostics(
                    diagnostics,
                    &previous_conflicts,
                    Some(&id),
                );
            }
            self.signal_readiness();
            self.cancel_orphaned_pending_sources(&mut pending_cancels);
        }
        drop(work);
        self.startup_ready.take();
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
    async fn final_failure_refreshes_only_its_source_without_replaying_calls() {
        use std::sync::atomic::AtomicUsize;
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let (address, http) = crate::test_tls::serve(Router::new().route(
            "/gone",
            get(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::NOT_FOUND
                }
            }),
        ))
        .await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.toml");
        let spec = directory.path().join("spec.json");
        std::fs::write(&spec, json!({"paths":{"/gone":{"get":{}}}}).to_string()).unwrap();
        std::fs::write(
            directory.path().join("other.json"),
            json!({"paths":{"/other":{"get":{}}}}).to_string(),
        )
        .unwrap();
        std::fs::write(&path, format!("version=1\n[wallets.payer]\nmode='static'\nprivate_key_env='UNUSED'\n[sources.api]\nspec='spec.json'\nbase_url='https://{address}'\n[sources.other]\nspec='other.json'\nbase_url='https://{address}'\n[servers.test]\nlisten='127.0.0.1:0'\nbearer_token_env='UNUSED'\nwallet='payer'\nsources=['api','other']\n")).unwrap();
        let deployment = Deployment::load(&path).await.unwrap();
        let catalog = Arc::new(crate::catalog_state::CatalogState::new(
            crate::catalog_state::CatalogSnapshot {
                views: BTreeMap::from([("test".into(), Vec::new())]),
                ..Default::default()
            },
        ));
        let runtime = Runtime::new(
            deployment,
            BTreeMap::from([("payer".into(), PaidClient::unsigned())]),
            catalog.clone(),
            None,
            false,
        );
        runtime.publish().await.unwrap();
        let old = catalog.read();
        let tool = old.views["test"]
            .iter()
            .find(|tool| tool.tool.path == "/gone")
            .unwrap()
            .clone();
        let other = old.views["test"]
            .iter()
            .find(|tool| tool.tool.path == "/other")
            .unwrap()
            .refresh
            .clone()
            .unwrap();
        let error = tool.invoke_output(&Default::default()).await.unwrap_err();
        assert_eq!(
            crate::payment::ResponseFacts::from_error(&error)
                .unwrap()
                .provider
                .unwrap()
                .as_u16(),
            404
        );
        assert!(tool.refresh.as_ref().unwrap().refreshing());
        assert!(tool.invoke_output(&Default::default()).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        std::fs::write(&spec, json!({"paths":{"/new":{"get":{}}}}).to_string()).unwrap();
        let stop = CancellationToken::new();
        let worker = tokio::spawn(runtime.run(stop.clone()));
        while catalog.read().generation == old.generation {
            tokio::task::yield_now().await;
        }
        let new = catalog.read();
        assert!(
            new.views["test"]
                .iter()
                .any(|tool| tool.tool.path == "/new")
        );
        assert!(
            !new.views["test"]
                .iter()
                .any(|tool| tool.tool.path == "/gone")
        );
        assert!(new.views["test"].iter().any(|tool| {
            tool.refresh
                .as_ref()
                .is_some_and(|binding| Arc::ptr_eq(binding, &other))
        }));
        assert!(tool.invoke_output(&Default::default()).await.is_err());
        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "only explicit calls reach the API; refresh and stale clones do not replay"
        );
        stop.cancel();
        worker.await.unwrap();
        http.abort();
        let _ = http.await;
    }

    #[tokio::test]
    async fn deferred_unsupported_contract_publishes_issues_and_empty_endpoint() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.toml");
        std::fs::write(
            directory.path().join("spec.json"),
            json!({"paths":{"/healthy":{"get":{}}}}).to_string(),
        )
        .unwrap();
        let mut config = String::from(
            "version=1\n[wallets.payer]\nmode='static'\nprivate_key_env='KEY'\n[sources.healthy]\nspec='spec.json'\nbase_url='https://example.com'\n[sources.late]\nspec='spec.json'\nbase_url='https://example.com'\n",
        );
        for (name, sources, wait) in [
            ("waiting", "['late']", true),
            ("nonwaiting", "['late','healthy']", false),
            ("healthy", "['healthy']", true),
        ] {
            config.push_str(&format!("[servers.{name}]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='payer'\nsources={sources}\nwait_for_catalogs={wait}\ninstructions_text=''\n"));
        }
        std::fs::write(&path, config).unwrap();
        let mut deployment = Deployment::load(&path).await.unwrap();
        let old = deployment.sources.remove("late").unwrap();
        deployment
            .unavailable_sources
            .insert("late".into(), old.config.prefix.clone().unwrap());
        deployment.refresh_selection(true).unwrap();
        let unsupported = Source::from_document("late", old.config,
            Arc::new(json!({"paths":{"/unsupported":{"post":{"parameters":[{"in":"cookie","name":"token"}]}}}}))).unwrap();
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
        let mut runtime = Runtime::new(
            deployment,
            BTreeMap::from([("payer".into(), PaidClient::unsigned())]),
            catalog.clone(),
            None,
            false,
        );
        let mut readiness = runtime.catalog_readiness();
        runtime.accept_source("late", unsupported).unwrap();
        assert!(!runtime.deployment.unavailable_sources.contains_key("late"));
        runtime.publish().await.unwrap();
        // Visibility and tools come from the same published generation before readiness resolves.
        let snapshot = catalog.read();
        assert!(snapshot.views["waiting"].is_empty());
        assert_eq!(snapshot.views["healthy"].len(), 1);
        assert_eq!(snapshot.views["nonwaiting"].len(), 1);
        assert!(
            snapshot.instructions["waiting"]
                .as_deref()
                .unwrap()
                .contains("1 API operations are omitted")
        );
        assert!(
            snapshot.instructions["nonwaiting"]
                .as_deref()
                .unwrap()
                .contains("Cookie parameters")
        );
        runtime.signal_readiness();
        assert!(readiness.remove("waiting").unwrap().await.unwrap().is_ok());
        assert!(readiness.remove("healthy").unwrap().await.unwrap().is_ok());
        assert!(!readiness.contains_key("nonwaiting"));
    }

    #[tokio::test]
    async fn conflicting_catalogs_withdraw_all_owners_in_either_completion_order() {
        let origin = crate::test_tls::listener().await;
        let base = format!("https://{}", origin.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                origin,
                Router::new().fallback(|uri: axum::http::Uri| async move { uri.path().to_owned() }),
            )
            .await
            .unwrap();
        });
        let mut final_views = Vec::new();
        for (routed, order) in [
            (false, ["a", "b", "c"]),
            (false, ["c", "b", "a"]),
            (true, ["a", "b", "c"]),
            (true, ["c", "b", "a"]),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("deployment.toml");
            let mut config =
                String::from("version=1\n[wallets.payer]\nmode='static'\nprivate_key_env='KEY'\n");
            for id in ["a", "b", "c"] {
                let mut operations = vec![
                    json!({"method":"GET","path":"/shared"}),
                    json!({"method":"GET","path":format!("/unique_{id}")}),
                ];
                if routed {
                    operations.push(json!({"method":"GET","path":"/not_here"}));
                }
                std::fs::write(
                    dir.path().join(format!("{id}.json")),
                    json!({"operations":operations}).to_string(),
                )
                .unwrap();
                config.push_str(&format!(
                    "[sources.{id}]\nspec='{id}.json'\nbase_url='{base}/{id}'\n"
                ));
                if routed {
                    let targets = if id == "a" {
                        "['combined','exact','isolated']"
                    } else {
                        "['combined','exact']"
                    };
                    config.push_str(&format!(
                        "send_tools=[{{to='exclude',tools=['not_here']}},{{to={targets}}}]\n"
                    ));
                }
            }
            for (name, sources) in [
                ("combined", "['a','b','c']"),
                ("isolated", "['a']"),
                ("exact", "['a','b','c']"),
            ] {
                let sources = if routed { "[]" } else { sources };
                config.push_str(&format!("[servers.{name}]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='payer'\nsources={sources}\n"));
            }
            std::fs::write(&path, config).unwrap();
            let mut deployment = Deployment::load(&path).await.unwrap();
            if routed {
                assert!(
                    deployment
                        .config
                        .servers
                        .values()
                        .all(|server| server.sources.is_empty())
                );
                assert_eq!(deployment.listeners["combined"].sources, ["a", "b", "c"]);
                assert_eq!(deployment.listeners["isolated"].sources, ["a"]);
            }
            let completed: BTreeMap<_, _> = std::mem::take(&mut deployment.sources)
                .into_iter()
                .map(|(id, source)| {
                    let mut cfg = source.config;
                    // Start with valid distinct prefixes, then introduce a collision
                    // while retaining the authored distribution through validation.
                    cfg.prefix = Some("common".into());
                    let source = Source::from_document(&id, cfg, source.document)
                        .unwrap()
                        .with_send_tools(&id, deployment.config.sources[&id].send_tools.clone())
                        .unwrap();
                    if routed {
                        assert!(source.send_tools.is_some());
                        assert!(source.tools.iter().any(|tool| tool.path == "/not_here"));
                    }
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
            deployment.listeners.get_mut("exact").unwrap().include_tools = vec![shared.clone()];
            assert!(
                select_listener_tools(
                    "combined",
                    &deployment.listeners["combined"],
                    &completed,
                    &BTreeMap::new()
                )
                .is_err()
            );
            deployment.unavailable_sources = completed
                .keys()
                .map(|id| (id.clone(), "common".into()))
                .collect();
            for (name, server) in &deployment.listeners {
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
                        .listeners
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
            assert_eq!(snapshot.views["isolated"].len(), 2);
            assert!(
                snapshot
                    .views
                    .values()
                    .all(|tools| { tools.iter().all(|bound| bound.tool.path != "/not_here") })
            );
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
        assert!(final_views.windows(2).all(|pair| pair[0] == pair[1]));
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
                    .with_test_dns(Arc::new(DeferredFixtureDns))
                    .with_test_root(crate::test_tls::CA),
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
        let api = crate::test_tls::listener().await;
        let base = format!("https://{}", api.local_addr().unwrap());
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
prefix='initial_a'
base_url='{base}'
[sources.b]
spec='spec.json'
prefix='initial_b'
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
        let origin = crate::test_tls::listener().await;
        let target = format!(
            "https://spec.example.com:{}/spec",
            origin.local_addr().unwrap().port()
        );
        drop(origin);
        let envelope = json!({"url":target,"method":"GET","statusCode":200,"headers":{},"body":r#"{"operations":[{"method":"GET","path":"/read"}]}"#,"redirectChain":[],"error":null});
        let (mut relay, fixture, relay_task) =
            crate::discovery_relay::tests::fixture(envelope, false, stall).await;
        crate::discovery_relay::tests::set_wallet(&mut relay, "selected");
        if let Some(payer) = payer {
            crate::discovery_relay::tests::set_client(&mut relay, payer);
        }
        if stall {
            // Only the existing response-inactivity policy ends accepted paid work.
            crate::discovery_relay::tests::set_timeout(&mut relay, Duration::from_millis(200));
        }
        for (id, prefix) in [("a", first), ("b", second)] {
            // Introduce the invalid identity at deferred admission, after the
            // valid initial inspection; both aliases still share one download.
            let provider = &mut deployment.config.sources.get_mut(id).unwrap().provider;
            provider.insert("spec".into(), toml::Value::String(target.clone()));
            provider.insert("prefix".into(), toml::Value::String(prefix.into()));
        }
        let (sources, unavailable, pending) = startup::load(
            &deployment.config,
            &deployment.wallet_resolution.source_bindings,
            &path,
            startup::CatalogPurpose::Serving,
            BTreeMap::from([("a".into(), relay.clone()), ("b".into(), relay)]),
            true,
            false,
            deployment.catalog_capacity.clone(),
        )
        .await
        .unwrap();
        deployment.sources = sources;
        deployment.unavailable_sources = unavailable;
        deployment.pending_sources = pending;
        deployment.defer_discovery = true;
        for (name, server) in &deployment.listeners {
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
        let catalog = running.deferred.catalog.clone();
        (dir, running, catalog, fixture, relay_task, api_task)
    }

    #[tokio::test]
    async fn ordinary_serving_binds_before_slow_unsigned_origin_without_relay() {
        let dir = tempfile::tempdir().unwrap();
        let origin = crate::test_tls::listener().await;
        let base = format!("https://{}", origin.local_addr().unwrap());
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
        let catalog = running.deferred.catalog.clone();
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
        let origin = crate::test_tls::listener().await;
        let base = format!("https://{}", origin.local_addr().unwrap());
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
pricing_key='pricing'
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
        assert!(!deployment.config.sources["failed"].allow_catalog_failure);
        assert_eq!(deployment.sources.len(), 2);
        assert_eq!(deployment.pending_sources.len(), 1);
        let running = deployment.bind(&env).await.unwrap();
        assert_ne!(running.addresses()[0].1.port(), 0);
        let catalog = running.deferred.catalog.clone();
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
    async fn relay_cap_refusal_settles_required_and_optional_catalogs() {
        if public_target_fixture("relay_cap_refusal_settles_required_and_optional_catalogs").await {
            return;
        }
        use crate::{
            payment::SpendPolicy,
            rotation::{
                base::BaseRpc,
                manager::ManagedPool,
                store::{AllocationKind, Store, StoreHandle},
            },
        };
        let dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let reserves = crate::rotation::test_capacity::small_reserve("2");
        let bootstrap_cap = SpendPolicy::dollars("0.005").unwrap();
        let bootstrap = state
            .ensure_capacity_group("bootstrap", &reserves, bootstrap_cap.max_atomic, 0)
            .unwrap();
        let destination = state
            .ensure_capacity_group("destination", &reserves, None, 0)
            .unwrap();
        state
            .allocate_capacity_kind(&bootstrap, AllocationKind::Small, true)
            .unwrap();
        let job = state.funding_jobs().unwrap().remove(0);
        state
            .record_credit(&job.wallet_id, "1000000", "fixture", 100)
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let managed = |pool, policy| {
            Arc::new(
                ManagedPool::new(
                    store.clone(),
                    pool,
                    BaseRpc::new("http://127.0.0.1:1").unwrap(),
                    policy,
                )
                .unwrap(),
            )
        };
        let payer = PaidClient::managed(managed(bootstrap, bootstrap_cap));
        for mode in ["required", "optional", "mixed"] {
            let optional = mode == "optional";
            let (_dir, mut running, catalog, fixture, relay, api) =
                prepared(false, false, Some(payer.clone())).await;
            let readiness = running.catalog_readiness.remove("one").unwrap();
            let mut runtime = running.deferred;
            assert!(
                runtime.deployment.pending_sources["a"]
                    .shares_download(&runtime.deployment.pending_sources["b"])
            );
            for source in ["a", "b"] {
                runtime
                    .deployment
                    .config
                    .sources
                    .get_mut(source)
                    .unwrap()
                    .allow_catalog_failure = optional;
            }
            let mut required_readiness = None;
            if mode == "mixed" {
                runtime
                    .deployment
                    .config
                    .sources
                    .get_mut("b")
                    .unwrap()
                    .allow_catalog_failure = true;
                let mut listener = runtime.deployment.listeners["one"].clone();
                listener.sources = vec!["a".into()];
                runtime.deployment.listeners.insert("two".into(), listener);
                runtime.deployment.listeners.get_mut("one").unwrap().sources =
                    vec!["healthy".into(), "b".into()];
                runtime.deployment.wallet_resolution.bindings.insert(
                    "two".into(),
                    runtime.deployment.wallet_resolution.bindings["one"].clone(),
                );
                runtime.deployment.refresh_selection(true).unwrap();
                let (send, receive) = tokio::sync::oneshot::channel();
                runtime.readiness.insert("two".into(), send);
                required_readiness = Some(receive);
                let mut snapshot = (*catalog.read()).clone();
                snapshot.views.insert("two".into(), Vec::new());
                snapshot.instructions.insert("two".into(), None);
                catalog.publish(snapshot);
            }
            let stop = CancellationToken::new();
            let deferred = tokio::spawn(runtime.run(stop.clone()));
            let result = tokio::time::timeout(Duration::from_secs(5), readiness)
                .await
                .expect("a catalog cannot wait for its own listener")
                .unwrap();
            assert_eq!(result.is_ok(), mode != "required", "{mode}: {result:?}");
            if mode != "required" {
                let snapshot = catalog.read();
                assert_eq!(snapshot.views["one"].len(), 1);
                assert!(snapshot.views["one"][0].tool.name.starts_with("healthy_"));
                let instructions = snapshot.instructions["one"].as_deref().unwrap();
                if optional {
                    assert!(instructions.contains("Source a: UNAVAILABLE"));
                }
                assert!(instructions.contains("Source b: UNAVAILABLE"));
            }
            assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.signed.load(Ordering::SeqCst), 0);
            let untouched_destination = destination.clone();
            store
                .call(move |s| {
                    assert!(!s.has_bootstrap_demand(&untouched_destination)?);
                    assert_eq!(s.funding_jobs()?.len(), 1);
                    Ok(())
                })
                .await
                .unwrap();
            if let Some(required) = required_readiness {
                assert!(
                    tokio::time::timeout(Duration::from_secs(5), required)
                        .await
                        .unwrap()
                        .unwrap()
                        .is_err()
                );
            }
            stop.cancel();
            drop(deferred.await.unwrap());
            relay.abort();
            api.abort();
        }
        drop((payer, store));
        worker.await.unwrap();
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
            let listener = crate::test_tls::listener().await;
            let base = format!("https://{}", listener.local_addr().unwrap());
            let timestamp = crate::rotation::base::now().unwrap();
            let rpc = tokio::spawn(async move {
                axum::serve(
                    listener,
                    Router::new().route(
                        "/",
                        axum::routing::post(move |request| {
                            crate::discovery_relay::readiness_tests::rpc(timestamp, request)
                        }),
                    ),
                )
                .await
                .unwrap();
            });
            let manager = Arc::new(
                ManagedPool::new(
                    store.clone(),
                    pool.clone(),
                    BaseRpc::new(&base).unwrap(),
                    crate::payment::SpendPolicy::dollars("1").unwrap(),
                )
                .unwrap(),
            );
            let (_dir, mut running, catalog, fixture, relay, api) =
                prepared(false, false, Some(PaidClient::managed(manager.clone()))).await;
            for source in ["a", "b"] {
                running
                    .deferred
                    .deployment
                    .config
                    .sources
                    .get_mut(source)
                    .unwrap()
                    .allow_catalog_failure = true;
            }
            let relay_owner = running.deferred.relays()[0].clone();
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
    async fn relay_funding_obstructions_unblock_waiting_listeners_and_cancel_orphaned_required_sources()
     {
        if public_target_fixture(
            "relay_funding_obstructions_unblock_waiting_listeners_and_cancel_orphaned_required_sources",
        )
        .await
        {
            return;
        }
        use crate::rotation::{
            base::BaseRpc,
            error::FundingCondition,
            manager::ManagedPool,
            store::{AllocationKind, Store, StoreHandle},
        };
        for mode in [
            "required_shortfall",
            "optional_shortfall",
            "optional_paused",
            "eager_shortfall",
            "mixed_shortfall",
        ] {
            let state_dir = tempfile::tempdir().unwrap();
            let mut state = Store::create(
                &state_dir.path().join("state"),
                &state_dir.path().join("key"),
                1,
                b"fixture",
            )
            .unwrap();
            let reserves = crate::rotation::test_capacity::small_reserve("5");
            let pool = state
                .ensure_capacity_group(
                    "relay",
                    &reserves,
                    Some(alloy_primitives::U256::from(1_000_000u64)),
                    0,
                )
                .unwrap();
            state
                .allocate_capacity_kind(&pool, AllocationKind::Small, true)
                .unwrap()
                .unwrap();
            let job = state.funding_jobs().unwrap().remove(0);
            if mode == "optional_paused" {
                state.pause_new_funding();
            } else {
                state
                    .observe_funding_condition(
                        &job.id,
                        &job.operation_id,
                        Some(FundingCondition::InsufficientFunds),
                    )
                    .unwrap();
            }
            let (store, worker) = StoreHandle::spawn(state);
            let rpc_listener = crate::test_tls::listener().await;
            let rpc_base = format!("https://{}", rpc_listener.local_addr().unwrap());
            let timestamp = crate::rotation::base::now().unwrap();
            let rpc = tokio::spawn(async move {
                axum::serve(
                    rpc_listener,
                    Router::new().route(
                        "/",
                        axum::routing::post(move |request| {
                            crate::discovery_relay::readiness_tests::rpc(timestamp, request)
                        }),
                    ),
                )
                .await
                .unwrap();
            });
            let manager = Arc::new(
                ManagedPool::new(
                    store.clone(),
                    pool.clone(),
                    BaseRpc::new(&rpc_base).unwrap(),
                    crate::payment::SpendPolicy::dollars("1").unwrap(),
                )
                .unwrap(),
            );
            let (_dir, mut running, catalog, fixture, relay, api) =
                prepared(false, false, Some(PaidClient::managed(manager.clone()))).await;
            let readiness_one = running.catalog_readiness.remove("one").unwrap();
            let mut runtime = running.deferred;
            runtime.can_allocate = mode != "optional_paused";
            let optional = !matches!(mode, "required_shortfall" | "eager_shortfall");
            for source in ["a", "b"] {
                runtime
                    .deployment
                    .config
                    .sources
                    .get_mut(source)
                    .unwrap()
                    .allow_catalog_failure = optional;
            }
            if matches!(mode, "optional_shortfall" | "eager_shortfall") {
                // Verify selection diagnostics fire when the listener settles on shortfall.
                runtime
                    .deployment
                    .listeners
                    .get_mut("one")
                    .unwrap()
                    .include_tools = vec!["missing_exact_tool".into()];
            }
            if mode == "eager_shortfall" {
                runtime
                    .deployment
                    .listeners
                    .get_mut("one")
                    .unwrap()
                    .wait_for_catalogs = false;
                runtime.readiness.remove("one");
            }
            let mut readiness_two = None;
            if mode == "mixed_shortfall" {
                runtime
                    .deployment
                    .config
                    .sources
                    .get_mut("a")
                    .unwrap()
                    .allow_catalog_failure = true;
                runtime
                    .deployment
                    .config
                    .sources
                    .get_mut("b")
                    .unwrap()
                    .allow_catalog_failure = false;
                let mut listener = runtime.deployment.listeners["one"].clone();
                listener.sources = vec!["b".into()];
                runtime.deployment.listeners.insert("two".into(), listener);
                runtime.deployment.listeners.get_mut("one").unwrap().sources =
                    vec!["healthy".into(), "a".into()];
                runtime.deployment.wallet_resolution.bindings.insert(
                    "two".into(),
                    runtime.deployment.wallet_resolution.bindings["one"].clone(),
                );
                runtime.deployment.refresh_selection(true).unwrap();
                let (send, receive) = tokio::sync::oneshot::channel();
                runtime.readiness.insert("two".into(), send);
                readiness_two = Some(receive);
                let mut snapshot = (*catalog.read()).clone();
                snapshot.views.insert("two".into(), Vec::new());
                snapshot.instructions.insert("two".into(), None);
                catalog.publish(snapshot);
            }
            use tracing::instrument::WithSubscriber;
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(log.reopen().unwrap())
                .finish();
            let stop = CancellationToken::new();
            let task = tokio::spawn(runtime.run(stop.clone()).with_subscriber(subscriber));
            let one_result = if mode == "eager_shortfall" {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !catalog.read().instructions["one"]
                        .as_deref()
                        .is_some_and(|text| {
                            text.contains("Source a: BLOCKED") && text.contains("Source b: BLOCKED")
                        })
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                // Wait for report_completed_diagnostics right after publish().
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !std::fs::read_to_string(log.path())
                        .unwrap()
                        .contains("missing_exact_tool")
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                Ok(())
            } else {
                tokio::time::timeout(Duration::from_secs(5), readiness_one)
                    .await
                    .unwrap()
                    .unwrap()
            };
            let logs = std::fs::read_to_string(log.path()).unwrap();
            match mode {
                "required_shortfall" => {
                    assert!(one_result.is_err(), "{mode}: {one_result:?}");
                    assert_eq!(
                        logs.matches("relay_wallet_insufficient_zec").count(),
                        1,
                        "wallet-level shortfall WARN must be deduplicated across sources: {logs}"
                    );
                    assert!(logs.contains("catalog_unavailable"), "{logs}");
                    // Supply funds after the sole required listener already closed:
                    // the orphaned relay task must have been cancelled and must never pay.
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
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
                    assert_eq!(fixture.signed.load(Ordering::SeqCst), 0);
                }
                "optional_shortfall" => {
                    assert!(one_result.is_ok(), "{mode}: {one_result:?}");
                    assert_eq!(
                        logs.matches("relay_wallet_insufficient_zec").count(),
                        1,
                        "{logs}"
                    );
                    assert!(
                        logs.contains("missing_exact_tool"),
                        "unmatched include_tools diagnostic must fire when listener settles on shortfall: {logs}"
                    );
                    let instructions = catalog.read().instructions["one"].clone().unwrap();
                    assert!(instructions.contains("BLOCKED"), "{instructions}");
                    assert!(
                        instructions.contains("treasury ZEC is insufficient"),
                        "{instructions}"
                    );
                    // Unrelated capacity wakeups while still unfunded must NOT
                    // flap MCP instructions or re-emit catalog_unavailable / selection WARNs.
                    let before_spurious_wakeup = std::fs::read_to_string(log.path()).unwrap();
                    let pool_for_wake = pool.clone();
                    store
                        .call(move |s| {
                            s.capacity_changes(&pool_for_wake).notify_waiters();
                            Ok(())
                        })
                        .await
                        .unwrap();
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    let after_spurious_wakeup = std::fs::read_to_string(log.path()).unwrap();
                    assert_eq!(
                        after_spurious_wakeup.matches("catalog_unavailable").count(),
                        before_spurious_wakeup
                            .matches("catalog_unavailable")
                            .count(),
                        "spurious capacity wakeup must not re-emit catalog_unavailable: {after_spurious_wakeup}"
                    );
                    assert_eq!(
                        after_spurious_wakeup.matches("missing_exact_tool").count(),
                        before_spurious_wakeup.matches("missing_exact_tool").count(),
                        "spurious capacity wakeup must not re-emit selection WARN: {after_spurious_wakeup}"
                    );
                    assert!(
                        catalog.read().instructions["one"]
                            .as_deref()
                            .is_some_and(|text| text.contains("BLOCKED")),
                        "spurious capacity wakeup must not flap BLOCKED back to PENDING"
                    );
                    // Background recovery still loads the omitted optional source once funded.
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
                        while catalog.read().instructions["one"]
                            .as_deref()
                            .is_some_and(|text| {
                                text.contains("BLOCKED") || text.contains("PENDING")
                            })
                        {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
                }
                "optional_paused" => {
                    assert!(one_result.is_ok(), "{mode}: {one_result:?}");
                    assert_eq!(
                        logs.matches("relay_wallet_unavailable").count(),
                        1,
                        "{logs}"
                    );
                    assert!(logs.contains("funding_disabled"), "{logs}");
                    let instructions = catalog.read().instructions["one"].clone().unwrap();
                    assert!(
                        instructions.contains("automatic relay wallet funding is disabled"),
                        "{instructions}"
                    );
                }
                "eager_shortfall" => {
                    assert!(
                        logs.contains("missing_exact_tool"),
                        "wait_for_catalogs = false listener must still emit selection diagnostics on shortfall: {logs}"
                    );
                    assert_eq!(
                        logs.matches("relay_wallet_insufficient_zec").count(),
                        1,
                        "{logs}"
                    );
                }
                "mixed_shortfall" => {
                    assert!(one_result.is_ok(), "{mode}: {one_result:?}");
                    let two_result =
                        tokio::time::timeout(Duration::from_secs(5), readiness_two.unwrap())
                            .await
                            .unwrap()
                            .unwrap();
                    assert!(two_result.is_err(), "{mode}: {two_result:?}");
                    // Listener "one" still references source "a", so background recovery
                    // publishes "a" onto "one" once funds arrive.
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
                        while catalog.read().views["one"].len() < 2 {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
                }
                _ => unreachable!(),
            }
            stop.cancel();
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap();
            drop(manager);
            drop(store);
            worker.await.unwrap();
            rpc.abort();
            relay.abort();
            api.abort();
        }
    }

    #[tokio::test]
    async fn waiting_for_relay_funds_heartbeat_reports_and_clears_on_download_resume() {
        if public_target_fixture(
            "waiting_for_relay_funds_heartbeat_reports_and_clears_on_download_resume",
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
        let state_dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &state_dir.path().join("state"),
            &state_dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let reserves = crate::rotation::test_capacity::small_reserve("5");
        let pool = state
            .ensure_capacity_group(
                "relay",
                &reserves,
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        state
            .allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let rpc_listener = crate::test_tls::listener().await;
        let rpc_base = format!("https://{}", rpc_listener.local_addr().unwrap());
        let timestamp = crate::rotation::base::now().unwrap();
        let rpc = tokio::spawn(async move {
            axum::serve(
                rpc_listener,
                Router::new().route(
                    "/",
                    axum::routing::post(move |request| {
                        crate::discovery_relay::readiness_tests::rpc(timestamp, request)
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let manager = Arc::new(
            ManagedPool::new(
                store.clone(),
                pool.clone(),
                BaseRpc::new(&rpc_base).unwrap(),
                crate::payment::SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        );
        // Stall relay HTTP responses so we can observe waiting_for_relay_funds
        // clear to false once the wallet is funded and active download begins.
        let (_dir, running, _catalog, _fixture, relay_task, api) =
            prepared(true, false, Some(PaidClient::managed(manager.clone()))).await;
        let relay = running.deferred.relays()[0].clone();
        let waiting = crate::discovery_relay::tests::dependency_wait_signal(&relay).await;
        let (status_tx, mut status_rx) = tokio::sync::watch::channel(Default::default());
        let target = format!(
            "https://spec.example.com:{}/spec",
            crate::test_tls::listener()
                .await
                .local_addr()
                .unwrap()
                .port()
        );
        let fetch = tokio::spawn({
            let relay = relay.clone();
            async move {
                relay
                    .fetch_when_ready_bounded(
                        &target,
                        4096,
                        "max_spec_bytes",
                        None,
                        crate::discovery_relay::CatalogReadiness {
                            status: Some(status_tx),
                            can_allocate: true,
                            log_blocks: false,
                        },
                        None,
                    )
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(5), waiting.notified())
            .await
            .unwrap();
        let dep = status_rx.borrow_and_update().clone();
        assert!(dep.waiting_for_relay_funds, "{dep:?}");
        assert_eq!(dep.relay_wallet.as_deref(), Some("selected"));
        assert!(!dep.is_relay_blocked(), "{dep:?}");

        // Fund the wallet: fetch resumes into the stalled HTTP download and
        // immediately clears waiting_for_relay_funds.
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
        tokio::time::timeout(Duration::from_secs(5), status_rx.changed())
            .await
            .unwrap()
            .unwrap();
        let resumed = status_rx.borrow_and_update().clone();
        assert!(!resumed.waiting_for_relay_funds, "{resumed:?}");

        // Exercise report_waiting_catalogs directly to verify awaiting_relay_funds,
        // relay_wallets, and omission of blocked optional sources.
        let mut runtime = running.deferred;
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        runtime
            .deployment
            .pending_sources
            .get_mut("a")
            .unwrap()
            .dependency = crate::discovery_relay::CatalogDependency {
            waiting_for_relay_funds: true,
            relay_wallet: Some("selected".into()),
            ..Default::default()
        };
        runtime
            .deployment
            .config
            .sources
            .get_mut("b")
            .unwrap()
            .allow_catalog_failure = true;
        runtime
            .deployment
            .pending_sources
            .get_mut("b")
            .unwrap()
            .dependency = crate::discovery_relay::CatalogDependency {
            waiting_for_relay_funds: true,
            relay_funding_blocked_reason: Some("treasury_insufficient_spendable_funds"),
            relay_wallet: Some("blocked_wallet".into()),
            ..Default::default()
        };
        let gate = runtime.deployment.catalog_capacity.clone();
        tracing::dispatcher::with_default(&subscriber.into(), || {
            runtime.report_waiting_catalogs(&gate);
        });
        let heartbeat_logs = std::fs::read_to_string(log.path()).unwrap();
        assert!(
            heartbeat_logs.contains("catalog_startup_waiting"),
            "{heartbeat_logs}"
        );
        assert!(
            heartbeat_logs.contains("awaiting_relay_funds=\"a\""),
            "{heartbeat_logs}"
        );
        assert!(
            heartbeat_logs.contains("relay_wallets=\"selected\""),
            "{heartbeat_logs}"
        );
        assert!(
            !heartbeat_logs.contains("blocked_wallet"),
            "blocked optional sources must not appear in awaiting_relay_funds: {heartbeat_logs}"
        );

        fetch.abort();
        drop(relay);
        drop(runtime);
        drop(manager);
        drop(store);
        worker.await.unwrap();
        rpc.abort();
        relay_task.abort();
        api.abort();
    }

    #[tokio::test]
    async fn closed_required_listener_cancels_non_blocked_sibling_pending_source() {
        if public_target_fixture(
            "closed_required_listener_cancels_non_blocked_sibling_pending_source",
        )
        .await
        {
            return;
        }
        use crate::rotation::{
            base::BaseRpc,
            error::FundingCondition,
            manager::ManagedPool,
            store::{AllocationKind, Store, StoreHandle},
        };
        let state_dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &state_dir.path().join("state"),
            &state_dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let reserves = crate::rotation::test_capacity::small_reserve("5");
        let blocked_pool = state
            .ensure_capacity_group(
                "blocked",
                &reserves,
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        let waiting_pool = state
            .ensure_capacity_group(
                "waiting",
                &reserves,
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        let blocked_job_id = state
            .allocate_capacity_kind(&blocked_pool, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        state
            .allocate_capacity_kind(&waiting_pool, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let blocked_job = state.funding_job(&blocked_job_id).unwrap().unwrap();
        state
            .observe_funding_condition(
                &blocked_job.id,
                &blocked_job.operation_id,
                Some(FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let rpc_listener = crate::test_tls::listener().await;
        let rpc_base = format!("https://{}", rpc_listener.local_addr().unwrap());
        let timestamp = crate::rotation::base::now().unwrap();
        let rpc = tokio::spawn(async move {
            axum::serve(
                rpc_listener,
                Router::new().route(
                    "/",
                    axum::routing::post(move |request| {
                        crate::discovery_relay::readiness_tests::rpc(timestamp, request)
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let blocked_manager = Arc::new(
            ManagedPool::new(
                store.clone(),
                blocked_pool.clone(),
                BaseRpc::new(&rpc_base).unwrap(),
                crate::payment::SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        );
        let waiting_manager = Arc::new(
            ManagedPool::new(
                store.clone(),
                waiting_pool.clone(),
                BaseRpc::new(&rpc_base).unwrap(),
                crate::payment::SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        );
        let (_dir, mut running, _catalog, fixture_a, relay_a_task, api) = prepared(
            false,
            false,
            Some(PaidClient::managed(blocked_manager.clone())),
        )
        .await;
        // Rebind source "b" onto a distinct non-blocked relay ("waiting" pool)
        // so "b" is NOT relay-blocked when "a" closes listener "one".
        let target_b = format!(
            "https://b.example.com:{}/spec",
            crate::test_tls::listener()
                .await
                .local_addr()
                .unwrap()
                .port()
        );
        let envelope_b = json!({"url":target_b,"method":"GET","statusCode":200,"headers":{},"body":r#"{"operations":[{"method":"GET","path":"/read"}]}"#,"redirectChain":[],"error":null});
        let (mut relay_b, fixture_b, relay_b_task) =
            crate::discovery_relay::tests::fixture(envelope_b, false, false).await;
        crate::discovery_relay::tests::set_wallet(&mut relay_b, "waiting");
        crate::discovery_relay::tests::set_client(
            &mut relay_b,
            PaidClient::managed(waiting_manager.clone()),
        );
        let mut cfg_b = running.deferred.deployment.pending_sources["b"]
            .config
            .clone();
        cfg_b.spec = target_b;
        cfg_b.relay_fallback = Some(relay_b);
        running
            .deferred
            .deployment
            .pending_sources
            .get_mut("b")
            .unwrap()
            .replace_config_for_test(cfg_b);

        let readiness_one = running.catalog_readiness.remove("one").unwrap();
        let mut runtime = running.deferred;
        runtime.can_allocate = true;
        let stop = CancellationToken::new();
        let task = tokio::spawn(runtime.run(stop.clone()));
        let one_result = tokio::time::timeout(Duration::from_secs(5), readiness_one)
            .await
            .unwrap()
            .unwrap();
        assert!(one_result.is_err(), "{one_result:?}");

        // Now fund the non-blocked sibling's "waiting" pool: because listener
        // "one" already closed, sibling source "b" must have been cancelled and
        // must never invoke or pay its relay.
        let waiting_pool_id = waiting_pool.clone();
        store
            .call(move |s| {
                for job in s.funding_jobs()? {
                    if job.pool_id == waiting_pool_id {
                        s.record_funding_balance(
                            &job.id,
                            &job.operation_id,
                            &job.wallet_id,
                            &job.recipient,
                            alloy_primitives::U256::from(5_000_000),
                        )?;
                    }
                }
                Ok(())
            })
            .await
            .unwrap();
        waiting_manager.reconcile().await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(fixture_a.signed.load(Ordering::SeqCst), 0);
        assert_eq!(fixture_b.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture_b.signed.load(Ordering::SeqCst), 0);

        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        drop(blocked_manager);
        drop(waiting_manager);
        drop(store);
        worker.await.unwrap();
        rpc.abort();
        relay_a_task.abort();
        relay_b_task.abort();
        api.abort();
    }

    #[tokio::test]
    async fn initially_closed_listener_prunes_orphaned_pending_sources_before_spawning() {
        if public_target_fixture(
            "initially_closed_listener_prunes_orphaned_pending_sources_before_spawning",
        )
        .await
        {
            return;
        }
        let (_dir, mut running, _catalog, fixture, relay, api) = prepared(false, false, None).await;
        // Simulate a listener whose required non-deferred source already failed
        // before Runtime::run starts (initial signal_readiness closes it).
        running.deferred.deployment.sources.remove("healthy");
        running
            .deferred
            .deployment
            .unavailable_sources
            .insert("healthy".into(), "initial failure".into());
        let readiness_one = running.catalog_readiness.remove("one").unwrap();
        let stop = CancellationToken::new();
        let task = tokio::spawn(running.deferred.run(stop.clone()));
        let one_result = tokio::time::timeout(Duration::from_secs(5), readiness_one)
            .await
            .unwrap()
            .unwrap();
        assert!(one_result.is_err(), "{one_result:?}");
        tokio::time::sleep(Duration::from_millis(120)).await;
        // Pending sources "a" and "b" were pruned before spawning, so zero
        // relay requests or payments occurred.
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.signed.load(Ordering::SeqCst), 0);
        stop.cancel();
        let returned = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(returned.deployment.pending_sources.is_empty());
        relay.abort();
        api.abort();
    }

    #[tokio::test]
    async fn standalone_relay_logs_deduplicated_wallet_block_warn_without_status_receiver() {
        if public_target_fixture(
            "standalone_relay_logs_deduplicated_wallet_block_warn_without_status_receiver",
        )
        .await
        {
            return;
        }
        use crate::rotation::{
            base::BaseRpc,
            error::FundingCondition,
            manager::ManagedPool,
            store::{AllocationKind, Store, StoreHandle},
        };
        use tracing::instrument::WithSubscriber;
        let state_dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &state_dir.path().join("state"),
            &state_dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let reserves = crate::rotation::test_capacity::small_reserve("5");
        let pool = state
            .ensure_capacity_group(
                "relay",
                &reserves,
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        let job_id = state
            .allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let job = state.funding_job(&job_id).unwrap().unwrap();
        state
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let rpc_listener = crate::test_tls::listener().await;
        let rpc_base = format!("https://{}", rpc_listener.local_addr().unwrap());
        let timestamp = crate::rotation::base::now().unwrap();
        let rpc = tokio::spawn(async move {
            axum::serve(
                rpc_listener,
                Router::new().route(
                    "/",
                    axum::routing::post(move |request| {
                        crate::discovery_relay::readiness_tests::rpc(timestamp, request)
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let manager = Arc::new(
            ManagedPool::new(
                store.clone(),
                pool.clone(),
                BaseRpc::new(&rpc_base).unwrap(),
                crate::payment::SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        );
        let (_dir, running, _catalog, _fixture, relay_task, api) =
            prepared(false, false, Some(PaidClient::managed(manager.clone()))).await;
        let relay = running.deferred.relays()[0].clone();
        let waiting = crate::discovery_relay::tests::dependency_wait_signal(&relay).await;
        let target = format!(
            "https://standalone.example.com:{}/spec",
            crate::test_tls::listener()
                .await
                .local_addr()
                .unwrap()
                .port()
        );
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let fetch = tokio::spawn(
            {
                let relay = relay.clone();
                async move {
                    relay
                        .fetch_when_ready_bounded(
                            &target,
                            4096,
                            "max_spec_bytes",
                            None,
                            crate::discovery_relay::CatalogReadiness {
                                status: None,
                                can_allocate: true,
                                log_blocks: true,
                            },
                            None,
                        )
                        .await
                }
            }
            .with_subscriber(subscriber),
        );
        tokio::time::timeout(Duration::from_secs(5), waiting.notified())
            .await
            .unwrap();
        // Wake capacity once more while still unfunded: standalone WARN must not duplicate.
        let pool_for_wake = pool.clone();
        store
            .call(move |s| {
                s.capacity_changes(&pool_for_wake).notify_waiters();
                Ok(())
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), waiting.notified())
            .await
            .unwrap();
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            logs.matches("relay_wallet_insufficient_zec").count(),
            1,
            "{logs}"
        );
        fetch.abort();
        drop(relay);
        drop(running);
        drop(manager);
        drop(store);
        worker.await.unwrap();
        rpc.abort();
        relay_task.abort();
        api.abort();
    }

    #[tokio::test]
    async fn relay_wallet_block_re_warns_after_recovery_episode() {
        if public_target_fixture("relay_wallet_block_re_warns_after_recovery_episode").await {
            return;
        }
        use crate::rotation::{
            base::BaseRpc,
            error::FundingCondition,
            manager::ManagedPool,
            store::{AllocationKind, Store, StoreHandle},
        };
        use tracing::instrument::WithSubscriber;
        let state_dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &state_dir.path().join("state"),
            &state_dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let reserves = crate::rotation::test_capacity::small_reserve("5");
        let pool = state
            .ensure_capacity_group(
                "relay",
                &reserves,
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        let job_id = state
            .allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let job = state
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|entry| entry.id == job_id)
            .unwrap();
        state
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let rpc_listener = crate::test_tls::listener().await;
        let rpc_base = format!("https://{}", rpc_listener.local_addr().unwrap());
        let timestamp = crate::rotation::base::now().unwrap();
        let rpc = tokio::spawn(async move {
            axum::serve(
                rpc_listener,
                Router::new().route(
                    "/",
                    axum::routing::post(move |request| {
                        crate::discovery_relay::readiness_tests::rpc(timestamp, request)
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let manager = Arc::new(
            ManagedPool::new(
                store.clone(),
                pool.clone(),
                BaseRpc::new(&rpc_base).unwrap(),
                crate::payment::SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        );
        let (_dir, mut running, catalog, _fixture, relay_task, api) =
            prepared(false, false, Some(PaidClient::managed(manager.clone()))).await;
        // Gate source "b"'s origin fetch behind a Notify so "b" stays out of
        // the relay until after "a" recovers, then falls back to the same relay
        // wallet in a second shortfall episode.
        let release_b = Arc::new(tokio::sync::Notify::new());
        let origin_b_gate = release_b.clone();
        let (origin_b_addr, origin_b) = crate::test_tls::serve(Router::new().route(
            "/spec-b",
            get(move || {
                let gate = origin_b_gate.clone();
                async move {
                    gate.notified().await;
                    axum::http::StatusCode::FORBIDDEN
                }
            }),
        ))
        .await;
        let origin_b_url = format!("https://spec.example.com:{}/spec-b", origin_b_addr.port());
        let mut cfg_b = running.deferred.deployment.pending_sources["b"]
            .config
            .clone();
        cfg_b.spec = origin_b_url;
        running
            .deferred
            .deployment
            .pending_sources
            .get_mut("b")
            .unwrap()
            .replace_config_for_test(cfg_b);

        running.deferred.can_allocate = true;
        for source in ["a", "b"] {
            running
                .deferred
                .deployment
                .config
                .sources
                .get_mut(source)
                .unwrap()
                .allow_catalog_failure = true;
        }
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let stop = CancellationToken::new();
        let task = tokio::spawn(
            running
                .deferred
                .run(stop.clone())
                .with_subscriber(subscriber),
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let text = catalog.read().instructions["one"]
                    .clone()
                    .unwrap_or_default();
                if text.contains("Source a: BLOCKED") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            logs.matches("relay_wallet_insufficient_zec").count(),
            1,
            "{logs}"
        );

        // 1. A transient shortfall clear while still unfunded (for example, an
        //    outgoing spendability change before FundingWorker re-assesses)
        //    must NOT duplicate the wallet-level WARN when shortfall recurs.
        let clear_job = job.id.clone();
        let clear_op = job.operation_id.clone();
        store
            .call(move |s| s.observe_funding_condition(&clear_job, &clear_op, None))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let text = catalog.read().instructions["one"]
                    .clone()
                    .unwrap_or_default();
                if text.contains("Source a: PENDING") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let retry_job = job.id.clone();
        let retry_op = job.operation_id.clone();
        store
            .call(move |s| {
                s.observe_funding_condition(
                    &retry_job,
                    &retry_op,
                    Some(FundingCondition::InsufficientFunds),
                )
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let text = catalog.read().instructions["one"]
                    .clone()
                    .unwrap_or_default();
                if text.contains("Source a: BLOCKED") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            logs.matches("relay_wallet_insufficient_zec").count(),
            1,
            "transient clear while still unfunded should not duplicate wallet WARN: {logs}"
        );

        // 2. Actual wallet recovery (a funded wallet clears waiting_for_relay_funds
        //    and loads source "a") resets deduplication so a later shortfall
        //    episode on source "b" emits a fresh wallet-level WARN.
        let wallet_id = job.wallet_id.clone();
        let pool_for_credit = pool.clone();
        store
            .call(move |s| {
                s.record_credit(&wallet_id, "5000000", "fixture", 101)?;
                s.capacity_changes(&pool_for_credit).notify_waiters();
                Ok(())
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while catalog.read().views["one"].len() < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        // Retire the funded wallet and allocate a fresh unfunded job with
        // InsufficientFunds, then release source "b" to enter relay fallback.
        let pool_for_second = pool.clone();
        let first_wallet = job.wallet_id.clone();
        store
            .call(move |s| {
                s.retire_wallet_for_test(&first_wallet)?;
                let second_id = s
                    .allocate_capacity_kind(&pool_for_second, AllocationKind::Small, true)?
                    .expect("second allocation");
                let second = s.funding_job(&second_id)?.expect("second job");
                s.observe_funding_condition(
                    &second.id,
                    &second.operation_id,
                    Some(FundingCondition::InsufficientFunds),
                )
            })
            .await
            .unwrap();
        release_b.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let text = catalog.read().instructions["one"]
                    .clone()
                    .unwrap_or_default();
                if text.contains("Source b: BLOCKED") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            logs.matches("relay_wallet_insufficient_zec").count(),
            2,
            "second shortfall episode after recovery must re-emit wallet WARN: {logs}"
        );

        stop.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        origin_b.abort();
        drop(manager);
        drop(store);
        worker.await.unwrap();
        rpc.abort();
        relay_task.abort();
        api.abort();
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
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(log.reopen().unwrap())
                .finish();
            let _logging = tracing::subscriber::set_default(subscriber);
            let (_dir, mut running, catalog, fixture, relay, api) =
                prepared(false, invalid_first, None).await;
            for source in ["a", "b"] {
                running
                    .deferred
                    .deployment
                    .config
                    .sources
                    .get_mut(source)
                    .unwrap()
                    .allow_catalog_failure = true;
            }
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
            let logs = std::fs::read_to_string(log.path()).unwrap();
            assert!(logs.contains("failure_stage=\"selection\""), "{logs}");
            assert!(
                logs.contains("Static tools use a reserved management namespace"),
                "{logs}"
            );
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
        let relay = running.deferred.relays[0].clone();
        let (fail, failed) = tokio::sync::oneshot::channel();
        running.deferred.failure = Some(failed);
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
        let runtime = &mut running.deferred;
        let mut config = runtime.deployment.sources["healthy"].config.clone();
        config.pricing_key = Some("pricing".into());
        let authored = Source::from_document(
            "healthy",
            config,
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
            &runtime.deployment.listeners["one"],
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

#[cfg(test)]
mod managed_refresh_tests;
