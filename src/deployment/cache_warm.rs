//! Explicit discovery in a dedicated CLI process. Direct warming is unsigned;
//! configured relay warming owns only its resolved wallets and starts no workers.
use super::*;

#[derive(Serialize)]
pub struct CacheWarmSummary {
    pub direct: bool,
    pub outcome: &'static str,
    pub sources: Vec<CacheWarmSource>,
}
#[derive(Serialize)]
pub struct CacheWarmSource {
    pub source: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_cache: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub omissions: Vec<crate::catalog::OperationIssue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub selection_notices: Vec<crate::catalog::SelectionNotice>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub override_issues: Vec<crate::catalog::OverrideIssue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CacheWarmFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<CacheWarmPricing>,
}
impl CacheWarmSource {
    pub(super) fn degraded(&self) -> bool {
        self.status != "ready"
            || incomplete_cache(self.catalog_cache)
            || self.pricing.as_ref().is_some_and(|pricing| {
                pricing.failure.is_some() || incomplete_cache(pricing.catalog_cache)
            })
    }

    pub(super) fn report(&self, completed: usize, total: usize) {
        let source = self.source.as_str();
        let status = self.status;
        let category = "catalog_warm_source";
        if let Some(failure) = &self.failure {
            tracing::error!(target: "x402_treazury::startup", category, source, status, completed, total, degraded = true,
                failure_stage = failure.stage, reason = failure.reason, http_status = failure.http_status,
                "Catalog warming could not prepare this source's API tools. {}", failure.message);
        } else if self.degraded() {
            let details = self.degradation_details();
            let pricing_failure = self
                .pricing
                .as_ref()
                .and_then(|pricing| pricing.failure.as_ref());
            tracing::warn!(target: "x402_treazury::startup", category, source, status, completed, total, degraded = true,
                catalog_cache = self.catalog_cache,
                pricing_cache = self.pricing.as_ref().and_then(|pricing| pricing.catalog_cache),
                pricing_failure_stage = pricing_failure.map(|failure| failure.stage),
                pricing_failure_reason = pricing_failure.map(|failure| failure.reason),
                pricing_http_status = pricing_failure.and_then(|failure| failure.http_status),
                omitted_operations = self.omissions.len(),
                missing_selections = self.selection_notices.len(),
                discarded_overrides = self.override_issues.len(),
                "Catalog warming finished. {details}");
        } else {
            tracing::info!(target: "x402_treazury::startup", category, source, status, completed, total, degraded = false,
                "Catalog warming source finished");
        }
    }

