//! Unsigned, one-shot discovery. Successful and failed attempts never refresh.
use crate::catalog::{Config, ToolSpec, operations};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{StreamExt, stream};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OnceCell, Semaphore};

mod evidence;
pub use evidence::{CachePath, Evidence, Outcome, Price, price_map};
struct CachedProbe {
    created: Instant,
    line: Option<String>,
    outcome: Outcome,
    http_status: Option<u16>,
}
struct Observation {
    line: Option<String>,
    outcome: Outcome,
    http_status: Option<u16>,
    cache: CachePath,
    expired: bool,
}
pub struct Discovery {
    pub prices: BTreeMap<(String, String), String>,
    pub evidence: Evidence,
}
type Entry = Arc<OnceCell<CachedProbe>>;
#[derive(Default)]
pub struct PricingCache {
    entries: Mutex<BTreeMap<(String, crate::network::HttpPolicy), Entry>>,
}
static CACHE: OnceLock<PricingCache> = OnceLock::new();
pub(crate) const CONCURRENCY: usize = 16;
static LIMIT: Semaphore = Semaphore::const_new(CONCURRENCY);
pub fn process_cache() -> &'static PricingCache {
    CACHE.get_or_init(PricingCache::default)
}
pub fn validate(cfg: &Config) -> Result<()> {
    ensure!(
        cfg.probe_concurrency > 0,
        "probe_concurrency must be positive"
    );
    ensure!(
        cfg.probe_timeout.is_finite() && cfg.probe_timeout > 0.0 && cfg.probe_timeout <= 86400.0,
        "probe_timeout must be in (0, 86400]"
    );
    ensure!(
        cfg.probe_ttl_seconds.is_finite() && cfg.probe_ttl_seconds > 0.0,
        "probe_ttl_seconds must be positive"
    );
    ensure!(
        !cfg.probe_methods.is_empty()
            && cfg
                .probe_methods
                .iter()
                .all(|m| m.eq_ignore_ascii_case("GET")),
        "pricing probes support GET only"
    );
    Ok(())
}
impl PricingCache {
    async fn get(
        &self,
        url: String,
        http: reqwest::Client,
        ttl: f64,
        policy: crate::network::HttpPolicy,
    ) -> Observation {
        let cell = self
            .entries
            .lock()
            .await
            .entry((url.clone(), policy))
            .or_default()
            .clone();
        let ready = cell.get().is_some();
        let initialized = std::sync::atomic::AtomicBool::new(false);
        let entry = cell
            .get_or_init(|| async {
                initialized.store(true, std::sync::atomic::Ordering::Relaxed);
                let _permit = LIMIT.acquire().await.expect("probe semaphore open");
                let (line, outcome, http_status) = probe_request(&http, &url).await;
                tracing::debug!(
                    discovered = line.is_some(),
                    "pricing attempt cached; no automatic retry"
                );
                CachedProbe {
                    created: Instant::now(),
                    line,
                    outcome,
                    http_status,
                }
            })
            .await;
        let expired = entry.created.elapsed().as_secs_f64() >= ttl;
        Observation {
            line: if expired { None } else { entry.line.clone() },
            outcome: entry.outcome,
            http_status: entry.http_status,
            cache: if initialized.load(std::sync::atomic::Ordering::Relaxed) {
                CachePath::Initialized
            } else if ready {
                CachePath::Hit
            } else {
                CachePath::Shared
            },
            expired,
        }
    }
    async fn probe<'a>(
        &self,
        cfg: &Config,
        tool: &'a ToolSpec,
        base: &str,
    ) -> Result<(&'a ToolSpec, Observation)> {
        let url = tool.route(base, &serde_json::Map::new())?.url;
        let http = crate::network::provider_discovery(
            &url,
            Duration::from_secs_f64(cfg.probe_timeout),
            cfg.transport(),
        )?;
        Ok((
            tool,
            self.get(url, http, cfg.probe_ttl_seconds, cfg.transport())
                .await,
        ))
    }
    pub async fn discover(
        &self,
        cfg: &Config,
        root: &Value,
        tools: &[ToolSpec],
        base: &str,
    ) -> Result<BTreeMap<(String, String), String>> {
        Ok(self.discover_observed(cfg, root, tools, base).await?.prices)
    }
    pub async fn discover_observed(
        &self,
        cfg: &Config,
        root: &Value,
        tools: &[ToolSpec],
        base: &str,
    ) -> Result<Discovery> {
        validate(cfg)?;
        let mut evidence = Evidence {
            enabled: cfg.probe_pricing,
            selected_tools: tools.len(),
            ..Default::default()
        };
        if !cfg.probe_pricing {
            return Ok(Discovery {
                prices: BTreeMap::new(),
                evidence,
            });
        }
        let priced: BTreeSet<_> = operations(root, cfg.pricing_key.as_deref())?
            .into_iter()
            .filter(|o| !o["pricing"].is_null() && o["pricing"] != serde_json::json!({}))
            .map(|o| {
                (
                    o["method"].as_str().unwrap_or("").to_uppercase(),
                    o["path"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect();
        let mut candidates = Vec::new();
        for tool in tools {
            if tool.help_url.is_some() {
                evidence.skipped_help += 1;
            } else if tool.method != "GET" {
                evidence.skipped_method += 1;
            } else if tool.path.contains('{') {
                evidence.skipped_template += 1;
            } else if priced.contains(&(tool.method.clone(), tool.path.clone())) {
                evidence.skipped_embedded += 1;
            } else {
                candidates.push(tool);
            }
        }
        candidates.sort_by_key(|t| (&t.path, &t.method));
        let before = candidates.len();
        candidates.dedup_by_key(|t| (&t.path, &t.method));
        evidence.skipped_duplicate = before - candidates.len();
        evidence.eligible = candidates.len();
        evidence.capped = candidates.len().saturating_sub(cfg.probe_max_endpoints);
        if candidates.len() > cfg.probe_max_endpoints {
            tracing::warn!(
                candidate_count = candidates.len(),
                limit_endpoints = cfg.probe_max_endpoints,
                "pricing discovery capped by probe_max_endpoints; remaining endpoints will have no probed price"
            );
        }
        candidates.truncate(cfg.probe_max_endpoints);
        let mut lines = BTreeMap::new();
        // Rolling per-source slots share the process-wide request semaphore.
        // Futures are scoped: cancellation drops probes, retaining completed cache entries.
        let mut probes = Vec::with_capacity(candidates.len());
        for tool in candidates {
            probes.push(self.probe(cfg, tool, base));
        }
        let mut pending =
            stream::iter(probes).buffer_unordered(cfg.probe_concurrency.min(CONCURRENCY));
        while let Some(result) = pending.next().await {
            let (tool, observation) = result?;
            evidence.observed += 1;
            evidence.expired += usize::from(observation.expired);
            *evidence.cache.entry(observation.cache).or_default() += 1;
            *evidence.outcomes.entry(observation.outcome).or_default() += 1;
            if let Some(code) = observation.http_status {
                *evidence.http_statuses.entry(code).or_default() += 1;
            }
            if let Some(line) = observation.line {
                lines.insert((tool.method.clone(), tool.path.clone()), line);
            }
        }
        evidence.available_prices = lines.len();
        evidence.validate()?;
        Ok(Discovery {
            prices: lines,
            evidence,
        })
    }
}
async fn probe_request(
    http: &reqwest::Client,
    url: &str,
) -> (Option<String>, Outcome, Option<u16>) {
    let response = match http.get(url).send().await {
        Ok(response) => response,
        Err(e) => {
            return (
                None,
                if e.is_timeout() {
                    Outcome::HttpTimeout
                } else if e.is_connect() {
                    Outcome::HttpConnect
                } else {
                    Outcome::HttpTransport
                },
                None,
            );
        }
    };
    crate::network::log_http(&response, "pricing");
    let status = Some(response.status().as_u16());
    if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
        return (None, Outcome::UnexpectedHttpStatus, status);
    }
    let mut outcome = Outcome::MissingHeader;
    for name in ["payment-required", "x-payment-required"] {
        let Some(raw) = response.headers().get(name) else {
            continue;
        };
        outcome = Outcome::MalformedChallenge;
        let Some(value) = STANDARD
            .decode(raw.as_bytes())
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            continue;
        };
        let line = value
            .get("accepts")
            .and_then(|v| v.get(0))
            .and_then(challenge_line);
        return (
            line.clone(),
            if line.is_some() {
                Outcome::Discovered
            } else {
                Outcome::UnusableOffer
            },
            status,
        );
    }
    (None, outcome, status)
}
fn challenge_line(c: &Value) -> Option<String> {
    let amount = c.get("amount").or_else(|| c.get("maxAmountRequired"))?;
    let raw = amount
        .as_str()
        .map(str::to_owned)
        .or_else(|| amount.as_u64().map(|n| n.to_string()))?;
    let atomic: u128 = raw.parse().ok()?;
    let asset = c["asset"].as_str()?;
    let scheme = c["scheme"].as_str().unwrap_or("exact");
    let network = c["network"].as_str().unwrap_or("");
    let usdc = asset.eq_ignore_ascii_case("USDC")
        || asset.eq_ignore_ascii_case("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
    let (amount, asset) = if usdc {
        let text = format!("{}.{:06}", atomic / 1_000_000, atomic % 1_000_000);
        (
            format!("${}", text.trim_end_matches('0').trim_end_matches('.')),
            "USDC".to_owned(),
        )
    } else {
        (format!("{atomic} atomic"), format!("asset {asset}"))
    };
    let denomination = if usdc {
        String::new()
    } else if network.is_empty() {
        format!("; {asset}")
    } else {
        format!("; {asset} on {network}")
    };
    Some(match scheme {
        "exact" => format!("Cost: ~{amount}/call [x402 probe{denomination}]."),
        "upto" => format!("Max: {amount}/call [x402 probe{denomination}]."),
        _ => format!("Amount: {amount} [x402 probe; scheme {scheme}{denomination}]."),
    })
}
