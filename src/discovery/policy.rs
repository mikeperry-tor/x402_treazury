use crate::deployment::MetaConfig;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub wallet: String,
    pub registry_file: Option<PathBuf>,
    #[serde(default = "sources")]
    pub max_sources: usize,
    #[serde(default = "per_source")]
    pub max_tools_per_source: usize,
    #[serde(default = "per_server")]
    pub max_tools_per_server: usize,
    #[serde(default = "bytes")]
    pub max_spec_bytes: usize,
    #[serde(default = "crate::limits::response_default")]
    pub max_response_bytes: usize,
    #[serde(default = "crate::limits::help_default")]
    pub max_help_bytes: usize,
    #[serde(default = "timeout")]
    pub fetch_timeout_seconds: u64,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}
fn sources() -> usize {
    16
}
fn per_source() -> usize {
    100
}
fn per_server() -> usize {
    200
}
fn bytes() -> usize {
    33554432
}
fn timeout() -> u64 {
    30
}
fn owned() -> usize {
    8
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Grant {
    pub enabled: bool,
    pub accept_sources: bool,
    pub allowed_targets: Option<Vec<String>>,
    pub allow_process_scope: bool,
    pub allow_persistence: bool,
    #[serde(default = "owned")]
    pub max_owned_sources: usize,
    pub wallet: Option<String>,
}
impl Policy {
    pub fn resolve(&mut self, path: &Path) {
        if let Some(file) = &mut self.registry_file
            && file.is_relative()
        {
            *file = path.parent().unwrap_or(Path::new(".")).join(&*file);
        }
    }
}
pub fn grant(config: &MetaConfig, id: &str) -> Grant {
    config.servers[id]
        .source_management
        .clone()
        .unwrap_or(Grant {
            max_owned_sources: 8,
            ..Default::default()
        })
}
pub fn wallet<'a>(policy: &'a Policy, g: &'a Grant) -> &'a str {
    g.wallet.as_deref().unwrap_or(&policy.wallet)
}
pub fn validate(config: &MetaConfig) -> Result<()> {
    if let Some(p) = &config.source_management {
        ensure!(
            config.wallets.contains_key(&p.wallet),
            "unknown source_management wallet"
        );
        ensure!(
            (1..=1024).contains(&p.max_sources)
                && (1..=10000).contains(&p.max_tools_per_source)
                && (1..=10000).contains(&p.max_tools_per_server),
            "invalid source management quota"
        );
        ensure!(
            (1024..=67108864).contains(&p.max_spec_bytes)
                && (1..=300).contains(&p.fetch_timeout_seconds),
            "invalid import limits"
        );
        ensure!(
            p.max_response_bytes > 0 && p.max_help_bytes > 0,
            "dynamic download limits must be positive"
        );
        for origin in &p.allowed_origins {
            let u = crate::network::public_url(origin)?;
            ensure!(
                u.origin().ascii_serialization() == *origin,
                "allowed_origins must be exact canonical HTTPS origins"
            );
        }
    }
    for id in config.servers.keys() {
        let g = grant(config, id);
        if config.servers[id].source_management.is_none() {
            continue;
        }
        let p = config
            .source_management
            .as_ref()
            .context("listener source_management requires deployment policy")?;
        ensure!(
            (1..=1024).contains(&g.max_owned_sources),
            "invalid max_owned_sources"
        );
        ensure!(
            !g.enabled || g.accept_sources,
            "source management writers must accept sources"
        );
        ensure!(
            config.wallets.contains_key(wallet(p, &g)),
            "unknown dynamic wallet override"
        );
        ensure!(
            !g.allow_persistence || (g.enabled && p.registry_file.is_some()),
            "persistent grants require registry_file and enabled management"
        );
        if let Some(targets) = &g.allowed_targets {
            for t in targets {
                ensure!(config.servers.contains_key(t), "unknown allowed target");
            }
        }
    }
    Ok(())
}
pub fn wallets(config: &MetaConfig) -> BTreeSet<String> {
    let Some(p) = &config.source_management else {
        return BTreeSet::new();
    };
    config
        .servers
        .keys()
        .filter_map(|id| {
            let g = grant(config, id);
            (g.accept_sources || g.enabled).then(|| wallet(p, &g).to_owned())
        })
        .collect()
}

impl Default for Grant {
    fn default() -> Self {
        Self {
            enabled: false,
            accept_sources: false,
            allowed_targets: None,
            allow_process_scope: false,
            allow_persistence: false,
            max_owned_sources: 8,
            wallet: None,
        }
    }
}

/// Configuration-only inspection; never opens registry, credentials or treasury state.
pub fn inspection(config: &MetaConfig) -> serde_json::Value {
    let Some(p) = &config.source_management else {
        return serde_json::Value::Null;
    };
    let bindings: std::collections::BTreeMap<_, _> = config
        .servers
        .keys()
        .filter_map(|id| {
            let g = grant(config, id);
            g.accept_sources
                .then(|| (id.clone(), wallet(p, &g).to_owned()))
        })
        .collect();
    serde_json::json!({"policy":p,"wallet_bindings":bindings,"grants":config.servers.keys().map(|id|(id,grant(config,id))).collect::<std::collections::BTreeMap<_,_>>(),"provider_filters_apply_to":"API tools only; management tools have separate grants"})
}
pub fn protected_paths(config: &MetaConfig, path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![path.to_owned()];
    if let Some(relay) = &config.discovery_relay {
        paths.push(
            path.parent()
                .unwrap_or(Path::new("."))
                .join(&relay.provider),
        );
    }
    for source in config.sources.values() {
        if let Some(file) = source.provider.get("extends").and_then(toml::Value::as_str) {
            paths.push(path.parent().unwrap_or(Path::new(".")).join(file));
        }
    }
    if let Some(t) = &config.treasury {
        paths.push(t.key_file.clone());
        paths.push(t.state_dir.clone());
    }
    paths
}
pub fn validate_registry_path(config: &MetaConfig, path: &Path) -> Result<()> {
    if let Some(file) = config
        .source_management
        .as_ref()
        .and_then(|p| p.registry_file.as_ref())
    {
        super::store::protect(file, &protected_paths(config, path))?;
    }
    Ok(())
}
