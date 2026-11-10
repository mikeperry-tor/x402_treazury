//! Bounded initial catalog loading and retained relay continuations for serving.
use super::catalog_evidence::{self, State};
use super::{EffectiveBindings, MetaConfig, SendToolsRule, Source, SourceConfig};
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
type Document = Arc<catalog::LoadedCatalog>;

#[derive(Clone)]
struct DeferredDocument(Arc<DeferredLoad>);
struct DeferredLoad {
    config: catalog::Config,
    prepared: OnceCell<std::result::Result<PreparedDocument, SharedFailure>>,
}
enum PreparedDocument {
    Ready(Document),
    Relay(Box<catalog::RelayContinuation>),
}
impl DeferredDocument {
    async fn resume(
        &self,
        gate: Arc<crate::network::DiscoveryCapacity>,
        can_allocate: bool,
        operator_status: Option<tokio::sync::watch::Sender<bool>>,
    ) -> Result<Document> {
        let prepared = self
            .0
            .prepared
            .get_or_init(|| async {
                catalog::prepare_json_discovery_bounded(
                    &self.0.config,
                    crate::network::global(),
                    Some(gate.clone()),
                )
                .await
                .map(|prepared| match prepared {
                    catalog::CatalogLoad::Ready(document) => {
                        PreparedDocument::Ready(Arc::new(document))
                    }
                    catalog::CatalogLoad::RelayPending(pending) => PreparedDocument::Relay(pending),
                })
                .map_err(|error| SharedFailure(Arc::new(error)))
            })
            .await;
        match prepared {
            Ok(PreparedDocument::Ready(document)) => Ok(document.clone()),
            Ok(PreparedDocument::Relay(pending)) => Ok(Arc::new(
                pending
                    .resume_when_ready(gate, if can_allocate { None } else { operator_status })
                    .await?,
            )),
            Err(error) => Err(anyhow::Error::new(error.clone())),
        }
    }
}
impl std::fmt::Debug for DeferredDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deferred remote catalog")
    }
}
#[derive(Clone, Debug)]
pub(super) struct PendingSource {
    send_tools: Option<Vec<SendToolsRule>>,
    pub config: catalog::Config,
    pub operator_required: bool,
    document: DeferredDocument,
}
impl PendingSource {
    pub async fn resume(
        &self,
        id: &str,
        gate: Arc<crate::network::DiscoveryCapacity>,
        can_allocate: bool,
        status: tokio::sync::watch::Sender<bool>,
    ) -> Result<Source> {
        let document = self
            .document
            .resume(gate.clone(), can_allocate, Some(status))
            .await?;
        if let Some(hints) = &document.hints {
            hints.slot.record_configuration(&self.config).await;
        }
        let mut source = Source::from_document(id, self.config.clone(), document)?
            .with_send_tools(id, self.send_tools.clone())?;
        source.enrich_pricing(Some(gate));
        Ok(source)
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

// Shared downloads preserve the typed cause behind Arc; never display its prose.
pub(super) fn catalog_failure(
    mut error: &anyhow::Error,
    stage: &'static str,
) -> catalog::diagnostics::Failure {
    while let Some(shared) = error.downcast_ref::<SharedFailure>() {
        error = shared.0.as_ref();
    }
    catalog::diagnostics::Failure::from_error(error, stage)
}
#[derive(Clone, Debug)]
enum Download {
    Ready(Document),
    Deferred(DeferredDocument),
}
enum SourceLoad {
    Ready(Source),
    Pending(PendingSource),
}
type DownloadResult = std::result::Result<Download, SharedFailure>;

/// One deployment load, one immutable network policy/runtime. Retain parsed remote
/// documents only for this load; aliases still generate/filter/bind independently.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum CatalogPurpose {
    #[default]
    Inspection,
    Serving,
    Qualification,
}
impl CatalogPurpose {
    pub(super) fn for_current_operation(self) -> Self {
        if crate::qualification::active() {
            Self::Qualification
        } else if catalog_evidence::collecting() {
            Self::Inspection
        } else {
            self
        }
    }
}

#[derive(Default)]
struct Downloads {
    defer: bool,
    purpose: CatalogPurpose,
    all_tools: bool,
    relay: crate::discovery_relay::Relays,
    entries: Mutex<BTreeMap<DownloadKey, Arc<OnceCell<DownloadResult>>>>,
    cache_directory: Option<std::path::PathBuf>,
    direct_warm_target: Option<crate::network::NetworkPolicy>,
    gate: Option<Arc<crate::network::DiscoveryCapacity>>,
}
impl Downloads {
    async fn configure(
        &self,
        id: &str,
        source: &SourceConfig,
        path: &Path,
        warn: bool,
    ) -> Result<catalog::Config> {
        let mut cfg = crate::config::resolve(source.provider.clone(), path)
            .await
            .with_context(|| format!("source {id}"))?
            .settings;
        cfg.cache_revision = Some(crate::http_cache::configuration_revision(&cfg));
        if self.all_tools {
            cfg.use_recommended_tools = false;
        }
        cfg.discovery_source = Some(id.to_owned());
        cfg.relay_fallback = self.relay.get(id).cloned();
        cfg.http_cache_directory = self.cache_directory.clone();
        cfg.http_cache_direct_warm_target = self.direct_warm_target.clone();
        if warn {
            crate::provider_status::warn(id, &cfg);
        }
        if cfg.prefix.is_none() {
            cfg.prefix = Some(id.to_owned());
        }
        if cfg.spec.starts_with("http") {
            let _ = crate::network::IsolationId::discovery(&cfg.spec)?;
        }
        ensure!(
            cfg.read_timeout_seconds
                .map(Duration::from_secs_f64)
                .is_none_or(|t| t >= Duration::from_millis(1)),
            "HTTP read inactivity timeout must be at least 1ms"
        );
        if let Some(slot) =
            crate::http_cache::Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes)
        {
            slot.check_revision().await;
        }
        for slot in crate::discovery_relay::slots(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes) {
            slot.check_revision().await;
        }
        Ok(cfg)
    }
    async fn load(&self, id: &str, cfg: &catalog::Config) -> Result<Download> {
        if !(cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://")) {
            return Ok(Download::Ready(Arc::new(
                catalog::load_json_discovery_bounded(cfg, crate::network::global(), None).await?,
            )));
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
            cfg.relay_fallback.as_ref().map(|r| r.key.clone()),
        );
        let cell = {
            let mut entries = self.entries.lock().expect("catalog download map poisoned");
            if entries.contains_key(&key) {
                tracing::debug!(target: "x402_treazury::startup", source = id, cache = "shared_load",
                    "Sharing compatible catalog download or parsed document");
            }
            entries.entry(key).or_default().clone()
        };
        // No detached tasks: cancellation drops the initializer and its HTTP request.
        cell.get_or_init(|| async {
            if self.defer {
                let mut config = cfg.clone();
                config.wait_for_startup_socks = true;
                Ok(Download::Deferred(DeferredDocument(Arc::new(
                    DeferredLoad {
                        config,
                        prepared: OnceCell::new(),
                    },
                ))))
            } else {
                catalog::load_json_discovery_bounded(
                    cfg,
                    crate::network::global(),
                    self.gate.clone(),
                )
                .await
                .map(|document| Download::Ready(Arc::new(document)))
                .map_err(|error| SharedFailure(Arc::new(error)))
            }
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

#[allow(clippy::too_many_arguments)]
pub(super) async fn load(
    config: &MetaConfig,
    bindings: &EffectiveBindings,
    path: &Path,
    purpose: CatalogPurpose,
    relay: crate::discovery_relay::Relays,
    defer: bool,
    all_tools: bool,
    capacity: Arc<crate::network::DiscoveryCapacity>,
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
        purpose,
        defer,
        all_tools,
        relay,
        gate: Some(capacity),
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
        let used = bindings.0.values().any(|sources| sources.contains_key(id));
        let cfg = downloads
            .configure(
                id,
                source,
                path,
                purpose != CatalogPurpose::Inspection && used,
            )
            .await;
        let downloads = &downloads;
        loads.push(async move { (id, load_one(id, source, cfg, downloads).await) });
    }
    // Logical owners may wait for server delays without occupying download slots.
    let mut pending = stream::iter(loads).buffer_unordered(total.max(1));
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
                    let source = match result {
                        Ok(SourceLoad::Pending(source)) => {
                            unavailable.insert(source_id.clone(), source.config.prefix.clone().expect("resolved prefix"));
                            deferred.insert(source_id.clone(), source);
                            tracing::debug!(source=source_id, state="pending", "Catalog load retained for serving");
                            continue;
                        }
                        Err(error) if purpose == CatalogPurpose::Serving
                            && config.sources[source_id].optional_startup && error.is::<RemoteCatalogUnavailable>() => {
                            failures += 1;
                            unavailable.insert(source_id.clone(), error.downcast_ref::<RemoteCatalogUnavailable>().expect("typed remote failure").prefix.clone());
                            tracing::warn!(source = source_id, state = "unavailable", "Optional source unavailable; required inventory remains complete");
                            continue;
                        }
                        Ok(SourceLoad::Ready(source)) => source,
                        Err(error) if collect_failures => {
                            failures += 1;
                            if first_error.is_none() { first_error = Some(error); }
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    if source.compatibility_unavailable() {
                        unavailable.insert(source_id.clone(), source.config.prefix.clone().expect("resolved prefix"));
                        tracing::debug!(source = source_id, omitted = source.issues.len(), "Catalog has no supported API operations; dependent waiting endpoints remain unavailable");
                    } else {
                        tracing::info!(target: "x402_treazury::startup", source = source_id, completed = sources.len() + 1, total,
                            elapsed_ms = started.elapsed().as_millis() as u64, "Catalog ready");
                    }
                    sources.insert(source_id.clone(), source);
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
    tracing::info!(target: "x402_treazury::startup", total, ready = sources.values().filter(|source| !source.compatibility_unavailable()).count(), unavailable = unavailable.len() - deferred.len(), pending = deferred.len(), elapsed_ms = started.elapsed().as_millis() as u64,
    "{}", if deferred.is_empty() {
        "Initial catalog loading finished"
    } else {
        "Catalogs are still loading; each waiting MCP endpoint will start when its required catalogs are ready"
    });
    Ok((sources, unavailable, deferred))
}

async fn load_one(
    id: &str,
    source: &SourceConfig,
    cfg: Result<catalog::Config>,
    downloads: &Downloads,
) -> Result<SourceLoad> {
    let mut progress = Progress {
        source: id,
        phase: "configuration",
        started: Instant::now(),
        finished: false,
    };
    tracing::debug!(target: "x402_treazury::startup", source = id, "Catalog load started");
    let result = match cfg {
        Ok(cfg) => Source::load(id, source, cfg, &mut progress, downloads).await,
        Err(error) => Err(error),
    }
    .and_then(|loaded| {
        if downloads.purpose != CatalogPurpose::Serving
            && let SourceLoad::Ready(source) = &loaded
            && !source.issues.is_empty()
        {
            return Err(catalog::IncompleteCatalog(source.issues.clone()).into());
        }
        Ok(loaded)
    });
    progress.finished = true;
    if let Err(error) = &result {
        let failure = catalog_failure(error, progress.phase);
        catalog_evidence::record(id, State::Failed, failure.stage, failure.http_status);
        tracing::warn!(target: "x402_treazury::startup", source = id, phase = progress.phase,
            failure_stage = failure.stage, reason = failure.reason,
            http_status = failure.http_status,
            elapsed_ms = progress.started.elapsed().as_millis() as u64,
            collecting_independent_results = catalog_evidence::collecting(),
            "Catalog load failed; complete inventory unavailable");
    } else if matches!(result, Ok(SourceLoad::Ready(_))) {
        catalog_evidence::record(id, State::Completed, "generation", None);
    }
    result
}

impl Source {
    async fn load(
        id: &str,
        source: &SourceConfig,
        cfg: catalog::Config,
        progress: &mut Progress<'_>,
        downloads: &Downloads,
    ) -> Result<SourceLoad> {
        progress.phase = "fetch_parse";
        let fetch_started = Instant::now();
        tracing::debug!(target: "x402_treazury::startup", source = id, phase = progress.phase,
            "Loading catalog document");
        let document = downloads.load(id, &cfg)
            .instrument(tracing::debug_span!(target: "x402_treazury::startup", "catalog_download", source = id))
            .await;
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
        let document = match document {
            Download::Ready(document) => document,
            Download::Deferred(document) => {
                return Ok(SourceLoad::Pending(PendingSource {
                    send_tools: source.send_tools.clone(),
                    config: cfg,
                    operator_required: false,
                    document,
                }));
            }
        };
        tracing::debug!(target: "x402_treazury::startup", source = id,
            elapsed_ms = fetch_started.elapsed().as_millis() as u64, "Catalog fetch/parse finished");
        progress.phase = "generation";
        if let Some(hints) = &document.hints {
            hints.slot.record_configuration(&cfg).await;
        }
        let mut source = Self::from_document(id, cfg, document)?
            .with_send_tools(id, source.send_tools.clone())?;
        if downloads.purpose.for_current_operation() == CatalogPurpose::Serving {
            source.enrich_pricing(downloads.gate.clone());
        }
        Ok(SourceLoad::Ready(source))
    }
    fn enrich_pricing(&mut self, gate: Option<Arc<crate::network::DiscoveryCapacity>>) {
        if CatalogPurpose::Serving.for_current_operation() != CatalogPurpose::Serving {
            return;
        }
        crate::pricing::discovery::attach_static(
            &self.config,
            &self.document,
            &self.base_url,
            &mut self.tools,
            gate,
        );
    }
    pub(super) fn from_document(
        id: &str,
        mut cfg: catalog::Config,
        document: impl Into<catalog::LoadedCatalog>,
    ) -> Result<Self> {
        let document = document.into();
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
        let base_url = base_url
            .map_err(|error| {
                if cfg.base_url.is_none()
                    && (cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://"))
                {
                    error.context(RemoteCatalogUnavailable {
                        prefix: cfg.prefix.clone().expect("resolved prefix"),
                    })
                } else {
                    error
                }
            })
            .context(catalog::LoadStage::Configuration)?;
        cfg.resolve_cover(&base_url, crate::network::global().policy.cover_enabled())
            .context(catalog::LoadStage::Configuration)?;
        let built = catalog::build_catalog(&cfg, &document, cfg.prefix.as_deref().unwrap())
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
            .context(catalog::LoadStage::Generation)
            .with_context(|| format!("source {id}: catalog generation"))?;
        let mut tools = built.tools;
        let issues = built.issues;
        for tool in &mut tools {
            tool.catalog_hints = document.hints.clone();
            if let Some(expires) = document.advertised_expires {
                tool.advertised_pricing.set_expires(expires);
            }
        }
        tracing::debug!(target: "x402_treazury::startup", source = id, tools = tools.len(), omitted_operations=issues.len(),
            elapsed_ms = generation_started.elapsed().as_millis() as u64, "Catalog generation finished");
        Ok(Self {
            send_tools: None,
            tools,
            issues,
            base_url,
            instructions: cfg.instructions_text.clone(),
            config: cfg,
            document: document.document,
        })
    }
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
    for slot in crate::discovery_relay::slots(cfg, &cfg.spec, "catalog", cfg.max_spec_bytes) {
        if slot.read().await.is_some_and(|e| e.metadata.fresh()) {
            catalog_cache = "fresh_via_relay";
            break;
        }
    }
    catalog_cache
}

pub(super) async fn warm(
    config: &MetaConfig,
    path: &Path,
    selected: &std::collections::BTreeSet<String>,
    direct: bool,
    relay: crate::discovery_relay::Relays,
) -> Result<Vec<super::cache_warm::CacheWarmSource>> {
    let downloads = Downloads {
        purpose: CatalogPurpose::Inspection,
        defer: false,
        all_tools: false,
        relay,
        gate: Some(Arc::new(crate::network::DiscoveryCapacity::new(
            config.startup.catalog_concurrency,
        ))),
        entries: Mutex::default(),
        cache_directory: config
            .treasury
            .as_ref()
            .map(|t| t.state_dir.join("http-cache")),
        direct_warm_target: direct.then(|| config.network.clone()),
    };
    let mut loads = Vec::with_capacity(selected.len());
    for id in selected {
        let cfg = downloads
            .configure(id, &config.sources[id], path, false)
            .await;
        let downloads = &downloads;
        loads.push(async move {
            let SourceLoad::Ready(source) =
                load_one(id, &config.sources[id], cfg, downloads).await?
            else {
                anyhow::bail!("cache warming cannot defer catalog discovery");
            };
            let catalog_cache = catalog_cache_status(&source.config).await;
            Ok::<_, anyhow::Error>(super::cache_warm::CacheWarmSource {
                source: id.clone(),
                catalog_cache,
            })
        });
    }
    let mut pending = stream::iter(loads).buffer_unordered(selected.len().max(1));
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

    #[test]
    fn acquired_advertised_expiry_reaches_static_tools_without_observed_cache() {
        let cfg: catalog::Config = toml::from_str(
            r#"
            spec = "https://api.example.com/openapi.json"
            base_url = "https://api.example.com"
            prefix = "priced"
            [pricing]
            source = "catalog"
            format = "payment-info"
            unit = "call"
            usd_as_usdc = true
        "#,
        )
        .unwrap();
        let document = Arc::new(serde_json::json!({"paths":{"/read":{"get":{
            "x-payment-info":{"price":{"currency":"USD","amount":"0.01"}}
        }}}}));
        for (expires, expected) in [
            (
                crate::http_cache::now() + 604800,
                Some(alloy_primitives::U256::from(10000)),
            ),
            (0, None),
        ] {
            let source = Source::from_document(
                "priced",
                cfg.clone(),
                catalog::LoadedCatalog {
                    document: document.clone(),
                    hints: None,
                    advertised_expires: Some(expires),
                },
            )
            .unwrap();
            assert_eq!(source.tools[0].advertised_pricing.amount(), expected);
            assert!(source.tools[0].catalog_hints.is_none());
        }
    }

    #[tokio::test]
    async fn cached_json_can_fail_generation_with_a_safe_specific_reason() {
        use crate::http_cache::{Entry, Metadata, Slot};
        let dir = tempfile::tempdir().unwrap();
        let cfg = catalog::Config {
            spec: "https://example.com/private-catalog?token=secret".into(),
            base_url: Some("https://example.com".into()),
            prefix: Some("fixture".into()),
            http_cache_directory: Some(dir.path().join("cache")),
            ..Default::default()
        };
        let headers = reqwest::header::HeaderMap::from_iter([(
            reqwest::header::CACHE_CONTROL,
            "max-age=600".parse().unwrap(),
        )]);
        let metadata = Metadata::from_headers(&headers, Duration::ZERO, None).unwrap();
        let slot = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
        slot.write(Some(Entry {
            hints: None,
            metadata,
            data: serde_json::json!({"paths":{"/read":{"get":{
                "description":"private upstream prose",
                "parameters":[{"in":"header","name":"Idempotency-Key","schema":{"type":"integer"}}]
            }}}})
            .to_string()
            .into_bytes(),
        }))
        .await;
        assert!(
            slot.read()
                .await
                .expect("cached fixture must be stored")
                .metadata
                .fresh()
        );
        let downloads = Downloads {
            defer: true,
            ..Default::default()
        };
        let Download::Deferred(document) = downloads.load("fixture", &cfg).await.unwrap() else {
            panic!("expected deferred document");
        };
        let pending = PendingSource {
            send_tools: None,
            config: cfg,
            operator_required: false,
            document,
        };
        let (source, logs) = crate::limits::tests::capture(async {
            let (status, _) = tokio::sync::watch::channel(false);
            pending
                .resume(
                    "fixture",
                    Arc::new(crate::network::DiscoveryCapacity::new(1)),
                    false,
                    status,
                )
                .await
                .expect("serving retains the unsupported operation for inspection")
        })
        .await;
        assert!(
            logs.contains("fresh HTTP disk cache; tool generation and validation still required")
        );
        assert!(!logs.contains("private upstream prose") && !logs.contains("token=secret"));
        assert!(source.compatibility_unavailable());
        assert!(source.tools.is_empty());
        assert_eq!(
            source.issues,
            vec![catalog::OperationIssue {
                method: "GET".into(),
                path: "/read".into(),
                reason: catalog::Unsupported::IdempotencyKeySchema,
            }]
        );
        let reason = catalog::omission_instructions(&source.issues).unwrap();
        assert!(reason.contains("Idempotency-Key schema"));
        assert!(!reason.contains("private upstream prose") && !reason.contains("token=secret"));
        let error = catalog::build_tools(&source.config, &source.document, "fixture").unwrap_err();
        assert!(error.is::<catalog::IncompleteCatalog>());
        assert!(
            catalog_failure(&error, "generation")
                .reason
                .contains("unsupported input contracts")
        );
    }

    #[test]
    fn shared_failures_retain_safe_stage_and_http_status() {
        let response = reqwest::Response::from(
            axum::http::Response::builder()
                .status(503)
                .body("private upstream prose")
                .unwrap(),
        );
        let error = response.error_for_status().unwrap_err();
        let shared = SharedFailure(Arc::new(
            anyhow::Error::new(error).context(catalog::LoadStage::Headers),
        ));
        let error = anyhow::Error::new(shared).context("private source context");
        let failure = catalog_failure(&error, "fetch_parse");
        assert_eq!(failure.stage, "headers");
        assert_eq!(failure.http_status, Some(503));
        assert_eq!(
            failure.reason,
            "The catalog server returned an unsuccessful HTTP status"
        );

        let error = anyhow::Error::new(SharedFailure(Arc::new(
            anyhow::anyhow!("private malformed body").context(catalog::LoadStage::Parse),
        )));
        assert_eq!(
            catalog_failure(&error, "fetch_parse").reason,
            "The catalog document is not valid JSON"
        );
    }

    #[tokio::test]
    async fn remote_aliases_defer_io_and_share_one_origin_request() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio_util::sync::CancellationToken;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg = catalog::Config {
            spec: format!("https://{}/spec", listener.local_addr().unwrap()),
            http_cache_enabled: false,
            ..Default::default()
        };
        let downloads = Downloads {
            defer: true,
            ..Default::default()
        };
        let Download::Deferred(first) = downloads.load("a", &cfg).await.unwrap() else {
            panic!("remote catalog must defer before serving");
        };
        let Download::Deferred(second) = downloads.load("b", &cfg).await.unwrap() else {
            panic!("remote alias must share deferred catalog");
        };
        assert!(Arc::ptr_eq(&first.0, &second.0));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        let listener = crate::test_tls::from_tcp(listener);
        let entered = CancellationToken::new();
        let release = CancellationToken::new();
        let requests = Arc::new(AtomicUsize::new(0));
        let (seen, accepted, wait) = (requests.clone(), entered.clone(), release.clone());
        let app = axum::Router::new().route(
            "/spec",
            axum::routing::get(move || {
                let (seen, accepted, wait) = (seen.clone(), accepted.clone(), wait.clone());
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    accepted.cancel();
                    wait.cancelled().await;
                    axum::Json(serde_json::json!({"operations":[]}))
                }
            }),
        );
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let origin = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
                .unwrap();
        });
        let gate = Arc::new(crate::network::DiscoveryCapacity::new(1));
        let task_gate = gate.clone();
        let mut resumes = tokio::spawn(async move {
            tokio::join!(
                first.resume(task_gate.clone(), true, None),
                second.resume(task_gate, true, None)
            )
        });
        tokio::select! {
            () = entered.cancelled() => {},
            result = &mut resumes => panic!("catalog resumed without reaching the fixture: {result:?}"),
        }
        assert_eq!(
            gate.available_permits(),
            1,
            "direct acquisition consumes no Tor slot"
        );
        assert_eq!(gate.live_scope_count(), 0);
        release.cancel();
        let (a, b) = resumes.await.unwrap();
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
        assert_eq!(gate.available_permits(), 1);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        stop.cancel();
        origin.await.unwrap();
    }
}

#[cfg(test)]
mod price_alias_tests {
    use super::*;
    use alloy_primitives::U256;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn first_load_records_all_aliases_and_second_alias_update_invalidates_shared_prices() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("deployment.toml");
        let requests = Arc::new(AtomicUsize::new(0));
        let calls = requests.clone();
        let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
            let calls = calls.clone();
            async move {
                assert_eq!(request.method(), "GET");
                assert_eq!(request.uri().path(), "/spec");
                assert!(!request.headers().contains_key("payment-signature"));
                calls.fetch_add(1, Ordering::SeqCst);
                r#"{"paths":{"/items":{"get":{"summary":"Read items."}}}}"#
            }
        });
        let (address, server) = crate::test_tls::serve(app).await;
        let url = format!("https://{address}/spec");
        let source = |base: &str, prefix: &str| -> SourceConfig {
            toml::from_str(&format!(
                "spec='{url}'\nbase_url='{base}'\nprefix='{prefix}'"
            ))
            .unwrap()
        };
        let first = source("https://first.example", "first");
        let mut second = source("https://second.example", "second");
        let mut previous = None;
        for round in 0..2 {
            let downloads = Downloads {
                cache_directory: Some(root.path().join("http-cache")),
                ..Default::default()
            };
            // Preflight both revisions before either alias starts its shared download.
            let a = downloads.configure("a", &first, &path, false).await;
            let b = downloads.configure("b", &second, &path, false).await;
            let (a, b) = tokio::join!(
                load_one("a", &first, a, &downloads),
                load_one("b", &second, b, &downloads)
            );
            let (SourceLoad::Ready(a), SourceLoad::Ready(b)) = (a.unwrap(), b.unwrap()) else {
                panic!("expected catalogs");
            };
            let hints = a.tools[0].catalog_hints.as_ref().unwrap();
            assert!(Arc::ptr_eq(
                hints,
                b.tools[0].catalog_hints.as_ref().unwrap()
            ));
            let key = hints.operation("GET", "/items");
            assert!(key.get().is_none());
            if round == 0 {
                key.learn(U256::from(3), crate::pricing::runtime::received().unwrap());
                assert_eq!(
                    b.tools[0]
                        .catalog_hints
                        .as_ref()
                        .unwrap()
                        .operation("GET", "/items")
                        .get(),
                    Some(U256::from(3))
                );
                previous = Some(key);
                second.provider.insert(
                    "base_url".into(),
                    toml::Value::String("https://updated.example".into()),
                );
            } else {
                assert!(previous.as_ref().unwrap().get().is_none());
            }
            assert_eq!(requests.load(Ordering::SeqCst), round + 1);
        }
        server.abort();
        let _ = server.await;
    }
}

