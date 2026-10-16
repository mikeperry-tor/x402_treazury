//! Bounded initial catalog loading and retained relay continuations for serving.
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
    Option<Duration>,
    usize,
    crate::network::IsolationId,
    crate::network::HttpPolicy,
    Option<std::path::PathBuf>,
    bool,
    u64,
    Option<String>,
);
type Document = Arc<serde_json::Value>;

#[derive(Clone)]
struct DeferredDocument(Arc<DeferredLoad>);
struct DeferredLoad {
    config: catalog::Config,
    http: reqwest::Client,
    prepared: OnceCell<std::result::Result<catalog::CatalogLoad, SharedFailure>>,
}
impl DeferredDocument {
    async fn resume(&self, gate: Arc<tokio::sync::Semaphore>) -> Result<Document> {
        let prepared = self
            .0
            .prepared
            .get_or_init(|| async {
                let _permit = gate.acquire().await.expect("owned catalog gate");
                catalog::prepare_json_discovery(&self.0.config, &self.0.http)
                    .await
                    .map_err(|error| SharedFailure(Arc::new(error)))
            })
            .await;
        match prepared {
            Ok(catalog::CatalogLoad::Ready(document)) => Ok(Arc::new(document.clone())),
            Ok(catalog::CatalogLoad::RelayPending(pending)) => {
                Ok(Arc::new(pending.resume_when_ready(gate).await?))
            }
            Err(error) => Err(anyhow::Error::new(error.clone())),
        }
    }
}
impl std::fmt::Debug for DeferredDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deferred remote catalog")
    }
}
impl std::fmt::Display for DeferredDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for DeferredDocument {}
#[derive(Clone, Debug)]
pub(super) struct PendingSource {
    pub config: catalog::Config,
    document: DeferredDocument,
}
impl std::fmt::Display for PendingSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("source awaiting remote catalog")
    }
}
impl std::error::Error for PendingSource {}
impl PendingSource {
    pub async fn resume(&self, id: &str, gate: Arc<tokio::sync::Semaphore>) -> Result<Source> {
        let document = self.document.resume(gate).await?;
        Source::from_document(id, self.config.clone(), document)
    }
}

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
    defer: bool,
    relay: crate::discovery_relay::Relays,
    entries: Mutex<BTreeMap<DownloadKey, Arc<OnceCell<DownloadResult>>>>,
    cache_directory: Option<std::path::PathBuf>,
    direct_warm_target: Option<crate::network::NetworkPolicy>,
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
            cfg.read_timeout_seconds.map(Duration::from_secs_f64),
            cfg.max_spec_bytes,
            crate::network::IsolationId::discovery(&cfg.spec)?,
            cfg.transport(),
            cfg.http_cache_directory.clone(),
            cfg.http_cache_enabled,
            cfg.catalog_cache_ttl_seconds,
            cfg.discovery_relay.as_ref().map(|r| r.key.clone()),
        );
        let cell = {
            let mut entries = self.entries.lock().expect("catalog download map poisoned");
            if entries.contains_key(&key) {
                tracing::info!(target: "x402_treazury::startup", source = id, cache = "shared_load",
                    "Sharing compatible catalog download or parsed document");
            }
            entries.entry(key).or_default().clone()
        };
        // No detached tasks: cancellation drops the initializer and its HTTP request.
        cell.get_or_init(|| async {
            let result = if self.defer {
                Err(DeferredDocument(Arc::new(DeferredLoad {
                    config: cfg.clone(),
                    http: http.clone(),
                    prepared: OnceCell::new(),
                }))
                .into())
            } else {
                catalog::load_json_discovery(cfg, http).await
            };
            result
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

#[derive(Debug)]
struct RemoteCatalogUnavailable {
    prefix: String,
}
impl std::fmt::Display for RemoteCatalogUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("remote catalog unavailable")
    }
}
impl std::error::Error for RemoteCatalogUnavailable {}

