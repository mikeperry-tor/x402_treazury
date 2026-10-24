//! Explicit discovery in a dedicated CLI process. Direct warming is unsigned;
//! configured relay warming owns only its resolved wallets and starts no workers.
use super::*;

#[derive(Serialize)]
pub struct CacheWarmSummary {
    pub direct: bool,
    pub sources: Vec<CacheWarmSource>,
}
#[derive(Serialize)]
pub struct CacheWarmSource {
    pub source: String,
    pub catalog_cache: &'static str,
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
    let mut config =
        MetaConfig::parse_with_network(&tokio::fs::read_to_string(path).await?, network_override)?;
    if let Some(policy) = &mut config.source_management {
        policy.resolve(path);
    }
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
            .await?
            .settings;
        ensure!(
            cfg.catalog_cache_ttl_seconds > 0,
            "source {id}: cache warming is disabled by catalog_cache_ttl_seconds=0"
        );
        ensure!(
            cfg.http_cache_enabled,
            "source {id}: cache warming is disabled by http_cache_enabled=false"
        );
    }
    let actual_policy = if direct {
        crate::network::NetworkPolicy::default()
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
        let deployment = Deployment {
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
            config_path: path.to_owned(),
        };
        let bindings = crate::discovery_relay::resolve_wallets(
            &policy,
            &deployment.config,
            &deployment.wallet_resolution,
            selected.iter().cloned(),
        )?;
        let needed = bindings.values().cloned().collect();
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
    let sources = startup::warm(&config, path, &selected, direct, relay).await?;
    drop(owner);
    Ok(CacheWarmSummary { direct, sources })
}
