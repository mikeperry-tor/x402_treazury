//! Bounded, scoped catalog futures: no detached tasks or partial inventory publication.
use super::catalog_evidence::{self, State};
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
    Option<std::path::PathBuf>,
    bool,
);
type Document = Arc<serde_json::Value>;

// Preserve the original typed failure across aliases without a second request.
#[derive(Clone, Debug)]
struct SharedFailure(Arc<anyhow::Error>);
impl std::fmt::Display for SharedFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}
impl std::error::Error for SharedFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref().as_ref())
    }
}
type DownloadResult = std::result::Result<Document, SharedFailure>;

/// One deployment load, one immutable network policy/runtime. Retain parsed remote
/// documents only for this load; aliases still generate/filter/bind independently.
#[derive(Default)]
struct Downloads {
    entries: Mutex<BTreeMap<DownloadKey, Arc<OnceCell<DownloadResult>>>>,
    cache_directory: Option<std::path::PathBuf>,
}
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
            cfg.http_cache_directory.clone(),
            cfg.http_cache_enabled,
        );
        let cell = {
            let mut entries = self.entries.lock().expect("catalog download map poisoned");
            if entries.contains_key(&key) {
                tracing::debug!(target: "x402_treazury::startup", source = id,
                    "Sharing compatible catalog download or parsed document");
            }
            entries.entry(key).or_default().clone()
        };
        // No detached tasks: cancellation drops the initializer and its HTTP request.
        cell.get_or_init(|| async {
            catalog::load_json_cached(
                &cfg.spec,
                http,
                cfg.max_spec_bytes,
                crate::http_cache::Slot::new(cfg, &cfg.spec, "catalog", cfg.max_spec_bytes),
            )
            .await
            .map(Arc::new)
            .map_err(|error| SharedFailure(Arc::new(error)))
        })
        .await
        .clone()
        .map_err(anyhow::Error::new)
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
            catalog_evidence::record(self.source, State::Cancelled, self.phase, None);
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
    catalog_evidence::register(config.sources.keys())?;
    let limit = config.startup.catalog_concurrency;
    tracing::info!(target: "x402_treazury::startup", total, concurrency = limit, "Loading catalogs; startup waits for all sources");
    let downloads = Downloads {
        entries: Mutex::default(),
        cache_directory: config
            .treasury
            .as_ref()
            .filter(|t| {
                t.state_dir.is_dir()
                    && !catalog_evidence::collecting()
                    && !crate::qualification::active()
            })
            .map(|t| t.state_dir.join("http-cache")),
    };
    let mut loads = Vec::with_capacity(total);
    for (id, source) in &config.sources {
        let used = config.servers.values().any(|s| s.sources.contains(id));
        loads.push(load_one(id, source, path, warn && used, &downloads));
    }
    let mut pending = stream::iter(loads).buffer_unordered(limit);
    let mut sources = BTreeMap::new();
    let collect_failures = catalog_evidence::collecting();
    let mut failures = 0usize;
    let mut first_error = None;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = pending.next() => match result {
                Some(result) => {
                    let (id, source) = match result {
                        Ok(source) => source,
                        Err(error) if collect_failures => {
                            failures += 1;
                            if first_error.is_none() { first_error = Some(error); }
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    sources.insert(id.clone(), source);
                    tracing::info!(target: "x402_treazury::startup", source = id, completed = sources.len(), total,
                        elapsed_ms = started.elapsed().as_millis() as u64, "Catalog ready");
                }
                None => break,
            },
            _ = heartbeat.tick() => tracing::debug!(target: "x402_treazury::startup", completed = sources.len(), total,
                failed = failures, remaining = total - sources.len() - failures, elapsed_ms = started.elapsed().as_millis() as u64,
                "Startup waiting for catalogs; remaining count includes queued sources"),
        }
    }
    if let Some(error) = first_error {
        tracing::warn!(target: "x402_treazury::startup", total, failed = failures,
            completed = sources.len(), "Catalog inspection finished with failures; no partial inventory published");
        return Err(error.context(format!("catalog inspection failed for {failures} of {total} sources; independent catalog results retained")));
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
    tracing::debug!(target: "x402_treazury::startup", source = id, "Catalog load started");
    let result = Source::load(id, source, path, warn, &mut progress, downloads).await;
    progress.finished = true;
    if let Err(error) = &result {
        let error = error
            .downcast_ref::<SharedFailure>()
            .map(|shared| shared.0.as_ref())
            .unwrap_or(error);
        let phase = error
            .downcast_ref::<catalog::LoadStage>()
            .map(|stage| stage.label())
            .unwrap_or(progress.phase);
        catalog_evidence::record(
            id,
            State::Failed,
            phase,
            error
                .downcast_ref::<reqwest::Error>()
                .and_then(|e| e.status())
                .map(|s| s.as_u16()),
        );
        tracing::warn!(target: "x402_treazury::startup", source = id, phase = progress.phase,
            failure_stage = phase,
            http_status = error.downcast_ref::<reqwest::Error>().and_then(|e| e.status()).map(|s| s.as_u16()),
            elapsed_ms = progress.started.elapsed().as_millis() as u64,
            collecting_independent_results = catalog_evidence::collecting(),
            "Catalog load failed; complete inventory unavailable");
    } else {
        catalog_evidence::record(id, State::Completed, "generation", None);
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
        cfg.http_cache_directory = downloads.cache_directory.clone();
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
        tracing::debug!(target: "x402_treazury::startup", source = id, phase = progress.phase,
            "Loading catalog document");
        let document = downloads.load(id, &cfg, &http)
            .instrument(tracing::debug_span!(target: "x402_treazury::startup", "catalog_download", source = id))
            .await
            .with_context(|| format!("source {id}: loading spec"))?;
        tracing::debug!(target: "x402_treazury::startup", source = id,
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
        cfg.resolve_cover(&base_url, crate::network::global().policy.cover_enabled())?;
        let parsed = reqwest::Url::parse(&base_url)?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some(),
            "source {id}: base_url must be HTTP(S)"
        );
        let tools = catalog::build_tools(&cfg, &document, cfg.prefix.as_deref().unwrap())
            .with_context(|| format!("source {id}: catalog generation"))?;
        tracing::debug!(target: "x402_treazury::startup", source = id, tools = tools.len(),
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
    tracing::debug!(target: "x402_treazury::startup", source = id, enabled = source.config.probe_pricing,
        "Startup pricing source started");
    crate::qualification::record_pricing(id, crate::qualification::PricingStage::Started).await?;
    let result = async {
        let discovery = crate::pricing::process_cache()
            .discover_observed(&source.config, &source.document, &selected, &source.base_url).await;
        let stage=match &discovery {
            Ok(d)=>crate::qualification::PricingStage::Completed{evidence:d.evidence.clone(),prices:d.prices.iter().map(|((method,path),line)|crate::pricing::Price{method:method.clone(),path:path.clone(),line:line.clone()}).collect()},
            Err(error)=>crate::qualification::PricingStage::Failed{failure:crate::qualification::failure_category(error)},
        };
        crate::qualification::record_pricing(id,stage).await?;
        let prices=discovery?.prices;
        tracing::info!(target: "x402_treazury::startup", source = id, enabled = source.config.probe_pricing, prices = prices.len(),
            elapsed_ms = progress.started.elapsed().as_millis() as u64, "Startup pricing source finished");
        if prices.is_empty() {
            tracing::debug!(target: "x402_treazury::startup", source = id,
                "No discovered prices; reusing original tool definitions");
            return Ok((id, source.tools.iter().cloned().map(|t| (t.name.clone(), t)).collect()));
        }
        let rebuild_started = Instant::now();
        let tools = catalog::build_tools_with_prices(
            &source.config, &source.document, source.config.prefix.as_deref().unwrap(), &prices,
        )?.into_iter().map(|t| (t.name.clone(), t)).collect();
        tracing::debug!(target: "x402_treazury::startup", source = id,
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