pub(super) async fn load(
    config: &MetaConfig,
    path: &Path,
    warn: bool,
    relay: crate::discovery_relay::Relays,
    defer: bool,
) -> Result<(
    BTreeMap<String, Source>,
    BTreeMap<String, String>,
    BTreeMap<String, PendingSource>,
)> {
    let started = Instant::now();
    let total = config.sources.len();
    catalog_evidence::register(config.sources.keys())?;
    let limit = config.startup.catalog_concurrency;
    tracing::info!(target: "x402_treazury::startup", total, concurrency = limit, defer_relay = defer, "Loading initial catalogs");
    let downloads = Downloads {
        defer,
        relay,
        direct_warm_target: None,
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
        let downloads = &downloads;
        loads.push(async move {
            (
                id,
                load_one(id, source, path, warn && used, downloads).await,
            )
        });
    }
    let mut pending = stream::iter(loads).buffer_unordered(limit);
    let mut sources = BTreeMap::new();
    let mut unavailable = BTreeMap::new();
    let mut deferred = BTreeMap::new();
    let collect_failures = catalog_evidence::collecting();
    let mut failures = 0usize;
    let mut first_error = None;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = pending.next() => match result {
                Some((source_id, result)) => {
                    let (id, source) = match result {
                        Err(error) if error.is::<PendingSource>() => {
                            let source = error.downcast::<PendingSource>().expect("typed pending source");
                            unavailable.insert(source_id.clone(), source.config.prefix.clone().expect("resolved prefix"));
                            deferred.insert(source_id.clone(), source);
                            tracing::info!(source=source_id, state="pending", "Catalog relay continuation retained for serving");
                            continue;
                        }
                        Err(error) if warn && !crate::qualification::active() && !catalog_evidence::collecting()
                            && config.sources[source_id].optional_startup && error.is::<RemoteCatalogUnavailable>() => {
                            failures += 1;
                            unavailable.insert(source_id.clone(), error.downcast_ref::<RemoteCatalogUnavailable>().expect("typed remote failure").prefix.clone());
                            tracing::warn!(source = source_id, state = "unavailable", "Optional source unavailable; required inventory remains complete");
                            continue;
                        }
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
                failed = failures, remaining = total - sources.len() - failures - deferred.len(), elapsed_ms = started.elapsed().as_millis() as u64,
                "Startup waiting for catalogs; remaining count includes queued sources"),
        }
    }
    if let Some(error) = first_error {
        tracing::warn!(target: "x402_treazury::startup", total, failed = failures,
            completed = sources.len(), "Catalog inspection finished with failures; no partial inventory published");
        return Err(error.context(format!("catalog inspection failed for {failures} of {total} sources; independent catalog results retained")));
    }
    tracing::info!(target: "x402_treazury::startup", total, ready = sources.len(), unavailable = unavailable.len() - deferred.len(), pending = deferred.len(), elapsed_ms = started.elapsed().as_millis() as u64, "Initial catalog loading finished");
    Ok((sources, unavailable, deferred))
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
        if error.is::<PendingSource>() {
            return result.map(|source| (id.to_owned(), source));
        }
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
        cfg.discovery_source = Some(id.to_owned());
        cfg.discovery_relay = downloads.relay.get(id).cloned();
        cfg.http_cache_directory = downloads.cache_directory.clone();
        cfg.http_cache_direct_warm_target = downloads.direct_warm_target.clone();
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
            cfg.read_timeout_seconds.map(Duration::from_secs_f64),
            cfg.transport(),
        )?;
        progress.phase = "fetch_parse";
        let fetch_started = Instant::now();
        tracing::debug!(target: "x402_treazury::startup", source = id, phase = progress.phase,
            "Loading catalog document");
        let document = downloads.load(id, &cfg, &http)
            .instrument(tracing::debug_span!(target: "x402_treazury::startup", "catalog_download", source = id))
            .await;
        if let Err(error) = &document
            && let Some(shared) = error.downcast_ref::<SharedFailure>()
            && let Some(pending) = shared.0.downcast_ref::<DeferredDocument>()
        {
            return Err(PendingSource {
                config: cfg,
                document: pending.clone(),
            }
            .into());
        }
        let document = document
            .map_err(|e| {
                if cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://") {
                    e.context(RemoteCatalogUnavailable {
                        prefix: cfg.prefix.clone().expect("resolved prefix"),
                    })
                } else {
                    e
                }
            })
            .with_context(|| format!("source {id}: loading spec"))?;
        tracing::debug!(target: "x402_treazury::startup", source = id,
            elapsed_ms = fetch_started.elapsed().as_millis() as u64, "Catalog fetch/parse finished");
        progress.phase = "generation";
        Self::from_document(id, cfg, document)
    }
    pub(super) fn from_document(
        id: &str,
        mut cfg: catalog::Config,
        document: Document,
    ) -> Result<Self> {
        let generation_started = Instant::now();
        let base_url: Result<String> = (|| {
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
            Ok(base_url)
        })();
        let base_url = base_url.map_err(|error| {
            if cfg.base_url.is_none()
                && (cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://"))
            {
                error.context(RemoteCatalogUnavailable {
                    prefix: cfg.prefix.clone().expect("resolved prefix"),
                })
            } else {
                error
            }
        })?;
        cfg.resolve_cover(&base_url, crate::network::global().policy.cover_enabled())?;
        let tools = catalog::build_tools(&cfg, &document, cfg.prefix.as_deref().unwrap())
            .map_err(|e| {
                if (cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://"))
                    && !e.is::<catalog::ToolConfigurationError>()
                {
                    e.context(RemoteCatalogUnavailable {
                        prefix: cfg.prefix.clone().expect("resolved prefix"),
                    })
                } else {
                    e
                }
            })
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
        let discovery = discovery?;
        let counts = &discovery.evidence.cache;
        let cache_count = |path| counts.get(&path).copied().unwrap_or(0);
        let disk_hits = cache_count(crate::pricing::CachePath::Disk);
        let process_hits = cache_count(crate::pricing::CachePath::Hit);
        let shared = cache_count(crate::pricing::CachePath::Shared);
        let network_initializations = cache_count(crate::pricing::CachePath::Initialized);
        let prices=discovery.prices;
        tracing::info!(target: "x402_treazury::startup", source = id, enabled = source.config.probe_pricing, prices = prices.len(),
            disk_hits, process_hits, shared, network_initializations,
            disk_cache = crate::http_cache::Slot::availability(&source.config, "pricing"),
            skipped_embedded = discovery.evidence.skipped_embedded,
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
            elapsed_ms = progress.started.elapsed().as_millis() as u64, "Startup pricing discovery failed");
    }
    result
}

async fn catalog_cache_status(cfg: &catalog::Config) -> &'static str {
    let mut catalog_cache = if cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://") {
        let slot = crate::http_cache::Slot::new(cfg, &cfg.spec, "catalog", cfg.max_spec_bytes);
        match if let Some(slot) = slot {
            slot.read().await
        } else {
            None
        } {
            Some(entry) if entry.metadata.fresh() => "fresh",
            Some(_) => "requires_revalidation",
            None => "not_stored",
        }
    } else {
        "local_file"
    };
    if let Some(slot) = crate::discovery_relay::slot(cfg, &cfg.spec, "catalog", cfg.max_spec_bytes)
        && slot.read().await.is_some_and(|e| e.metadata.fresh())
    {
        catalog_cache = "fresh_via_relay";
    }
    catalog_cache
}

