//! Explicit paid discovery through the Curl response envelope, never a transport retry.
use crate::{
    catalog::{Config, RoutedRequest},
    payment::PaidClient,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub provider: PathBuf,
    pub wallet: String,
    #[serde(default)]
    pub serve: bool,
    #[serde(default)]
    pub warm: bool,
    /// Empty means all authored sources.
    #[serde(default)]
    pub sources: Vec<String>,
}
impl Policy {
    pub fn validate(&self, config: &crate::deployment::MetaConfig) -> Result<()> {
        ensure!(
            config.wallets.contains_key(&self.wallet),
            "discovery_relay.wallet must name a declared wallet"
        );
        for source in &self.sources {
            ensure!(
                config.sources.contains_key(source),
                "discovery_relay: unknown source {source}"
            );
        }
        Ok(())
    }
}
#[derive(Default)]
struct State {
    stopped: bool,
    responses: BTreeMap<String, Arc<Response>>,
}
pub struct Relay {
    client: PaidClient,
    endpoint: String,
    response_limit: usize,
    pub key: String,
    sources: Vec<String>,
    state: tokio::sync::Mutex<State>,
}
impl std::fmt::Debug for Relay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscoveryRelay")
    }
}
pub struct Response {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
    pub delay: std::time::Duration,
    received: Instant,
}
impl Response {
    pub fn cache_delay(&self) -> std::time::Duration {
        self.delay.saturating_add(self.received.elapsed())
    }
}
impl Relay {
    pub async fn new(policy: &Policy, path: &Path, client: PaidClient) -> Result<Arc<Self>> {
        let provider = if policy.provider.is_absolute() {
            policy.provider.clone()
        } else {
            path.parent()
                .unwrap_or(Path::new("."))
                .join(&policy.provider)
        };
        let table = toml::from_str(&tokio::fs::read_to_string(&provider).await?)?;
        let cfg = crate::config::resolve(table, &provider).await?.settings;
        ensure!(
            !cfg.spec.starts_with("http://") && !cfg.spec.starts_with("https://"),
            "discovery relay requires a local bootstrap catalog"
        );
        let document: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&cfg.spec).await?)?;
        let tools = crate::catalog::build_tools(&cfg, &document, "relay")?;
        ensure!(
            tools
                .iter()
                .any(|t| t.method == "POST" && t.path == "/curl"),
            "discovery relay provider must expose POST /curl"
        );
        let base = cfg
            .base_url
            .as_deref()
            .context("discovery relay requires base_url")?;
        let endpoint = format!("{}/curl", base.trim_end_matches('/'));
        let url = reqwest::Url::parse(&endpoint)?;
        ensure!(
            url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
            "discovery relay requires public HTTPS without credentials"
        );
        // Enablement and source selection gate access, but do not change the
        // representation: warming must remain reusable after changing those flags.
        let key = serde_json::to_string(&(
            &policy.wallet,
            &endpoint,
            cfg.allow_http1,
            cfg.allow_tls12,
            cfg.timeout.to_bits(),
            cfg.max_response_bytes,
        ))?;
        Ok(Arc::new(Self {
            client: client
                .with_transport(cfg.transport())
                .with_timeout(std::time::Duration::from_secs_f64(cfg.timeout))
                .with_download_limits(cfg.max_response_bytes, cfg.max_help_bytes)
                .public_destinations(),
            endpoint,
            response_limit: cfg.max_response_bytes,
            key,
            sources: policy.sources.clone(),
            state: Default::default(),
        }))
    }
    pub fn allows(&self, source: &str) -> bool {
        self.sources.is_empty() || self.sources.iter().any(|s| s == source)
    }
    pub async fn stop(&self) {
        self.state.lock().await.stopped = true;
        tracing::warn!("Discovery relay disabled for this run after failure; no paid retry");
    }
    pub async fn fetch(
        &self,
        url: &str,
        expected: u16,
        limit: usize,
        setting: &'static str,
    ) -> Result<Arc<Response>> {
        let target = reqwest::Url::parse(url)?;
        ensure!(
            target.scheme() == "https"
                && target.username().is_empty()
                && target.password().is_none(),
            "discovery relay targets must be HTTPS without credentials"
        );
        let mut state = self.state.lock().await;
        if let Some(response) = state.responses.get(url).cloned() {
            if response.status != expected {
                state.stopped = true;
                anyhow::bail!("discovery relay origin status mismatch; disabled for this run");
            }
            if response.body.len() > limit {
                state.stopped = true;
                return Err(crate::limits::exceeded(
                    "relayed discovery body",
                    setting,
                    limit,
                ));
            }
            return Ok(response.clone());
        }
        ensure!(
            !state.stopped,
            "discovery relay disabled after error or cancellation; no paid retry"
        );
        // Set BEFORE the first await: cancellation/uncertain settlement cannot restart spending.
        // Serializing paid fetches also prevents queued pricing probes draining a failing wallet.
        state.stopped = true;
        tracing::warn!(
            "Using configured paid discovery relay; target content and headers are supplied by relay"
        );
        let started = Instant::now();
        let result = async {
            let output = self
                .client
                .execute_response(RoutedRequest {
                    method: "POST".into(),
                    url: self.endpoint.clone(),
                    query: BTreeMap::new(),
                    body: Some(serde_json::json!({"url":url,"method":"GET"})),
                })
                .await.map_err(|_| anyhow::anyhow!("discovery relay payment, transport or response failed; check wallet and max_response_bytes={}", self.response_limit))?;
            decode(
                &output.bytes,
                url,
                expected,
                limit,
                setting,
                started.elapsed(),
            )
        }
        .await;
        match result {
            Ok(response) => {
                let response = Arc::new(response);
                state.responses.insert(url.to_owned(), response.clone());
                state.stopped = false;
                Ok(response)
            }
            Err(error) => {
                tracing::warn!("Discovery relay failed; disabled for this run, no paid retry");
                Err(error.context("discovery relay disabled for this run; no paid retry"))
            }
        }
    }
}
fn decode(
    bytes: &[u8],
    url: &str,
    expected: u16,
    limit: usize,
    setting: &'static str,
    delay: std::time::Duration,
) -> Result<Response> {
    let value: serde_json::Value = serde_json::from_slice(bytes).context("invalid relay JSON")?;
    ensure!(
        value["error"].is_null() || value["error"] == "",
        "relay reported failure"
    );
    ensure!(value["url"].as_str() == Some(url), "relay target mismatch");
    ensure!(value["method"] == "GET", "relay method mismatch");
    ensure!(
        value["statusCode"].as_u64() == Some(u64::from(expected)),
        "relay origin status mismatch"
    );
    ensure!(
        value["redirectChain"].as_array().is_some_and(Vec::is_empty),
        "discovery relay redirects are unsupported"
    );
    ensure!(
        value.get("truncated").is_none_or(|v| v == false),
        "relay returned truncated body"
    );
    let body = value["body"]
        .as_str()
        .context("relay body must be text")?
        .as_bytes()
        .to_vec();
    if body.len() > limit {
        return Err(crate::limits::exceeded(
            "relayed discovery body",
            setting,
            limit,
        ));
    }
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in value["headers"]
        .as_object()
        .context("relay headers missing")?
    {
        headers.append(
            reqwest::header::HeaderName::from_bytes(name.as_bytes())?,
            reqwest::header::HeaderValue::from_str(
                value.as_str().context("relay header must be text")?,
            )?,
        );
    }
    if expected == 200 {
        let _: serde_json::Value =
            serde_json::from_slice(&body).context("relay catalog is not complete JSON")?;
    } else if expected == 402 {
        ensure!(
            crate::pricing::price_headers(&headers, Some(402), None)
                .0
                .is_some(),
            "relay pricing response lacks a usable payment header"
        );
    }
    Ok(Response {
        status: expected,
        headers,
        body,
        delay,
        received: Instant::now(),
    })
}
pub fn eligible(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|e| e.downcast_ref::<reqwest::Error>())
        .any(|e| {
            e.is_connect() || e.is_timeout() || e.status() == Some(reqwest::StatusCode::FORBIDDEN)
        })
}
pub(crate) fn slot(
    cfg: &Config,
    url: &str,
    kind: &str,
    limit: usize,
) -> Option<crate::http_cache::Slot> {
    let relay = cfg.discovery_relay.as_ref()?;
    crate::http_cache::Slot::new(cfg, url, kind, limit).map(|s| s.relay(&relay.key))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{
        Router,
        extract::State as AxumState,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};
    const TARGET: &str = "https://example.com/openapi.json";
    fn envelope(url: &str) -> Value {
        json!({"url":url,"method":"GET","statusCode":200,"headers":{"cache-control":"max-age=600"},"body":"{\"openapi\":\"3.0.0\",\"paths\":{}}","redirectChain":[],"error":null})
    }
    #[derive(Clone)]
    pub(crate) struct Fixture {
        calls: Arc<AtomicUsize>,
        pub(crate) signed: Arc<AtomicUsize>,
        body: Value,
        reject: bool,
        arrived: Arc<tokio::sync::Notify>,
        stall: bool,
    }
    async fn handler(
        AxumState(f): AxumState<Fixture>,
        headers: HeaderMap,
    ) -> axum::response::Response {
        f.calls.fetch_add(1, Ordering::SeqCst);
        if headers.contains_key("payment-signature") {
            f.signed.fetch_add(1, Ordering::SeqCst);
            f.arrived.notify_one();
            if f.stall {
                std::future::pending::<()>().await;
            }
            if f.reject {
                return StatusCode::BAD_GATEWAY.into_response();
            }
            return axum::Json(f.body).into_response();
        }
        let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/curl","description":"fixture","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"10000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
        (
            StatusCode::PAYMENT_REQUIRED,
            [(
                "payment-required",
                STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
            )],
            "",
        )
            .into_response()
    }
    pub(crate) async fn fixture(
        body: Value,
        reject: bool,
        stall: bool,
    ) -> (Arc<Relay>, Fixture, tokio::task::JoinHandle<()>) {
        let f = Fixture {
            calls: Default::default(),
            signed: Default::default(),
            body,
            reject,
            arrived: Default::default(),
            stall,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/curl", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/curl", post(handler))
            .with_state(f.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // Fixture-only local egress. Production construction always enforces public HTTPS.
        let payer = crate::payment::Payer::new(
            "0000000000000000000000000000000000000000000000000000000000000001",
            crate::payment::SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap();
        (
            Arc::new(Relay {
                client: PaidClient::new(payer),
                endpoint,
                response_limit: crate::limits::RESPONSE_BYTES,
                key: "fixture".into(),
                sources: vec![],
                state: Default::default(),
            }),
            f,
            task,
        )
    }
    #[tokio::test]
    async fn paid_failure_stops_all_queued_targets_and_never_replays() {
        let (relay, f, task) = fixture(envelope(TARGET), true, false).await;
        let results = futures_util::future::join_all((0..16).map(|i| {
            let relay = relay.clone();
            async move {
                relay
                    .fetch(
                        &format!("https://example.com/{i}"),
                        200,
                        4096,
                        "max_spec_bytes",
                    )
                    .await
            }
        }))
        .await;
        assert!(results.iter().all(Result::is_err));
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
        task.abort();
    }
    #[tokio::test]
    async fn aliases_share_paid_success_and_invalid_catalog_trips_breaker() {
        let (relay, f, task) = fixture(envelope(TARGET), false, false).await;
        let (a, b) = tokio::join!(
            relay.fetch(TARGET, 200, 4096, "max_spec_bytes"),
            relay.fetch(TARGET, 200, 4096, "max_spec_bytes")
        );
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        task.abort();
        let mut invalid = envelope(TARGET);
        invalid["body"] = json!("broken JSON");
        let (relay, f, task) = fixture(invalid, false, false).await;
        assert!(
            relay
                .fetch(TARGET, 200, 4096, "max_spec_bytes")
                .await
                .is_err()
        );
        assert!(
            relay
                .fetch("https://example.com/another", 200, 4096, "max_spec_bytes")
                .await
                .is_err()
        );
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        task.abort();
    }
    #[tokio::test]
    async fn cancellation_after_signed_submission_disables_relay() {
        let (relay, f, task) = fixture(envelope(TARGET), false, true).await;
        let r = relay.clone();
        let fetch = tokio::spawn(async move { r.fetch(TARGET, 200, 4096, "max_spec_bytes").await });
        tokio::time::timeout(std::time::Duration::from_secs(10), f.arrived.notified())
            .await
            .unwrap();
        fetch.abort();
        let _ = fetch.await;
        assert!(
            relay
                .fetch(TARGET, 200, 4096, "max_spec_bytes")
                .await
                .is_err()
        );
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        task.abort();
    }
    #[test]
    fn rejects_redirects_truncation_wrong_target_and_oversize() {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        for (field, value) in [
            ("redirectChain", json!([{}])),
            ("truncated", json!(true)),
            ("url", json!("https://other.example")),
            ("statusCode", json!(403)),
            ("body", json!("truncated JSON")),
        ] {
            let mut v = envelope(TARGET);
            v[field] = value;
            assert!(
                decode(
                    &serde_json::to_vec(&v).unwrap(),
                    TARGET,
                    200,
                    4096,
                    "max_spec_bytes",
                    Default::default()
                )
                .is_err(),
                "{field}"
            );
        }
        let err = decode(
            &serde_json::to_vec(&envelope(TARGET)).unwrap(),
            TARGET,
            200,
            1,
            "max_spec_bytes",
            Default::default(),
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("max_spec_bytes=1"));
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert!(logs.contains("download limit exceeded") && logs.contains("limit_bytes=1"));
        assert!(!logs.contains(TARGET));
        let mut v = envelope(TARGET);
        v["statusCode"] = json!(402);
        assert!(
            decode(
                &serde_json::to_vec(&v).unwrap(),
                TARGET,
                402,
                4096,
                "max_response_bytes",
                Default::default()
            )
            .is_err()
        );
        v["headers"]["payment-required"] =
            json!(STANDARD.encode(r#"{"accepts":[{"amount":"1000","asset":"USDC"}]}"#));
        assert!(
            decode(
                &serde_json::to_vec(&v).unwrap(),
                TARGET,
                402,
                4096,
                "max_response_bytes",
                Default::default()
            )
            .is_ok()
        );
    }
    #[test]
    fn sharing_a_response_does_not_renew_origin_freshness() {
        let mut response = decode(
            &serde_json::to_vec(&envelope(TARGET)).unwrap(),
            TARGET,
            200,
            4096,
            "max_spec_bytes",
            Default::default(),
        )
        .unwrap();
        response.received = Instant::now() - std::time::Duration::from_secs(601);
        assert!(
            !crate::http_cache::Metadata::from_headers(
                &response.headers,
                response.cache_delay(),
                None
            )
            .unwrap()
            .fresh()
        );
    }
    #[tokio::test]
    async fn blocked_catalog_falls_back_once_and_only_relay_cache_reuses_result() {
        let origin_calls = Arc::new(AtomicUsize::new(0));
        let count = origin_calls.clone();
        let app = Router::new().route(
            "/spec",
            axum::routing::get(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    StatusCode::FORBIDDEN
                }
            }),
        );
        let (address, origin) = crate::test_tls::serve(app).await;
        let url = "https://api.example.com/spec".to_owned();
        let _socks = crate::test_socks::Socks::start(
            BTreeMap::from([("api.example.com".into(), address)]),
            crate::test_socks::Fault::None,
        )
        .await;
        let context = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
            mode: crate::network::Mode::Tor,
            socks_endpoint: Some(_socks.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA);
        let http = context
            .http(
                &crate::network::IsolationId::discovery(&url).unwrap(),
                &url,
                std::time::Duration::from_secs(5),
            )
            .unwrap();
        let (relay, f, task) = fixture(envelope(&url), false, false).await;
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            spec: url.clone(),
            discovery_relay: Some(relay),
            http_cache_directory: Some(dir.path().join("http-cache")),
            ..Default::default()
        };
        crate::catalog::load_json_discovery(&cfg, &http)
            .await
            .unwrap();
        crate::catalog::load_json_discovery(&cfg, &http)
            .await
            .unwrap();
        assert_eq!(origin_calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        assert!(
            crate::http_cache::Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes)
                .unwrap()
                .read()
                .await
                .is_none()
        );
        let without = Config {
            discovery_relay: None,
            ..cfg
        };
        assert!(
            crate::catalog::load_json_discovery(&without, &http)
                .await
                .is_err()
        );
        assert_eq!(origin_calls.load(Ordering::SeqCst), 2);
        task.abort();
        origin.abort();
    }
}
