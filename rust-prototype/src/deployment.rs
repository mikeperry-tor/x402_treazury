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
    pub sources: BTreeMap<String, SourceConfig>,
    pub wallets: BTreeMap<String, WalletConfig>,
    pub servers: BTreeMap<String, ListenerConfig>,
}
pub type SourceConfig = toml::Table;
fn default_cap() -> String {
    "1.00".into()
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WalletConfig {
    pub mode: String,
    pub private_key_env: String,
    #[serde(default = "default_cap")]
    pub max_price_usd: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    pub listen: SocketAddr,
    pub bearer_token_env: String,
    pub wallet: String,
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
    http: reqwest::Client,
    instructions: Option<String>,
}
#[derive(Serialize)]
pub struct Inventory {
    pub server: String,
    pub listen: SocketAddr,
    pub wallet: String,
    pub tools: Vec<InventoryTool>,
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
}
fn pattern(text: &str) -> Result<Regex> {
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
    fn validate(&self) -> Result<()> {
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
            .chain(self.servers.keys())
        {
            ensure!(
                identifier.is_match(name),
                "invalid identifier {name}: use lowercase letters, digits and underscores"
            );
        }
        for (name, wallet) in &self.wallets {
            ensure!(
                wallet.mode == "static",
                "wallet {name}: only static mode is implemented"
            );
            ensure!(
                !wallet.private_key_env.trim().is_empty(),
                "wallet {name}: private_key_env is required"
            );
            SpendPolicy::dollars(&wallet.max_price_usd)
                .with_context(|| format!("wallet {name}: invalid max_price_usd"))?;
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
            ensure!(
                self.wallets.contains_key(&server.wallet),
                "server {name}: unknown wallet {}",
                server.wallet
            );
            ensure!(
                !server.sources.is_empty(),
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
}
impl Deployment {
    pub async fn show_config(path: &Path) -> Result<serde_json::Value> {
        let config: MetaConfig = toml::from_str(&tokio::fs::read_to_string(path).await?)?;
        config.validate()?;
        let mut resolved = BTreeMap::new();
        for (id, source) in &config.sources {
            let mut provider = crate::config::resolve(source.clone(), path).await?;
            if provider.settings.prefix.is_none() {
                provider.settings.prefix = Some(id.clone());
                provider
                    .origins
                    .insert("prefix".into(), "source identifier".into());
            }
            resolved.insert(id, provider);
        }
        Ok(
            serde_json::json!({"version":config.version,"sources":resolved,"wallets":config.wallets,"servers":config.servers}),
        )
    }
    pub async fn load(path: &Path) -> Result<Self> {
        let config: MetaConfig = toml::from_str(&tokio::fs::read_to_string(path).await?)
            .context("invalid meta-config")?;
        config.validate()?;
        let mut sources = BTreeMap::new();
        for (id, source) in &config.sources {
            let mut cfg = crate::config::resolve(source.clone(), path)
                .await
                .with_context(|| format!("source {id}"))?
                .settings;
            if cfg.prefix.is_none() {
                cfg.prefix = Some(id.clone());
            }
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs_f64(cfg.timeout))
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
            let document = catalog::load_json(&cfg.spec, &http)
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
            sources.insert(
                id.clone(),
                Source {
                    tools,
                    base_url,
                    http,
                    instructions: cfg.instructions_text.clone(),
                    config: cfg,
                    document,
                },
            );
        }
        let mut selected = BTreeMap::new();
        for (name, server) in &config.servers {
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
                    if (!server.tags.is_empty()
                        && !server.tags.iter().any(|t| tags.contains(t.as_str())))
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
            ensure!(!tools.is_empty(), "server {name}: no tools selected");
            selected.insert(name.clone(), tools);
        }
        Ok(Self {
            config,
            sources,
            selected,
        })
    }
    pub fn inventory(&self) -> Vec<Inventory> {
        self.config
            .servers
            .iter()
            .map(|(name, s)| Inventory {
                server: name.clone(),
                listen: s.listen,
                wallet: s.wallet.clone(),
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
    pub async fn bind(mut self, env: &BTreeMap<String, String>) -> Result<RunningDeployment> {
        let secret = |key: &str| {
            env.get(key)
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .with_context(|| format!("required environment variable {key} is missing or empty"))
        };
        // Validate every credential before opening any port.
        let mut tokens = BTreeMap::new();
        for (name, cfg) in &self.config.servers {
            tokens.insert(name.clone(), secret(&cfg.bearer_token_env)?);
        }
        let mut wallets = BTreeMap::new();
        for cfg in self.config.servers.values() {
            if !wallets.contains_key(&cfg.wallet) {
                let w = &self.config.wallets[&cfg.wallet];
                let payer = Payer::new(
                    &secret(&w.private_key_env)?,
                    SpendPolicy::dollars(&w.max_price_usd)?,
                )
                .with_context(|| format!("wallet {}: invalid signing configuration", cfg.wallet))?;
                wallets.insert(
                    cfg.wallet.clone(),
                    PaidClient::new(reqwest::Client::new(), payer),
                );
            }
        }
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
        let mut listeners = Vec::new();
        for (name, cfg) in &self.config.servers {
            let selected = &self.selected[name];
            let bindings = selected
                .iter()
                .map(|(id, t)| {
                    let source = &self.sources[id];
                    (
                        t.clone(),
                        wallets[&cfg.wallet].with_http(source.http.clone()),
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
            let listener = TcpListener::bind(cfg.listen)
                .await
                .with_context(|| format!("server {name}: cannot bind {}", cfg.listen))?;
            listeners.push((
                name.clone(),
                listener,
                http_app(server, tokens.remove(name).unwrap()),
            ));
        }
        Ok(RunningDeployment { listeners })
    }
}
pub struct RunningDeployment {
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
        let mut tasks = JoinSet::new();
        for (name, listener, app) in self.listeners {
            let stopped = stop.clone();
            tasks.spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(stopped.cancelled_owned())
                    .await
                    .with_context(|| format!("server {name} stopped unexpectedly"))
            });
        }
        let result = tokio::select! {
            _ = shutdown.cancelled() => Ok(()),
            ended = tasks.join_next() => match ended {
                Some(Ok(Err(e))) => Err(e),
                Some(Err(e)) => Err(e.into()),
                _ => Err(anyhow::anyhow!("MCP listener exited unexpectedly")),
            }
        };
        stop.cancel();
        if tokio::time::timeout(Duration::from_secs(10), async {
            while tasks.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            bail!("shutdown deadline exceeded; pending paid calls may have unknown outcomes");
        }
        result
    }
}
