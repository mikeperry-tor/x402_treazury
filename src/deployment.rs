//! Resolve catalogs (optionally through a paid relay), then bind authenticated listeners.
use crate::{
    catalog::{self, Config, ToolSpec},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, http_app_with_auth},
};
use anyhow::{Context, Result, bail, ensure};
pub mod catalog_evidence;
mod startup;
pub use startup::StartupConfig;
mod cache_warm;
pub use cache_warm::{CacheWarmSummary, warm_cache};

/// Full frozen catalogs may exceed ordinary tool-result/evidence limits. Includes
/// all source documents and inventories, with no response-schema truncation.
pub const QUALIFICATION_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::Path,
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinSet};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetaConfig {
    pub version: u32,
    #[serde(default)]
    pub startup: StartupConfig,
    #[serde(default)]
    pub network: crate::network::NetworkPolicy,
    pub treasury: Option<TreasuryConfig>,
    pub funding: Option<FundingConfig>,
    pub source_management: Option<crate::discovery::policy::Policy>,
    pub discovery_relay: Option<crate::discovery_relay::Policy>,
    #[serde(default)]
    pub sources: BTreeMap<String, SourceConfig>,
    #[serde(default)]
    pub wallets: BTreeMap<String, WalletConfig>,
    #[serde(default)]
    pub wallet_templates: BTreeMap<String, WalletConfig>,
    pub wallet_assignment: Option<Assignment>,
    pub servers: BTreeMap<String, ListenerConfig>,
}
#[derive(Deserialize, Serialize)]
pub struct SourceConfig {
    pub wallet: Option<String>,
    #[serde(flatten)]
    pub provider: toml::Table,
}
pub use crate::rotation::assignment::WalletBinding;
use crate::rotation::assignment::{self, Assignment, Resolution, WalletSummary};
pub use crate::rotation::config::WalletConfig;
use crate::rotation::config::{FundingConfig, TreasuryConfig};
fn auth_enabled() -> bool {
    true
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    pub listen: SocketAddr,
    #[serde(default = "auth_enabled")]
    pub auth: bool,
    pub allowed_hosts: Option<Vec<String>>,
    #[serde(default)]
    pub disable_host_check: bool,
    #[serde(default)]
    pub bearer_token_env: String,
    pub wallet: Option<String>,
    pub source_management: Option<crate::discovery::policy::Grant>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub include_tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub exclude_tags: Vec<String>,
    #[serde(default)]
    pub exclude_tools: Vec<String>,
    pub max_response_chars: Option<usize>,
}
struct Source {
    config: Config,
    document: std::sync::Arc<serde_json::Value>,
    tools: Vec<ToolSpec>,
    base_url: String,
    instructions: Option<String>,
}
#[derive(Serialize)]
pub struct Inventory {
    pub server: String,
    pub listen: SocketAddr,
    pub default_wallet: Option<String>,
    pub wallet_bindings: BTreeMap<String, WalletBinding>,
    pub tools: Vec<InventoryTool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub management_tools: Vec<rmcp::model::Tool>,
}
#[derive(Serialize)]
pub struct InventoryTool {
    pub source: String,
    #[serde(flatten)]
    pub tool: ToolSpec,
}
pub struct Deployment {
    initialized: Option<InitializedWallets>,
    config: MetaConfig,
    sources: BTreeMap<String, Source>,
    selected: BTreeMap<String, Vec<(String, ToolSpec)>>,
    wallet_resolution: Resolution,
    config_path: std::path::PathBuf,
}
pub(crate) fn pattern(text: &str) -> Result<Regex> {
    ensure!(!text.is_empty(), "empty tool selector");
    // Only '*' and '?' are special; brackets and regex metacharacters are literal.
    let mut expression = String::from("^");
    for c in text.chars() {
        match c {
            '*' => expression.push_str(".*"),
            '?' => expression.push('.'),
            _ => expression.push_str(&regex::escape(&c.to_string())),
        }
    }
    expression.push('$');
    Ok(Regex::new(&expression)?)
}
impl MetaConfig {
    fn validate(&self) -> Result<Resolution> {
        self.startup.validate()?;
        self.network.validate()?;
        crate::discovery::policy::validate(self)?;
        if let Some(relay) = &self.discovery_relay {
            relay.validate(self)?;
        }
        ensure!(
            self.version == 1,
            "unsupported meta-config version {}",
            self.version
        );
        ensure!(!self.servers.is_empty(), "at least one server is required");
        let identifier = Regex::new("^[a-z][a-z0-9_]*$")?;
        for name in self
            .sources
            .keys()
            .chain(self.wallets.keys())
            .chain(self.wallet_templates.keys())
            .chain(self.servers.keys())
        {
            ensure!(
                identifier.is_match(name),
                "invalid identifier {name}: use lowercase letters, digits and underscores"
            );
        }
        for (name, wallet) in &self.wallets {
            wallet
                .validate()
                .with_context(|| format!("wallet {name}"))?;
        }
        for (name, source) in &self.sources {
            if let Some(wallet) = &source.wallet {
                ensure!(
                    self.wallets.contains_key(wallet),
                    "source {name}: unknown wallet {wallet}"
                );
            }
        }
        let mut addresses = BTreeSet::new();
        for (name, server) in &self.servers {
            crate::server::host::HostPolicy::new(
                server.allowed_hosts.clone(),
                server.disable_host_check,
            )
            .with_context(|| format!("server {name}: invalid Host policy"))?;
            ensure!(
                server.listen.ip().is_loopback(),
                "server {name}: this release requires a loopback listen address"
            );
            ensure!(
                server.listen.port() == 0 || addresses.insert(server.listen),
                "duplicate listener {}",
                server.listen
            );
            ensure!(
                !server.auth || !server.bearer_token_env.trim().is_empty(),
                "server {name}: bearer_token_env is required unless auth = false"
            );
            ensure!(
                server.auth || server.bearer_token_env.is_empty(),
                "server {name}: omit bearer_token_env when auth = false"
            );
            if let Some(wallet) = &server.wallet {
                ensure!(
                    self.wallets.contains_key(wallet),
                    "server {name}: unknown wallet {wallet}"
                );
            }
            ensure!(
                !server.sources.is_empty()
                    || server
                        .source_management
                        .as_ref()
                        .is_some_and(|g| g.accept_sources),
                "server {name}: sources cannot be empty"
            );
            ensure!(
                server.max_response_chars != Some(0),
                "server {name}: max_response_chars must be positive"
            );
            let mut seen = BTreeSet::new();
            for source in &server.sources {
                ensure!(
                    self.sources.contains_key(source),
                    "server {name}: unknown source {source}"
                );
                ensure!(
                    seen.insert(source),
                    "server {name}: duplicate source {source}"
                );
            }
            for selector in server.include_tools.iter().chain(&server.exclude_tools) {
                pattern(selector)?;
            }
        }
        let resolution = assignment::resolve(self)?;
        if resolution.wallets.values().any(WalletConfig::managed) {
            self.treasury
                .as_ref()
                .context("managed profiles require [treasury]")?
                .validate()?;
            self.funding
                .as_ref()
                .context("managed profiles require [funding]")?
                .validate()?;
        } else {
            if let Some(t) = &self.treasury {
                t.validate()?;
            }
            if let Some(f) = &self.funding {
                f.validate()?;
            }
        }
        Ok(resolution)
    }
}
impl Deployment {
    pub async fn show_config(path: &Path) -> Result<serde_json::Value> {
        let mut config: MetaConfig = toml::from_str(&tokio::fs::read_to_string(path).await?)?;
        if let Some(p) = &mut config.source_management {
            p.resolve(path);
        }
        let wallet_resolution = config.validate()?;
        if let Some(t) = &mut config.treasury {
            t.resolve(path);
        }
        crate::discovery::policy::validate_registry_path(&config, path)?;
        let mut resolved = BTreeMap::new();
        for (id, source) in &config.sources {
            let mut provider = crate::config::resolve(source.provider.clone(), path).await?;
            if provider.settings.prefix.is_none() {
                provider.settings.prefix = Some(id.clone());
                provider
                    .origins
                    .insert("prefix".into(), "source identifier".into());
            }
            let mut resolved_source = serde_json::to_value(provider)?;
            resolved_source["wallet"] = serde_json::to_value(&source.wallet)?;
            resolved.insert(id, resolved_source);
        }
        Ok(
            serde_json::json!({"discovery_relay":config.discovery_relay,"startup":config.startup,"source_management":crate::discovery::policy::inspection(&config),"network":config.network.inspection(),"version":config.version,"treasury":config.treasury,"treasury_identity":config.treasury.as_ref().map(|t| if t.id.is_empty() { "from_wallet_state_at_runtime" } else { "explicit_expected_id" }),"funding":config.funding,"base_rpc_policy":config.funding.as_ref().map(|f| f.base_rpc_policy()),"sources":resolved,"wallets":config.wallets,"servers":config.servers,"wallet_bindings":wallet_resolution.bindings,"wallet_templates":config.wallet_templates,"wallet_assignment":config.wallet_assignment,"resolved_wallets":wallet_resolution.wallets,"generated_wallets":wallet_resolution.generated,"wallet_summary":wallet_resolution.summary}),
        )
    }
    pub async fn load(path: &Path) -> Result<Self> {
        Self::load_with_warnings(path, false, None).await
    }
    /// Warn for listener-bound sources before catalog I/O, including failed fetches.
    pub async fn load_for_serving(path: &Path) -> Result<Self> {
        Self::load_for_serving_with_relay(path, &std::env::vars().collect(), Default::default())
            .await
    }
    pub async fn load_for_serving_with_relay(
        path: &Path,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
    ) -> Result<Self> {
        Self::load_with_warnings(path, true, Some((env, restriction))).await
    }
    async fn load_with_warnings(
        path: &Path,
        warn: bool,
        relay_env: Option<(
            &BTreeMap<String, String>,
            crate::rotation::restriction::FundingRestriction,
        )>,
    ) -> Result<Self> {
        let mut config: MetaConfig = toml::from_str(&tokio::fs::read_to_string(path).await?)
            .context("invalid meta-config")?;
        if let Some(p) = &mut config.source_management {
            p.resolve(path);
        }
        let wallet_resolution = config.validate()?;
        crate::network::install(config.network.clone())?;
        if let Some(t) = &mut config.treasury {
            t.resolve(path);
        }
        crate::discovery::policy::validate_registry_path(&config, path)?;
        let mut deployment = Self {
            initialized: None,
            config,
            wallet_resolution,
            sources: BTreeMap::new(),
            selected: BTreeMap::new(),
            config_path: path.to_owned(),
        };
        let relay = if let Some(policy) = deployment
            .config
            .discovery_relay
            .clone()
            .filter(|p| p.serve)
            && let Some((env, restriction)) = relay_env
        {
            ensure!(
                !crate::qualification::active() && !catalog_evidence::collecting(),
                "paid discovery relay is unavailable during qualification"
            );
            // Validate listener credentials before discovery can spend.
            for server in deployment.config.servers.values().filter(|s| s.auth) {
                required_secret(env, &server.bearer_token_env)?;
            }
            let initialized = deployment
                .initialize_wallets(env, restriction, None)
                .await?;
            let relay = crate::discovery_relay::Relay::new(
                &policy,
                path,
                initialized.wallets[&policy.wallet].clone(),
            )
            .await?;
            deployment.initialized = Some(initialized);
            Some(relay)
        } else {
            None
        };
        let config = &deployment.config;
        let wallet_resolution = &deployment.wallet_resolution;
        let sources = startup::load(config, path, warn, relay).await?;
        let mut selected = BTreeMap::new();
        for (name, server) in &config.servers {
            selected.insert(name.clone(), select_listener_tools(name, server, &sources)?);
        }
        let mut cover_owners = BTreeMap::new();
        for (server, tools) in &selected {
            for (id, tool) in tools {
                let source = &sources[id];
                if tool.help_url.is_some() {
                    continue;
                }
                if let Some(cover) = &source.config.cover_traffic {
                    let route =
                        if tool.path.starts_with("https://") || tool.path.starts_with("http://") {
                            tool.path.clone()
                        } else {
                            source.base_url.clone()
                        };
                    cover.validate_origin(&route)?;
                    let key = (
                        wallet_resolution.bindings[server][id].wallet.clone(),
                        reqwest::Url::parse(&route)?.origin().ascii_serialization(),
                        source.config.transport(),
                        crate::network::global()
                            .request_timeout(Duration::from_secs_f64(source.config.timeout))
                            .as_millis(),
                    );
                    if let Some(previous) = cover_owners.insert(key, cover) {
                        ensure!(
                            previous == cover,
                            "conflicting cover settings for shared wallet/origin"
                        );
                    }
                }
            }
        }
        deployment.sources = sources;
        deployment.selected = selected;
        Ok(deployment)
    }
    /// Credential-free qualification snapshot, returned only by an explicit CLI inspection.
    pub async fn inspect_catalogs(
        path: &Path,
    ) -> (Result<Self>, Vec<catalog_evidence::Observation>) {
        catalog_evidence::capture(Self::load(path)).await
    }
    pub fn qualification_snapshot(&self) -> serde_json::Value {
        let sources: BTreeMap<_,_> = self.sources.iter().map(|(id, s)| (id.clone(), serde_json::json!({"settings":s.config,"document":s.document,"base_url":s.base_url}))).collect();
        serde_json::json!({"version":1,"deployment":self.config,"sources":sources,"inventory":self.inventory()})
    }
    pub fn wallet_summary(&self) -> &WalletSummary {
        &self.wallet_resolution.summary
    }
    pub fn inventory(&self) -> Vec<Inventory> {
        self.config
            .servers
            .iter()
            .map(|(name, s)| Inventory {
                server: name.clone(),
                listen: s.listen,
                default_wallet: s.wallet.clone(),
                wallet_bindings: self.wallet_resolution.bindings[name].clone(),
                management_tools: {
                    let g = crate::discovery::policy::grant(&self.config, name);
                    let mut tools =
                        crate::discovery::tools::definitions(g.enabled, g.accept_sources);
                    if self.config.network.cover_enabled()
                        && self.selected[name].iter().any(|(id, t)| {
                            t.help_url.is_none() && self.sources[id].config.cover_traffic.is_some()
                        })
                    {
                        tools.push(crate::cover::status::definition());
                    }
                    tools
                },
                tools: self.selected[name]
                    .iter()
                    .map(|(source, tool)| InventoryTool {
                        source: source.clone(),
                        tool: tool.clone(),
                    })
                    .collect(),
            })
            .collect()
    }
    pub fn tag_inventory(&self) -> Result<BTreeMap<String, BTreeMap<String, usize>>> {
        self.sources
            .iter()
            .map(|(id, source)| Ok((id.clone(), catalog::tag_counts(&source.document)?)))
            .collect()
    }
    async fn initialize_wallets(
        &self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
        only_wallet: Option<&str>,
    ) -> Result<InitializedWallets> {
        #[cfg(not(feature = "zcash"))]
        let _ = restriction;
        let secret = |key: &str| required_secret(env, key);
        // Static-only serving does not unlock state, but must not reuse a known
        // managed profile name as a different wallet identity.
        if !self.wallet_resolution.wallets.iter().any(|(name, wallet)| {
            only_wallet.is_none_or(|selected| selected == name) && wallet.managed()
        }) && let Some(t) = &self.config.treasury
            && t.state_dir.exists()
        {
            let dir = t.state_dir.clone();
            let state =
                tokio::task::spawn_blocking(move || crate::rotation::store::status(&dir)).await??;
            ensure!(
                t.id.is_empty() || state.treasury_id == t.id,
                "state identity mismatch"
            );
            for pool in state.pools {
                ensure!(
                    !self.wallet_resolution.wallets.contains_key(&pool.name),
                    "managed pool {} cannot become static without explicit retirement",
                    pool.name
                );
            }
        }
        let mut wallets = BTreeMap::new();
        // Only effective static profiles for selected tools need keys. An overridden
        // default or a source removed by listener filters must not load a signer.
        let mut used_wallets: BTreeSet<_> = self
            .selected
            .iter()
            .flat_map(|(server, tools)| {
                tools.iter().map(|(source, _)| {
                    self.wallet_resolution.bindings[server][source]
                        .wallet
                        .clone()
                })
            })
            .collect();
        used_wallets.extend(crate::discovery::policy::wallets(&self.config));
        if let Some(policy) = &self.config.discovery_relay
            && (only_wallet.is_some() || (policy.serve && self.sources.is_empty()))
        {
            used_wallets.insert(policy.wallet.clone());
        }
        used_wallets.retain(|name| only_wallet.is_none_or(|selected| selected == name));
        for name in used_wallets {
            if let WalletConfig::Static {
                private_key_env,
                max_price_usd,
            } = &self.wallet_resolution.wallets[&name]
            {
                let payer = Payer::new(
                    &secret(private_key_env)?,
                    SpendPolicy::dollars(max_price_usd)?,
                )
                .with_context(|| format!("wallet {name}: invalid signing configuration"))?;
                wallets.insert(name, PaidClient::new(payer));
            }
        }
        #[cfg(feature = "zcash")]
        let mut treasury = None;
        #[cfg(feature = "zcash")]
        let mut funding_runtime = None;
        #[cfg(feature = "zcash")]
        let mut managed_pools = Vec::new();
        if self.wallet_resolution.wallets.iter().any(|(name, wallet)| {
            only_wallet.is_none_or(|selected| selected == name) && wallet.managed()
        }) {
            #[cfg(not(feature = "zcash"))]
            bail!("managed serving requires a build with --features zcash");
            #[cfg(feature = "zcash")]
            {
                use crate::rotation::{
                    base::{BaseRpc, secure_endpoint},
                    manager::ManagedPool,
                };
                let t = self.config.treasury.as_ref().unwrap();
                let f = self.config.funding.as_ref().unwrap();
                // Resolve endpoints now; the treasury sync worker starts only when serving.
                let sync_settings = crate::treasury::SyncSettings::new(
                    t.indexer_endpoint(|name| env.get(name).cloned())?,
                    t.confirmations,
                    t.max_sync_age_seconds,
                )?;
                secure_endpoint(&t.submission_endpoint(|name| env.get(name).cloned())?)?;
                if let Some(key) = &f.near_api_key_env {
                    secret(key)?;
                }
                let urls = f.base_rpc_urls(|name| env.get(name).cloned())?;
                tracing::warn!(
                    provider_count = urls.len(),
                    default_fallbacks = f.base_rpc_fallback_url_envs.is_none(),
                    "Base RPC policy active; verification may disclose wallet addresses to fallback providers; see config show base_rpc_policy"
                );
                let base = BaseRpc::with_fallbacks(
                    &urls,
                    f.base_confirmations,
                    f.base_max_block_age_seconds,
                )?;
                let mut owner = crate::treasury::Treasury::open(
                    t.state_dir.clone(),
                    t.key_file.clone(),
                    t.runtime_id()?,
                )
                .await?;
                owner.configure_sync(sync_settings);
                let store = owner.store_handle();
                if let Some(permits) = crate::qualification::managed_permits() {
                    store
                        .call(move |s| s.install_funding_permits(permits))
                        .await?;
                }
                if restriction == crate::rotation::restriction::FundingRestriction::DenyNewFunding {
                    store
                        .call(|s| {
                            s.deny_new_funding();
                            Ok(())
                        })
                        .await?;
                }
                let managed = self
                    .wallet_resolution
                    .wallets
                    .iter()
                    .filter(|(_, w)| w.managed())
                    .map(|(n, _)| n.clone())
                    .collect();
                let statics = self
                    .wallet_resolution
                    .wallets
                    .iter()
                    .filter(|(_, w)| !w.managed())
                    .map(|(n, _)| n.clone())
                    .collect();
                store
                    .call(move |s| s.configure_profiles(&managed, &statics))
                    .await?;
                for (name, w) in &self.wallet_resolution.wallets {
                    if only_wallet.is_some_and(|selected| selected != name) {
                        continue;
                    }
                    if let WalletConfig::ZcashRotation {
                        deposit_size,
                        max_price_usd,
                        wait_seconds,
                        ..
                    } = w
                    {
                        let pool = owner
                            .ensure_pool(name.clone(), deposit_size.clone())
                            .await?;
                        let manager = ManagedPool::new(
                            store.clone(),
                            pool,
                            base.clone(),
                            deposit_size,
                            SpendPolicy::dollars(max_price_usd)?,
                            *wait_seconds,
                        )?;
                        let manager = std::sync::Arc::new(manager);
                        managed_pools.push(manager.clone());
                        wallets.insert(name.clone(), PaidClient::managed(manager));
                    }
                }
                tracing::info!(target: "x402_treazury::startup", auto_fund = f.auto_fund,
                    "{}", if f.auto_fund { "automatic managed funding enabled: bootstrap and replacements are subject to source limits and any qualification restrictions" } else { "automatic managed funding disabled: new bootstrap and replacement transfers are paused" });
                if f.auto_fund {
                    let (handle, commands) = crate::treasury::actor::channel();
                    let key = f.near_api_key_env.as_deref().map(secret).transpose()?;
                    let session = f.near_user_session_env.as_deref().map(secret).transpose()?;
                    let backend = crate::rotation::funding::Backend {
                        treasury: handle,
                        store: store.clone(),
                        near: crate::rotation::near::NearClient::with_session(
                            key.as_deref(),
                            session.as_deref(),
                        )?,
                        base,
                        wallets: self.wallet_resolution.wallets.clone(),
                        funding: f.clone(),
                        daily_limit: u64::try_from(crate::rotation::config::zatoshis(
                            &t.daily_input_zec,
                        )?)?,
                    };
                    let sender = crate::treasury::submission::GrpcSubmission::new(
                        t.submission_endpoint(|name| env.get(name).cloned())?,
                        t.indexer_endpoint(|name| env.get(name).cloned())?,
                    )?;
                    funding_runtime = Some((
                        commands,
                        sender,
                        crate::rotation::funding::FundingWorker {
                            store: store.clone(),
                            backend,
                            poll_seconds: f.poll_seconds,
                        },
                    ));
                }
                treasury = Some(owner);
            }
        }
        Ok(InitializedWallets {
            wallets,
            #[cfg(feature = "zcash")]
            treasury,
            #[cfg(feature = "zcash")]
            funding_runtime,
            #[cfg(feature = "zcash")]
            managed_pools,
        })
    }

