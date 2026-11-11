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
pub mod routing;
#[cfg(test)]
mod routing_tests;
use routing::{EffectiveBindings, SendToolsRule};
mod readiness;
mod selection_diagnostics;
mod startup;
pub use startup::StartupConfig;
mod bootstrap;
pub use bootstrap::{BootstrapSummary, bootstrap_wallets, bootstrap_wallets_with_network};
mod cache_warm;
pub(crate) mod existing_read;
pub use cache_warm::{CacheWarmSummary, warm_cache, warm_cache_with_network};

/// Full frozen catalogs may exceed ordinary tool-result/evidence limits. Includes
/// all source documents and inventories, with no response-schema truncation.
pub const QUALIFICATION_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::{Path, PathBuf},
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
    pub relay_fallback: Option<crate::discovery_relay::Policy>,
    #[serde(default)]
    pub sources: BTreeMap<String, SourceConfig>,
    #[serde(default)]
    pub wallets: BTreeMap<String, WalletConfig>,
    #[serde(default)]
    pub wallet_templates: BTreeMap<String, WalletConfig>,
    pub wallet_assignment: Option<Assignment>,
    pub wallet_bootstrap: Option<assignment::BootstrapPolicy>,
    pub servers: BTreeMap<String, ListenerConfig>,
}
#[derive(Deserialize, Serialize)]
pub struct SourceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_tools: Option<Vec<SendToolsRule>>,
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
fn wait_for_catalogs_default() -> bool {
    true
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    pub listen: SocketAddr,
    /// Wait for this listener's configured catalogs before accepting MCP requests.
    #[serde(default = "wait_for_catalogs_default")]
    pub wait_for_catalogs: bool,
    #[serde(default = "auth_enabled")]
    pub auth: bool,
    pub allowed_hosts: Option<Vec<String>>,
    #[serde(default)]
    pub disable_host_check: bool,
    #[serde(default)]
    pub bearer_token_env: String,
    pub wallet: Option<String>,
    pub swap_control: Option<crate::direct_swap::Config>,
    #[serde(default)]
    pub provider_management_local: bool,
    #[serde(default)]
    pub provider_management_remote: bool,
    #[serde(default)]
    pub remote_provider_directories: Vec<crate::discovery::policy::RemoteDirectory>,
    #[serde(default)]
    pub raw_url_calls: bool,
    pub source_registry_file: Option<PathBuf>,
    /// Advertise discovery wrappers instead of every API schema.
    #[serde(default)]
    pub discover_on_demand: bool,
    pub instructions_text: Option<String>,
    pub discovery_search_tool_instructions_text: Option<String>,
    pub providers_search_remote_tool_instructions_text: Option<String>,
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
}
impl ListenerConfig {
    fn token(&self, env: &BTreeMap<String, String>) -> Result<Option<String>> {
        if !self.auth {
            return Ok(None);
        }
        required_secret(env, &self.bearer_token_env).map(Some)
    }
}

fn listener_management_tools(listener: &ListenerConfig) -> Vec<rmcp::model::Tool> {
    let mut tools = crate::discovery::tools::listener_definitions(
        crate::discovery::policy::EndpointPolicy::from(listener),
        listener.discover_on_demand,
        listener.discovery_search_tool_instructions_text.as_deref(),
        listener
            .providers_search_remote_tool_instructions_text
            .as_deref(),
    );
    if !listener.discover_on_demand
        && let Some(config) = &listener.swap_control
    {
        tools.extend(crate::direct_swap::definitions(config));
    }
    tools
}

