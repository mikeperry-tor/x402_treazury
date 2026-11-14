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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CacheWarmFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<CacheWarmPricing>,
}
impl CacheWarmSource {
    pub(super) fn degraded(&self) -> bool {
        self.status != "ready"
            || !self.selection_notices.is_empty()
            || matches!(
                self.catalog_cache,
                Some("not_stored" | "requires_revalidation")
            )
            || self.pricing.as_ref().is_some_and(|p| {
                p.status == "unavailable"
                    || matches!(
                        p.catalog_cache,
                        Some("not_stored" | "requires_revalidation")
                    )
            })
    }
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
    config.validate()?;
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
    // Preflight every selected provider and opt-out before any outbound request.
    for id in &selected {
        let source = config
            .sources
            .get(id)
            .with_context(|| format!("unknown cache-warming source {id}"))?;
        let cfg = crate::config::resolve(source.provider.clone(), path)
            .await
            .with_context(|| format!("source {id}"))?
            .settings;
        crate::catalog::validate_tool_selection(&cfg)
            .with_context(|| format!("source {id}: invalid tool selections"))?;
        ensure!(
            !cfg.catalog_cache_ttl_seconds.disabled(),
            "source {id}: cache warming is disabled by catalog_cache_ttl_seconds=0"
        );
        ensure!(
            cfg.http_cache_enabled,
            "source {id}: cache warming is disabled by http_cache_enabled=false"
        );
    }
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
            sources = selected.len(),
            "Explicit direct cache warming: selected unsigned catalogs bypass the configured network; fresh entries may be reused by that policy"
        );
    }
    // Direct warming retains its unsigned behavior. Relay warming uses only the
    // resolved wallets and never starts treasury sync, funding workers or listeners.
    let policy = config.relay_fallback.clone().filter(|p| {
        p.provider_metadata
            && p.has_method("GET")
            && !direct
            && (p.sources.is_empty() || selected.iter().any(|id| p.sources.contains(id)))
    });
    let mut owner = None;
    let relay = if let Some(policy) = policy {
        crate::discovery_relay::SearchRelay::load(&policy, path).await?;
        if let Some(funding) = &mut config.funding {
            funding.auto_fund = false;
        }
        tracing::info!(
            "Catalog warming uses existing wallets; new wallet funding is disabled for this command; the deployment configuration is unchanged"
        );
        let deployment = Deployment {
            direct_history: None,
            catalog_capacity: std::sync::Arc::new(crate::network::DiscoveryCapacity::new(
                config.startup.catalog_concurrency,
            )),
            listeners: EffectiveBindings::resolve(&config)?.listeners(&config),
            search_relay: None,
            prebound: BTreeMap::new(),
            pending_sources: BTreeMap::new(),
            defer_discovery: false,
            unavailable_sources: Default::default(),
            wallet_resolution: config.validate()?,
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
            selected.iter().cloned(),
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
        config = deployment.config;
        owner = Some(initialized);
        relay
    } else {
        Default::default()
    };
    let stop = CancellationToken::new();
    let _cancel_on_drop = stop.clone().drop_guard();
    let path = path.to_owned();
    let stopped = stop.clone();
    // Retain accepted relay ownership if the CLI waiter disappears.
    let task = tokio::spawn(async move {
        let (result, cancelled) = {
            tokio::select! {
                result = startup::warm(&config, &path, &selected, direct, relay.clone()) => (result, false),
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
        let sources = match result {
            Err(error) => return Err(error),
            Ok(sources) => {
                cleanup?;
                sources
            }
        };
        let outcome = if sources.iter().any(CacheWarmSource::degraded) {
            "degraded"
        } else {
            "ok"
        };
        tracing::info!(sources = sources.len(), outcome, "Catalog warming finished");
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