    /// Prepare startup prices without binding listeners or opening additional wallets.
    /// Inspection is unsigned; a serving load may have explicitly prepared a paid relay.
    /// Uses the process cache; repeated calls never refresh completed attempts.
    pub async fn discover_prices(&mut self) -> Result<()> {
        let started = std::time::Instant::now();
        tracing::info!(target: "x402_treazury::startup", request_concurrency = crate::pricing::CONCURRENCY, "Startup pricing discovery started; serving waits for discovery");
        let mut pending = Vec::new();
        for (id, source) in &self.sources {
            let selected: Vec<_> = self
                .selected
                .values()
                .flatten()
                .filter(|(s, _)| s == id)
                .map(|(_, t)| t.clone())
                .collect();
            if !selected.is_empty() {
                pending.push(startup::price_source(id, source, selected));
            }
        }
        use futures_util::{StreamExt, TryStreamExt, stream};
        let completed: Vec<_> = stream::iter(pending)
            .buffer_unordered(crate::pricing::CONCURRENCY)
            .try_collect()
            .await?;
        // Apply only after every source succeeds; completion order cannot change inventory.
        for (id, tools) in completed {
            for (s, t) in self.selected.values_mut().flatten() {
                if s == id {
                    *t = tools[&t.name].clone();
                }
            }
        }
        tracing::info!(target: "x402_treazury::startup", elapsed_ms = started.elapsed().as_millis() as u64, "Startup pricing discovery finished");
        Ok(())
    }

