//! Resolve catalogs (optionally through a paid relay), then bind authenticated listeners.
#[cfg(test)]
use crate::mcp_wire::McpResponse;
use crate::{
    catalog::{self, Config, ToolSpec},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, http_app_with_auth},
};
use anyhow::{Context, Result, bail, ensure};
pub mod catalog_evidence;
mod startup;
pub use startup::StartupConfig;
#[cfg(feature = "zcash")]
mod bootstrap;
#[cfg(feature = "zcash")]
pub use bootstrap::{BootstrapSummary, bootstrap_wallets};
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
    #[serde(default)]
    pub optional_startup: bool,
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
    #[serde(default)]
    pub source_management: bool,
    /// Advertise discovery wrappers instead of every API schema.
    #[serde(default)]
    pub discover_on_demand: bool,
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
#[derive(Clone)]
struct Source {
    config: Config,
    document: std::sync::Arc<serde_json::Value>,
    tools: Vec<ToolSpec>,
    base_url: String,
    instructions: Option<String>,
}
#[derive(Serialize)]
pub struct Inventory {
    pub discover_on_demand: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unavailable_sources: Vec<String>,
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
    prebound: BTreeMap<String, TcpListener>,
    unavailable_sources: BTreeMap<String, String>,
    pending_sources: BTreeMap<String, startup::PendingSource>,
    defer_discovery: bool,
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
    pub(crate) fn validate(&self) -> Result<Resolution> {
        self.validate_capabilities(false)
    }
    fn validate_capabilities(&self, shared_only: bool) -> Result<Resolution> {
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
        self.validate_declarations()?;
        self.validate_listeners()?;
        let resolution = assignment::resolve(self)?;
        self.validate_resolved_funding(&resolution, shared_only)?;
        Ok(resolution)
    }

    fn validate_declarations(&self) -> Result<()> {
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
        Ok(())
    }

    fn validate_listeners(&self) -> Result<()> {
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
                !server.sources.is_empty() || server.source_management,
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
        Ok(())
    }

    fn validate_resolved_funding(&self, resolution: &Resolution, shared_only: bool) -> Result<()> {
        if resolution.wallets.values().any(WalletConfig::managed) {
            let treasury = self
                .treasury
                .as_ref()
                .context("managed profiles require [treasury]")?;
            if shared_only {
                treasury.validate_shared()?;
            } else {
                treasury.validate()?;
            }
            let funding = self
                .funding
                .as_ref()
                .context("managed profiles require [funding]")?;
            if !shared_only {
                funding.validate()?;
            }
        } else {
            if let Some(t) = &self.treasury {
                if shared_only {
                    t.validate_shared()?;
                } else {
                    t.validate()?;
                }
            }
            if let Some(f) = &self.funding
                && !shared_only
            {
                f.validate()?;
            }
        }
        Ok(())
    }
}
#[cfg(feature = "zcash")]
async fn install_funding_restrictions(
    store: &crate::rotation::store::StoreHandle,
    restriction: crate::rotation::restriction::FundingRestriction,
    auto_fund: bool,
) -> Result<()> {
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
    if !auto_fund {
        store
            .call(|s| {
                s.pause_new_funding();
                Ok(())
            })
            .await?;
    }
    Ok(())
}

