use crate::deployment::MetaConfig;
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Operator-selected directory workflows; the two Coinbase scopes are independent.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum RemoteDirectory {
    #[serde(rename = "coinbase")]
    Coinbase,
    #[serde(rename = "coinbase_curated")]
    CoinbaseCurated,
    #[serde(rename = "x402-list")]
    X402List,
}
impl RemoteDirectory {
    pub fn key(self) -> &'static str {
        match self {
            Self::Coinbase => "coinbase",
            Self::CoinbaseCurated => "coinbase_curated",
            Self::X402List => "x402-list",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct EndpointPolicy {
    pub local: bool,
    pub remote: bool,
    pub raw: bool,
    pub directories: Vec<RemoteDirectory>,
}
impl From<&crate::deployment::ListenerConfig> for EndpointPolicy {
    fn from(listener: &crate::deployment::ListenerConfig) -> Self {
        let mut directories = Vec::new();
        for directory in &listener.remote_provider_directories {
            if !directories.contains(directory) {
                directories.push(*directory);
            }
        }
        Self {
            local: listener.provider_management_local,
            remote: listener.provider_management_remote,
            raw: listener.raw_url_calls,
            directories,
        }
    }
}
impl EndpointPolicy {
    pub fn any(&self) -> bool {
        self.management() || self.raw
    }
    pub fn management(&self) -> bool {
        self.local || self.remote
    }
    pub fn remote_search(&self) -> bool {
        self.remote && !self.directories.is_empty()
    }
    pub fn has_x402_list(&self) -> bool {
        self.directories.contains(&RemoteDirectory::X402List)
    }
}

pub fn resolve(config: &mut MetaConfig, path: &Path) {
    for listener in config.servers.values_mut() {
        if let Some(file) = &mut listener.source_registry_file
            && file.is_relative()
        {
            *file = path.parent().unwrap_or(Path::new(".")).join(&*file);
        }
    }
}
pub fn enabled(config: &MetaConfig) -> bool {
    config
        .servers
        .values()
        .any(|listener| EndpointPolicy::from(listener).any())
}
pub fn validate(config: &MetaConfig) -> Result<()> {
    for (id, listener) in &config.servers {
        if EndpointPolicy::from(listener).any() {
            let wallet = listener.wallet.as_ref().with_context(|| {
                format!("server {id}: provider management or raw URL calls require wallet")
            })?;
            ensure!(
                config.wallets.contains_key(wallet),
                "server {id}: unknown dynamic wallet"
            );
        }
    }
    Ok(())
}
pub fn wallets(config: &MetaConfig) -> BTreeSet<String> {
    config
        .servers
        .values()
        .filter(|listener| EndpointPolicy::from(*listener).any())
        .filter_map(|listener| listener.wallet.clone())
        .collect()
}
pub fn protected_paths(config: &MetaConfig, path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![path.to_owned()];
    if let Some(relay) = &config.relay_fallback {
        paths.extend(
            relay
                .provider_paths()
                .map(|provider| path.parent().unwrap_or(Path::new(".")).join(provider)),
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
    let protected = protected_paths(config, path);
    for listener in config.servers.values() {
        if let Some(file) = &listener.source_registry_file {
            super::store::protect(file, &protected)?;
            for other in config
                .servers
                .values()
                .filter_map(|listener| listener.source_registry_file.as_ref())
                .filter(|other| *other != file)
            {
                super::store::protect(file, std::slice::from_ref(other))?;
            }
        }
    }
    Ok(())
}
