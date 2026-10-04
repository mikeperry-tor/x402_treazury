//! Bounded, scoped catalog futures: no detached tasks or partial inventory publication.
use super::{MetaConfig, Source, SourceConfig};
use crate::catalog;
use anyhow::{Context, Result, ensure};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StartupConfig {
    pub catalog_concurrency: usize,
}
impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            catalog_concurrency: 16,
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
                "Catalog load cancelled; no partial inventory published");
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
    let mut loads = Vec::with_capacity(total);
    for (id, source) in &config.sources {
        let used = config.servers.values().any(|s| s.sources.contains(id));
        loads.push(load_one(id, source, path, warn && used));
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
) -> Result<(String, Source)> {
    let mut progress = Progress {
        source: id,
        phase: "configuration",
        started: Instant::now(),
        finished: false,
    };
    tracing::info!(target: "x402_treazury::startup", source = id, "Catalog load started");
    let result = Source::load(id, source, path, warn, &mut progress).await;
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
        let http = crate::network::discovery(
            if cfg.spec.starts_with("http") {
                &cfg.spec
            } else {
                "https://local.invalid"
            },
            Duration::from_secs_f64(cfg.timeout),
        )?;
        progress.phase = "fetch_parse";
        let fetch_started = Instant::now();
        tracing::info!(target: "x402_treazury::startup", source = id, phase = progress.phase,
            "Loading catalog document");
        let document = catalog::load_json_with_limit(&cfg.spec, &http, cfg.max_spec_bytes)
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