async fn discover_warm_pricing(
    config: &MetaConfig,
    id: &str,
    source: &Source,
    pricing: &crate::pricing::PricingCache,
) -> Result<(crate::pricing::Evidence, usize)> {
    let tools = super::cache_warm::selected_tools(config, id, source)?;
    let discovery = pricing
        .discover_observed(&source.config, &source.document, &tools, &source.base_url)
        .await?;
    let fresh_entries = fresh_pricing_entries(source, &tools, &discovery.prices).await?;
    Ok((discovery.evidence, fresh_entries))
}

async fn fresh_pricing_entries(
    source: &Source,
    tools: &[catalog::ToolSpec],
    prices: &BTreeMap<(String, String), String>,
) -> Result<usize> {
    let cfg = &source.config;
    let mut fresh_pricing_entries = 0;
    let mut inspected = std::collections::BTreeSet::new();
    for tool in tools {
        if prices.contains_key(&(tool.method.clone(), tool.path.clone())) {
            let url = tool.route(&source.base_url, &serde_json::Map::new())?.url;
            if !inspected.insert(url.clone()) {
                continue;
            }
            for slot in [
                crate::http_cache::Slot::new(cfg, &url, "pricing", 64 * 1024),
                crate::discovery_relay::slot(cfg, &url, "pricing", 64 * 1024),
            ]
            .into_iter()
            .flatten()
            {
                if slot
                    .read()
                    .await
                    .is_some_and(|entry| entry.metadata.fresh())
                {
                    fresh_pricing_entries += 1;
                    break;
                }
            }
        }
    }
    Ok(fresh_pricing_entries)
}

