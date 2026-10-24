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
    prepared: OnceCell<std::result::Result<PreparedDocument, SharedFailure>>,
}
enum PreparedDocument {
    Ready(Document),
    Relay(Box<catalog::RelayContinuation>),
}
impl DeferredDocument {
    async fn resume(
        &self,
        gate: Arc<tokio::sync::Semaphore>,
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
    pub config: catalog::Config,
    pub operator_required: bool,
    document: DeferredDocument,
}
impl PendingSource {
    pub async fn resume(
        &self,
        id: &str,
        gate: Arc<tokio::sync::Semaphore>,
        can_allocate: bool,
        status: tokio::sync::watch::Sender<bool>,
    ) -> Result<Source> {
        let document = self
            .document
            .resume(gate, can_allocate, Some(status))
            .await?;
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
#[derive(Default)]
struct Downloads {
    defer: bool,
    all_tools: bool,
    relay: crate::discovery_relay::Relays,
    entries: Mutex<BTreeMap<DownloadKey, Arc<OnceCell<DownloadResult>>>>,
    cache_directory: Option<std::path::PathBuf>,
    direct_warm_target: Option<crate::network::NetworkPolicy>,
    gate: Option<Arc<tokio::sync::Semaphore>>,
}
impl Downloads {
    async fn load(&self, id: &str, cfg: &catalog::Config) -> Result<Download> {
        if !(cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://")) {
            return Ok(Download::Ready(Arc::new(
                catalog::load_json_discovery(cfg).await?,
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
                tracing::info!(target: "x402_treazury::startup", source = id, cache = "shared_load",
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

pub(super) async fn load(
    config: &MetaConfig,
    path: &Path,
    warn: bool,
    relay: crate::discovery_relay::Relays,
    defer: bool,
    all_tools: bool,
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
        all_tools,
        relay,
        gate: Some(Arc::new(tokio::sync::Semaphore::new(limit))),
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
                        Ok(SourceLoad::Ready(source)) => source,
                        Err(error) if collect_failures => {
                            failures += 1;
                            if first_error.is_none() { first_error = Some(error); }
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    sources.insert(source_id.clone(), source);
                    tracing::info!(target: "x402_treazury::startup", source = source_id, completed = sources.len(), total,
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
) -> Result<SourceLoad> {
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
        path: &Path,
        warn: bool,
        progress: &mut Progress<'_>,
        downloads: &Downloads,
    ) -> Result<SourceLoad> {
        let mut cfg = crate::config::resolve(source.provider.clone(), path)
            .await
            .with_context(|| format!("source {id}"))?
            .settings;
        if downloads.all_tools {
            cfg.use_recommended_tools = false;
        }
        cfg.discovery_source = Some(id.to_owned());
        cfg.relay_fallback = downloads.relay.get(id).cloned();
        cfg.http_cache_directory = downloads.cache_directory.clone();
        cfg.http_cache_direct_warm_target = downloads.direct_warm_target.clone();
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
                    config: cfg,
                    operator_required: false,
                    document,
                }));
            }
        };
        tracing::debug!(target: "x402_treazury::startup", source = id,
            elapsed_ms = fetch_started.elapsed().as_millis() as u64, "Catalog fetch/parse finished");
        progress.phase = "generation";
        Self::from_document(id, cfg, document).map(SourceLoad::Ready)
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
            .context(catalog::LoadStage::Generation)
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
        defer: false,
        all_tools: false,
        relay,
        gate: Some(Arc::new(tokio::sync::Semaphore::new(
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
        loads.push(async {
            let SourceLoad::Ready(source) =
                load_one(id, &config.sources[id], path, false, &downloads).await?
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
            config: cfg,
            operator_required: false,
            document,
        };
        let (error, logs) = crate::limits::tests::capture(async {
            let (status, _) = tokio::sync::watch::channel(false);
            pending
                .resume(
                    "fixture",
                    Arc::new(tokio::sync::Semaphore::new(1)),
                    false,
                    status,
                )
                .await
                .err()
                .expect("invalid key schema must reject generation")
        })
        .await;
        assert!(
            logs.contains("fresh HTTP disk cache; tool generation and validation still required")
        );
        assert!(!logs.contains("private upstream prose") && !logs.contains("token=secret"));
        let failure = catalog_failure(&error, "fetch_parse");
        assert_eq!(failure.stage, "generation");
        assert!(failure.reason.contains("Idempotency-Key schema"));
        assert!(error.is::<RemoteCatalogUnavailable>());
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
            tokio::join!(
                first.resume(task_gate.clone(), true, None),
                second.resume(task_gate, true, None)
            )
        });
        observed.await.unwrap();
        assert_eq!(gate.available_permits(), 0);
        release.send(()).unwrap();
        let (a, b) = resumes.await.unwrap();
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
        assert_eq!(gate.available_permits(), 1);
        let listener = origin.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    }
}