#[derive(Clone)]
struct Source {
    send_tools: Option<Vec<SendToolsRule>>,
    config: Config,
    document: std::sync::Arc<serde_json::Value>,
    tools: Vec<ToolSpec>,
    issues: Vec<catalog::OperationIssue>,
    base_url: String,
    instructions: Option<String>,
}
#[derive(Serialize)]
pub struct Inventory {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub omitted_operations: BTreeMap<String, Vec<catalog::OperationIssue>>,
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
    pub recommended: bool,
    #[serde(flatten)]
    pub tool: ToolSpec,
}
pub struct Deployment {
    direct_history: Option<bool>,
    listeners: BTreeMap<String, ListenerConfig>,
    prebound: BTreeMap<String, TcpListener>,
    search_relay: Option<std::sync::Arc<crate::discovery_relay::SearchRelay>>,
    unavailable_sources: BTreeMap<String, String>,
    pending_sources: BTreeMap<String, startup::PendingSource>,
    catalog_capacity: std::sync::Arc<crate::network::DiscoveryCapacity>,
    defer_discovery: bool,
    initialized: Option<InitializedWallets>,
    config: MetaConfig,
    sources: BTreeMap<String, Source>,
    selected: BTreeMap<String, Vec<(String, ToolSpec)>>,
    conflicting_tools: BTreeMap<String, Vec<String>>,
    selection_errors: BTreeMap<String, &'static str>,
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
    /// Apply invocation-local network and relay overrides before deployment validation.
    pub fn parse_with_network(
        text: &str,
        network_override: Option<&crate::network::NetworkConfig>,
    ) -> Result<Self> {
        let mut config: Self = toml::from_str(text).context("invalid meta-config")?;
        if let Some(policy) = network_override {
            config.network = policy.network.clone();
            if let Some(relay) = &policy.relay_fallback {
                config.relay_fallback = Some(relay.clone());
            }
        }
        Ok(config)
    }
    pub(crate) fn validate(&self) -> Result<Resolution> {
        self.validate_capabilities(false)
    }
    fn validate_capabilities(&self, shared_only: bool) -> Result<Resolution> {
        self.startup.validate()?;
        self.network.validate()?;
        crate::discovery::policy::validate(self)?;
        if let Some(relay) = &self.relay_fallback {
            relay.validate(self)?;
        }
        ensure!(
            self.version == 1,
            "unsupported meta-config version {}",
            self.version
        );
        ensure!(!self.servers.is_empty(), "at least one server is required");
        self.validate_declarations()?;
        let bindings = EffectiveBindings::resolve(self)?;
        self.validate_listeners(&bindings.listeners(self))?;
        let resolution = assignment::resolve_with_bindings(self, bindings)?;
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

    fn validate_listeners(&self, listeners: &BTreeMap<String, ListenerConfig>) -> Result<()> {
        let mut addresses = BTreeSet::new();
        for (name, server) in listeners {
            crate::server::host::HostPolicy::new(
                server.allowed_hosts.clone(),
                server.disable_host_check,
            )
            .with_context(|| format!("server {name}: invalid Host policy"))?
            .validate_auth_enabled(server.auth)
            .with_context(|| format!("server {name}"))?;
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
                    || crate::discovery::policy::EndpointPolicy::from(server).any()
                    || server.discover_on_demand
                    || server.swap_control.is_some(),
                "server {name}: sources cannot be empty"
            );
            if let Some(control) = &server.swap_control {
                ensure!(
                    server.auth,
                    "server {name}: swap_control requires authentication"
                );
                ensure!(
                    self.treasury.is_some(),
                    "server {name}: swap_control requires [treasury]"
                );
                control.validate()?;
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
            if !shared_only {
                self.funding
                    .as_ref()
                    .context("managed profiles require [funding]")?
                    .validate()?;
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
                && !self.servers.values().any(|s| s.swap_control.is_some())
            {
                f.validate()?;
            }
        }
        Ok(())
    }
}
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

fn configure_metadata_payer(
    bootstrap: Option<&assignment::BootstrapPolicy>,
    wallets: &mut BTreeMap<String, PaidClient>,
) {
    if crate::qualification::active() || catalog_evidence::collecting() {
        return;
    }
    let Some(payer) = bootstrap.and_then(|policy| wallets.get(&policy.bootstrap_group)) else {
        return;
    };
    let payer = std::sync::Arc::new(payer.relay_authority().without_bootstrap());
    for client in wallets.values_mut() {
        *client = client.clone().with_metadata_payer(payer.clone());
    }
}

fn validate_selected_cover(
    wallet_resolution: &Resolution,
    bootstrap: Option<&assignment::BootstrapPolicy>,
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
                let destination = &wallet_resolution.bindings[server][id].wallet;
                let bootstrap_owner = bootstrap
                    .filter(|policy| policy.deferred_groups.contains(destination))
                    .map(|policy| &policy.bootstrap_group);
                let origin = reqwest::Url::parse(&route)?.origin().ascii_serialization();
                let transport = source.config.transport();
                let timeout = crate::network::global()
                    .read_timeout(
                        source
                            .config
                            .read_timeout_seconds
                            .map(Duration::from_secs_f64),
                    )
                    .as_millis();
                for authority in std::iter::once(destination).chain(bootstrap_owner) {
                    let key = (authority.clone(), origin.clone(), transport, timeout);
                    if let Some(previous) = cover_owners.insert(key, cover) {
                        ensure!(
                            previous == cover,
                            "conflicting cover settings for shared wallet/origin"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

impl Deployment {
    pub async fn show_config(path: &Path) -> Result<serde_json::Value> {
        Self::show_config_with_network(path, None).await
    }
    pub async fn show_config_with_network(
        path: &Path,
        network_override: Option<&crate::network::NetworkConfig>,
    ) -> Result<serde_json::Value> {
        let mut config = MetaConfig::parse_with_network(
            &tokio::fs::read_to_string(path).await.with_context(|| {
                format!("failed to read deployment config file '{}'", path.display())
            })?,
            network_override,
        )?;
        crate::discovery::policy::resolve(&mut config, path);
        let wallet_resolution = config.validate()?;
        if let Some(t) = &mut config.treasury {
            t.resolve(path);
        }
        crate::discovery::policy::validate_registry_path(&config, path)?;
        let discovery_wallets = config
            .relay_fallback
            .as_ref()
            .filter(|policy| policy.provider_metadata && policy.has_method("GET"))
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
            resolved_source["send_tools"] = serde_json::to_value(&source.send_tools)?;
            resolved_source["wallet"] = serde_json::to_value(&source.wallet)?;
            resolved.insert(id, resolved_source);
        }
        Ok(
            serde_json::json!({"source_bindings":wallet_resolution.source_bindings,"discovery_wallets":discovery_wallets,"relay_fallback":config.relay_fallback,"startup":config.startup,"network":config.network.inspection(),"version":config.version,"treasury":config.treasury,"treasury_identity":config.treasury.as_ref().map(|t| if t.id.is_empty() { "from_wallet_state_at_runtime" } else { "explicit_expected_id" }),"funding":config.funding,"base_rpc_policy":config.funding.as_ref().map(|f| f.base_rpc_policy()),"sources":resolved,"wallets":config.wallets,"servers":config.servers,"wallet_bindings":wallet_resolution.bindings,"wallet_templates":config.wallet_templates,"wallet_assignment":config.wallet_assignment,"wallet_bootstrap":wallet_resolution.bootstrap,"resolved_wallets":wallet_resolution.wallets,"generated_wallets":wallet_resolution.generated,"wallet_summary":wallet_resolution.summary}),
        )
    }
    pub async fn load(path: &Path) -> Result<Self> {
        Self::load_with_purpose(path, startup::CatalogPurpose::Inspection, None, false, None).await
    }
    /// Strict operator inspection; bypassing advice retains all ordinary filters.
    pub async fn load_for_inspection(path: &Path, all_tools: bool) -> Result<Self> {
        Self::load_for_inspection_with_network(path, all_tools, None).await
    }
    pub async fn load_for_inspection_with_network(
        path: &Path,
        all_tools: bool,
        network_override: Option<&crate::network::NetworkConfig>,
    ) -> Result<Self> {
        Self::load_with_purpose(
            path,
            startup::CatalogPurpose::Inspection,
            None,
            all_tools,
            network_override,
        )
        .await
    }
    /// Warn for listener-bound sources before catalog I/O, including failed fetches.
    /// Keyless serving keeps provider warnings but cannot bootstrap or pay a relay.
    pub async fn load_for_unsigned_serving(path: &Path) -> Result<Self> {
        Self::load_with_purpose(path, startup::CatalogPurpose::Serving, None, false, None).await
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
        Self::load_for_serving_with_relay_and_network(path, env, restriction, None).await
    }
    pub async fn load_for_serving_with_relay_and_network(
        path: &Path,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
        network_override: Option<&crate::network::NetworkConfig>,
    ) -> Result<Self> {
        Self::load_with_purpose(
            path,
            startup::CatalogPurpose::Serving,
            Some((env, restriction)),
            false,
            network_override,
        )
        .await
    }
    // Saved work retains observation ownership even if its listener or capability
    // table is removed. Only an existing authenticated principal gets status tools.
    fn recover_direct_controls(config: &mut MetaConfig) -> Result<bool> {
        if crate::qualification::active() || catalog_evidence::collecting() {
            return Ok(false);
        }
        let Some(treasury) = &config.treasury else {
            return Ok(false);
        };
        let owners = crate::rotation::store::direct_operation_owners(
            &treasury.state_dir,
            (!treasury.id.is_empty()).then_some(treasury.id.as_str()),
        )?;
        for (name, listener) in &mut config.servers {
            if listener.auth && listener.swap_control.is_none() && owners.contains(name) {
                listener.swap_control = Some(crate::direct_swap::Config::default());
            }
        }
        Ok(!owners.is_empty())
    }

    fn prepare_serving_recovery(&mut self) -> Result<()> {
        if self.direct_history.is_some()
            || crate::qualification::active()
            || catalog_evidence::collecting()
        {
            return Ok(());
        }
        let history = Self::recover_direct_controls(&mut self.config)?;
        // Recovery changes only controls; keep resolved routing and socket metadata.
        for (name, listener) in &mut self.listeners {
            listener.swap_control = self.config.servers[name].swap_control.clone();
        }
        self.refresh_selection(true)?;
        self.direct_history = Some(history);
        Ok(())
    }

    fn direct_required(&self) -> bool {
        self.direct_history.unwrap_or(false)
            || self.listeners.values().any(|s| s.swap_control.is_some())
    }

    fn validate_listener_credentials(&self, env: &BTreeMap<String, String>) -> Result<()> {
        for server in self.listeners.values().filter(|s| s.auth) {
            server.token(env)?;
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
        for (name, cfg) in &self.listeners {
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
            if let Some(policy) = self
                .config
                .relay_fallback
                .clone()
                .filter(|p| p.provider_metadata && p.has_method("GET"))
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

    async fn load_with_purpose(
        path: &Path,
        purpose: startup::CatalogPurpose,
        relay_env: Option<(
            &BTreeMap<String, String>,
            crate::rotation::restriction::FundingRestriction,
        )>,
        all_tools: bool,
        network_override: Option<&crate::network::NetworkConfig>,
    ) -> Result<Self> {
        let purpose = purpose.for_current_operation();
        let mut config = MetaConfig::parse_with_network(
            &tokio::fs::read_to_string(path).await.with_context(|| {
                format!("failed to read deployment config file '{}'", path.display())
            })?,
            network_override,
        )?;
        crate::discovery::policy::resolve(&mut config, path);
        let scoped_funding = relay_env.is_some()
            && !crate::qualification::active()
            && !catalog_evidence::collecting();
        if let Some(t) = &mut config.treasury {
            t.resolve(path);
        }
        let direct_history = if purpose == startup::CatalogPurpose::Serving && relay_env.is_some() {
            Some(Self::recover_direct_controls(&mut config)?)
        } else {
            None
        };
        let wallet_resolution = config.validate_capabilities(scoped_funding)?;
        crate::network::install(config.network.clone())?;
        crate::discovery::policy::validate_registry_path(&config, path)?;
        // Validate every configured method/format locally before wallet state or I/O.
        let search_relay = match config.relay_fallback.as_ref() {
            Some(policy) => {
                let relay = crate::discovery_relay::SearchRelay::load(policy, path).await?;
                Some(std::sync::Arc::new(relay))
            }
            None => None,
        };
        let mut deployment = Self {
            direct_history,
            catalog_capacity: std::sync::Arc::new(crate::network::DiscoveryCapacity::new(
                config.startup.catalog_concurrency,
            )),
            listeners: wallet_resolution.source_bindings.listeners(&config),
            prebound: BTreeMap::new(),
            search_relay,
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
            conflicting_tools: BTreeMap::new(),
            selection_errors: BTreeMap::new(),
            config_path: path.to_owned(),
        };
        if let Some((env, _)) = relay_env
            && deployment
                .config
                .relay_fallback
                .as_ref()
                .is_some_and(|p| p.provider_metadata && p.has_method("GET"))
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
            let (sources, unavailable_sources, pending_sources) = startup::load(
                config,
                &wallet_resolution.source_bindings,
                path,
                purpose,
                relay,
                deployment.defer_discovery,
                all_tools,
                deployment.catalog_capacity.clone(),
            )
            .await?;
            deployment.unavailable_sources = unavailable_sources;
            deployment.pending_sources = pending_sources;
            deployment.sources = sources;
            deployment.refresh_selection(purpose == startup::CatalogPurpose::Serving)?;
            selection_diagnostics::report_completed(
                &deployment.listeners,
                &deployment.sources,
                &deployment.pending_sources,
                None,
            );
            Ok(())
        }
        .await;
        if let Err(error) = loaded {
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
        let recommendations: BTreeMap<_, _> = self
            .sources
            .iter()
            .map(|(id, source)| {
                (
                    id.clone(),
                    source
                        .config
                        .recommendation_matcher()
                        .expect("loaded source recommendation selectors are validated"),
                )
            })
            .collect();
        self.listeners
            .iter()
            .map(|(name, s)| Inventory {
                omitted_operations: s
                    .sources
                    .iter()
                    .filter_map(|id| {
                        self.sources
                            .get(id)
                            .filter(|source| !source.issues.is_empty())
                            .map(|source| (id.clone(), source.issues.clone()))
                    })
                    .collect(),
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
                management_tools: listener_management_tools(s),
                tools: self.selected[name]
                    .iter()
                    .map(|(source, tool)| InventoryTool {
                        source: source.clone(),
                        recommended: recommendations[source](&tool.name),
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
        self.listeners
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

    fn managed_connection_preflight(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<(FundingConnections, crate::rotation::base::BaseRpc)> {
        Ok((
            self.funding_connection_preflight(env)?,
            self.base_connection_preflight(env)?,
        ))
    }
    fn funding_connection_preflight(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<FundingConnections> {
        Ok(FundingConnections {
            treasury: self.treasury_connection_preflight(env)?,
            near: self.near_connection_preflight(env)?,
        })
    }
    fn treasury_connection_preflight(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<TreasuryConnections> {
        let t = self.config.treasury.as_ref().unwrap();
        t.validate()?;
        let sync_settings = crate::treasury::SyncSettings::new(
            t.indexer_endpoint(|name| env.get(name).cloned())?,
            t.confirmations,
        )?;
        let sender = crate::treasury::submission::GrpcSubmission::new(
            t.submission_endpoint(|name| env.get(name).cloned())?,
            t.indexer_endpoint(|name| env.get(name).cloned())?,
        )?;
        Ok(TreasuryConnections {
            sync_settings,
            sender,
        })
    }
    fn near_connection_preflight(&self, env: &BTreeMap<String, String>) -> Result<NearSource> {
        let f = self
            .config
            .funding
            .as_ref()
            .context("NEAR swap configuration unavailable")?;
        f.validate_swap()?;
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
        let client =
            crate::rotation::near::NearClient::with_session(key.as_deref(), session.as_deref())?;
        Ok(NearSource {
            client,
            funding: f.clone(),
        })
    }
    fn optional_near_source(&self, env: &BTreeMap<String, String>) -> Option<NearSource> {
        self.config.funding.as_ref()?;
        match self.near_connection_preflight(env) {
            Ok(source) => Some(source),
            Err(error) => {
                tracing::warn!(
                    category = "funding_configuration_unavailable", reason = %error,
                    "NEAR swap configuration unavailable; foreign asset swaps and managed funding are unavailable; native Zcash tools do not require NEAR; correct NEAR configuration and restart"
                );
                None
            }
        }
    }
    fn base_connection_preflight(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<crate::rotation::base::BaseRpc> {
        use crate::rotation::base::BaseRpc;
        let f = self
            .config
            .funding
            .as_ref()
            .context("Base configuration unavailable")?;
        f.validate_base()?;
        let urls = f.base_rpc_urls(|name| env.get(name).cloned())?;
        if self.config.network.mode == crate::network::Mode::Direct {
            tracing::warn!(
                provider_count = urls.len(),
                default_fallbacks = f.base_rpc_fallback_url_envs.is_none(),
                "Direct Base RPC verification exposes your client IP address and queried wallet addresses to each contacted RPC provider, including fallbacks; see config show base_rpc_policy"
            );
        }
        let base = BaseRpc::with_fallbacks(&urls)?;
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
        let direct = self.listeners.values().any(|s| s.swap_control.is_some());
        ensure!(
            !direct || !crate::qualification::active(),
            "qualification forbids swap control"
        );
        self.validate_static_identities(only_wallets).await?;
        let mut wallets = self.static_wallets(env, only_wallets, relay_wallets)?;
        let mut runtime = ManagedRuntime::None;
        let mut managed_pools = Vec::new();
        if self.wallet_resolution.wallets.iter().any(|(name, wallet)| {
            only_wallets.is_none_or(|selected| selected.contains(name)) && wallet.managed()
        }) {
            use crate::rotation::manager::ManagedPool;
            let mut capabilities = self.wallet_setup(env, true, allow_store_only).await?;
            let store = capabilities.store();
            let setup: Result<()> = async {
            let auto_fund = self.config.funding.as_ref().is_some_and(|f| f.auto_fund) && capabilities.funding_available();
            if let ManagedSetup::Treasury(treasury) = &mut capabilities {
                treasury.owner.configure_sync(treasury.source.sync_settings.clone());
            }
            install_funding_restrictions(&store, restriction, auto_fund)
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
                let selected = only_wallets.is_none_or(|selected| selected.contains(name));
                // Explicit bootstrap still processes existing jobs outside its
                // allocation selection. Those jobs need current claim policy.
                if !selected && !(auto_fund
                    && restriction == crate::rotation::restriction::FundingRestriction::Ordinary)
                {
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
                            if selected {
                                return s.ensure_capacity_group(&group_name, &reserves, cap, retirement).map(Some);
                            }
                            if let Some(pool) = s.advisory_pools(&[group_name])?.first() {
                                s.configure_capacity_group(pool, cap, retirement, &reserves)?;
                            }
                            Ok(None)
                        })
                        .await?;
                    let Some(pool) = pool else { continue };
                    let client = if let Some(base) = capabilities.base() {
                        let manager = std::sync::Arc::new(ManagedPool::new(store.clone(), pool, base.clone(), policy)?);
                        managed_pools.push(manager.clone());
                        PaidClient::managed(manager)
                    } else {
                        let client = PaidClient::unavailable(crate::rotation::error::AdmissionError::WalletNotReady(
                            "managed Base configuration unavailable; correct configuration and restart"));
                        if !crate::qualification::active()
                            && self.wallet_resolution.bootstrap.as_ref().is_some_and(|policy| policy.deferred_groups.contains(name))
                        {
                            client.with_bootstrap_demand(store.clone(), pool)
                        } else {
                            client
                        }
                    };
                    wallets.insert(name.clone(), client);
                }
            }
            // Qualification retains named destination authority and reviewed allocations.
            if !crate::qualification::active() && let Some(policy) = &self.wallet_resolution.bootstrap {
                let bootstrap = wallets.get(&policy.bootstrap_group).and_then(PaidClient::managed_pool);
                if let Some(bootstrap) = bootstrap {
                    for name in &policy.deferred_groups {
                        if let Some(client) = wallets.get_mut(name) {
                            *client = client.clone().with_bootstrap(bootstrap.clone());
                        }
                    }
                }
            }
            configure_metadata_payer(self.wallet_resolution.bootstrap.as_ref(), &mut wallets);
            Ok(())
            }.await;
            if let Err(error) = setup {
                drop(managed_pools);
                drop(wallets);
                drop(store);
                capabilities.close().await?;
                return Err(error);
            }
            runtime = match capabilities {
                ManagedSetup::StoreOnly { owner, .. } => ManagedRuntime::StoreOnly(owner),
                ManagedSetup::Treasury(treasury) => {
                    let TreasurySetup {
                        mut owner,
                        base,
                        source,
                        near,
                    } = *treasury;
                    let (handle, commands) = crate::treasury::actor::channel();
                    let direct = self.direct_service(store.clone(), handle.clone(), near.as_ref());
                    let (Some(base), Some(near)) = (base, near) else {
                        runtime = ManagedRuntime::Direct(Box::new(DirectRuntime {
                            owner,
                            commands,
                            sender: source.sender,
                            service: direct.expect(
                                "SDK opened without managed funding only for direct Zcash work",
                            ),
                        }));
                        return Ok(InitializedWallets {
                            wallets,
                            runtime,
                            managed_pools,
                        });
                    };
                    let NearSource {
                        client: near,
                        funding: f,
                    } = near;
                    tracing::info!(target: "x402_treazury::startup", auto_fund = f.auto_fund,
                        "{}", if f.auto_fund { "Automatic wallet funding enabled; new transfers use spendable treasury funds" } else { "Automatic wallet funding disabled; new funding transfers are paused" });
                    let backend = crate::rotation::funding::Backend {
                        treasury: handle,
                        store: store.clone(),
                        near,
                        base,
                        wallets: self.wallet_resolution.wallets.clone(),
                        funding: f.clone(),
                    };
                    owner.configure_funding_discovery(
                        crate::rotation::funding::FundingBackend::allocation_groups(&backend),
                    );
                    ManagedRuntime::Treasury(Box::new(TreasuryRuntime {
                        direct,
                        owner,
                        funding: (
                            commands,
                            source.sender,
                            crate::rotation::funding::FundingWorker::new(
                                store.clone(),
                                backend,
                                f.poll_seconds,
                            )
                            .with_bootstrap_participants(
                                self.wallet_resolution
                                    .bootstrap
                                    .as_ref()
                                    .map(|policy| policy.deferred_groups.iter().cloned().collect())
                                    .unwrap_or_default(),
                            ),
                        ),
                    }))
                }
            };
        }
        if matches!(runtime, ManagedRuntime::None) && self.direct_required() {
            runtime = match self.wallet_setup(env, false, allow_store_only).await? {
                ManagedSetup::StoreOnly { owner, .. } => ManagedRuntime::StoreOnly(owner),
                ManagedSetup::Treasury(treasury) => {
                    let TreasurySetup {
                        mut owner,
                        source,
                        near,
                        ..
                    } = *treasury;
                    owner.configure_sync(source.sync_settings.clone());
                    let (handle, commands) = crate::treasury::actor::channel();
                    let service = self
                        .direct_service(owner.store_handle(), handle, near.as_ref())
                        .unwrap();
                    ManagedRuntime::Direct(Box::new(DirectRuntime {
                        owner,
                        commands,
                        sender: source.sender,
                        service,
                    }))
                }
            };
        }
        Ok(InitializedWallets {
            wallets,
            runtime,
            managed_pools,
        })
    }

    // Base admission, SDK ownership and direct swap admission degrade independently.
    async fn wallet_setup(
        &self,
        env: &BTreeMap<String, String>,
        with_base: bool,
        best_effort: bool,
    ) -> Result<ManagedSetup> {
        let t = self.config.treasury.as_ref().unwrap();
        let direct_admission = self
            .listeners
            .values()
            .any(|s| s.swap_control.as_ref().is_some_and(|c| c.enabled()));
        if !best_effort && (with_base || direct_admission) {
            let (source, near, base) = if with_base {
                let (source, base) = self.managed_connection_preflight(env)?;
                (source.treasury, Some(source.near), Some(base))
            } else {
                (
                    self.treasury_connection_preflight(env)?,
                    self.optional_near_source(env),
                    None,
                )
            };
            let owner = crate::treasury::Treasury::open(
                t.state_dir.clone(),
                t.key_file.clone(),
                t.runtime_id()?,
            )
            .await?;
            return Ok(ManagedSetup::Treasury(Box::new(TreasurySetup {
                owner,
                base,
                source,
                near,
            })));
        }
        let shared = crate::treasury::Treasury::open_shared_state(
            t.state_dir.clone(),
            t.key_file.clone(),
            t.runtime_id()?,
            crate::rotation::store::TreasuryNetwork::Mainnet,
        )
        .await?;
        let base = if with_base {
            match self.base_connection_preflight(env) {
                Ok(base) => Some(base),
                Err(error) => {
                    tracing::warn!(
                        category = "base_configuration_unavailable", reason = %error,
                        "Managed Base configuration unavailable; independent tools remain available; correct Base configuration and restart"
                    );
                    None
                }
            }
        } else {
            None
        };
        if base.is_none() && !self.direct_required() {
            return Ok(self.store_only_setup(shared, base));
        }
        let near = self.optional_near_source(env);
        if near.is_none() && !self.direct_required() {
            return Ok(self.store_only_setup(shared, base));
        }
        let source = match self.treasury_connection_preflight(env) {
            Ok(source) => source,
            Err(error) => {
                tracing::warn!(
                    category = "funding_configuration_unavailable", reason = %error,
                    "Treasury connection configuration unavailable; new funding and Zcash operations are unavailable; independent Base payments and saved status remain available; correct treasury connections and restart"
                );
                return Ok(self.store_only_setup(shared, base));
            }
        };
        match crate::treasury::Treasury::reconstruct(shared).await? {
            crate::treasury::OpenedTreasury::Ready(owner) => {
                Ok(ManagedSetup::Treasury(Box::new(TreasurySetup {
                    owner: *owner,
                    base,
                    source,
                    near,
                })))
            }
            crate::treasury::OpenedTreasury::SnapshotUnavailable(shared) => {
                tracing::warn!(
                    category = "treasury_snapshot_unavailable",
                    "Treasury SDK unavailable; new funding and direct swaps are unavailable; existing Base payments and saved status retain shared ownership"
                );
                Ok(self.store_only_setup(*shared, base))
            }
        }
    }

    fn store_only_setup(
        &self,
        store: crate::rotation::store::Store,
        base: Option<crate::rotation::base::BaseRpc>,
    ) -> ManagedSetup {
        let mut owner = StoreOnlyOwner::new(store);
        if self.direct_required() {
            owner.direct = Some(crate::direct_swap::Service::status_only(
                owner.store.clone(),
            ));
        }
        ManagedSetup::StoreOnly {
            owner,
            base: base.map(Box::new),
        }
    }

    fn direct_service(
        &self,
        store: crate::rotation::store::StoreHandle,
        treasury: crate::treasury::actor::TreasuryHandle,
        near: Option<&NearSource>,
    ) -> Option<crate::direct_swap::Service> {
        self.direct_required().then(|| match near {
            Some(near) => crate::direct_swap::Service::new(
                store,
                treasury,
                near.client.clone(),
                near.funding.clone(),
            ),
            None => crate::direct_swap::Service::native(
                store,
                treasury,
                self.config
                    .funding
                    .as_ref()
                    .map(|f| f.poll_seconds)
                    .filter(|seconds| *seconds > 0)
                    .unwrap_or_else(crate::rotation::config::poll),
            ),
        })
    }

    // Catalogs belong to sources; selection failures belong to endpoints. Keep
    // successful acquisition even when one destination has invalid narrowing.
    fn refresh_selection(&mut self, serving: bool) -> Result<()> {
        let mut selected = BTreeMap::new();
        let mut conflicts = BTreeMap::new();
        let mut errors = BTreeMap::new();
        for (name, listener) in &self.listeners {
            let inventory = listener_tool_inventory(
                name,
                listener,
                &self.sources,
                &self.unavailable_sources,
                serving,
            );
            let inventory = match inventory {
                Ok(inventory) => inventory,
                Err(error) if serving => {
                    let failure = startup::catalog_failure(&error, "selection");
                    errors.insert(name.clone(), failure.reason);
                    ListenerInventory {
                        tools: Vec::new(),
                        conflicts: Vec::new(),
                    }
                }
                Err(error) => return Err(error),
            };
            // Reserved identities invalidate source admission, rather than the
            // endpoint's narrowing. A late invalid source must not withdraw
            // healthy tools already published on this listener.
            if (crate::discovery::policy::enabled(&self.config)
                || listener.discover_on_demand
                || listener.swap_control.is_some())
                && inventory.tools.iter().any(|(_, tool)| {
                    tool.name.starts_with("dyn_") || tool.name.starts_with("x402_treazury_")
                })
            {
                return Err(anyhow::anyhow!("server {name}: static tool uses reserved namespace")
                    .context(catalog::diagnostics::Problem("Static tools use a reserved management namespace; configure a distinct source prefix or listener selection")));
            }
            selected.insert(name.clone(), inventory.tools);
            conflicts.insert(name.clone(), inventory.conflicts);
        }
        // This validation intentionally spans endpoints sharing one payer/origin.
        // Unlike endpoint filters, a conflicting cover policy is a shared resource
        // failure; preserve the existing source admission boundary for that case.
        let bootstrap = self
            .wallet_resolution
            .bootstrap
            .as_ref()
            .filter(|_| !crate::qualification::active() && !catalog_evidence::collecting());
        validate_selected_cover(&self.wallet_resolution, bootstrap, &self.sources, &selected)?;
        self.selected = selected;
        self.conflicting_tools = conflicts;
        self.selection_errors = errors;
        Ok(())
    }

    fn build_servers(&self, wallets: &BTreeMap<String, PaidClient>) -> BTreeMap<String, Server> {
        let mut servers = BTreeMap::new();
        for (name, cfg) in &self.listeners {
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
                            .with_document_relay(self.search_relay.clone())
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
            let source_instructions = used
                .into_iter()
                .filter_map(|id| {
                    self.sources[id]
                        .instructions
                        .as_ref()
                        .map(|s| format!("[{id}]\n{s}"))
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let mut instructions = cfg.instructions_text.clone().unwrap_or(source_instructions);
            let mut text = Some(instructions);
            for id in &cfg.sources {
                if let Some(source) = self.sources.get(id) {
                    catalog::append_omissions(&mut text, id, &source.issues);
                }
            }
            instructions = text.unwrap_or_default();
            if let Some(reason) = self.selection_errors.get(name) {
                instructions.push_str(&format!("\n\nUNAVAILABLE endpoint tool selection: {reason}. Correct the endpoint/source selections and restart."));
            }
            for id in &cfg.sources {
                if self.unavailable_sources.contains_key(id) {
                    if self
                        .pending_sources
                        .get(id)
                        .is_some_and(|pending| pending.operator_required)
                    {
                        instructions.push_str(&format!("\n\nSource {id}: OPERATOR_REQUIRED (new relay payer creation is unavailable). Observation continues; its tools will appear if discovery becomes possible."));
                    } else if self.pending_sources.contains_key(id) {
                        instructions.push_str(&format!("\n\nSource {id}: PENDING (catalog discovery is waiting). Its tools will appear when discovery succeeds."));
                    } else if self
                        .sources
                        .get(id)
                        .is_some_and(Source::compatibility_unavailable)
                    {
                        instructions.push_str(&format!("\n\nSource {id}: UNAVAILABLE (all selected API operations have unsupported input contracts). Inspect omitted operations and correct the source configuration."));
                    } else {
                        instructions.push_str(&format!("\n\nSource {id}: UNAVAILABLE (catalog discovery failed). Its tools are unavailable, not intentionally empty. Restart to retry."));
                    }
                }
            }
            if let Some(conflicts) = self
                .conflicting_tools
                .get(name)
                .filter(|names| !names.is_empty())
            {
                instructions.push_str(&format!("\n\nUNAVAILABLE ambiguous tool names: {}. Multiple configured sources define these names; all conflicting bindings are omitted. Configure distinct prefixes or source selections and restart.", conflicts.join(", ")));
            }
            let mut server =
                Server::from_bindings(bindings, (!instructions.is_empty()).then_some(instructions));
            server.name = name.clone();
            server.discover_on_demand = cfg.discover_on_demand;
            server.discovery_search_tool_instructions_text =
                cfg.discovery_search_tool_instructions_text.clone();
            server.providers_search_remote_tool_instructions_text =
                cfg.providers_search_remote_tool_instructions_text.clone();
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
        for (name, cfg) in &self.listeners {
            tokens.insert(name.clone(), cfg.token(env)?);
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
            !self.listeners.values().any(|s| s.swap_control.is_some()),
            "unsigned qualification forbids swap control"
        );
        ensure!(
            !crate::discovery::policy::enabled(&self.config),
            "unsigned qualification forbids provider management and raw URL calls"
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
            runtime: ManagedRuntime::None,
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
        if restriction == crate::rotation::restriction::FundingRestriction::DenyNewFunding
            && let Some(store) = initialized.runtime.store()
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
        configure_metadata_payer(
            self.wallet_resolution.bootstrap.as_ref(),
            &mut initialized.wallets,
        );
        Ok(())
    }

    async fn bind_catalog_listeners(
        &mut self,
        wallets: BTreeMap<String, PaidClient>,
        mut tokens: BTreeMap<String, Option<String>>,
        direct: Option<crate::direct_swap::Service>,
    ) -> Result<BoundListeners> {
        let mut servers = self.build_servers(&wallets);
        let mut snapshot = crate::catalog_state::CatalogSnapshot::default();
        for (name, server) in &servers {
            let tools = server.catalog.read().views["default"].clone();
            snapshot
                .instructions
                .insert(name.clone(), server.instructions.clone());
            snapshot.views.insert(name.clone(), tools);
        }
        let catalog = std::sync::Arc::new(crate::catalog_state::CatalogState::new(snapshot));
        let protected = crate::discovery::policy::protected_paths(&self.config, &self.config_path);
        let manager = if crate::discovery::policy::enabled(&self.config) {
            Some(
                crate::discovery::Manager::new_with_search(
                    self.listeners.clone(),
                    wallets,
                    catalog.clone(),
                    protected,
                    self.search_relay.clone(),
                )
                .await?,
            )
        } else {
            None
        };
        let mut prebound = std::mem::take(&mut self.prebound);
        let reuse_sockets = !prebound.is_empty();
        ensure!(
            !reuse_sockets || prebound.keys().eq(self.listeners.keys()),
            "prebound listener inventory changed"
        );
        let mut listeners = Vec::new();
        let mut accepted_work = Vec::new();
        for (name, cfg) in &self.listeners {
            let mut server = servers.remove(name).unwrap();
            server.catalog = catalog.clone();
            server.catalog_server = name.clone();
            server.host_policy = crate::server::host::HostPolicy::new(
                cfg.allowed_hosts.clone(),
                cfg.disable_host_check,
            )?;
            if crate::discovery::policy::EndpointPolicy::from(cfg).any() {
                manager
                    .as_ref()
                    .context("provider capabilities unavailable")?
                    .validate_client(name)
                    .with_context(|| {
                        format!("server {name}: provider capability wallet unavailable")
                    })?;
            }
            server.discovery = manager.clone();
            server.direct_swap = direct.clone().zip(cfg.swap_control.clone());
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
                http_app_with_auth(server, tokens.remove(name).unwrap())?,
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
        let setup = async {
            if !unsigned {
                self.prepare_serving_recovery()?;
            }
            self.binding_wallets(env, restriction, unsigned).await
        }
        .await;
        let (tokens, initialized) = match setup {
            Ok(setup) => setup,
            Err(error) => {
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
        let runtime = initialized.runtime;
        let managed_pools = initialized.managed_pools;
        let deferred_wallets = wallets.clone();
        let bound = self
            .bind_catalog_listeners(wallets, tokens, runtime.direct_service())
            .await;
        let BoundListeners {
            listeners,
            accepted_work,
            catalog,
            manager,
        } = match bound {
            Ok(bound) => bound,
            Err(error) => {
                drop(deferred_wallets);
                drop(managed_pools);
                drop(self);
                runtime.close().await?;
                return Err(error);
            }
        };
        let local_startup_pools = (!self.defer_discovery)
            .then(|| readiness::funding_pools(&self, &deferred_wallets))
            .flatten();
        let local_catalogs_ready = self
            .listeners
            .keys()
            .all(|name| matches!(self.endpoint_readiness(name), Some(Ok(()))));
        let can_allocate = runtime.funding_available()
            && self.config.funding.as_ref().is_some_and(|f| f.auto_fund)
            && restriction == crate::rotation::restriction::FundingRestriction::Ordinary;
        let initial_readiness = (!self.defer_discovery).then(|| self.resolved_catalog_readiness());
        let mut deferred = self.defer_discovery.then(|| {
            deferred::Runtime::new(self, deferred_wallets, catalog, manager, can_allocate)
        });
        let catalog_readiness = match deferred.as_mut() {
            Some(runtime) => runtime.catalog_readiness(),
            None => initial_readiness.expect("nondeferred catalog results"),
        };
        let startup_catalogs = match deferred.as_mut() {
            Some(runtime) => runtime.startup_readiness(),
            None => {
                let (send, receive) = tokio::sync::oneshot::channel();
                if local_catalogs_ready {
                    let _ = send.send(local_startup_pools);
                }
                receive
            }
        };
        Ok(RunningDeployment {
            startup_catalogs,
            deferred,
            catalog_readiness,
            listeners,
            accepted_work,
            #[cfg(test)]
            listener_failure: None,
            #[cfg(test)]
            funding_failure: None,
            runtime,
            managed_pools,
        })
    }
}

#[cfg(test)]
fn select_listener_tools(
    name: &str,
    server: &ListenerConfig,
    sources: &BTreeMap<String, Source>,
    unavailable: &BTreeMap<String, String>,
) -> Result<Vec<(String, ToolSpec)>> {
    listener_tool_inventory(name, server, sources, unavailable, false)
        .map(|inventory| inventory.tools)
}

struct ListenerInventory {
    tools: Vec<(String, ToolSpec)>,
    conflicts: Vec<String>,
}

impl Source {
    fn compatibility_unavailable(&self) -> bool {
        self.tools.is_empty() && !self.issues.is_empty()
    }
    // One transient assignment/tag view feeds publication and selector diagnostics.
    fn listener_tools(&self, name: &str, listener: &ListenerConfig) -> Result<Vec<&ToolSpec>> {
        let operations = if listener.tags.is_empty() && listener.exclude_tags.is_empty() {
            Vec::new()
        } else {
            catalog::operations(&self.document, None)?
        };
        let routing = self
            .send_tools
            .as_ref()
            .map(|rules| routing::CompiledRules::new(rules, self.config.prefix.as_deref().unwrap()))
            .transpose()?;
        Ok(self
            .tools
            .iter()
            .filter(|tool| {
                routing.as_ref().is_none_or(|rules| {
                    rules
                        .decision(&tool.name)
                        .targets
                        .iter()
                        .any(|target| target == name)
                }) && catalog::matches_operation_tags(
                    &operations,
                    tool,
                    &listener.tags,
                    &listener.exclude_tags,
                )
            })
            .collect())
    }
}

// Keep ambiguous names in the inventory until selectors have been validated.
// Serving omits every owner, so completion order cannot select a provider/payer.
fn listener_tool_inventory(
    name: &str,
    server: &ListenerConfig,
    sources: &BTreeMap<String, Source>,
    unavailable: &BTreeMap<String, String>,
    omit_conflicts: bool,
) -> Result<ListenerInventory> {
    let mut available = BTreeMap::new();
    let incomplete = server.sources.iter().any(|id| unavailable.contains_key(id));
    for source in &server.sources {
        if unavailable.contains_key(source) {
            continue;
        }
        for tool in sources[source].listener_tools(name, server)? {
            match available.entry(tool.name.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(Some((source.clone(), tool.clone())));
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    ensure!(
                        omit_conflicts,
                        "server {name}: duplicate tool {}",
                        tool.name
                    );
                    entry.insert(None);
                }
            }
        }
    }
    let compile = |selectors: &[String], require_match: bool| -> Result<Vec<Regex>> {
        selectors
            .iter()
            .map(|text| {
                let re = pattern(text)?;
                if require_match && !available.keys().any(|n| re.is_match(n))
                    && !text.contains(['*', '?'])
                    && !selector_has_unavailable_owner(text, server, sources, unavailable)
                {
                    let removals =
                        selection_diagnostics::selection_removals(name, server, sources)?;
                    let causes = selection_diagnostics::removal_causes(&removals, &re);
                    let detail = if causes.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "; matching tools were removed before listener name filtering: {}",
                            causes.join("; ")
                        )
                    };
                    return Err(anyhow::anyhow!("server {name}: unknown tool selector {text}{detail}")
                        .context(catalog::diagnostics::Problem("A listener tool selector matches no assigned tools; inspect source send_tools rules and listener filters")));
                }
                Ok(re)
            })
            .collect()
    };
    let include = compile(&server.include_tools, true)?;
    let exclude = compile(&server.exclude_tools, false)?;
    let conflicts: Vec<_> = available
        .iter()
        .filter(|(_, binding)| binding.is_none())
        .map(|(name, _)| name.clone())
        .collect();
    let tools: Vec<_> = available
        .into_iter()
        .filter(|(n, _)| {
            (include.is_empty() || include.iter().any(|p| p.is_match(n)))
                && !exclude.iter().any(|p| p.is_match(n))
        })
        .filter_map(|(_, t)| t)
        .collect();
    if tools.is_empty()
        && !crate::discovery::policy::EndpointPolicy::from(server).any()
        && !server.discover_on_demand
        && server.swap_control.is_none()
        && !incomplete
        && conflicts.is_empty()
    {
        return Err(anyhow::anyhow!("server {name}: no tools selected")
            .context(catalog::diagnostics::Problem("The endpoint has no selected tools or discovery capabilities; review source assignments and listener filters")));
    }
    Ok(ListenerInventory { tools, conflicts })
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

struct FundingConnections {
    treasury: TreasuryConnections,
    near: NearSource,
}
struct TreasuryConnections {
    sync_settings: crate::treasury::SyncSettings,
    sender: crate::treasury::submission::GrpcSubmission,
}
struct NearSource {
    client: crate::rotation::near::NearClient,
    funding: FundingConfig,
}

struct StoreOnlyOwner {
    direct: Option<crate::direct_swap::Service>,
    store: crate::rotation::store::StoreHandle,
    worker: tokio::task::JoinHandle<()>,
}
impl StoreOnlyOwner {
    fn new(store: crate::rotation::store::Store) -> Self {
        let (store, worker) = crate::rotation::store::StoreHandle::spawn(store);
        Self {
            store,
            worker,
            direct: None,
        }
    }
    async fn close(self) -> Result<()> {
        drop(self.direct);
        drop(self.store);
        self.worker.await.context("store worker failed")
    }
}

// Preflight may leave Base usable without a reconstructable SDK. Keep those
// capabilities together instead of treating independent Options as permission.
enum ManagedSetup {
    StoreOnly {
        owner: StoreOnlyOwner,
        base: Option<Box<crate::rotation::base::BaseRpc>>,
    },
    Treasury(Box<TreasurySetup>),
}
struct TreasurySetup {
    owner: crate::treasury::Treasury,
    base: Option<crate::rotation::base::BaseRpc>,
    source: TreasuryConnections,
    near: Option<NearSource>,
}
impl ManagedSetup {
    fn store(&self) -> crate::rotation::store::StoreHandle {
        match self {
            Self::StoreOnly { owner, .. } => owner.store.clone(),
            Self::Treasury(treasury) => treasury.owner.store_handle(),
        }
    }
    fn base(&self) -> Option<&crate::rotation::base::BaseRpc> {
        match self {
            Self::StoreOnly { base, .. } => base.as_deref(),
            Self::Treasury(treasury) => treasury.base.as_ref(),
        }
    }
    fn funding_available(&self) -> bool {
        matches!(self, Self::Treasury(t) if t.base.is_some() && t.near.is_some())
    }
    async fn close(self) -> Result<()> {
        match self {
            Self::StoreOnly { owner, .. } => owner.close().await,
            Self::Treasury(treasury) => treasury.owner.close().await,
        }
    }
}

// Dormant capabilities retain one treasury or shared Store owner through serving.
// A treasury owner always carries its matching command/funding runtime.
enum ManagedRuntime {
    None,
    StoreOnly(StoreOnlyOwner),
    Treasury(Box<TreasuryRuntime>),
    Direct(Box<DirectRuntime>),
}
struct DirectRuntime {
    owner: crate::treasury::Treasury,
    commands: crate::treasury::actor::TreasuryCommands,
    sender: crate::treasury::submission::GrpcSubmission,
    service: crate::direct_swap::Service,
}
struct TreasuryRuntime {
    direct: Option<crate::direct_swap::Service>,
    owner: crate::treasury::Treasury,
    funding: FundingRuntime,
}
impl ManagedRuntime {
    fn direct_service(&self) -> Option<crate::direct_swap::Service> {
        match self {
            Self::Treasury(t) => t.direct.clone(),
            Self::Direct(t) => Some(t.service.clone()),
            Self::StoreOnly(t) => t.direct.clone(),
            _ => None,
        }
    }
    fn store(&self) -> Option<crate::rotation::store::StoreHandle> {
        match self {
            Self::None => None,
            Self::StoreOnly(owner) => Some(owner.store.clone()),
            Self::Treasury(treasury) => Some(treasury.owner.store_handle()),
            Self::Direct(direct) => Some(direct.owner.store_handle()),
        }
    }
    fn funding_available(&self) -> bool {
        matches!(self, Self::Treasury(_))
    }
    async fn close(self) -> Result<()> {
        match self {
            Self::None => Ok(()),
            Self::Direct(direct) => {
                let DirectRuntime {
                    owner,
                    commands,
                    sender,
                    service,
                } = *direct;
                drop((commands, sender, service));
                owner.close().await
            }
            Self::StoreOnly(owner) => owner.close().await,
            Self::Treasury(treasury) => {
                let TreasuryRuntime {
                    owner,
                    funding,
                    direct,
                } = *treasury;
                drop(direct);
                // Funding owns Store handles: release them before joining its owner.
                drop(funding);
                owner.close().await
            }
        }
    }
}

struct InitializedWallets {
    wallets: BTreeMap<String, PaidClient>,
    runtime: ManagedRuntime,
    managed_pools: Vec<std::sync::Arc<crate::rotation::manager::ManagedPool>>,
}
type FundingRuntime = (
    crate::treasury::actor::TreasuryCommands,
    crate::treasury::submission::GrpcSubmission,
    crate::rotation::funding::FundingWorker<crate::rotation::funding::Backend>,
);
pub struct RunningDeployment {
    startup_catalogs: tokio::sync::oneshot::Receiver<readiness::FundingPools>,
    deferred: Option<deferred::Runtime>,
    catalog_readiness: BTreeMap<String, tokio::sync::oneshot::Receiver<Result<()>>>,
    accepted_work: Vec<crate::server::work::Work>,
    #[cfg(test)]
    listener_failure: Option<(String, tokio::sync::oneshot::Receiver<()>)>,
    #[cfg(test)]
    funding_failure: Option<tokio::sync::oneshot::Receiver<()>>,
    runtime: ManagedRuntime,
    managed_pools: Vec<std::sync::Arc<crate::rotation::manager::ManagedPool>>,
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
        let direct_service = self.runtime.direct_service();
        let (treasury_task, funding_task, retained_runtime) = match self.runtime {
            ManagedRuntime::Treasury(treasury) => {
                let TreasuryRuntime {
                    owner,
                    funding: (commands, sender, worker),
                    direct,
                } = *treasury;
                drop(direct);
                (
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
                    ManagedRuntime::None,
                )
            }
            ManagedRuntime::Direct(direct) => {
                let DirectRuntime {
                    owner,
                    commands,
                    sender,
                    service,
                } = *direct;
                drop(service);
                (
                    Some(tokio::spawn({
                        let stopped = stop.clone();
                        async move {
                            let _stop_on_exit = stopped.clone().drop_guard();
                            owner.run_commands(commands, sender, stopped).await
                        }
                    })),
                    None,
                    ManagedRuntime::None,
                )
            }
            runtime => (None, None, runtime),
        };
        let mut reconciliation = JoinSet::new();
        if let Some(service) = direct_service {
            let stopped = reconciliation_stop.clone();
            reconciliation.spawn(async move {
                if let Err(error) = service.run(stopped).await {
                    tracing::error!(category = %crate::rotation::base::safe_diagnostic(&error), "Direct payment processing and observation stopped; accepted payments remain saved; restart to resume processing");
                }
            });
        }
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
        let mut catalog_readiness = self.catalog_readiness;
        let mut listening = Vec::new();
        for (name, listener, app) in self.listeners {
            let stopped = listener_stop.clone();
            let readiness = catalog_readiness.remove(&name);
            #[cfg(test)]
            let failure = if listener_failure.as_ref().is_some_and(|(id, _)| id == &name) {
                listener_failure.take().map(|(_, signal)| signal)
            } else {
                None
            };
            let (listening_send, listening_receive) = tokio::sync::oneshot::channel();
            listening.push(listening_receive);
            tasks.spawn(async move {
                if let Some(readiness) = readiness {
                    tracing::info!(server = name, "MCP endpoint waiting for catalogs");
                    let ready = tokio::select! {
                        biased;
                        _ = stopped.cancelled() => return Ok(()),
                        ready = readiness => ready
                            .context("catalog discovery worker stopped before endpoint readiness")
                            .and_then(|result| result),
                    };
                    if let Err(error) = ready {
                        // Only fixed diagnostics and authored identities cross this
                        // boundary; upstream errors/content must never be included.
                        tracing::error!(server = name,
                            failure_stage = error.downcast_ref::<catalog::LoadStage>().map(|stage| stage.label()),
                            reason = %format_args!("{error:#}"),
                            "MCP endpoint unavailable; catalog startup failed; correct the reported cause and restart");
                        drop(listener);
                        drop(app);
                        stopped.cancelled().await;
                        return Ok(());
                    }
                }
                if stopped.is_cancelled() {
                    return Ok(());
                }
                let address = listener.local_addr()?;
                tracing::info!(server = name, %address, "MCP listening at /mcp");
                let _ = listening_send.send(());
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
        let mut startup_announcement =
            Box::pin(readiness::announce(self.startup_catalogs, listening));
        let mut announcement_finished = false;
        let mut funding_stopped = false;
        let result = loop {
            tokio::select! {
                _ = shutdown.cancelled() => break Ok(()),
                _ = &mut startup_announcement, if !announcement_finished => announcement_finished = true,
                _ = stop.cancelled(), if !funding_stopped => {
                    funding_stopped = true;
                    tracing::error!(category = "treasury_funding_stopped",
                        "Treasury/funding stopped; restart required for funding. Listeners and Base reconciliation remain available; payments retain payer permission checks, durable journals and all liabilities");
                },
                ended = tasks.join_next() => break match ended {
                    Some(Ok(Err(e))) => Err(e),
                    Some(Err(e)) => Err(e.into()),
                    _ => Err(anyhow::anyhow!("MCP listener exited unexpectedly")),
                }
            }
        };
        drop(startup_announcement);
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
        while reconciliation.join_next().await.is_some() {}
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
            let store_result = retained_runtime.close().await;
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
mod direct_swap_tests;
#[cfg(test)]
mod store_loss_tests;
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
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
[wallets.shared.reserves.small]
funding_amount_usdc="5"
[sources.test]
spec="spec.json"
base_url="https://example.com"
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
        let timestamp = crate::rotation::base::now().unwrap();
        let rpc_task = tokio::spawn(async move {
            let app = axum::Router::new().route("/",post(move |Json(v):Json<Value>| {
                let seen = seen.clone();
                async move {
                    let result = match v["method"].as_str().unwrap() {
                        "eth_chainId" => json!("0x2105"),
                        "eth_getBlockByNumber" => {
                            let height = if matches!(v["params"][0].as_str(), Some("latest" | "finalized")) {100} else {
                                u64::from_str_radix(v["params"][0].as_str().unwrap().trim_start_matches("0x"),16).unwrap()
                            };
                            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{timestamp:x}")})
                        },
                        "eth_call" => { seen.add_permits(1); json!(format!("0x{:064x}",5_000_000)) },
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
            crate::rotation::test_capacity::small_reserve("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        store
            .record_funding_balance(
                &job.id,
                &job.operation_id,
                &job.wallet_id,
                &job.recipient,
                alloy_primitives::U256::from(5_000_000),
            )
            .unwrap();
        store.low_balance(&pool, &job.wallet_id).unwrap();
        let refresh_pool = pool.clone();
        let refresh_wallet = job.wallet_id.clone();
        let (store, store_task) = crate::rotation::store::StoreHandle::spawn(store);
        running.managed_pools.push(Arc::new(
            crate::rotation::manager::ManagedPool::new(
                store.clone(),
                pool,
                crate::rotation::base::BaseRpc::new(&rpc_url).unwrap(),
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
        // A handler count precedes client publication. Anchor the virtual clock
        // only after the shared wallet acquisition has published its cooldown.
        let next_due = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let due = store.balance_due(&job.wallet_id).await;
                if due > tokio::time::Instant::now() {
                    break due;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        failure.send(()).unwrap();
        store
            .call(move |s| s.low_balance(&refresh_pool, &refresh_wallet))
            .await
            .unwrap();
        // Advance the owned cooldown plus one maintenance wake, then run time
        // normally for actual RPC/MCP IO and Store work. The follow-up must
        // survive the funding stop without cancelling reconciliation.
        tokio::time::pause();
        tokio::time::advance(
            next_due.saturating_duration_since(tokio::time::Instant::now())
                + Duration::from_secs(5),
        )
        .await;
        tokio::time::resume();
        tokio::time::timeout(Duration::from_secs(5), reads.acquire())
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
            1
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
[wallets.shared]
mode = "static"
private_key_env = "KEY"
[servers.one]
provider_management_remote = true
source_registry_file = "registry.sqlite"
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

#[cfg(test)]
mod initialization_tests;

#[cfg(test)]
mod validation_tests;

#[cfg(test)]
#[path = "deployment/store_only_tests.rs"]
mod store_only_tests;

#[cfg(test)]
mod prebind_tests;

mod deferred;