fn validate_selected_cover(
    wallet_resolution: &Resolution,
    sources: &BTreeMap<String, Source>,
    selected: &BTreeMap<String, Vec<(String, ToolSpec)>>,
) -> Result<()> {
    let mut cover_owners = BTreeMap::new();
    for (server, tools) in selected {
        for (id, tool) in tools {
            let source = &sources[id];
            if tool.help_url.is_some() {
                continue;
            }
            if let Some(cover) = &source.config.cover_traffic {
                let route = if tool.path.starts_with("https://") || tool.path.starts_with("http://")
                {
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
                        .read_timeout(
                            source
                                .config
                                .read_timeout_seconds
                                .map(Duration::from_secs_f64),
                        )
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
    Ok(())
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
        let discovery_wallets = config
            .discovery_relay
            .as_ref()
            .map(|policy| {
                crate::discovery_relay::resolve_wallets(
                    policy,
                    &config,
                    &wallet_resolution,
                    config.sources.keys().cloned(),
                )
            })
            .transpose()?
            .unwrap_or_default();

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
            resolved_source["optional_startup"] = serde_json::json!(source.optional_startup);
            resolved_source["wallet"] = serde_json::to_value(&source.wallet)?;
            resolved.insert(id, resolved_source);
        }
        Ok(
            serde_json::json!({"discovery_wallets":discovery_wallets,"discovery_relay":config.discovery_relay,"startup":config.startup,"source_management":crate::discovery::policy::inspection(&config),"network":config.network.inspection(),"version":config.version,"treasury":config.treasury,"treasury_identity":config.treasury.as_ref().map(|t| if t.id.is_empty() { "from_wallet_state_at_runtime" } else { "explicit_expected_id" }),"funding":config.funding,"base_rpc_policy":config.funding.as_ref().map(|f| f.base_rpc_policy()),"sources":resolved,"wallets":config.wallets,"servers":config.servers,"wallet_bindings":wallet_resolution.bindings,"wallet_templates":config.wallet_templates,"wallet_assignment":config.wallet_assignment,"resolved_wallets":wallet_resolution.wallets,"generated_wallets":wallet_resolution.generated,"wallet_summary":wallet_resolution.summary}),
        )
    }
    pub async fn load(path: &Path) -> Result<Self> {
        Self::load_with_warnings(path, false, None).await
    }
    /// Warn for listener-bound sources before catalog I/O, including failed fetches.
    /// Keyless serving keeps provider warnings but cannot bootstrap or pay a relay.
    pub async fn load_for_unsigned_serving(path: &Path) -> Result<Self> {
        Self::load_with_warnings(path, true, None).await
    }
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
    fn validate_listener_credentials(&self, env: &BTreeMap<String, String>) -> Result<()> {
        for server in self.config.servers.values().filter(|s| s.auth) {
            required_secret(env, &server.bearer_token_env)?;
        }
        Ok(())
    }

    async fn validate_serving_inputs(
        &self,
        path: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<()> {
        // Validate credentials and authored providers before funding can start.
        self.validate_listener_credentials(env)?;
        for source in self.config.sources.values() {
            crate::config::resolve(source.provider.clone(), path).await?;
        }
        Ok(())
    }

    async fn prebind_listeners(&mut self) -> Result<()> {
        let mut sockets = BTreeMap::new();
        for (name, cfg) in &self.config.servers {
            let socket = TcpListener::bind(cfg.listen)
                .await
                .with_context(|| format!("server {name}: cannot bind {}", cfg.listen))?;
            sockets.insert(name.clone(), socket);
        }
        self.prebound = sockets;
        Ok(())
    }

    async fn initialize_discovery_relay(
        &mut self,
        path: &Path,
        relay_env: Option<(
            &BTreeMap<String, String>,
            crate::rotation::restriction::FundingRestriction,
        )>,
    ) -> Result<crate::discovery_relay::Relays> {
        Ok(
            if let Some(policy) = self.config.discovery_relay.clone().filter(|p| p.serve)
                && let Some((env, restriction)) = relay_env
            {
                ensure!(
                    !crate::qualification::active() && !catalog_evidence::collecting(),
                    "paid discovery relay is unavailable during qualification"
                );
                // Validate listener credentials before discovery can spend.
                self.validate_listener_credentials(env)?;
                let bindings = crate::discovery_relay::resolve_wallets(
                    &policy,
                    &self.config,
                    &self.wallet_resolution,
                    self.config.sources.keys().cloned(),
                )?;
                let needed = bindings.values().cloned().collect();
                if self.initialized.is_none() {
                    self.initialized = Some(
                        self.initialize_wallets_mode(env, restriction, None, &needed, true)
                            .await?,
                    );
                }
                crate::discovery_relay::build(
                    &policy,
                    path,
                    &bindings,
                    &self
                        .initialized
                        .as_ref()
                        .expect("initialized discovery wallets")
                        .wallets,
                )
                .await?
            } else {
                Default::default()
            },
        )
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
        let scoped_funding = relay_env.is_some()
            && !crate::qualification::active()
            && !catalog_evidence::collecting();
        let wallet_resolution = config.validate_capabilities(scoped_funding)?;
        crate::network::install(config.network.clone())?;
        if let Some(t) = &mut config.treasury {
            t.resolve(path);
        }
        crate::discovery::policy::validate_registry_path(&config, path)?;
        let mut deployment = Self {
            prebound: BTreeMap::new(),
            unavailable_sources: BTreeMap::new(),
            pending_sources: BTreeMap::new(),
            defer_discovery: relay_env.is_some()
                && !crate::qualification::active()
                && !catalog_evidence::collecting(),
            initialized: None,
            config,
            wallet_resolution,
            sources: BTreeMap::new(),
            selected: BTreeMap::new(),
            config_path: path.to_owned(),
        };
        if let Some((env, _)) = relay_env
            && deployment
                .config
                .discovery_relay
                .as_ref()
                .is_some_and(|p| p.serve)
        {
            ensure!(
                !crate::qualification::active() && !catalog_evidence::collecting(),
                "paid discovery relay is unavailable during qualification"
            );
            deployment.validate_serving_inputs(path, env).await?;
            deployment.prebind_listeners().await?;
        }
        if let Some((env, _)) = relay_env
            && deployment.prebound.is_empty()
        {
            deployment.validate_serving_inputs(path, env).await?;
        }
        let loaded: Result<()> = async {
            let relay = deployment
                .initialize_discovery_relay(path, relay_env)
                .await?;
            let config = &deployment.config;
            let wallet_resolution = &deployment.wallet_resolution;
            let (sources, unavailable_sources, pending_sources) =
                startup::load(config, path, warn, relay, deployment.defer_discovery).await?;
            let mut selected = BTreeMap::new();
            for (name, server) in &config.servers {
                let tools = select_listener_tools(name, server, &sources, &unavailable_sources)?;
                selected.insert(name.clone(), tools);
            }
            validate_selected_cover(wallet_resolution, &sources, &selected)?;
            deployment.unavailable_sources = unavailable_sources;
            deployment.pending_sources = pending_sources;
            deployment.sources = sources;
            deployment.selected = selected;
            Ok(())
        }
        .await;
        if let Err(error) = loaded {
            #[cfg(feature = "zcash")]
            if let Some(initialized) = deployment.initialized.take() {
                initialized.shutdown().await?;
            }
            return Err(error);
        }
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
                discover_on_demand: s.discover_on_demand,
                unavailable_sources: s
                    .sources
                    .iter()
                    .filter(|id| self.unavailable_sources.contains_key(*id))
                    .cloned()
                    .collect(),
                server: name.clone(),
                listen: s.listen,
                default_wallet: s.wallet.clone(),
                wallet_bindings: self.wallet_resolution.bindings[name].clone(),
                management_tools: crate::discovery::tools::listener_definitions(
                    s.source_management,
                    s.discover_on_demand,
                ),
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
    async fn validate_static_identities(
        &self,
        only_wallets: Option<&BTreeSet<String>>,
    ) -> Result<()> {
        // Static-only serving does not unlock state, but must not reuse a known
        // managed profile name as a different wallet identity.
        if !self.wallet_resolution.wallets.iter().any(|(name, wallet)| {
            only_wallets.is_none_or(|selected| selected.contains(name)) && wallet.managed()
        }) && let Some(t) = &self.config.treasury
            && t.state_dir.exists()
        {
            let dir = t.state_dir.clone();
            let (treasury_id, pool_names) =
                tokio::task::spawn_blocking(move || crate::rotation::store::static_ownership(&dir))
                    .await??;
            ensure!(
                t.id.is_empty() || treasury_id == t.id,
                "state identity mismatch"
            );
            for name in pool_names {
                ensure!(
                    !self.wallet_resolution.wallets.contains_key(&name),
                    "managed pool {} cannot become static without explicit retirement",
                    name
                );
            }
        }
        Ok(())
    }

    fn pending_wallets(&self) -> BTreeSet<String> {
        self.config
            .servers
            .iter()
            .flat_map(|(name, listener)| {
                listener
                    .sources
                    .iter()
                    .filter(|id| self.pending_sources.contains_key(*id))
                    .map(|id| self.wallet_resolution.bindings[name][id].wallet.clone())
            })
            .collect()
    }

    fn static_wallets(
        &self,
        env: &BTreeMap<String, String>,
        only_wallets: Option<&BTreeSet<String>>,
        relay_wallets: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, PaidClient>> {
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
        used_wallets.extend(self.pending_wallets());
        used_wallets.extend(crate::discovery::policy::wallets(&self.config));
        used_wallets.extend(relay_wallets.iter().cloned());
        used_wallets.retain(|name| only_wallets.is_none_or(|selected| selected.contains(name)));
        for name in used_wallets {
            if let WalletConfig::Static {
                private_key_env,
                max_api_payment_usdc,
            } = &self.wallet_resolution.wallets[&name]
            {
                let payer = Payer::new(
                    &required_secret(env, private_key_env)?,
                    SpendPolicy::dollars(max_api_payment_usdc)?,
                )
                .with_context(|| format!("wallet {name}: invalid signing configuration"))?;
                wallets.insert(name, PaidClient::new(payer));
            }
        }
        Ok(wallets)
    }

    #[cfg(feature = "zcash")]
    fn managed_connection_preflight(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<(FundingConnections, crate::rotation::base::BaseRpc)> {
        Ok((
            self.funding_connection_preflight(env)?,
            self.base_connection_preflight(env)?,
        ))
    }
    #[cfg(feature = "zcash")]
    fn funding_connection_preflight(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<FundingConnections> {
        use crate::rotation::base::secure_endpoint;
        let t = self.config.treasury.as_ref().unwrap();
        let f = self.config.funding.as_ref().unwrap();
        t.validate()?;
        f.validate()?;
        // Resolve source dependencies before any funding worker starts.
        let sync_settings = crate::treasury::SyncSettings::new(
            t.indexer_endpoint(|name| env.get(name).cloned())?,
            t.confirmations,
            t.max_sync_age_seconds,
        )?;
        secure_endpoint(&t.submission_endpoint(|name| env.get(name).cloned())?)?;
        let key = f
            .near_api_key_env
            .as_deref()
            .map(|name| required_secret(env, name))
            .transpose()?;
        let session = f
            .near_user_session_env
            .as_deref()
            .map(|name| required_secret(env, name))
            .transpose()?;
        // Presence and header syntax are source-capability prerequisites.
        let near =
            crate::rotation::near::NearClient::with_session(key.as_deref(), session.as_deref())?;
        let sender = crate::treasury::submission::GrpcSubmission::new(
            t.submission_endpoint(|name| env.get(name).cloned())?,
            t.indexer_endpoint(|name| env.get(name).cloned())?,
        )?;
        Ok(FundingConnections {
            sync_settings,
            near,
            sender,
        })
    }
    #[cfg(feature = "zcash")]
    fn base_connection_preflight(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<crate::rotation::base::BaseRpc> {
        use crate::rotation::base::BaseRpc;
        let f = self.config.funding.as_ref().unwrap();
        f.validate_base()?;
        let urls = f.base_rpc_urls(|name| env.get(name).cloned())?;
        if self.config.network.mode == crate::network::Mode::Direct {
            tracing::warn!(
                provider_count = urls.len(),
                default_fallbacks = f.base_rpc_fallback_url_envs.is_none(),
                "Direct Base RPC verification exposes your client IP address and queried wallet addresses to each contacted RPC provider, including fallbacks; see config show base_rpc_policy"
            );
        }
        let base =
            BaseRpc::with_fallbacks(&urls, f.base_confirmations, f.base_max_block_age_seconds)?;
        Ok(base)
    }

    async fn initialize_wallets(
        &self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
        only_wallets: Option<&BTreeSet<String>>,
        relay_wallets: &BTreeSet<String>,
    ) -> Result<InitializedWallets> {
        self.initialize_wallets_mode(env, restriction, only_wallets, relay_wallets, false)
            .await
    }

    async fn initialize_wallets_mode(
        &self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
        only_wallets: Option<&BTreeSet<String>>,
        relay_wallets: &BTreeSet<String>,
        allow_store_only: bool,
    ) -> Result<InitializedWallets> {
        #[cfg(not(feature = "zcash"))]
        let _ = (restriction, allow_store_only);
        self.validate_static_identities(only_wallets).await?;
        let mut wallets = self.static_wallets(env, only_wallets, relay_wallets)?;
        #[cfg(feature = "zcash")]
        let mut treasury = None;
        #[cfg(feature = "zcash")]
        let mut store_only = None;
        #[cfg(feature = "zcash")]
        let mut funding_runtime = None;
        #[cfg(feature = "zcash")]
        let mut managed_pools = Vec::new();
        if self.wallet_resolution.wallets.iter().any(|(name, wallet)| {
            only_wallets.is_none_or(|selected| selected.contains(name)) && wallet.managed()
        }) {
            #[cfg(not(feature = "zcash"))]
            bail!("managed serving requires a build with --features zcash");
            #[cfg(feature = "zcash")]
            {
                use crate::rotation::manager::ManagedPool;
                let t = self.config.treasury.as_ref().unwrap();
                let f = self.config.funding.as_ref().unwrap();
                let (mut owner, base, mut source) = if allow_store_only {
                    let shared = crate::treasury::Treasury::open_shared_state(
                        t.state_dir.clone(),
                        t.key_file.clone(),
                        t.runtime_id()?,
                        crate::rotation::store::TreasuryNetwork::Mainnet,
                    )
                    .await?;
                    let base = self.base_connection_preflight(env);
                    match base {
                        Err(_) => {
                            tracing::warn!(
                                category = "base_configuration_unavailable",
                                "Managed Base configuration unavailable; independent static service remains available; correct Base configuration and restart"
                            );
                            store_only = Some(StoreOnlyOwner::new(shared));
                            (None, None, None)
                        }
                        Ok(base) => match self.funding_connection_preflight(env) {
                            Ok(source) => match crate::treasury::Treasury::reconstruct(shared)
                                .await?
                            {
                                crate::treasury::OpenedTreasury::Ready(owner) => {
                                    (Some(*owner), Some(base), Some(source))
                                }
                                crate::treasury::OpenedTreasury::SnapshotUnavailable(shared) => {
                                    tracing::warn!(
                                        category = "treasury_snapshot_unavailable",
                                        "Treasury SDK unavailable; Base service retains shared ownership; funding unavailable"
                                    );
                                    store_only = Some(StoreOnlyOwner::new(*shared));
                                    (None, Some(base), None)
                                }
                            },
                            Err(_) => {
                                tracing::warn!(
                                    category = "funding_configuration_unavailable",
                                    "Funding configuration unavailable; Base service retains shared ownership; correct funding configuration and restart"
                                );
                                store_only = Some(StoreOnlyOwner::new(shared));
                                (None, Some(base), None)
                            }
                        },
                    }
                } else {
                    let (source, base) = self.managed_connection_preflight(env)?;
                    let owner = crate::treasury::Treasury::open(
                        t.state_dir.clone(),
                        t.key_file.clone(),
                        t.runtime_id()?,
                    )
                    .await?;
                    (Some(owner), Some(base), Some(source))
                };
                if let (Some(owner), Some(source)) = (&mut owner, &source) {
                    owner.configure_sync(source.sync_settings.clone());
                }
                let store = match &owner {
                    Some(owner) => owner.store_handle(),
                    None => store_only
                        .as_ref()
                        .expect("retained shared owner")
                        .store
                        .clone(),
                };
                let setup: Result<()> = async {
                install_funding_restrictions(&store, restriction, f.auto_fund && owner.is_some())
                    .await?;
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
                    if only_wallets.is_some_and(|selected| !selected.contains(name)) {
                        continue;
                    }
                    if let WalletConfig::ZcashRotation {
                        reserves,
                        max_api_payment_usdc,
                        max_retirement_remainder_usdc,
                        ..
                    } = w
                    {
                        let policy = SpendPolicy::dollars(max_api_payment_usdc)?;
                        let group_name = name.clone();
                        let reserves = reserves.clone();
                        let retirement = crate::rotation::config::retirement_remainder(
                            max_retirement_remainder_usdc,
                        )?;
                        let cap = policy.max_atomic;
                        let pool = store
                            .call(move |s| {
                                s.ensure_capacity_group(&group_name, &reserves, cap, retirement)
                            })
                            .await?;
                        let client = if let Some(base) = &base {
                            let manager = std::sync::Arc::new(ManagedPool::new(store.clone(), pool, base.clone(), policy)?);
                            managed_pools.push(manager.clone());
                            PaidClient::managed(manager)
                        } else {
                            PaidClient::unavailable(crate::rotation::error::AdmissionError::WalletNotReady(
                                "managed Base configuration unavailable; correct configuration and restart"))
                        };
                        wallets.insert(name.clone(), client);
                    }
                }
                if owner.is_some() {
                    let source = source.take().expect("validated funding connections");
                    tracing::info!(target: "x402_treazury::startup", auto_fund = f.auto_fund,
                    "{}", if f.auto_fund { "automatic managed funding enabled: bootstrap and replacements are subject to actual spendable treasury funds, ZEC/fee safeguards and any qualification restrictions" } else { "automatic managed funding disabled: new bootstrap and replacement transfers are paused" });
                    let (handle, commands) = crate::treasury::actor::channel();
                    let backend = crate::rotation::funding::Backend {
                        treasury: handle,
                        store: store.clone(),
                        near: source.near,
                        base: base.expect("validated Base connection"),
                        wallets: self.wallet_resolution.wallets.clone(),
                        funding: f.clone(),
                        max_network_fee: crate::rotation::config::zatoshis(
                            &t.max_funding_transaction_fee_zec,
                        )? as u64,
                    };
                    funding_runtime = Some((
                        commands,
                        source.sender,
                        crate::rotation::funding::FundingWorker::new(
                            store.clone(),
                            backend,
                            f.poll_seconds,
                        ),
                    ));
                    treasury = owner.take();
                }
                    Ok(())
                }.await;
                if let Err(error) = setup {
                    drop(funding_runtime);
                    drop(managed_pools);
                    drop(wallets);
                    drop(store);
                    if let Some(owner) = owner {
                        owner.close().await?;
                    }
                    if let Some(owner) = store_only {
                        owner.close().await?;
                    }
                    return Err(error);
                }
            }
        }
        Ok(InitializedWallets {
            wallets,
            #[cfg(feature = "zcash")]
            treasury,
            #[cfg(feature = "zcash")]
            store_only,
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
                            .with_source_identity(id)
                            .with_transport(source.config.transport())
                            .with_cover(
                                source.config.cover_traffic.clone(),
                                crate::cover::Scope {
                                    listener: name.clone(),
                                    source: id.clone(),
                                },
                            )
                            .with_timeout(
                                source
                                    .config
                                    .read_timeout_seconds
                                    .map(Duration::from_secs_f64),
                            )
                            .with_download_limits(
                                source.config.max_response_bytes,
                                source.config.max_help_bytes,
                            ),
                        source.base_url.clone(),
                    )
                })
                .collect();
            let used: BTreeSet<_> = selected.iter().map(|(id, _)| id).collect();
            let mut instructions = used
                .into_iter()
                .filter_map(|id| {
                    self.sources[id]
                        .instructions
                        .as_ref()
                        .map(|s| format!("[{id}]\n{s}"))
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            for id in &cfg.sources {
                if self.unavailable_sources.contains_key(id) {
                    if self.pending_sources.contains_key(id) {
                        instructions.push_str(&format!("\n\nSource {id}: PENDING (catalog discovery is waiting). Its tools will appear when discovery succeeds."));
                    } else {
                        instructions.push_str(&format!("\n\nSource {id}: UNAVAILABLE (catalog discovery failed). Its tools are unavailable, not intentionally empty. Restart to retry."));
                    }
                }
            }
            let mut server = Server::from_bindings(
                bindings,
                (!instructions.is_empty()).then_some(instructions),
                cfg.max_response_chars,
            );
            server.name = name.clone();
            server.discover_on_demand = cfg.discover_on_demand;
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

    async fn binding_wallets(
        &mut self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
        unsigned: bool,
    ) -> Result<(BTreeMap<String, Option<String>>, InitializedWallets)> {
        // Validate every credential before opening any port.
        let mut tokens = BTreeMap::new();
        for (name, cfg) in &self.config.servers {
            tokens.insert(
                name.clone(),
                if cfg.auth {
                    Some(required_secret(env, &cfg.bearer_token_env)?)
                } else {
                    None
                },
            );
        }
        let initialized = if unsigned {
            self.unsigned_wallets()?
        } else if self.initialized.is_some() {
            self.complete_reused_wallets(env, restriction).await?;
            self.initialized
                .take()
                .expect("initialized serving wallets")
        } else {
            self.initialize_wallets_mode(
                env,
                restriction,
                None,
                &BTreeSet::new(),
                !crate::qualification::active() && !catalog_evidence::collecting(),
            )
            .await?
        };
        Ok((tokens, initialized))
    }

    fn unsigned_wallets(&self) -> Result<InitializedWallets> {
        ensure!(
            self.config.source_management.is_none()
                && self.config.servers.values().all(|s| !s.source_management),
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
        Ok(InitializedWallets {
            wallets: self
                .wallet_resolution
                .wallets
                .keys()
                .map(|name| (name.clone(), PaidClient::unsigned()))
                .collect(),
            #[cfg(feature = "zcash")]
            treasury: None,
            #[cfg(feature = "zcash")]
            store_only: None,
            #[cfg(feature = "zcash")]
            funding_runtime: None,
            #[cfg(feature = "zcash")]
            managed_pools: Vec::new(),
        })
    }

    async fn complete_reused_wallets(
        &mut self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
    ) -> Result<()> {
        let secret = |key: &str| required_secret(env, key);
        let pending_wallets = self.pending_wallets();
        let initialized = self
            .initialized
            .as_mut()
            .expect("initialized serving wallets");
        #[cfg(feature = "zcash")]
        if restriction == crate::rotation::restriction::FundingRestriction::DenyNewFunding
            && let Some(store) = initialized
                .treasury
                .as_ref()
                .map(|treasury| treasury.store_handle())
                .or_else(|| {
                    initialized
                        .store_only
                        .as_ref()
                        .map(|owner| owner.store.clone())
                })
        {
            store
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
        for name in needed.into_iter().chain(pending_wallets) {
            if !initialized.wallets.contains_key(&name)
                && let WalletConfig::Static {
                    private_key_env,
                    max_api_payment_usdc,
                } = &self.wallet_resolution.wallets[&name]
            {
                let payer = Payer::new(
                    &secret(private_key_env)?,
                    SpendPolicy::dollars(max_api_payment_usdc)?,
                )
                .with_context(|| format!("wallet {name}: invalid signing configuration"))?;
                initialized.wallets.insert(name, PaidClient::new(payer));
            }
        }
        Ok(())
    }

    async fn bind_catalog_listeners(
        &mut self,
        wallets: BTreeMap<String, PaidClient>,
        mut tokens: BTreeMap<String, Option<String>>,
    ) -> Result<BoundListeners> {
        if !self.defer_discovery {
            self.discover_prices().await?;
        }
        let mut servers = self.build_servers(&wallets);
        let mut snapshot = crate::catalog_state::CatalogSnapshot::default();
        for (name, server) in &servers {
            let tools = server.catalog.read().views["default"].clone();
            if self.config.source_management.is_some()
                || self.config.servers[name].discover_on_demand
            {
                ensure!(
                    tools
                        .iter()
                        .all(|t| !t.tool.name.starts_with("x402_treazury_")
                            && !t.tool.name.starts_with("dyn_")),
                    "static tool uses reserved namespace"
                );
            }
            snapshot
                .instructions
                .insert(name.clone(), server.instructions.clone());
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
        let mut prebound = std::mem::take(&mut self.prebound);
        let reuse_sockets = !prebound.is_empty();
        ensure!(
            !reuse_sockets || prebound.keys().eq(self.config.servers.keys()),
            "prebound listener inventory changed"
        );
        let mut listeners = Vec::new();
        let mut accepted_work = Vec::new();
        for (name, cfg) in &self.config.servers {
            let mut server = servers.remove(name).unwrap();
            server.catalog = catalog.clone();
            server.catalog_server = name.clone();
            server.host_policy = crate::server::host::HostPolicy::new(
                cfg.allowed_hosts.clone(),
                cfg.disable_host_check,
            )?;
            if cfg.source_management {
                manager
                    .as_ref()
                    .context("source management unavailable")?
                    .directory(name)
                    .with_context(|| {
                        format!("server {name}: directory search wallet unavailable")
                    })?;
            }
            server.discovery = manager.clone();
            let listener = if reuse_sockets {
                prebound.remove(name).context("prebound listener missing")?
            } else {
                TcpListener::bind(cfg.listen)
                    .await
                    .with_context(|| format!("server {name}: cannot bind {}", cfg.listen))?
            };
            accepted_work.push(server.work.clone());
            listeners.push((
                name.clone(),
                listener,
                http_app_with_auth(server, tokens.remove(name).unwrap()),
            ));
        }
        Ok(BoundListeners {
            listeners,
            accepted_work,
            catalog,
            manager,
        })
    }

    async fn bind_mode(
        mut self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
        unsigned: bool,
    ) -> Result<RunningDeployment> {
        let setup = self.binding_wallets(env, restriction, unsigned).await;
        let (tokens, initialized) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                #[cfg(feature = "zcash")]
                if let Some(initialized) = self.initialized.take() {
                    // Source relay clients retain store handles. Release them
                    // before shutdown waits for the treasury worker to exit.
                    drop(self);
                    initialized.shutdown().await?;
                }
                return Err(error);
            }
        };
        let wallets = initialized.wallets;
        #[cfg(feature = "zcash")]
        let treasury = initialized.treasury;
        #[cfg(feature = "zcash")]
        let store_only = initialized.store_only;
        #[cfg(feature = "zcash")]
        let funding_runtime = initialized.funding_runtime;
        #[cfg(feature = "zcash")]
        let managed_pools = initialized.managed_pools;
        let deferred_wallets = wallets.clone();
        let bound = self.bind_catalog_listeners(wallets, tokens).await;
        let BoundListeners {
            listeners,
            accepted_work,
            catalog,
            manager,
        } = match bound {
            Ok(bound) => bound,
            Err(error) => {
                drop(deferred_wallets);
                #[cfg(feature = "zcash")]
                {
                    drop(funding_runtime);
                    drop(managed_pools);
                    drop(self);
                    if let Some(treasury) = treasury {
                        treasury.close().await?;
                    }
                    if let Some(owner) = store_only {
                        owner.close().await?;
                    }
                }
                return Err(error);
            }
        };
        let deferred = self
            .defer_discovery
            .then(|| deferred::Runtime::new(self, deferred_wallets, catalog, manager));
        Ok(RunningDeployment {
            deferred,
            listeners,
            accepted_work,
            #[cfg(test)]
            listener_failure: None,
            #[cfg(test)]
            funding_failure: None,
            #[cfg(feature = "zcash")]
            treasury,
            #[cfg(feature = "zcash")]
            store_only,
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
    unavailable: &BTreeMap<String, String>,
) -> Result<Vec<(String, ToolSpec)>> {
    let mut available = BTreeMap::new();
    let incomplete = server.sources.iter().any(|id| unavailable.contains_key(id));
    for source in &server.sources {
        if unavailable.contains_key(source) {
            continue;
        }
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
                    } else if selector_has_unavailable_owner(text, server, sources, unavailable) {
                        tracing::warn!(server = name, selector = text, "Tool selector belongs to an unavailable source; validate when its catalog becomes available");
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
        !tools.is_empty() || server.source_management || incomplete,
        "server {name}: no tools selected"
    );
    Ok(tools)
}

fn selector_has_unavailable_owner(
    selector: &str,
    server: &ListenerConfig,
    sources: &BTreeMap<String, Source>,
    unavailable: &BTreeMap<String, String>,
) -> bool {
    // Prefixes come from the resolved provider (including inherited/custom
    // prefixes), not the source ID. Overlapping namespaces cannot establish
    // ownership: do not hide a required source's typo behind an optional outage.
    let owners: Vec<_> = server
        .sources
        .iter()
        .filter(|id| {
            let prefix = unavailable
                .get(*id)
                .map(String::as_str)
                .or_else(|| sources.get(*id).and_then(|s| s.config.prefix.as_deref()));
            prefix.is_some_and(|prefix| {
                selector
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('_'))
            })
        })
        .collect();
    !owners.is_empty() && owners.iter().all(|id| unavailable.contains_key(*id))
}

fn required_secret(env: &BTreeMap<String, String>, key: &str) -> Result<String> {
    env.get(key)
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .with_context(|| format!("required environment variable {key} is missing or empty"))
}

struct BoundListeners {
    catalog: std::sync::Arc<crate::catalog_state::CatalogState>,
    manager: Option<std::sync::Arc<crate::discovery::Manager>>,
    listeners: Vec<(String, TcpListener, axum::Router)>,
    accepted_work: Vec<crate::server::work::Work>,
}

#[cfg(feature = "zcash")]
struct FundingConnections {
    sync_settings: crate::treasury::SyncSettings,
    near: crate::rotation::near::NearClient,
    sender: crate::treasury::submission::GrpcSubmission,
}

#[cfg(feature = "zcash")]
struct StoreOnlyOwner {
    store: crate::rotation::store::StoreHandle,
    worker: tokio::task::JoinHandle<()>,
}
#[cfg(feature = "zcash")]
impl StoreOnlyOwner {
    fn new(store: crate::rotation::store::Store) -> Self {
        let (store, worker) = crate::rotation::store::StoreHandle::spawn(store);
        Self { store, worker }
    }
    async fn close(self) -> Result<()> {
        drop(self.store);
        self.worker.await.context("store worker failed")
    }
}

struct InitializedWallets {
    wallets: BTreeMap<String, PaidClient>,
    #[cfg(feature = "zcash")]
    treasury: Option<crate::treasury::Treasury>,
    #[cfg(feature = "zcash")]
    store_only: Option<StoreOnlyOwner>,
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
    deferred: Option<deferred::Runtime>,
    accepted_work: Vec<crate::server::work::Work>,
    #[cfg(test)]
    listener_failure: Option<(String, tokio::sync::oneshot::Receiver<()>)>,
    #[cfg(test)]
    funding_failure: Option<tokio::sync::oneshot::Receiver<()>>,
    #[cfg(feature = "zcash")]
    funding_runtime: Option<FundingRuntime>,
    #[cfg(feature = "zcash")]
    managed_pools: Vec<std::sync::Arc<crate::rotation::manager::ManagedPool>>,
    #[cfg(feature = "zcash")]
    treasury: Option<crate::treasury::Treasury>,
    #[cfg(feature = "zcash")]
    store_only: Option<StoreOnlyOwner>,
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
        let reconciliation_stop = CancellationToken::new();
        let _reconciliation_on_drop = reconciliation_stop.clone().drop_guard();
        #[cfg(test)]
        if let Some(failure) = self.funding_failure {
            let stopped = stop.clone();
            tokio::spawn(async move {
                if failure.await.is_ok() {
                    stopped.cancel();
                }
            });
        }
        #[cfg(feature = "zcash")]
        let (treasury_task, funding_task) = match (self.treasury, self.funding_runtime) {
            (Some(owner), Some((commands, sender, worker))) => (
                Some(tokio::spawn({
                    let stopped = stop.clone();
                    async move {
                        let _stop_on_exit = stopped.clone().drop_guard();
                        owner.run_commands(commands, sender, stopped).await
                    }
                })),
                Some(tokio::spawn({
                    let stopped = stop.clone();
                    async move {
                        let _stop_on_exit = stopped.clone().drop_guard();
                        worker.run(stopped).await
                    }
                })),
            ),
            (Some(owner), None) => (
                Some(tokio::spawn({
                    let stopped = stop.clone();
                    async move {
                        let _stop_on_exit = stopped.clone().drop_guard();
                        owner.run_sync(stopped).await
                    }
                })),
                None,
            ),
            (None, _) => (None, None),
        };
        #[cfg(feature = "zcash")]
        let mut reconciliation = JoinSet::new();
        #[cfg(feature = "zcash")]
        for pool in self.managed_pools {
            let stopped = reconciliation_stop.clone();
            reconciliation.spawn(async move {
                if let Err(error) = pool.run_reconciliation(stopped).await {
                    tracing::warn!(category = %crate::rotation::base::safe_diagnostic(&error), "background reconciliation owner stopped; unresolved obligations remain durable");
                }
            });
        }
        let listener_stop = CancellationToken::new();
        let _listeners_on_drop = listener_stop.clone().drop_guard();
        let mut tasks = JoinSet::new();
        #[cfg(test)]
        let mut listener_failure = self.listener_failure;
        for (name, listener, app) in self.listeners {
            let stopped = listener_stop.clone();
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
        let deferred_stop = CancellationToken::new();
        let _deferred_on_drop = deferred_stop.clone().drop_guard();
        let deferred_relays = self
            .deferred
            .as_ref()
            .map(|runtime| runtime.relays())
            .unwrap_or_default();
        let deferred_task = self
            .deferred
            .map(|runtime| tokio::spawn(runtime.run(deferred_stop.clone())));
        let mut funding_stopped = false;
        let result = loop {
            tokio::select! {
                _ = shutdown.cancelled() => break Ok(()),
                _ = stop.cancelled(), if !funding_stopped => {
                    funding_stopped = true;
                    tracing::error!(category = "treasury_funding_stopped",
                        "Treasury/funding stopped; restart required for funding. Listeners and Base reconciliation remain available; payments still require confirmed capacity and retain all liabilities");
                },
                ended = tasks.join_next() => break match ended {
                    Some(Ok(Err(e))) => Err(e),
                    Some(Err(e)) => Err(e.into()),
                    _ => Err(anyhow::anyhow!("MCP listener exited unexpectedly")),
                }
            }
        };
        deferred_stop.cancel();
        crate::server::log_http_shutdown();
        for work in &self.accepted_work {
            work.close();
        }
        listener_stop.cancel();
        if let Some(engine) = &crate::network::global().cover {
            engine.stop_ranges().await;
        }
        // Funding/treasury/reconciliation remain available to already accepted
        // calls. Closing admission prevents a new call from extending this drain.
        tokio::join!(
            crate::server::wait_with_progress("http_listeners", async {
                while tasks.join_next().await.is_some() {}
            }),
            async {
                for work in &self.accepted_work {
                    work.drain().await;
                }
            }
        );
        let (deferred_owner, deferred_result) = match deferred_task {
            Some(task) => match task.await {
                Ok(owner) => (Some(owner), Ok(())),
                Err(error) => (
                    None,
                    Err(anyhow::Error::new(error).context("deferred discovery worker failed")),
                ),
            },
            None => (None, Ok(())),
        };
        for relay in deferred_relays {
            relay.drain().await;
        }
        drop(deferred_owner);
        stop.cancel();
        reconciliation_stop.cancel();
        #[cfg(feature = "zcash")]
        while reconciliation.join_next().await.is_some() {}
        #[cfg(feature = "zcash")]
        {
            // Drop the funding worker's store handle before joining the owner;
            // a stopped owner retains the shared store until serving releases it.
            let funding_result = match funding_task {
                Some(task) => task.await.context("funding worker failed").and_then(|r| r),
                None => Ok(()),
            };
            let treasury_result = match treasury_task {
                Some(task) => task.await.context("treasury worker failed").and_then(|r| r),
                None => Ok(()),
            };
            let store_result = match self.store_only {
                Some(owner) => owner.close().await,
                None => Ok(()),
            };
            funding_result?;
            treasury_result?;
            store_result?;
        }
        if let Some(engine) = &crate::network::global().cover {
            engine.emit_summary();
        }
        result.and(deferred_result)
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[cfg(feature = "zcash")]
    #[tokio::test]
    async fn strict_near_preflight_fails_before_treasury_state_is_opened() {
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
version=1
[treasury]
state_dir="state"
[funding]
auto_fund=false
near_api_key_env="KEY"
near_user_session_env="SESSION"
[wallets.shared]
mode="zcash_rotation"
max_api_payment_usdc="1"
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[wallets.shared.reserves.small]
funding_amount_usdc="5"
[sources.test]
spec="spec.json"
base_url="https://example.com"
probe_pricing=false
[servers.one]
listen="127.0.0.1:0"
wallet="shared"
sources=["test"]
bearer_token_env="TOKEN"
"#,
        )
        .unwrap();
        for (key, session, expected) in [
            (Some("key"), None, "SESSION"),
            (None, Some("session"), "KEY"),
            (Some("key"), Some(""), "SESSION"),
            (Some("bad\nkey"), Some("session"), "invalid NEAR API key"),
            (
                Some("key"),
                Some("bad\rsession"),
                "invalid NEAR user session",
            ),
        ] {
            let mut env = BTreeMap::from([("TOKEN".into(), "fixture".into())]);
            for (name, value) in [("KEY", key), ("SESSION", session)] {
                if let Some(value) = value {
                    env.insert(name.into(), value.into());
                }
            }
            let deployment = Deployment::load(&path).await.unwrap();
            let error = match deployment
                .initialize_wallets(&env, Default::default(), None, &BTreeSet::new())
                .await
            {
                Ok(_) => panic!("invalid credentials accepted"),
                Err(error) => format!("{error:#}"),
            };
            assert!(error.contains(expected), "{error}");
            assert!(!dir.path().join("state").exists());
        }
        let deployment = Deployment::load(&path).await.unwrap();
        let env = BTreeMap::from([
            ("KEY".into(), "key".into()),
            ("SESSION".into(), "session".into()),
        ]);
        deployment.managed_connection_preflight(&env).unwrap();
        assert!(!dir.path().join("state").exists());
    }
    #[cfg(feature = "zcash")]
    #[tokio::test]
    async fn funding_failure_keeps_listeners_and_base_reconciliation_available() {
        use axum::{Json, routing::post};
        use serde_json::{Value, json};
        use std::sync::Arc;
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
version=1
[sources.test]
spec="spec.json"
base_url="https://example.com"
probe_pricing=false
[wallets.shared]
mode="static"
private_key_env="KEY"
[servers.one]
listen="127.0.0.1:0"
sources=["test"]
wallet="shared"
bearer_token_env="TOKEN"
[servers.two]
listen="127.0.0.1:0"
sources=["test"]
wallet="shared"
bearer_token_env="TOKEN"
"#,
        )
        .unwrap();
        let env = BTreeMap::from([
            ("KEY".into(), format!("{:064x}", 1)),
            ("TOKEN".into(), "fixture".into()),
        ]);
        let mut running = Deployment::load(&path)
            .await
            .unwrap()
            .bind(&env)
            .await
            .unwrap();
        let addresses = running.addresses();
        let reads = Arc::new(tokio::sync::Semaphore::new(0));
        let seen = reads.clone();
        let rpc = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc_url = format!("http://{}", rpc.local_addr().unwrap());
        let rpc_task = tokio::spawn(async move {
            let app = axum::Router::new().route("/",post(move |Json(v):Json<Value>| {
                let seen = seen.clone();
                async move {
                    let result = match v["method"].as_str().unwrap() {
                        "eth_chainId" => { seen.add_permits(1); json!("0x2105") },
                        "eth_getBlockByNumber" => {
                            let height = if v["params"][0] == "latest" {100} else {
                                u64::from_str_radix(v["params"][0].as_str().unwrap().trim_start_matches("0x"),16).unwrap()
                            };
                            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",crate::rotation::base::now().unwrap())})
                        },
                        "eth_call" => json!(format!("0x{:064x}",5_000_000)),
                        _ => panic!("unexpected RPC"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
                }
            }));
            axum::serve(rpc, app).await.unwrap();
        });
        let mut store = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture snapshot",
        )
        .unwrap();
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "test",
            crate::rotation::test_capacity::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let (store, store_task) = crate::rotation::store::StoreHandle::spawn(store);
        running.managed_pools.push(Arc::new(
            crate::rotation::manager::ManagedPool::new(
                store.clone(),
                pool,
                crate::rotation::base::BaseRpc::new(&rpc_url, 12, 120).unwrap(),
                crate::payment::SpendPolicy::dollars("5").unwrap(),
            )
            .unwrap(),
        ));
        let (failure, signal) = tokio::sync::oneshot::channel();
        running.funding_failure = Some(signal);
        let stop = CancellationToken::new();
        let task = tokio::spawn(running.serve(stop.clone()));
        tokio::time::timeout(Duration::from_secs(5), reads.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        failure.send(()).unwrap();
        // A second complete sweep must start after the funding stop. Cancelling
        // reconciliation along with funding would strand this wait.
        tokio::time::timeout(Duration::from_secs(10), reads.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        let http =
            crate::network::discovery("https://deployment-fixture.invalid", Duration::from_secs(5))
                .unwrap();
        for (_, address) in addresses {
            let response: Value = http
                .post(format!("http://{address}/mcp"))
                .bearer_auth("fixture")
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
                .send()
                .await
                .unwrap()
                .mcp_json()
                .await
                .unwrap();
            assert!(!response["result"]["tools"].as_array().unwrap().is_empty());
        }
        assert!(!task.is_finished());
        assert_eq!(
            store
                .call(|s| Ok(s.status()?.funding_jobs.len()))
                .await
                .unwrap(),
            2
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(store);
        store_task.await.unwrap();
        rpc_task.abort();
    }

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

#[cfg(all(test, feature = "zcash"))]
mod initialization_tests;

#[cfg(test)]
mod validation_tests;

#[cfg(all(test, feature = "zcash"))]
#[path = "deployment/store_only_tests.rs"]
mod store_only_tests;

#[cfg(all(test, feature = "zcash"))]
mod prebind_tests;

mod deferred;
