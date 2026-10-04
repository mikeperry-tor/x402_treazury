//! Bounded, scoped catalog futures: no detached tasks or partial inventory publication.
use super::{MetaConfig, Source, SourceConfig};
use crate::catalog;
use anyhow::{Context, Result, ensure};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::OnceCell;
use tracing::Instrument;

type DownloadKey = (
    String,
    Duration,
    usize,
    crate::network::IsolationId,
    crate::network::HttpPolicy,
);
type Document = Arc<serde_json::Value>;

/// One deployment load, one immutable network policy/runtime. Retain parsed remote
/// documents only for this load; aliases still generate/filter/bind independently.
#[derive(Default)]
struct Downloads(Mutex<BTreeMap<DownloadKey, Arc<OnceCell<Document>>>>);
impl Downloads {
    async fn load(
        &self,
        id: &str,
        cfg: &catalog::Config,
        http: &reqwest::Client,
    ) -> Result<Document> {
        if !(cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://")) {
            return Ok(Arc::new(
                catalog::load_json_with_limit(&cfg.spec, http, cfg.max_spec_bytes).await?,
            ));
        }
        // Exact URL (including query), requested timeout and byte limit are deliberately
        // conservative. Never let a permissive alias bypass another alias's policy.
        let key = (
            cfg.spec.clone(),
            Duration::from_secs_f64(cfg.timeout),
            cfg.max_spec_bytes,
            crate::network::IsolationId::discovery(&cfg.spec)?,
            cfg.transport(),
        );
        let cell = {
            let mut entries = self.0.lock().expect("catalog download map poisoned");
            if entries.contains_key(&key) {
                tracing::info!(target: "x402_treazury::startup", source = id,
                    "Sharing compatible catalog download or parsed document");
            }
            entries.entry(key).or_default().clone()
        };
        // No detached tasks: cancellation drops the initializer and its HTTP request.
        let document = cell
            .get_or_try_init(|| async {
                Ok::<_, anyhow::Error>(Arc::new(
                    catalog::load_json_with_limit(&cfg.spec, http, cfg.max_spec_bytes).await?,
                ))
            })
            .await?;
        Ok(document.clone())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StartupConfig {
    pub catalog_concurrency: usize,
}
impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            catalog_concurrency: 2,
        }
    }
}
impl StartupConfig {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            (1..=64).contains(&self.catalog_concurrency),
            "startup.catalog_concurrency must be in 1..64"
        );
        Ok(())
    }
}

// A dropped request future must also leave visible evidence (failure or caller cancellation).
struct Progress<'a> {
    source: &'a str,
    phase: &'static str,
    started: Instant,
    finished: bool,
}
impl Drop for Progress<'_> {
    fn drop(&mut self) {
        if !self.finished {
            tracing::warn!(target: "x402_treazury::startup", source = self.source,
                phase = self.phase, elapsed_ms = self.started.elapsed().as_millis() as u64,
                "Startup source work cancelled; no partial inventory published");
        }
    }
}

pub(super) async fn load(
    config: &MetaConfig,
    path: &Path,
    warn: bool,
) -> Result<BTreeMap<String, Source>> {
    let started = Instant::now();
    let total = config.sources.len();
    let limit = config.startup.catalog_concurrency;
    tracing::info!(target: "x402_treazury::startup", total, concurrency = limit, "Loading catalogs; startup waits for all sources");
    let downloads = Downloads::default();
    let mut loads = Vec::with_capacity(total);
    for (id, source) in &config.sources {
        let used = config.servers.values().any(|s| s.sources.contains(id));
        loads.push(load_one(id, source, path, warn && used, &downloads));
    }
    let mut pending = stream::iter(loads).buffer_unordered(limit);
    let mut sources = BTreeMap::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = pending.next() => match result {
                Some(result) => {
                    let (id, source) = result?;
                    sources.insert(id.clone(), source);
                    tracing::info!(target: "x402_treazury::startup", source = id, completed = sources.len(), total,
                        elapsed_ms = started.elapsed().as_millis() as u64, "Catalog ready");
                }
                None => break,
            },
            _ = heartbeat.tick() => tracing::info!(target: "x402_treazury::startup", completed = sources.len(), total,
                remaining = total - sources.len(), elapsed_ms = started.elapsed().as_millis() as u64,
                "Startup waiting for catalogs; remaining count includes queued sources"),
        }
    }
    tracing::info!(target: "x402_treazury::startup", total, elapsed_ms = started.elapsed().as_millis() as u64, "All catalogs loaded");
    Ok(sources)
}