    fn build_servers(&self, wallets: &BTreeMap<String, PaidClient>) -> BTreeMap<String, Server> {
        let mut servers = BTreeMap::new();
        for (name, cfg) in &self.config.servers {
            let selected = &self.selected[name];
            let bindings = selected
                .iter()
                .map(|(id, t)| {
                    let source = &self.sources[id];
                    (
                        t.clone(),
                        wallets[&self.wallet_resolution.bindings[name][id].wallet]
                            .clone()
                            .with_transport(source.config.transport())
                            .with_cover(
                                source.config.cover_traffic.clone(),
                                crate::cover::status::Scope {
                                    listener: name.clone(),
                                    source: id.clone(),
                                },
                            )
                            .with_timeout(Duration::from_secs_f64(source.config.timeout))
                            .with_download_limits(
                                source.config.max_response_bytes,
                                source.config.max_help_bytes,
                            ),
                        source.base_url.clone(),
                    )
                })
                .collect();
            let used: BTreeSet<_> = selected.iter().map(|(id, _)| id).collect();
            let instructions = used
                .into_iter()
                .filter_map(|id| {
                    self.sources[id]
                        .instructions
                        .as_ref()
                        .map(|s| format!("[{id}]\n{s}"))
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let mut server = Server::from_bindings(
                bindings,
                (!instructions.is_empty()).then_some(instructions),
                cfg.max_response_chars,
            );
            server.name = name.clone();
            servers.insert(name.clone(), server);
        }
        servers
    }

    pub async fn bind(self, env: &BTreeMap<String, String>) -> Result<RunningDeployment> {
        self.bind_restricted(env, Default::default()).await
    }

    /// Qualification restrictions only remove authority; they never authorize execution.
    pub async fn bind_restricted(
        self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
    ) -> Result<RunningDeployment> {
        self.bind_mode(env, restriction, false).await
    }

    /// Keyless qualification only; no treasury/store/signers or agent mutations are opened.
    pub async fn bind_unsigned(self, env: &BTreeMap<String, String>) -> Result<RunningDeployment> {
        self.bind_mode(
            env,
            crate::rotation::restriction::FundingRestriction::DenyNewFunding,
            true,
        )
        .await
    }

    async fn bind_mode(
        mut self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
        unsigned: bool,
    ) -> Result<RunningDeployment> {
        let secret = |key: &str| required_secret(env, key);
        // Validate every credential before opening any port.
        let mut tokens = BTreeMap::new();
        for (name, cfg) in &self.config.servers {
            tokens.insert(
                name.clone(),
                if cfg.auth {
                    Some(secret(&cfg.bearer_token_env)?)
                } else {
                    None
                },
            );
        }
        let initialized = if unsigned {
            ensure!(
                self.config.source_management.is_none()
                    && self
                        .config
                        .servers
                        .values()
                        .all(|s| s.source_management.is_none()),
                "unsigned qualification forbids agent source management"
            );
            ensure!(
                !self
                    .wallet_resolution
                    .wallets
                    .values()
                    .any(WalletConfig::managed),
                "unsigned qualification forbids managed wallet profiles"
            );
            ensure!(
                !self.config.funding.as_ref().is_some_and(|f| f.auto_fund),
                "unsigned qualification forbids auto_fund"
            );
            tracing::warn!(
                "unsigned-only qualification: no signing keys or treasury state opened; all payment challenges will be refused"
            );
            InitializedWallets {
                wallets: self
                    .wallet_resolution
                    .wallets
                    .keys()
                    .map(|name| (name.clone(), PaidClient::unsigned()))
                    .collect(),
                #[cfg(feature = "zcash")]
                treasury: None,
                #[cfg(feature = "zcash")]
                funding_runtime: None,
                #[cfg(feature = "zcash")]
                managed_pools: Vec::new(),
            }
        } else if let Some(mut initialized) = self.initialized.take() {
            #[cfg(feature = "zcash")]
            if restriction == crate::rotation::restriction::FundingRestriction::DenyNewFunding
                && let Some(treasury) = &initialized.treasury
            {
                treasury
                    .store_handle()
                    .call(|store| {
                        store.deny_new_funding();
                        Ok(())
                    })
                    .await?;
            }
            // Catalog selection is now known: unused/overridden static profiles
            // must still not require credentials merely because relay is enabled.
            let needed: BTreeSet<_> = self
                .selected
                .iter()
                .flat_map(|(server, tools)| {
                    tools.iter().map(|(source, _)| {
                        self.wallet_resolution.bindings[server][source]
                            .wallet
                            .clone()
                    })
                })
                .collect();
            for name in needed {
                if !initialized.wallets.contains_key(&name)
                    && let WalletConfig::Static {
                        private_key_env,
                        max_price_usd,
                    } = &self.wallet_resolution.wallets[&name]
                {
                    let payer = Payer::new(
                        &secret(private_key_env)?,
                        SpendPolicy::dollars(max_price_usd)?,
                    )
                    .with_context(|| format!("wallet {name}: invalid signing configuration"))?;
                    initialized.wallets.insert(name, PaidClient::new(payer));
                }
            }
            initialized
        } else {
            self.initialize_wallets(env, restriction, None).await?
        };
        let wallets = initialized.wallets;
        #[cfg(feature = "zcash")]
        let treasury = initialized.treasury;
        #[cfg(feature = "zcash")]
        let funding_runtime = initialized.funding_runtime;
        #[cfg(feature = "zcash")]
        let managed_pools = initialized.managed_pools;
        self.discover_prices().await?;
        let mut servers = self.build_servers(&wallets);
        let mut snapshot = crate::catalog_state::CatalogSnapshot::default();
        for (name, server) in &servers {
            server.validate_cover()?;
            let tools = server.catalog.read().views["default"].clone();
            if self.config.source_management.is_some() {
                ensure!(
                    tools.iter().all(|t| !t.tool.name.starts_with("treazury_")
                        && !t.tool.name.starts_with("dyn_")),
                    "static tool uses reserved namespace"
                );
            }
            snapshot.views.insert(name.clone(), tools);
        }
        let catalog = std::sync::Arc::new(crate::catalog_state::CatalogState::new(snapshot));
        let protected = crate::discovery::policy::protected_paths(&self.config, &self.config_path);
        let manager = if let Some(policy) = self.config.source_management.clone() {
            Some(
                crate::discovery::Manager::new(
                    policy,
                    self.config.servers.clone(),
                    wallets,
                    catalog.clone(),
                    protected,
                )
                .await?,
            )
        } else {
            None
        };
        let mut listeners = Vec::new();
        for (name, cfg) in &self.config.servers {
            let mut server = servers.remove(name).unwrap();
            server.catalog = catalog.clone();
            server.catalog_server = name.clone();
            server.host_policy = crate::server::host::HostPolicy::new(
                cfg.allowed_hosts.clone(),
                cfg.disable_host_check,
            )?;
            server.discovery = manager.clone();
            let listener = TcpListener::bind(cfg.listen)
                .await
                .with_context(|| format!("server {name}: cannot bind {}", cfg.listen))?;
            listeners.push((
                name.clone(),
                listener,
                http_app_with_auth(server, tokens.remove(name).unwrap()),
            ));
        }
        Ok(RunningDeployment {
            listeners,
            #[cfg(test)]
            listener_failure: None,
            #[cfg(feature = "zcash")]
            treasury,
            #[cfg(feature = "zcash")]
            funding_runtime,
            #[cfg(feature = "zcash")]
            managed_pools,
        })
    }
}

fn select_listener_tools(
    name: &str,
    server: &ListenerConfig,
    sources: &BTreeMap<String, Source>,
) -> Result<Vec<(String, ToolSpec)>> {
    let mut available = BTreeMap::new();
    for source in &server.sources {
        let operations = catalog::operations(&sources[source].document, None)?;
        for tool in &sources[source].tools {
            if !catalog::matches_operation_tags(
                &operations,
                tool,
                &server.tags,
                &server.exclude_tags,
            ) {
                continue;
            }

            ensure!(
                available
                    .insert(tool.name.clone(), (source.clone(), tool.clone()))
                    .is_none(),
                "server {name}: duplicate tool {}",
                tool.name
            );
        }
    }
    let compile = |selectors: &[String]| -> Result<Vec<Regex>> {
        selectors
            .iter()
            .map(|text| {
                let re = pattern(text)?;
                if !available.keys().any(|n| re.is_match(n)) {
                    if text.contains(['*', '?']) {
                        tracing::warn!(
                            server = name,
                            selector = text,
                            "Tool pattern matches nothing"
                        );
                    } else {
                        bail!("server {name}: unknown tool selector {text}");
                    }
                }
                Ok(re)
            })
            .collect()
    };
    let include = compile(&server.include_tools)?;
    let exclude = compile(&server.exclude_tools)?;
    let tools: Vec<_> = available
        .into_iter()
        .filter(|(n, _)| {
            (include.is_empty() || include.iter().any(|p| p.is_match(n)))
                && !exclude.iter().any(|p| p.is_match(n))
        })
        .map(|(_, t)| t)
        .collect();
    ensure!(
        !tools.is_empty()
            || server
                .source_management
                .as_ref()
                .is_some_and(|g| g.accept_sources),
        "server {name}: no tools selected"
    );
    Ok(tools)
}

fn required_secret(env: &BTreeMap<String, String>, key: &str) -> Result<String> {
    env.get(key)
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .with_context(|| format!("required environment variable {key} is missing or empty"))
}

struct InitializedWallets {
    wallets: BTreeMap<String, PaidClient>,
    #[cfg(feature = "zcash")]
    treasury: Option<crate::treasury::Treasury>,
    #[cfg(feature = "zcash")]
    funding_runtime: Option<FundingRuntime>,
    #[cfg(feature = "zcash")]
    managed_pools: Vec<std::sync::Arc<crate::rotation::manager::ManagedPool>>,
}
#[cfg(feature = "zcash")]
type FundingRuntime = (
    crate::treasury::actor::TreasuryCommands,
    crate::treasury::submission::GrpcSubmission,
    crate::rotation::funding::FundingWorker<crate::rotation::funding::Backend>,
);
pub struct RunningDeployment {
    #[cfg(test)]
    listener_failure: Option<(String, tokio::sync::oneshot::Receiver<()>)>,
    #[cfg(feature = "zcash")]
    funding_runtime: Option<FundingRuntime>,
    #[cfg(feature = "zcash")]
    managed_pools: Vec<std::sync::Arc<crate::rotation::manager::ManagedPool>>,
    #[cfg(feature = "zcash")]
    treasury: Option<crate::treasury::Treasury>,
    listeners: Vec<(String, TcpListener, axum::Router)>,
}
impl RunningDeployment {
    pub fn addresses(&self) -> Vec<(String, SocketAddr)> {
        self.listeners
            .iter()
            .map(|(name, l, _)| (name.clone(), l.local_addr().expect("bound listener")))
            .collect()
    }
    pub async fn serve(self, shutdown: CancellationToken) -> Result<()> {
        let stop = CancellationToken::new();
        let _cancel_on_drop = stop.clone().drop_guard();
        #[cfg(feature = "zcash")]
        let (treasury_task, funding_task) = match (self.treasury, self.funding_runtime) {
            (Some(owner), Some((commands, sender, worker))) => (
                Some(tokio::spawn(owner.run_commands(
                    commands,
                    sender,
                    stop.clone(),
                ))),
                Some(tokio::spawn({
                    let stopped = stop.clone();
                    async move {
                        let result = worker.run(stopped.clone()).await;
                        stopped.cancel();
                        result
                    }
                })),
            ),
            (Some(owner), None) => (Some(tokio::spawn(owner.run_sync(stop.clone()))), None),
            (None, _) => (None, None),
        };
        #[cfg(feature = "zcash")]
        let mut reconciliation = JoinSet::new();
        #[cfg(feature = "zcash")]
        for pool in self.managed_pools {
            let stopped = stop.clone();
            reconciliation.spawn(async move {
                let mut failures = 0u32;
                loop {
                    tokio::select! {
                        _ = stopped.cancelled() => break,
                        result = pool.reconcile() => {
                            failures = if result.is_err() { failures.saturating_add(1) } else { 0 };
                            if result.is_err() {
                                tracing::warn!(retry_seconds = reconciliation_delay(failures), "background payment reconciliation unavailable; backing off without releasing payment reservations");
                            }
                        }
                    }
                    tokio::select! { _ = stopped.cancelled()=>break, _ = tokio::time::sleep(Duration::from_secs(reconciliation_delay(failures)))=>{} }
                }
            });
        }
        let mut tasks = JoinSet::new();
        #[cfg(test)]
        let mut listener_failure = self.listener_failure;
        for (name, listener, app) in self.listeners {
            let stopped = stop.clone();
            #[cfg(test)]
            let failure = if listener_failure.as_ref().is_some_and(|(id, _)| id == &name) {
                listener_failure.take().map(|(_, signal)| signal)
            } else {
                None
            };
            tasks.spawn(async move {
                let serving =
                    axum::serve(listener, app).with_graceful_shutdown(stopped.cancelled_owned());
                #[cfg(test)]
                let serving = async move {
                    match failure {
                        Some(signal) => tokio::select! {
                            result = serving => result,
                            _ = signal => Err(std::io::Error::other("injected listener failure")),
                        },
                        None => serving.await,
                    }
                };
                serving
                    .await
                    .with_context(|| format!("server {name} stopped unexpectedly"))
            });
        }
        let mut result = tokio::select! {
            _ = shutdown.cancelled() => Ok(()),
            _ = stop.cancelled() => Err(anyhow::anyhow!("treasury or funding worker stopped")),
            ended = tasks.join_next() => match ended {
                Some(Ok(Err(e))) => Err(e),
                Some(Err(e)) => Err(e.into()),
                _ => Err(anyhow::anyhow!("MCP listener exited unexpectedly")),
            }
        };
        crate::server::log_http_shutdown();
        if let Some(engine) = &crate::network::global().cover {
            engine.stop_ranges().await;
        }
        stop.cancel();
        if tokio::time::timeout(crate::server::SHUTDOWN_TIMEOUT, async {
            while tasks.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            result = Err(anyhow::anyhow!(crate::server::SHUTDOWN_TIMEOUT_MESSAGE));
        }
        #[cfg(feature = "zcash")]
        while reconciliation.join_next().await.is_some() {}
        #[cfg(feature = "zcash")]
        if let Some(task) = funding_task {
            // Drop the worker's StoreHandle before waiting for treasury close.
            task.await.context("funding worker failed")??;
        }
        #[cfg(feature = "zcash")]
        if let Some(task) = treasury_task {
            task.await.context("treasury worker failed")??;
        }
        if let Some(engine) = &crate::network::global().cover {
            engine.emit_summary();
        }
        result
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[tokio::test]
    async fn failed_listener_stops_siblings_and_releases_registry_ownership() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("spec.json"),
            r#"{"paths":{"/test":{"get":{}}}}"#,
        )
        .unwrap();
        let path = dir.path().join("servers.toml");
        std::fs::write(
            &path,
            r#"
version = 1
[sources.test]
spec = "spec.json"
base_url = "https://example.com"
probe_pricing = false
[wallets.shared]
mode = "static"
private_key_env = "KEY"
[source_management]
wallet = "shared"
registry_file = "registry.sqlite"
[servers.one]
listen = "127.0.0.1:0"
sources = ["test"]
wallet = "shared"
bearer_token_env = "TOKEN"
[servers.two]
listen = "127.0.0.1:0"
sources = ["test"]
wallet = "shared"
bearer_token_env = "TOKEN"
"#,
        )
        .unwrap();
        let env = BTreeMap::from([
            ("KEY".into(), format!("{:064x}", 1)),
            ("TOKEN".into(), "fixture".into()),
        ]);
        let deployment = Deployment::load(&path).await.unwrap();
        let mut running = deployment.bind(&env).await.unwrap();
        let addresses = running.addresses();
        let (send, signal) = tokio::sync::oneshot::channel();
        running.listener_failure = Some(("one".into(), signal));
        let task = tokio::spawn(running.serve(CancellationToken::new()));
        // Both real listeners have entered serving before failure injection.
        for (_, address) in &addresses {
            let http =
                crate::network::discovery(&format!("http://{address}"), Duration::from_secs(5))
                    .unwrap();
            assert_eq!(
                http.post(format!("http://{address}/mcp"))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                401
            );
        }
        send.send(()).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(format!("{error:#}").contains("injected listener failure"));
        for (_, address) in addresses {
            TcpListener::bind(address).await.unwrap();
        }
        // Rebinding also reacquires the same registry's exclusive ownership.
        drop(
            Deployment::load(&path)
                .await
                .unwrap()
                .bind(&env)
                .await
                .unwrap(),
        );
    }
}

// Read-only background polls back off after failures; paid calls still fail closed.
#[cfg(feature = "zcash")]
fn reconciliation_delay(failures: u32) -> u64 {
    if failures == 0 {
        5
    } else {
        15u64.saturating_mul(1u64 << failures.min(5)).min(300)
    }
}
#[cfg(all(test, feature = "zcash"))]
#[test]
fn reconciliation_backoff_is_bounded_and_resets_after_success() {
    assert_eq!(
        (0..7).map(reconciliation_delay).collect::<Vec<_>>(),
        vec![5, 30, 60, 120, 240, 300, 300]
    );
    assert_eq!(reconciliation_delay(u32::MAX), 300);
    assert_eq!(reconciliation_delay(0), 5);
}
