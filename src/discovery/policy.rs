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
    /// Existing static GET tool used for directory search.
    #[serde(default = "directory_tool")]
    pub directory_tool: String,
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
fn directory_tool() -> String {
    "x402_list_services".into()
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
impl Policy {
    pub fn resolve(&mut self, path: &Path) {
        if let Some(file) = &mut self.registry_file
            && file.is_relative()
        {
            *file = path.parent().unwrap_or(Path::new(".")).join(&*file);
        }
    }
}
pub fn wallet<'a>(policy: &'a Policy, listener: &'a crate::deployment::ListenerConfig) -> &'a str {
    listener.wallet.as_deref().unwrap_or(&policy.wallet)
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
    for (id, listener) in &config.servers {
        if listener.source_management {
            let p = config
                .source_management
                .as_ref()
                .context("source_management=true requires deployment source_management settings")?;
            ensure!(
                config.wallets.contains_key(wallet(p, listener)),
                "server {id}: unknown dynamic wallet"
            );
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
        .values()
        .filter(|l| l.source_management)
        .map(|l| wallet(p, l).to_owned())
        .collect()
}
/// Configuration-only inspection; never opens registry, credentials or treasury state.
pub fn inspection(config: &MetaConfig) -> serde_json::Value {
    let Some(p) = &config.source_management else {
        return serde_json::Value::Null;
    };
    let bindings: std::collections::BTreeMap<_, _> = config
        .servers
        .iter()
        .filter(|(_, l)| l.source_management)
        .map(|(id, l)| (id.clone(), wallet(p, l).to_owned()))
        .collect();
    serde_json::json!({"policy":p,"wallet_bindings":bindings,"persistent":p.registry_file.is_some(),"scope":"endpoint_local"})
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
