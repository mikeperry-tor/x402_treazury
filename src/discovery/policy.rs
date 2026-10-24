use crate::deployment::MetaConfig;
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

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
        .any(|listener| listener.source_management)
}
pub fn validate(config: &MetaConfig) -> Result<()> {
    for (id, listener) in &config.servers {
        if listener.source_management {
            let wallet = listener
                .wallet
                .as_ref()
                .with_context(|| format!("server {id}: source_management requires wallet"))?;
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
        .filter(|listener| listener.source_management)
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