    fn degradation_details(&self) -> String {
        let mut details = Vec::new();
        if self.status == "unavailable" {
            details.push("The catalog has no selected API tools".to_owned());
        }
        if !self.omissions.is_empty() {
            let reasons: BTreeSet<_> = self
                .omissions
                .iter()
                .map(|issue| issue.reason.explanation())
                .collect();
            details.push(format!(
                "Some API tools were omitted ({})",
                reasons.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        if !self.selection_notices.is_empty() {
            details.push("Some configured tool selections matched no tools".to_owned());
        }
        for issue in &self.override_issues {
            details.push(format!("The catalog ignored an override ({issue})"));
        }
        if incomplete_cache(self.catalog_cache) {
            details.push("The catalog cache has no fresh entry".to_owned());
        }
        if let Some(pricing) = &self.pricing {
            if let Some(failure) = &pricing.failure {
                details.push(format!(
                    "Advertised pricing is unavailable. {}",
                    failure.message
                ));
            }
            if incomplete_cache(pricing.catalog_cache) {
                details.push("The pricing cache has no fresh entry".to_owned());
            }
        }
        details.join(". ")
    }
}

fn incomplete_cache(status: Option<&str>) -> bool {
    matches!(status, Some("not_stored" | "requires_revalidation"))
}
#[derive(Serialize)]
pub struct CacheWarmPricing {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_cache: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CacheWarmFailure>,
}
#[derive(Serialize)]
pub struct CacheWarmFailure {
    pub stage: &'static str,
    pub reason: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}
impl From<crate::catalog::diagnostics::Failure> for CacheWarmFailure {
    fn from(failure: crate::catalog::diagnostics::Failure) -> Self {
        let reason = failure.code();
        let message = if reason == failure.reason {
            match reason {
                "pricing_document_invalid" => {
                    "Advertised pricing metadata has an invalid format; API tools remain usable with challenge-based prices"
                }
                "pricing_inventory_unavailable" => {
                    "Advertised prices could not be matched to API operations; API tools remain usable with challenge-based prices"
                }
                _ => {
                    "The catalog document could not be accepted; inspect provider configuration and strict catalog inspection"
                }
            }
        } else {
            failure.reason
        };
        Self {
            stage: failure.stage,
            reason,
            message,
            http_status: failure.http_status,
        }
    }
}

/// Warm only explicitly selected authored sources (all if selection is empty).
/// Direct fetching is permitted only by this explicit command, never as fallback.
pub async fn warm_cache(
    path: &Path,
    selected: &[String],
    direct: bool,
) -> Result<CacheWarmSummary> {
    warm_cache_with_network(path, selected, direct, None).await
}
pub async fn warm_cache_with_network(
    path: &Path,
    selected: &[String],
    direct: bool,
    network_override: Option<&crate::network::NetworkConfig>,
) -> Result<CacheWarmSummary> {
    ensure!(
        !crate::qualification::active() && !catalog_evidence::collecting(),
        "cache warming is not a qualification operation"
    );
    let mut config = MetaConfig::parse_with_network(
        &tokio::fs::read_to_string(path).await.with_context(|| {
            format!("failed to read deployment config file '{}'", path.display())
        })?,
        network_override,
    )?;
    crate::discovery::policy::resolve(&mut config, path);
    let wallet_resolution = config.validate()?;
    if let Some(treasury) = &mut config.treasury {
        treasury.resolve(path);
    }
    crate::discovery::policy::validate_registry_path(&config, path)?;
    let treasury = config
        .treasury
        .as_ref()
        .context("cache warming requires treasury.state_dir")?;
    ensure!(
        treasury.state_dir.is_dir(),
        "cache warming requires an existing treasury state directory; initialize it with wallet init first"
    );
    let selected: BTreeSet<_> = if selected.is_empty() {
        config.sources.keys().cloned().collect()
    } else {
        selected.iter().cloned().collect()
    };
    ensure!(
        !selected.is_empty(),
        "no sources selected for cache warming"
    );
    let sources = resolve_sources(&config, path, selected).await?;
    let actual_policy = if direct {
        crate::network::NetworkPolicy {
            trusted_ca_files: config.network.trusted_ca_files.clone(),
            ..Default::default()
        }
    } else {
        config.network.clone()
    };
    crate::network::install(actual_policy)?;
    if direct {
        tracing::warn!(
            sources = sources.len(),
            "Explicit direct cache warming: selected unsigned catalogs bypass the configured network; fresh entries may be reused by that policy"
        );
    }
    // Direct warming retains its unsigned behavior. Relay warming uses only the
    // resolved wallets and never starts treasury sync, funding workers or listeners.
    let policy = config.relay_fallback.clone().filter(|p| {
        p.provider_metadata
            && p.has_method("GET")
            && !direct
            && (p.sources.is_empty() || sources.keys().any(|id| p.sources.contains(id)))
    });
    let (relay, owner) = if let Some(policy) = policy {
        crate::discovery_relay::SearchRelay::load(&policy, path).await?;
        if let Some(funding) = &mut config.funding {
            funding.auto_fund = false;
        }
        let deployment = Deployment {
            direct_history: None,
            catalog_capacity: std::sync::Arc::new(crate::network::DiscoveryCapacity::new(
                config.startup.catalog_concurrency,
            )),
            listeners: wallet_resolution.source_bindings.listeners(&config),
            search_relay: None,
            prebound: BTreeMap::new(),
            pending_sources: BTreeMap::new(),
            defer_discovery: false,
            unavailable_sources: Default::default(),
            wallet_resolution,
            config,
            initialized: None,
            sources: BTreeMap::new(),
            selected: BTreeMap::new(),
            conflicting_tools: BTreeMap::new(),
            selection_errors: BTreeMap::new(),
            config_path: path.to_owned(),
        };
        let bindings = crate::discovery_relay::resolve_wallets(
            &policy,
            &deployment.config,
            &deployment.wallet_resolution,
            sources.keys().cloned(),
        )?;
        let needed: BTreeSet<_> = bindings.values().cloned().collect();
        let initialized = deployment
            .initialize_wallets(
                &std::env::vars().collect(),
                crate::rotation::restriction::FundingRestriction::DenyNewFunding,
                Some(&needed),
                &needed,
            )
            .await?;
        let relay =
            crate::discovery_relay::build(&policy, path, &bindings, &initialized.wallets).await?;
        tracing::info!(
            "Catalog warming uses existing relay wallets. Automatic funding is disabled for this command."
        );
        config = deployment.config;
        (relay, Some(initialized))
    } else {
        (Default::default(), None)
    };
    let stop = CancellationToken::new();
    let _cancel_on_drop = stop.clone().drop_guard();
    let stopped = stop.clone();
    // Retain accepted relay ownership if the CLI waiter disappears.
    let task = tokio::spawn(async move {
        let (result, cancelled) = {
            tokio::select! {
                result = startup::warm(&config, sources, direct, relay.clone()) => (result, false),
                _ = stopped.cancelled() => {
                    tracing::info!("Catalog warming stopped; waiting for accepted relay work to finish");
                    (Err(anyhow::anyhow!("catalog warming cancelled")), true)
                },
            }
        };
        // Dropping unsigned acquisition futures above cannot orphan accepted paid I/O.
        futures_util::future::join_all(relay.values().map(|relay| relay.drain())).await;
        drop(relay);
        let cleanup = if let Some(owner) = owner {
            owner.shutdown().await
        } else {
            Ok(())
        };
        if cancelled && cleanup.is_ok() {
            tracing::info!("Catalog warming stopped; accepted relay work has finished");
        }
        let sources = result?;
        cleanup?;
        let outcome = if sources.iter().any(CacheWarmSource::degraded) {
            "degraded"
        } else {
            "ok"
        };
        tracing::info!(
            category = "catalog_warm_complete",
            sources = sources.len(),
            outcome,
            "Catalog warming finished"
        );
        Ok(CacheWarmSummary {
            direct,
            outcome,
            sources,
        })
    });
    crate::wallet_cli::finish_on_shutdown("catalog warming", &stop, async {
        task.await.context("catalog warming supervisor failed")?
    })
    .await
}

pub(super) async fn resolve_sources(
    config: &MetaConfig,
    path: &Path,
    selected: BTreeSet<String>,
) -> Result<BTreeMap<String, Config>> {
    let mut sources = BTreeMap::new();
    for id in selected {
        let source = config
            .sources
            .get(&id)
            .with_context(|| format!("unknown cache-warming source {id}"))?;
        let cfg = crate::config::resolve(source.provider.clone(), path)
            .await
            .with_context(|| format!("source {id}"))?
            .settings;
        cfg.check_override_container()
            .with_context(|| format!("source {id}: invalid tool overrides"))?;
        ensure!(
            !cfg.catalog_cache_ttl_seconds.disabled(),
            "source {id}: cache warming is disabled by catalog_cache_ttl_seconds=0"
        );
        ensure!(
            cfg.http_cache_enabled,
            "source {id}: cache warming is disabled by http_cache_enabled=false"
        );
        sources.insert(id, cfg);
    }
    Ok(sources)
}