#[cfg(test)]
mod external_pricing_refresh_tests {
    use super::*;
    use alloy_primitives::U256;
    use axum::response::IntoResponse;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn inspection_and_qualification_do_not_start_optional_metadata() {
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let (address, server) = crate::test_tls::serve(axum::Router::new().fallback(move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                axum::Json(serde_json::json!({"x402Version":2,"items":[]}))
            }
        }))
        .await;
        let root = tempfile::tempdir().unwrap();
        let spec = root.path().join("schema.json");
        std::fs::write(&spec, r#"{"paths":{"/read":{"get":{}}}}"#).unwrap();
        let definition: SourceConfig = toml::from_str(&format!(
            "spec='{}'\nbase_url='https://api.example.com'\n[pricing]\nsource='https://{address}/prices'\nformat='x402-discovery'\n", spec.display()
        )).unwrap();
        let path = root.path().join("deployment.toml");
        let mut sources = Vec::new();
        for purpose in [CatalogPurpose::Inspection, CatalogPurpose::Qualification] {
            let downloads = Downloads {
                purpose,
                ..Default::default()
            };
            let cfg = downloads
                .configure("fixture", &definition, &path, false)
                .await;
            let SourceLoad::Ready(source) = load_one("fixture", &definition, cfg, &downloads)
                .await
                .unwrap()
            else {
                panic!("ready source expected")
            };
            assert!(!source.tools[0].advertised_pricing.has_acquisition_owner());
            sources.push(source);
        }
        // Evidence collection overrides even a caller requesting serving.
        let downloads = Downloads {
            purpose: CatalogPurpose::Serving,
            ..Default::default()
        };
        let cfg = downloads
            .configure("fixture", &definition, &path, false)
            .await;
        let (source, _) =
            catalog_evidence::capture(load_one("fixture", &definition, cfg, &downloads)).await;
        let SourceLoad::Ready(source) = source.unwrap() else {
            panic!("ready source expected")
        };
        assert!(!source.tools[0].advertised_pricing.has_acquisition_owner());
        sources.push(source);
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        // The same definition still starts acquisition for ordinary serving.
        let cfg = downloads
            .configure("fixture", &definition, &path, false)
            .await;
        let SourceLoad::Ready(serving) = load_one("fixture", &definition, cfg, &downloads)
            .await
            .unwrap()
        else {
            panic!("ready source expected")
        };
        assert!(serving.tools[0].advertised_pricing.has_acquisition_owner());
        // Serving's network request is covered by the held-body test below;
        // retaining all inspection sources proves they own no pending work.
        drop((sources, serving));
        server.abort();
    }

    #[tokio::test]
    async fn direct_metadata_does_not_consume_tor_capacity() {
        use axum::body::{Body, Bytes};
        use axum::serve::Listener as _;
        let dir = tempfile::tempdir().unwrap();
        let arrived = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let listener = crate::test_tls::listener().await;
        let base = format!("https://{}", listener.local_addr().unwrap());
        let schema = serde_json::json!({"paths":{"/read":{"get":{}}}});
        let prices = serde_json::json!({"x402Version":2,"items":[{"resource":format!("{base}/read"),"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000"}]}]}).to_string();
        let app = axum::Router::new()
            .route(
                "/spec",
                axum::routing::get({
                    let schema = schema.clone();
                    move || async move { axum::Json(schema) }
                }),
            )
            .route(
                "/prices",
                axum::routing::get({
                    let arrived = arrived.clone();
                    let release = release.clone();
                    move || {
                        let (arrived, release, prices) =
                            (arrived.clone(), release.clone(), prices.clone());
                        async move {
                            // One real body chunk progresses before the next is held.
                            let chunks = stream::once(async move {
                                arrived.notify_one();
                                Ok::<_, std::io::Error>(Bytes::from_static(b" "))
                            })
                            .chain(stream::once(async move {
                                release.notified().await;
                                Ok::<_, std::io::Error>(Bytes::from(prices))
                            }));
                            Body::from_stream(chunks)
                        }
                    }
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let local = dir.path().join("schema.json");
        std::fs::write(&local, schema.to_string()).unwrap();
        let definition: SourceConfig = toml::from_str(&format!(
            "spec='{}'\nbase_url='{base}'\n[pricing]\nsource='{base}/prices'\nformat='x402-discovery'\n", local.display()
        )).unwrap();
        let gate = Arc::new(crate::network::DiscoveryCapacity::new(1));
        let downloads = Downloads {
            purpose: CatalogPurpose::Serving,
            gate: Some(gate.clone()),
            ..Default::default()
        };
        let path = dir.path().join("deployment.toml");
        let cfg = downloads
            .configure("optional", &definition, &path, false)
            .await;
        let SourceLoad::Ready(optional) = load_one("optional", &definition, cfg, &downloads)
            .await
            .unwrap()
        else {
            panic!("ready source expected")
        };
        arrived.notified().await;
        assert_eq!(gate.available_permits(), 1);
        assert_eq!(gate.live_scope_count(), 0);
        assert!(optional.tools[0].advertised_pricing.amount().is_none());
        let required: SourceConfig =
            toml::from_str(&format!("spec='{base}/spec'\nbase_url='{base}'\n")).unwrap();
        let cfg = downloads
            .configure("required", &required, &path, false)
            .await;
        // Watchdog only: completion must precede releasing the metadata body.
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            load_one("required", &required, cfg, &downloads),
        )
        .await;
        let still_pending = optional.tools[0].advertised_pricing.amount().is_none();
        release.notify_one();
        let SourceLoad::Ready(required) = result.unwrap().unwrap() else {
            panic!("ready source expected")
        };
        assert_eq!(required.tools.len(), 1);
        assert!(still_pending);
        tokio::time::timeout(Duration::from_secs(5), async {
            while optional.tools[0].advertised_pricing.amount() != Some(U256::from(1000)) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn unchanged_openapi_304_still_refreshes_expired_external_prices() {
        use crate::payment::{PaidClient, SpendPolicy};
        use crate::rotation::{
            base::BaseRpc,
            manager::ManagedPool,
            store::{Store, StoreHandle},
        };
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let root = tempfile::tempdir().unwrap();
        let unsigned = Arc::new(AtomicUsize::new(0));
        let signed = Arc::new(AtomicUsize::new(0));
        let price = Arc::new(AtomicUsize::new(1000));
        let catalogs = Arc::new(AtomicUsize::new(0));
        let metadata = Arc::new(AtomicUsize::new(0));
        let metadata_validations = Arc::new(AtomicUsize::new(0));
        let arrived = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let listener = crate::test_tls::listener().await;
        use axum::serve::Listener as _;
        let base = format!("https://{}", listener.local_addr().unwrap());
        let app=axum::Router::new().route("/read", axum::routing::get({
            let unsigned=unsigned.clone();let signed=signed.clone();
            move |headers: axum::http::HeaderMap| {
                let unsigned=unsigned.clone();let signed=signed.clone();
                async move {
                    if headers.contains_key("payment-signature") {
                        signed.fetch_add(1, Ordering::SeqCst);
                        return "paid".into_response();
                    }
                    unsigned.fetch_add(1, Ordering::SeqCst);
                    let offer=json!({"x402Version":2,"resource":{"url":"https://fixture.example/read","description":"fixture","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"100","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                    (axum::http::StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(offer.to_string()))]).into_response()
                }
            }
        })).fallback({
            let price=price.clone();let catalogs=catalogs.clone();let metadata=metadata.clone();let metadata_validations=metadata_validations.clone();let arrived=arrived.clone();let release=release.clone();let base=base.clone();
            move |request:axum::extract::Request| {
                let price=price.clone();let catalogs=catalogs.clone();let metadata=metadata.clone();let metadata_validations=metadata_validations.clone();let arrived=arrived.clone();let release=release.clone();let base=base.clone();
                async move {
                    assert!(!request.headers().contains_key("payment-signature"));
                    if request.uri().path()=="/spec" {
                        catalogs.fetch_add(1,Ordering::SeqCst);
                        if request.headers().contains_key("if-none-match") {
                            (axum::http::StatusCode::NOT_MODIFIED,[("etag","\"same\"")],"").into_response()
                        } else {
                            ([("etag","\"same\"")],axum::Json(json!({"openapi":"3.0.3","paths":{"/read":{"get":{}}}}))).into_response()
                        }
                    } else {
                        assert_eq!(request.uri().path(),"/prices");
                        metadata.fetch_add(1,Ordering::SeqCst);
                        if request.headers().contains_key("if-none-match"){metadata_validations.fetch_add(1,Ordering::SeqCst);}
                        if price.load(Ordering::SeqCst)==2000 {arrived.notify_one();release.notified().await;}
                        ([("etag","\"price\"")],axum::Json(json!({"x402Version":2,"items":[{"resource":format!("{base}/read"),"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":price.load(Ordering::SeqCst).to_string()}]}]}))).into_response()
                    }
                }
            }
        });
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let definition:SourceConfig=toml::from_str(&format!("spec='{base}/spec'\nbase_url='{base}'\nprefix='external'\ncatalog_cache_ttl_seconds=120\n[pricing]\nsource='{base}/prices'\nformat='x402-discovery'\n")).unwrap();
        let path = root.path().join("deployment.toml");
        let downloads = || Downloads {
            purpose: CatalogPurpose::Serving,
            cache_directory: Some(root.path().join("cache")),
            ..Default::default()
        };
        let first_downloads = downloads();
        let cfg = first_downloads
            .configure("source", &definition, &path, false)
            .await;
        let SourceLoad::Ready(first) = load_one("source", &definition, cfg, &first_downloads)
            .await
            .unwrap()
        else {
            panic!("ready source expected")
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while first.tools[0].advertised_pricing.amount() != Some(U256::from(1000)) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for url in [format!("{base}/spec"), format!("{base}/prices")] {
            let slot = crate::http_cache::Slot::new(
                &first.config,
                &url,
                "catalog",
                first.config.max_spec_bytes,
            )
            .unwrap();
            let mut entry = slot.read().await.unwrap();
            entry.metadata.expires = 0;
            slot.write(Some(entry)).await;
        }
        price.store(2000, Ordering::SeqCst);
        let second_downloads = downloads();
        let cfg = second_downloads
            .configure("source", &definition, &path, false)
            .await;
        let SourceLoad::Ready(second) = load_one("source", &definition, cfg, &second_downloads)
            .await
            .unwrap()
        else {
            panic!("ready source expected")
        };
        arrived.notified().await;
        assert!(second.tools[0].advertised_pricing.amount().is_none());
        assert!(
            second.tools[0]
                .route(&second.base_url, &Default::default())
                .is_ok()
        );
        let mut state = Store::create(
            &root.path().join("wallet-state"),
            &root.path().join("wallet-key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            state,
            "external_prices",
            crate::rotation::test_capacity::two_reserves("0.01"),
            Some(U256::from(10000)),
            0
        )
        .unwrap();
        let jobs = state.funding_jobs().unwrap();
        for job in &jobs {
            state
                .record_credit(&job.wallet_id, "10000", "fixture", 100)
                .unwrap();
        }
        let db = rusqlite::Connection::open(root.path().join("wallet-state/state.sqlite")).unwrap();
        db.execute(
            "UPDATE wallets SET role='RETAINED' WHERE id=?1",
            [&jobs[0].wallet_id],
        )
        .unwrap();
        db.execute(
            "UPDATE wallets SET role='ACTIVE' WHERE id=?1",
            [&jobs[1].wallet_id],
        )
        .unwrap();
        // Model consumed reserve generations as well as the resulting roles.
        db.execute(
            "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1",
            [],
        )
        .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let client = PaidClient::managed(Arc::new(
            ManagedPool::new(
                store.clone(),
                pool,
                BaseRpc::new(&format!("{base}/rpc-must-not-run")).unwrap(),
                SpendPolicy::dollars("0.01").unwrap(),
            )
            .unwrap(),
        ));
        let bound = crate::catalog_state::bind(vec![(
            second.tools[0].clone(),
            client.clone(),
            second.base_url.clone(),
        )])
        .remove(0);
        let observed = bound
            .tool
            .catalog_hints
            .as_ref()
            .unwrap()
            .operation("GET", "/read");
        // Optional metadata is still in flight. The first ordinary challenge is
        // fetched and signed by healthy ACTIVE, without waiting for publication.
        assert_eq!(
            bound.invoke_output(&Default::default()).await.unwrap().text,
            "paid"
        );
        assert_eq!(
            db.query_row("SELECT wallet_id FROM payment_attempts", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            jobs[1].wallet_id
        );
        assert_eq!(unsigned.load(Ordering::SeqCst), 1);
        assert_eq!(signed.load(Ordering::SeqCst), 1);
        assert_eq!(observed.get(), Some(U256::from(100)));
        // Remove this call's learned offer so the next call proves that later
        // advertised publication itself changes selection.
        observed.catalog.state.lock().unwrap().prices.clear();
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            while second.tools[0].advertised_pricing.amount() != Some(U256::from(2000)) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            first.tools[0].advertised_pricing.amount(),
            Some(U256::from(1000))
        );
        assert_eq!(
            bound.invoke_output(&Default::default()).await.unwrap().text,
            "paid"
        );
        observed.catalog.state.lock().unwrap().prices.clear();
        // Expired advertisements must restore unknown-price ACTIVE selection.
        bound.tool.advertised_pricing.set_expires(0);
        assert!(bound.tool.advertised_pricing.amount().is_none());
        assert_eq!(
            bound.invoke_output(&Default::default()).await.unwrap().text,
            "paid"
        );
        let payers: Vec<String> = db
            .prepare("SELECT wallet_id FROM payment_attempts ORDER BY rowid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            payers,
            [
                jobs[1].wallet_id.clone(),
                jobs[0].wallet_id.clone(),
                jobs[1].wallet_id.clone()
            ]
        );
        assert_eq!(
            unsigned.load(Ordering::SeqCst),
            3,
            "one own unsigned challenge per call; no selection probes"
        );
        assert_eq!(signed.load(Ordering::SeqCst), 3);
        assert_eq!(
            db.query_row("SELECT count(*) FROM funding_jobs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(catalogs.load(Ordering::SeqCst), 2);
        assert_eq!(metadata.load(Ordering::SeqCst), 2);
        assert_eq!(metadata_validations.load(Ordering::SeqCst), 1);
        client.drain().await;
        drop(bound);
        drop(client);
        drop(store);
        worker.await.unwrap();
        server.abort();
    }
}
