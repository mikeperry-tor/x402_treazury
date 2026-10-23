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
    /// Only legacy syntax grants implicit Curl format metadata.
    #[serde(skip_serializing)]
    pub provider: Option<PathBuf>,
    pub fallbacks: BTreeMap<String, Vec<PathBuf>>,
    pub wallet: Option<String>,
    pub serve: bool,
    pub warm: bool,
    pub search: bool,
    pub sources: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    provider: Option<PathBuf>,
    fallbacks: Option<BTreeMap<String, Vec<PathBuf>>>,
    #[serde(default)]
    wallet: Option<String>,
    #[serde(default)]
    serve: bool,
    #[serde(default)]
    warm: bool,
    #[serde(default = "search_default")]
    search: bool,
    #[serde(default)]
    sources: Vec<String>,
}

fn search_default() -> bool {
    true
}

impl TryFrom<Input> for Policy {
    type Error = anyhow::Error;

    fn try_from(input: Input) -> Result<Self> {
        ensure!(
            input.provider.is_none() || input.fallbacks.is_none(),
            "discovery_relay.provider cannot be combined with discovery_relay.fallbacks"
        );
        let fallbacks = match input.fallbacks {
            Some(methods) => methods,
            None => input
                .provider
                .as_ref()
                .map(|path| BTreeMap::from([("GET".into(), vec![path.clone()])]))
                .unwrap_or_default(),
        };
        for method in fallbacks.keys() {
            ensure!(
                matches!(method.as_str(), "GET" | "HEAD" | "POST"),
                "discovery_relay.fallbacks: unsupported method {method}; expected GET, HEAD or POST"
            );
        }
        Ok(Self {
            provider: input.provider,
            fallbacks,
            wallet: input.wallet,
            serve: input.serve,
            warm: input.warm,
            search: input.search,
            sources: input.sources,
        })
    }
}

impl Policy {
    pub fn legacy(provider: PathBuf) -> Self {
        Self {
            fallbacks: BTreeMap::from([("GET".into(), vec![provider.clone()])]),
            provider: Some(provider),
            ..Default::default()
        }
    }

    pub fn fallbacks(&self) -> &BTreeMap<String, Vec<PathBuf>> {
        &self.fallbacks
    }

    pub fn has_method(&self, method: &str) -> bool {
        self.fallbacks
            .get(method)
            .is_some_and(|providers| !providers.is_empty())
    }

    pub fn is_legacy(&self) -> bool {
        self.provider.is_some()
    }

    /// Resolve every list entry relative to the file declaring this section.
    /// Absolute entries remain absolute; no existence checks or catalogs are loaded.
    pub fn resolve_paths(&mut self, declaring: &Path) -> Result<()> {
        let directory = declaring.parent().unwrap_or(Path::new("."));
        for path in self.fallbacks.values_mut().flatten() {
            *path = std::path::absolute(directory.join(&*path))?;
        }
        if let Some(path) = &mut self.provider {
            *path = std::path::absolute(directory.join(&*path))?;
        }
        Ok(())
    }

    pub fn provider_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.fallbacks.values().flatten()
    }

    pub fn validate(&self, config: &crate::deployment::MetaConfig) -> Result<()> {
        if let Some(wallet) = &self.wallet {
            ensure!(
                config.wallets.contains_key(wallet),
                "discovery_relay.wallet must name a declared wallet"
            );
        }
        for source in &self.sources {
            ensure!(
                config.sources.contains_key(source),
                "discovery_relay: unknown source {source}"
            );
        }
        Ok(())
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            provider: None,
            fallbacks: BTreeMap::new(),
            wallet: None,
            serve: false,
            warm: false,
            search: true,
            sources: Vec::new(),
        }
    }
}
