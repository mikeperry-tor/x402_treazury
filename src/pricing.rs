//! Unsigned, one-shot discovery. Successful and failed attempts never refresh.
use crate::catalog::{Config, ToolSpec, operations};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OnceCell, Semaphore};

type Entry = Arc<OnceCell<(Instant, Option<String>)>>;
#[derive(Default)]
pub struct PricingCache {
    entries: Mutex<BTreeMap<String, Entry>>,
}
static CACHE: OnceLock<PricingCache> = OnceLock::new();
static LIMIT: Semaphore = Semaphore::const_new(4);
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
    async fn get(&self, url: String, http: reqwest::Client, ttl: f64) -> Option<String> {
        let cell = self
            .entries
            .lock()
            .await
            .entry(url.clone())
            .or_default()
            .clone();
        let (created, line) = cell
            .get_or_init(|| async {
                let _permit = LIMIT.acquire().await.expect("probe semaphore open");
                let result = async {
                    let response = http.get(&url).send().await.ok()?;
                    if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
                        return None;
                    }
                    for name in ["payment-required", "x-payment-required"] {
                        let Some(raw) = response.headers().get(name) else {
                            continue;
                        };
                        let Some(decoded) = STANDARD.decode(raw.as_bytes()).ok() else {
                            continue;
                        };
                        let Some(value) = serde_json::from_slice::<Value>(&decoded).ok() else {
                            continue;
                        };
                        return challenge_line(value.get("accepts")?.get(0)?);
                    }
                    None
                }
                .await;
                tracing::debug!(
                    url,
                    discovered = result.is_some(),
                    "pricing attempt cached; no automatic retry"
                );
                (Instant::now(), result)
            })
            .await;
        if created.elapsed().as_secs_f64() < ttl {
            line.clone()
        } else {
            None
        }
    }
    pub async fn discover(
        &self,
        cfg: &Config,
        root: &Value,
        tools: &[ToolSpec],
        base: &str,
    ) -> Result<BTreeMap<(String, String), String>> {
        validate(cfg)?;
        if !cfg.probe_pricing {
            return Ok(BTreeMap::new());
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
        let mut candidates: Vec<_> = tools
            .iter()
            .filter(|t| {
                t.method == "GET"
                    && t.help_url.is_none()
                    && !t.path.contains('{')
                    && !priced.contains(&(t.method.clone(), t.path.clone()))
            })
            .collect();
        candidates.sort_by_key(|t| (&t.path, &t.method));
        candidates.dedup_by_key(|t| (&t.path, &t.method));
        if candidates.len() > cfg.probe_max_endpoints {
            tracing::warn!(
                candidate_count = candidates.len(),
                limit_endpoints = cfg.probe_max_endpoints,
                "pricing discovery capped by probe_max_endpoints; remaining endpoints will have no probed price"
            );
        }
        candidates.truncate(cfg.probe_max_endpoints);
        let mut lines = BTreeMap::new();
        // Each batch bounds per-source concurrency; LIMIT bounds the whole process.
        for batch in candidates.chunks(cfg.probe_concurrency.min(4)) {
            let mut pending = Vec::new();
            for tool in batch {
                let url = tool.route(base, &serde_json::Map::new())?.url;
                let http =
                    crate::network::discovery(&url, Duration::from_secs_f64(cfg.probe_timeout))?;
                pending.push((tool, self.get(url, http, cfg.probe_ttl_seconds)));
            }
            // Scoped futures are polled concurrently without detached work.
            let mut futures: Vec<_> = pending.into_iter().map(|(t, f)| (t, Box::pin(f))).collect();
            std::future::poll_fn(|cx| {
                let mut i = 0;
                while i < futures.len() {
                    if let std::task::Poll::Ready(line) = futures[i].1.as_mut().poll(cx) {
                        let (tool, _) = futures.swap_remove(i);
                        if let Some(line) = line {
                            lines.insert((tool.method.clone(), tool.path.clone()), line);
                        }
                    } else {
                        i += 1;
                    }
                }
                if futures.is_empty() {
                    std::task::Poll::Ready(())
                } else {
                    std::task::Poll::Pending
                }
            })
            .await;
        }
        Ok(lines)
    }
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
    let tail = if network.is_empty() {
        asset
    } else {
        format!("{asset} on {network}")
    };
    Some(format!(
        "{} {amount} per call (x402 {scheme}, {tail}).",
        if scheme == "upto" {
            "Metered: up to"
        } else {
            "Price:"
        }
    ))
}