async fn load_one(
    id: &str,
    source: &SourceConfig,
    path: &Path,
    warn: bool,
    downloads: &Downloads,
) -> Result<(String, Source)> {
    let mut progress = Progress {
        source: id,
        phase: "configuration",
        started: Instant::now(),
        finished: false,
    };
    tracing::info!(target: "x402_treazury::startup", source = id, "Catalog load started");
    let result = Source::load(id, source, path, warn, &mut progress, downloads).await;
    progress.finished = true;
    if result.is_err() {
        tracing::warn!(target: "x402_treazury::startup", source = id, phase = progress.phase,
            elapsed_ms = progress.started.elapsed().as_millis() as u64, "Catalog load failed; aborting startup");
    }
    result.map(|source| (id.to_owned(), source))
}

impl Source {
    async fn load(
        id: &str,
        source: &SourceConfig,
        path: &Path,
        warn: bool,
        progress: &mut Progress<'_>,
        downloads: &Downloads,
    ) -> Result<Self> {
        let mut cfg = crate::config::resolve(source.provider.clone(), path)
            .await
            .with_context(|| format!("source {id}"))?
            .settings;
        if warn {
            crate::provider_status::warn(id, &cfg);
        }
        if cfg.prefix.is_none() {
            cfg.prefix = Some(id.to_owned());
        }
        let http = crate::network::provider_discovery(
            if cfg.spec.starts_with("http") {
                &cfg.spec
            } else {
                "https://local.invalid"
            },
            Duration::from_secs_f64(cfg.timeout),
            cfg.transport(),
        )?;
        progress.phase = "fetch_parse";
        let fetch_started = Instant::now();
        tracing::info!(target: "x402_treazury::startup", source = id, phase = progress.phase,
            "Loading catalog document");
        let document = downloads.load(id, &cfg, &http)
            .instrument(tracing::info_span!(target: "x402_treazury::startup", "catalog_download", source = id))
            .await
            .with_context(|| format!("source {id}: loading spec"))?;
        tracing::info!(target: "x402_treazury::startup", source = id,
            elapsed_ms = fetch_started.elapsed().as_millis() as u64, "Catalog fetch/parse finished");
        progress.phase = "generation";
        let generation_started = Instant::now();
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
        tracing::info!(target: "x402_treazury::startup", source = id, tools = tools.len(),
            elapsed_ms = generation_started.elapsed().as_millis() as u64, "Catalog generation finished");
        Ok(Self {
            tools,
            base_url,
            instructions: cfg.instructions_text.clone(),
            config: cfg,
            document,
        })
    }
}

/// Discover/rebuild a source without mutating any published listener inventory.
pub(super) async fn price_source<'a>(
    id: &'a str,
    source: &Source,
    selected: Vec<catalog::ToolSpec>,
) -> Result<(&'a str, BTreeMap<String, catalog::ToolSpec>)> {
    let mut progress = Progress {
        source: id,
        phase: "pricing",
        started: Instant::now(),
        finished: false,
    };
    tracing::info!(target: "x402_treazury::startup", source = id, enabled = source.config.probe_pricing,
        "Startup pricing source started");
    let result = async {
        let prices = crate::pricing::process_cache()
            .discover(&source.config, &source.document, &selected, &source.base_url).await?;
        tracing::info!(target: "x402_treazury::startup", source = id, prices = prices.len(),
            elapsed_ms = progress.started.elapsed().as_millis() as u64, "Startup pricing source finished");
        if prices.is_empty() {
            tracing::info!(target: "x402_treazury::startup", source = id,
                "No discovered prices; reusing original tool definitions");
            return Ok((id, source.tools.iter().cloned().map(|t| (t.name.clone(), t)).collect()));
        }
        let rebuild_started = Instant::now();
        let tools = catalog::build_tools_with_prices(
            &source.config, &source.document, source.config.prefix.as_deref().unwrap(), &prices,
        )?.into_iter().map(|t| (t.name.clone(), t)).collect();
        tracing::info!(target: "x402_treazury::startup", source = id,
            elapsed_ms = rebuild_started.elapsed().as_millis() as u64, "Startup pricing descriptions rebuilt");
        Ok((id, tools))
    }.await;
    progress.finished = true;
    if result.is_err() {
        tracing::warn!(target: "x402_treazury::startup", source = id,
            elapsed_ms = progress.started.elapsed().as_millis() as u64, "Startup pricing failed; aborting startup");
    }
    result
}
