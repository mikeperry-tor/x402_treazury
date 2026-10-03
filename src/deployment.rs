//! Resolve catalogs without secrets, then bind every authenticated listener before serving.
use crate::{
    catalog::{self, Config, ToolSpec},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, http_app},
};
use anyhow::{Context, Result, bail, ensure};
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
    pub network: crate::network::NetworkPolicy,
    pub treasury: Option<TreasuryConfig>,
    pub funding: Option<FundingConfig>,
    pub source_management: Option<crate::discovery::policy::Policy>,
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
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    pub listen: SocketAddr,
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
    document: serde_json::Value,
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
        self.network.validate()?;
        crate::discovery::policy::validate(self)?;
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
                !server.bearer_token_env.trim().is_empty(),
                "server {name}: bearer_token_env is required"
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
            serde_json::json!({"source_management":crate::discovery::policy::inspection(&config),"network":config.network.inspection(),"version":config.version,"treasury":config.treasury,"funding":config.funding,"sources":resolved,"wallets":config.wallets,"servers":config.servers,"wallet_bindings":wallet_resolution.bindings,"wallet_templates":config.wallet_templates,"wallet_assignment":config.wallet_assignment,"resolved_wallets":wallet_resolution.wallets,"generated_wallets":wallet_resolution.generated,"wallet_summary":wallet_resolution.summary}),
        )
    }
    pub async fn load(path: &Path) -> Result<Self> {
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
        let mut sources = BTreeMap::new();
        for (id, source) in &config.sources {
            sources.insert(id.clone(), Source::load(id, source, path).await?);
        }
        let mut selected = BTreeMap::new();
        for (name, server) in &config.servers {
            selected.insert(name.clone(), select_listener_tools(name, server, &sources)?);
        }
        Ok(Self {
            config,
            sources,
            selected,
            wallet_resolution,
            config_path: path.to_owned(),
        })
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
                    crate::discovery::tools::definitions(g.enabled, g.accept_sources)
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
    ) -> Result<InitializedWallets> {
        let secret = |key: &str| required_secret(env, key);
        // Static-only serving does not unlock state, but must not reuse a known
        // managed profile name as a different wallet identity.
        if !self
            .wallet_resolution
            .wallets
            .values()
            .any(WalletConfig::managed)
            && let Some(t) = &self.config.treasury
            && t.state_dir.exists()
        {
            let dir = t.state_dir.clone();
            let state =
                tokio::task::spawn_blocking(move || crate::rotation::store::status(&dir)).await??;
            ensure!(state.treasury_id == t.id, "state identity mismatch");
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
        if self
            .wallet_resolution
            .wallets
            .values()
            .any(WalletConfig::managed)
        {
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
                    secret(&t.indexer_url_env)?,
                    t.confirmations,
                    t.max_sync_age_seconds,
                )?;
                secure_endpoint(&secret(&t.submission_url_env)?)?;
                if let Some(key) = &f.near_api_key_env {
                    secret(key)?;
                }
                let base = BaseRpc::new(
                    &secret(&f.base_rpc_url_env)?,
                    f.base_confirmations,
                    f.base_max_block_age_seconds,
                )?;
                let mut owner = crate::treasury::Treasury::open(
                    t.state_dir.clone(),
                    t.key_file.clone(),
                    t.id.clone(),
                )
                .await?;
                owner.configure_sync(sync_settings);
                let store = owner.store_handle();
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
                        secret(&t.submission_url_env)?,
                        secret(&t.indexer_url_env)?,
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

    async fn discover_prices(&mut self) -> Result<()> {
        for (id, source) in &self.sources {
            let selected: Vec<_> = self
                .selected
                .values()
                .flatten()
                .filter(|(s, _)| s == id)
                .map(|(_, t)| t.clone())
                .collect();
            if selected.is_empty() {
                continue;
            }
            let prices = crate::pricing::process_cache()
                .discover(
                    &source.config,
                    &source.document,
                    &selected,
                    &source.base_url,
                )
                .await?;
            let tools: BTreeMap<_, _> = catalog::build_tools_with_prices(
                &source.config,
                &source.document,
                source.config.prefix.as_deref().unwrap(),
                &prices,
            )?
            .into_iter()
            .map(|t| (t.name.clone(), t))
            .collect();
            for (s, t) in self.selected.values_mut().flatten() {
                if s == id {
                    *t = tools[&t.name].clone();
                }
            }
        }
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

    pub async fn bind(mut self, env: &BTreeMap<String, String>) -> Result<RunningDeployment> {
        let secret = |key: &str| required_secret(env, key);
        // Validate every credential before opening any port.
        let mut tokens = BTreeMap::new();
        for (name, cfg) in &self.config.servers {
            tokens.insert(name.clone(), secret(&cfg.bearer_token_env)?);
        }
        let initialized = self.initialize_wallets(env).await?;
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
            let tools = server.catalog.read().views["default"].clone();
            if self.config.source_management.is_some() {
                ensure!(
                    tools.iter().all(|t| !t.tool.name.starts_with("treazure_")
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
            server.discovery = manager.clone();
            let listener = TcpListener::bind(cfg.listen)
                .await
                .with_context(|| format!("server {name}: cannot bind {}", cfg.listen))?;
            listeners.push((
                name.clone(),
                listener,
                http_app(server, tokens.remove(name).unwrap()),
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
impl Source {
    async fn load(id: &str, source: &SourceConfig, path: &Path) -> Result<Self> {
        let mut cfg = crate::config::resolve(source.provider.clone(), path)
            .await
            .with_context(|| format!("source {id}"))?
            .settings;
        if cfg.prefix.is_none() {
            cfg.prefix = Some(id.to_owned());
        }
        let http = crate::network::discovery(
            if cfg.spec.starts_with("http") {
                &cfg.spec
            } else {
                "https://local.invalid"
            },
            Duration::from_secs_f64(cfg.timeout),
        )?;
        let document = catalog::load_json_with_limit(&cfg.spec, &http, cfg.max_spec_bytes)
            .await
            .with_context(|| format!("source {id}: loading spec"))?;
        let base_url = cfg
            .base_url
            .clone()
            .or_else(|| {
                document
                    .pointer("/servers/0/url")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
            .with_context(|| format!("source {id}: base_url required"))?;
        let parsed = reqwest::Url::parse(&base_url)?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some(),
            "source {id}: base_url must be HTTP(S)"
        );
        let tools = catalog::build_tools(&cfg, &document, cfg.prefix.as_deref().unwrap())
            .with_context(|| format!("source {id}: catalog generation"))?;
        Ok(Self {
            tools,
            base_url,
            instructions: cfg.instructions_text.clone(),
            config: cfg,
            document,
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
            let tags: BTreeSet<_> = operations
                .iter()
                .filter(|op| {
                    op["path"] == tool.path
                        && op["method"]
                            .as_str()
                            .is_some_and(|m| m.eq_ignore_ascii_case(&tool.method))
                })
                .flat_map(|op| {
                    op["tags"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_str)
                })
                .collect();
            if (!server.tags.is_empty() && !server.tags.iter().any(|t| tags.contains(t.as_str())))
                || server
                    .exclude_tags
                    .iter()
                    .any(|t| tags.contains(t.as_str()))
            {
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
                loop {
                    tokio::select! {
                        _ = stopped.cancelled() => break,
                        result = pool.reconcile() => { if result.is_err() { tracing::debug!("background Base reconciliation unavailable"); } }
                    }
                    tokio::select! { _ = stopped.cancelled()=>break, _ = tokio::time::sleep(Duration::from_secs(5))=>{} }
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
