//! Unsigned, one-shot discovery. Successful and failed attempts never refresh.
use crate::catalog::{Config, ToolSpec, operations_with_credits};
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
    disk: bool,
    lifetime: Duration,
    observed_age: Duration,
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
type ProbeKey = (
    String,
    crate::network::HttpPolicy,
    Option<std::path::PathBuf>,
    Option<Vec<u8>>,
    bool,
    bool,
    Option<String>,
);
#[derive(Default)]
pub struct PricingCache {
    entries: Mutex<BTreeMap<ProbeKey, Entry>>,
}
static CACHE: OnceLock<PricingCache> = OnceLock::new();
pub(crate) const CONCURRENCY: usize = 16;
static LIMIT: Semaphore = Semaphore::const_new(CONCURRENCY);
pub fn process_cache() -> &'static PricingCache {
    CACHE.get_or_init(PricingCache::default)
}
pub fn validate(cfg: &Config) -> Result<()> {
    if let Some(credits) = &cfg.credit_pricing {
        credits.validate()?;
    }
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
    async fn get(&self, url: String, http: reqwest::Client, ttl: f64, cfg: &Config) -> Observation {
        let cell = self
            .entries
            .lock()
            .await
            .entry((
                url.clone(),
                cfg.transport(),
                cfg.http_cache_directory.clone(),
                cfg.http_cache_direct_warm_target
                    .as_ref()
                    .map(|p| serde_json::to_vec(p).expect("serializable network policy")),
                cfg.http_cache_enabled,
                crate::qualification::active(),
                cfg.discovery_relay.as_ref().map(|r| r.key.clone()),
            ))
            .or_default()
            .clone();
        let ready = cell.get().is_some();
        let initialized = std::sync::atomic::AtomicBool::new(false);
        let entry = cell
            .get_or_init(|| async {
                initialized.store(true, std::sync::atomic::Ordering::Relaxed);
                let _permit = LIMIT.acquire().await.expect("probe semaphore open");
                let slot = crate::http_cache::Slot::new(cfg, &url, "pricing", 64 * 1024);
                let entry = probe_request(&http, &url, slot, ttl, cfg).await;
                tracing::debug!(
                    discovered = entry.line.is_some(),
                    "pricing attempt cached; no automatic retry"
                );
                entry
            })
            .await;
        let expired = entry.created.elapsed() >= entry.lifetime
            || entry
                .created
                .elapsed()
                .saturating_add(entry.observed_age)
                .as_secs_f64()
                >= ttl;
        Observation {
            line: if expired { None } else { entry.line.clone() },
            outcome: entry.outcome,
            http_status: entry.http_status,
            cache: if initialized.load(std::sync::atomic::Ordering::Relaxed) {
                if entry.disk {
                    CachePath::Disk
                } else {
                    CachePath::Initialized
                }
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
        Ok((tool, self.get(url, http, cfg.probe_ttl_seconds, cfg).await))
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
        let priced: BTreeSet<_> = operations_with_credits(
            root,
            cfg.pricing_key.as_deref(),
            cfg.credit_pricing
                .as_ref()
                .map(|c| c.credit_cost_key.as_str()),
        )?
        .into_iter()
        .filter(|o| {
            ["pricing", "credit_pricing"].iter().any(|key| {
                (*key != "credit_pricing" || cfg.credit_pricing.is_some())
                    && !o[key].is_null()
                    && o[key] != serde_json::json!({})
            })
        })
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
    slot: Option<crate::http_cache::Slot>,
    ttl: f64,
    cfg: &Config,
) -> CachedProbe {
    let relay_slot = crate::discovery_relay::slot(cfg, url, "pricing", 64 * 1024);
    for slot in [&slot, &relay_slot].into_iter().flatten() {
        if let Some(entry) = slot.read().await.filter(|e| e.metadata.fresh()) {
            if let Ok(line) = String::from_utf8(entry.data) {
                tracing::info!("Pricing estimate loaded from fresh HTTP disk cache");
                if cfg.discovery_relay.is_some() {
                    tracing::warn!(
                        "Discovery relay enabled: pricing cache may contain relay-supplied estimates"
                    );
                }
                let lifetime = Duration::from_secs(
                    entry
                        .metadata
                        .expires
                        .saturating_sub(crate::http_cache::now()),
                );
                return CachedProbe {
                    created: Instant::now(),
                    line: Some(line),
                    outcome: Outcome::Discovered,
                    http_status: Some(402),
                    disk: true,
                    lifetime,
                    observed_age: Duration::from_secs(
                        crate::http_cache::now().saturating_sub(entry.metadata.stored),
                    ),
                };
            }
            tracing::warn!("Invalid HTTP cached pricing estimate; fetching origin");
        }
        // 402 is not heuristically cacheable. Never conditionally validate a stored
        // payment challenge: persist only its derived display estimate with explicit freshness.
        slot.write(None).await;
    }
    let mut result = probe_network(http, url, slot.is_some()).await;
    let mut write_slot = slot;
    if result.1 != Outcome::Discovered
        && let Some(relay) = &cfg.discovery_relay
    {
        let reason = match result.1 {
            Outcome::HttpTimeout => "timeout",
            Outcome::HttpConnect => "connection",
            Outcome::HttpTransport => "transport",
            Outcome::UnexpectedHttpStatus => "http_status",
            Outcome::MissingHeader => "missing_payment_header",
            Outcome::MalformedChallenge => "malformed_payment_challenge",
            Outcome::UnusableOffer => "unusable_payment_offer",
            Outcome::Discovered => unreachable!("successful probes do not use fallback"),
        };
        crate::discovery_relay::log_fallback(cfg, "pricing", reason, result.2);
        match relay.fetch(url, 402, 64 * 1024, "max_response_bytes").await {
            Ok(response) => {
                let metadata = crate::http_cache::Metadata::from_headers(
                    &response.headers,
                    response.cache_delay(),
                    None,
                );
                result = price_headers(&response.headers, Some(402), metadata);
                write_slot = relay_slot;
            }
            Err(error) => {
                crate::discovery_relay::log_failure(cfg, "pricing", &error);
            }
        }
    }
    let (line, outcome, status, metadata) = result;
    let slot = write_slot;
    if let (Some(slot), Some(mut metadata), Some(line)) = (&slot, metadata, &line) {
        metadata.expires = metadata
            .expires
            .min(metadata.stored.saturating_add(ttl as u64));
        if metadata.fresh() {
            slot.write(Some(crate::http_cache::Entry {
                metadata,
                data: line.as_bytes().to_vec(),
            }))
            .await;
        }
    }
    CachedProbe {
        created: Instant::now(),
        line,
        outcome,
        http_status: status,
        disk: false,
        lifetime: Duration::MAX,
        observed_age: Duration::ZERO,
    }
}
async fn probe_network(
    http: &reqwest::Client,
    url: &str,
    persist: bool,
) -> (
    Option<String>,
    Outcome,
    Option<u16>,
    Option<crate::http_cache::Metadata>,
) {
    let started = Instant::now();
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
                None,
            );
        }
    };
    crate::network::log_http(&response, "pricing");
    let status = Some(response.status().as_u16());
    if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
        return (None, Outcome::UnexpectedHttpStatus, status, None);
    }
    let metadata = if persist {
        crate::http_cache::Metadata::from_headers(response.headers(), started.elapsed(), None)
    } else {
        None
    };
    price_headers(response.headers(), status, metadata)
}
pub(crate) fn price_headers(
    headers: &reqwest::header::HeaderMap,
    status: Option<u16>,
    metadata: Option<crate::http_cache::Metadata>,
) -> (
    Option<String>,
    Outcome,
    Option<u16>,
    Option<crate::http_cache::Metadata>,
) {
    let mut outcome = Outcome::MissingHeader;
    for name in ["payment-required", "x-payment-required"] {
        let Some(raw) = headers.get(name) else {
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
            metadata,
        );
    }
    (None, outcome, status, None)
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

#[cfg(test)]
mod relay_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test]
    async fn relay_pricing_persists_only_estimates_and_shares_process_attempts() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let app = axum::Router::new().route(
            "/price",
            axum::routing::get(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::FORBIDDEN
                }
            }),
        );
        let (address, origin) = crate::test_tls::serve(app).await;
        let socks = crate::test_socks::Socks::start(
            BTreeMap::from([("api.example.com".into(), address)]),
            crate::test_socks::Fault::None,
        )
        .await;
        let url = "https://api.example.com/price".to_owned();
        let context = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
            mode: crate::network::Mode::Tor,
            socks_endpoint: Some(socks.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA);
        let http = context
            .http(
                &crate::network::IsolationId::discovery(&url).unwrap(),
                &url,
                Duration::from_secs(5),
            )
            .unwrap();
        let challenge = STANDARD.encode(r#"{"accepts":[{"amount":"1000","asset":"USDC"}]}"#);
        let body = serde_json::json!({"url":url,"method":"GET","statusCode":402,"body":"","redirectChain":[],"error":null,"headers":{"cache-control":"max-age=600","payment-required":challenge}});
        let (relay, fixture, task) =
            crate::discovery_relay::tests::fixture(body, false, false).await;
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            discovery_relay: Some(relay),
            http_cache_directory: Some(dir.path().join("http-cache")),
            ..Default::default()
        };
        let cache = PricingCache::default();
        let first = cache.get(url.clone(), http.clone(), 600.0, &cfg).await;
        assert_eq!(first.outcome, Outcome::Discovered);
        assert!(first.line.unwrap().contains("$0.001"));
        let second = cache.get(url.clone(), http.clone(), 600.0, &cfg).await;
        assert_eq!(second.cache, CachePath::Hit);
        let fresh = PricingCache::default()
            .get(url.clone(), http.clone(), 600.0, &cfg)
            .await;
        assert_eq!(fresh.cache, CachePath::Disk);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
        let saved = crate::discovery_relay::slot(&cfg, &url, "pricing", 64 * 1024)
            .unwrap()
            .read()
            .await
            .unwrap();
        let text = String::from_utf8(saved.data).unwrap();
        assert!(!text.contains(&challenge));
        assert!(text.starts_with("Cost:"));
        assert!(
            crate::http_cache::Slot::new(&cfg, &url, "pricing", 64 * 1024)
                .unwrap()
                .read()
                .await
                .is_none()
        );
        task.abort();
        origin.abort();
    }

    #[tokio::test]
    async fn every_failed_probe_uses_one_relay_and_failed_relay_stops() {
        for status in [0, 403, 429, 503, 500, 200, 402] {
            for reject in [false, true] {
                let log = tempfile::NamedTempFile::new().unwrap();
                let subscriber = tracing_subscriber::fmt()
                    .without_time()
                    .with_ansi(false)
                    .with_writer(log.reopen().unwrap())
                    .finish();
                let _guard = tracing::subscriber::set_default(subscriber);
                let calls = Arc::new(AtomicUsize::new(0));
                let count = calls.clone();
                let app = axum::Router::new().route(
                    "/price",
                    axum::routing::get(move || {
                        count.fetch_add(1, Ordering::SeqCst);
                        async move { axum::http::StatusCode::from_u16(status).unwrap() }
                    }),
                );
                let (address, origin) = crate::test_tls::serve(app).await;
                let socks = crate::test_socks::Socks::start(
                    BTreeMap::from([("api.example.com".into(), address)]),
                    if status == 0 {
                        crate::test_socks::Fault::Refuse
                    } else {
                        crate::test_socks::Fault::None
                    },
                )
                .await;
                let url = "https://api.example.com/price";
                let context = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
                    mode: crate::network::Mode::Tor,
                    socks_endpoint: Some(socks.address),
                    ..Default::default()
                })
                .unwrap()
                .with_test_root(crate::test_tls::CA);
                let http = context
                    .http(
                        &crate::network::IsolationId::discovery(url).unwrap(),
                        url,
                        Duration::from_secs(5),
                    )
                    .unwrap();
                let challenge = STANDARD.encode(
                    serde_json::to_vec(&serde_json::json!({
                        "accepts": [{"amount":"1000", "asset":"USDC", "network":"eip155:8453"}]
                    }))
                    .unwrap(),
                );
                let envelope = serde_json::json!({"url":url,"method":"GET","statusCode":402,
                    "headers":{"payment-required":challenge},"body":"","redirectChain":[],"error":null});
                let (relay, fixture, task) =
                    crate::discovery_relay::tests::fixture(envelope, reject, false).await;
                let cfg = Config {
                    discovery_source: Some("pricing_provider".into()),
                    discovery_relay: Some(relay),
                    ..Default::default()
                };
                let result = probe_request(&http, url, None, 60.0, &cfg).await;
                assert_eq!(calls.load(Ordering::SeqCst), usize::from(status != 0));
                assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
                if reject {
                    assert!(result.line.is_none());
                    assert_ne!(result.outcome, Outcome::Discovered);
                    // Even another caller cannot restart paid fallback after failure.
                    assert!(
                        cfg.discovery_relay
                            .as_ref()
                            .unwrap()
                            .fetch(url, 402, 65536, "max_response_bytes")
                            .await
                            .is_err()
                    );
                    assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
                } else {
                    assert!(result.line.is_some());
                    assert_eq!(result.outcome, Outcome::Discovered);
                }
                let logs = std::fs::read_to_string(log.path()).unwrap();
                assert!(logs.contains("source=\"pricing_provider\""), "{logs}");
                assert!(logs.contains("stage=\"pricing\""), "{logs}");
                if status == 0 {
                    assert!(logs.contains("connection"), "{logs}");
                } else {
                    assert!(logs.contains(&format!("http_status={status}")), "{logs}");
                }
                if status == 402 {
                    assert!(logs.contains("missing_payment_header"), "{logs}");
                }
                if reject {
                    assert!(logs.contains("discovery fetch failed, no retry"), "{logs}");
                }
                assert!(!logs.contains(url), "{logs}");
                task.abort();
                origin.abort();
            }
        }
    }
}