pub(super) async fn warm(
    config: &MetaConfig,
    path: &Path,
    selected: &std::collections::BTreeSet<String>,
    direct: bool,
    discover_pricing: bool,
    relay: crate::discovery_relay::Relays,
) -> Result<Vec<super::cache_warm::CacheWarmSource>> {
    let downloads = Downloads {
        defer: false,
        relay,
        entries: Mutex::default(),
        cache_directory: config
            .treasury
            .as_ref()
            .map(|t| t.state_dir.join("http-cache")),
        direct_warm_target: direct.then(|| config.network.clone()),
    };
    let pricing = crate::pricing::PricingCache::default();
    let mut pending = stream::iter(selected.iter().map(|id| async {
        let (_, source) = load_one(id, &config.sources[id], path, false, &downloads).await?;
        let catalog_cache = catalog_cache_status(&source.config).await;
        let (evidence, fresh_pricing_entries) = if discover_pricing {
            let (evidence, fresh_entries) =
                discover_warm_pricing(config, id, &source, &pricing).await?;
            (Some(evidence), fresh_entries)
        } else {
            (None, 0)
        };
        Ok::<_, anyhow::Error>(super::cache_warm::CacheWarmSource {
            source: id.clone(),
            catalog_cache,
            pricing: evidence,
            fresh_pricing_entries,
        })
    }))
    .buffer_unordered(config.startup.catalog_concurrency);
    let mut results = Vec::new();
    while let Some(result) = pending.next().await {
        results.push(result?);
    }
    results.sort_by(|a, b| a.source.cmp(&b.source));
    Ok(results)
}

#[cfg(test)]
mod warm_tests;

#[cfg(test)]
mod deferred_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn remote_aliases_defer_io_and_share_one_origin_request() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg = catalog::Config {
            spec: format!("http://{}/spec", listener.local_addr().unwrap()),
            http_cache_enabled: false,
            ..Default::default()
        };
        let downloads = Downloads {
            defer: true,
            ..Default::default()
        };
        let http = reqwest::Client::new();
        let pending = |error: anyhow::Error| {
            error
                .downcast_ref::<SharedFailure>()
                .unwrap()
                .0
                .downcast_ref::<DeferredDocument>()
                .unwrap()
                .clone()
        };
        let first = pending(downloads.load("a", &cfg, &http).await.unwrap_err());
        let second = pending(downloads.load("b", &cfg, &http).await.unwrap_err());
        assert!(Arc::ptr_eq(&first.0, &second.0));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        let (accepted, observed) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel();
        let origin = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(stream.read(&mut bytes).await.unwrap() > 0);
            accepted.send(()).unwrap();
            wait.await.unwrap();
            let body = r#"{"operations":[]}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            listener
        });
        let gate = Arc::new(tokio::sync::Semaphore::new(1));
        let task_gate = gate.clone();
        let resumes = tokio::spawn(async move {
            tokio::join!(first.resume(task_gate.clone()), second.resume(task_gate))
        });
        observed.await.unwrap();
        assert_eq!(gate.available_permits(), 0);
        release.send(()).unwrap();
        let (a, b) = resumes.await.unwrap();
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(gate.available_permits(), 1);
        let listener = origin.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    }
}
