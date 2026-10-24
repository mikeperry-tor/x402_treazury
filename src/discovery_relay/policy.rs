//! Operator-authored method lists; parsing and inspection perform no provider I/O.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "Input")]
pub struct Policy {
    pub relays: BTreeMap<String, Vec<PathBuf>>,
    pub wallet: Option<String>,
    pub source_discovery: bool,
    pub api_requests: bool,
    pub provider_metadata: bool,
    pub sources: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    #[serde(default)]
    relays: BTreeMap<String, Vec<PathBuf>>,
    #[serde(default)]
    wallet: Option<String>,
    #[serde(default = "enabled_default")]
    source_discovery: bool,
    #[serde(default)]
    api_requests: bool,
    #[serde(default = "enabled_default")]
    provider_metadata: bool,
    #[serde(default)]
    sources: Vec<String>,
}

fn enabled_default() -> bool {
    true
}

impl TryFrom<Input> for Policy {
    type Error = anyhow::Error;

    fn try_from(input: Input) -> Result<Self> {
        for method in input.relays.keys() {
            ensure!(
                matches!(method.as_str(), "GET" | "HEAD" | "POST"),
                "relay_fallback.relays: unsupported method {method}; expected GET, HEAD or POST"
            );
        }
        Ok(Self {
            relays: input.relays,
            wallet: input.wallet,
            source_discovery: input.source_discovery,
            api_requests: input.api_requests,
            provider_metadata: input.provider_metadata,
            sources: input.sources,
        })
    }
}

impl Policy {
    pub fn relays(&self) -> &BTreeMap<String, Vec<PathBuf>> {
        &self.relays
    }

    pub fn has_method(&self, method: &str) -> bool {
        self.relays
            .get(method)
            .is_some_and(|providers| !providers.is_empty())
    }

    /// Resolve every list entry relative to the file declaring this section.
    /// Absolute entries remain absolute; no existence checks or catalogs are loaded.
    pub fn resolve_paths(&mut self, declaring: &Path) -> Result<()> {
        let directory = declaring.parent().unwrap_or(Path::new("."));
        for path in self.relays.values_mut().flatten() {
            *path = std::path::absolute(directory.join(&*path))?;
        }
        Ok(())
    }

    pub fn provider_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.relays.values().flatten()
    }

    pub fn validate(&self, config: &crate::deployment::MetaConfig) -> Result<()> {
        if let Some(wallet) = &self.wallet {
            ensure!(
                config.wallets.contains_key(wallet),
                "relay_fallback.wallet must name a declared wallet"
            );
        }
        for source in &self.sources {
            ensure!(
                config.sources.contains_key(source),
                "relay_fallback: unknown source {source}"
            );
        }
        Ok(())
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            relays: BTreeMap::new(),
            wallet: None,
            source_discovery: true,
            api_requests: false,
            provider_metadata: true,
            sources: Vec::new(),
        }
    }
}
