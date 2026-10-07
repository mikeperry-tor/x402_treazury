//! Explicit unsigned discovery in a dedicated CLI process. No Deployment or payer
//! is returned, so the direct-warming exception cannot become a serving policy.
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
    pub pricing: Option<crate::pricing::Evidence>,
    pub fresh_pricing_entries: usize,
}

/// Warm only explicitly selected authored sources (all if selection is empty).
/// Direct fetching is permitted only by this explicit command, never as fallback.
pub async fn warm_cache(
    path: &Path,
    selected: &[String],
    direct: bool,
    discover_pricing: bool,
) -> Result<CacheWarmSummary> {
    ensure!(
        !crate::qualification::active() && !catalog_evidence::collecting(),
        "cache warming is not a qualification operation"
    );
    let mut config: MetaConfig =
        toml::from_str(&tokio::fs::read_to_string(path).await?).context("invalid meta-config")?;
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
            "Explicit direct cache warming: selected unsigned catalogs/pricing bypass the configured network; fresh entries may be reused by that policy"
        );
    }
    let sources = startup::warm(&config, path, &selected, direct, discover_pricing).await?;
    Ok(CacheWarmSummary { direct, sources })
}

/// Preserve listener tag/name restrictions without downloading unrelated sources.
/// Unknown exact selectors belonging to another source are validated by normal
/// deployment startup, not inferred from this deliberately partial inventory.
pub(super) fn selected_tools(
    config: &MetaConfig,
    id: &str,
    source: &Source,
) -> Result<Vec<ToolSpec>> {
    let operations = catalog::operations(&source.document, None)?;
    let mut selected = BTreeMap::new();
    for server in config
        .servers
        .values()
        .filter(|s| s.sources.iter().any(|s| s == id))
    {
        let include = server
            .include_tools
            .iter()
            .map(|s| pattern(s))
            .collect::<Result<Vec<_>>>()?;
        let exclude = server
            .exclude_tools
            .iter()
            .map(|s| pattern(s))
            .collect::<Result<Vec<_>>>()?;
        for tool in &source.tools {
            if catalog::matches_operation_tags(
                &operations,
                tool,
                &server.tags,
                &server.exclude_tags,
            ) && (include.is_empty() || include.iter().any(|p| p.is_match(&tool.name)))
                && !exclude.iter().any(|p| p.is_match(&tool.name))
            {
                selected.insert(tool.name.clone(), tool.clone());
            }
        }
    }
    Ok(selected.into_values().collect())
}
