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
    #[serde(default)]
    pub wallet: Option<String>,
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
        if let Some(wallet) = &self.wallet {
            ensure!(
                config.wallets.contains_key(wallet),
                "discovery_relay.wallet must name a declared wallet"
            );
        }
        for source in &self.sources {
            ensure!(
                config.sources.contains_key(source),
                "discovery_relay: unknown source {source}"
            );
        }
        Ok(())
    }
}
/// The stable logical profile, never a current rotating address or listener order.
pub(crate) fn resolve_wallets(
    policy: &Policy,
    config: &crate::deployment::MetaConfig,
    resolution: &crate::rotation::assignment::Resolution,
    selected: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>> {
    use sha2::{Digest, Sha256};
    let mut result = BTreeMap::new();
    for source in selected {
        if !policy.sources.is_empty() && !policy.sources.contains(&source) {
            continue;
        }
        let wallet = if let Some(wallet) = &policy.wallet {
            wallet.clone()
        } else {
            let mut candidates = std::collections::BTreeSet::new();
            if let Some(wallet) = &config.sources[&source].wallet {
                candidates.insert(wallet.clone());
            }
            for bindings in resolution.bindings.values() {
                if let Some(binding) = bindings.get(&source) {
                    candidates.insert(binding.wallet.clone());
                }
            }
            candidates.into_iter().max_by_key(|wallet| {
                // Length-delimited JSON prevents ambiguous source/profile concatenation.
                (Sha256::digest(serde_json::to_vec(&("discovery-wallet-v1", &source, wallet)).expect("serializable identifiers")).to_vec(), wallet.clone())
            }).with_context(|| format!("source {source}: discovery relay needs an assigned wallet; set sources.{source}.wallet or discovery_relay.wallet"))?
        };
        result.insert(source, wallet);
    }
    Ok(result)
}
pub(crate) type Relays = BTreeMap<String, Arc<Relay>>;
pub(crate) async fn build(
    policy: &Policy,
    path: &Path,
    bindings: &BTreeMap<String, String>,
    clients: &BTreeMap<String, PaidClient>,
) -> Result<Relays> {
    let state = Arc::default();
    let mut wallets = BTreeMap::new();
    let mut relays = BTreeMap::new();
    for (source, wallet) in bindings {
        if let std::collections::btree_map::Entry::Vacant(entry) = wallets.entry(wallet.clone()) {
            let mut scoped = policy.clone();
            scoped.wallet = Some(wallet.clone());
            let relay =
                Relay::new_scoped(&scoped, path, clients[wallet].clone(), Arc::clone(&state))
                    .await?;
            entry.insert(relay);
        }
        relays.insert(source.clone(), wallets[wallet].clone());
    }
    Ok(relays)
}

#[derive(Default)]
struct State {
    stopped: bool,
    responses: BTreeMap<(String, String), Arc<Response>>,
    failures: BTreeMap<(String, String), TargetFailure>,
}
/// A completed Curl response reporting failure for this target, not the relay service.
#[derive(Clone, Copy, Debug)]
struct TargetFailure {
    reason: &'static str,
    status: Option<u16>,
    limit: Option<(&'static str, usize)>,
}
impl std::fmt::Display for TargetFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Curl target failed: {}; no retry for this target",
            self.reason
        )?;
        if let Some((setting, limit)) = self.limit {
            write!(f, "; relayed discovery body exceeds {setting}={limit}")?;
        }
        Ok(())
    }
}
impl std::error::Error for TargetFailure {}
pub struct Relay {
    client: PaidClient,
    endpoint: String,
    response_limit: usize,
    pub key: String,
    sources: Vec<String>,
    state: Arc<tokio::sync::Mutex<State>>,
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
    pub fn catalog_cache_age(&self) -> std::time::Duration {
        self.received.elapsed()
    }
    pub fn cache_delay(&self) -> std::time::Duration {
        self.delay.saturating_add(self.received.elapsed())
    }
}
impl Relay {
    pub async fn new(policy: &Policy, path: &Path, client: PaidClient) -> Result<Arc<Self>> {
        Self::new_scoped(policy, path, client, Arc::default()).await
    }
    async fn new_scoped(
        policy: &Policy,
        path: &Path,
        client: PaidClient,
        state: Arc<tokio::sync::Mutex<State>>,
    ) -> Result<Arc<Self>> {
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
            cfg.read_timeout_seconds.map(f64::to_bits),
            cfg.max_response_bytes,
        ))?;
        Ok(Arc::new(Self {
            client: client
                .with_transport(cfg.transport())
                .with_timeout(
                    cfg.read_timeout_seconds
                        .map(std::time::Duration::from_secs_f64),
                )
                .with_download_limits(cfg.max_response_bytes, cfg.max_help_bytes)
                .public_destinations(),
            endpoint,
            response_limit: cfg.max_response_bytes,
            key,
            sources: policy.sources.clone(),
            state,
        }))
    }
    pub fn allows(&self, source: &str) -> bool {
        self.sources.is_empty() || self.sources.iter().any(|s| s == source)
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
        let key = (self.key.clone(), url.to_owned());
        if let Some(failure) = state.failures.get(&key) {
            return Err((*failure).into());
        }
        if let Some(response) = state.responses.get(&key).cloned() {
            if response.status != expected {
                anyhow::bail!("cached discovery relay origin status mismatch; no new fetch");
            }
            if response.body.len() > limit {
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
                state.responses.insert(key, response.clone());
                state.stopped = false;
                Ok(response)
            }
            Err(error) if error.downcast_ref::<TargetFailure>().is_some() => {
                let failure = *error
                    .downcast_ref::<TargetFailure>()
                    .expect("typed target failure");
                state.failures.insert(key, failure);
                state.stopped = false;
                tracing::warn!(
                    reason = failure.reason,
                    http_status = failure.status,
                    "Curl target fetch failed; no retry for this target, other targets remain enabled"
                );
                Err(error)
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
    ensure!(value["url"].as_str() == Some(url), "relay target mismatch");
    ensure!(value["method"] == "GET", "relay method mismatch");
    // Validate attribution and envelope shape before treating a failure as target-local.
    ensure!(
        value["error"].is_null() || value["error"].is_string(),
        "invalid relay error field"
    );
    let status = value["statusCode"]
        .as_u64()
        .and_then(|s| u16::try_from(s).ok())
        .context("invalid relay origin status")?;
    ensure!(
        value["headers"].is_object()
            && value["body"].is_string()
            && value["redirectChain"].is_array(),
        "invalid relay response envelope"
    );
    let target_failure = |reason| TargetFailure {
        reason,
        status: (status != 0).then_some(status),
        limit: None,
    };
    if value["error"].as_str().is_some_and(|s| !s.is_empty()) {
        return Err(target_failure("origin_request").into());
    }
    if status != expected {
        return Err(target_failure("http_status").into());
    }
    if !value["redirectChain"]
        .as_array()
        .expect("validated array")
        .is_empty()
    {
        return Err(target_failure("redirect").into());
    }
    if !value.get("truncated").is_none_or(|v| v == false) {
        return Err(target_failure("truncated_body").into());
    }
    let body = value["body"]
        .as_str()
        .context("relay body must be text")?
        .as_bytes()
        .to_vec();
    if body.len() > limit {
        return Err(
            crate::limits::exceeded("relayed discovery body", setting, limit).context(
                TargetFailure {
                    limit: Some((setting, limit)),
                    ..target_failure("body_limit")
                },
            ),
        );
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
            serde_json::from_slice(&body).context(target_failure("catalog_parse"))?;
    } else if expected == 402 {
        ensure!(
            crate::pricing::price_headers(&headers, Some(402), None)
                .0
                .is_some(),
            target_failure("unusable_pricing")
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
pub(crate) fn log_fallback(cfg: &Config, stage: &str, reason: &str, http_status: Option<u16>) {
    tracing::warn!(
        source = cfg
            .discovery_source
            .as_deref()
            .or(cfg.prefix.as_deref())
            .unwrap_or("standalone"),
        stage,
        reason,
        http_status,
        "Discovery fetch failed; trying configured paid Curl relay once"
    );
}
pub(crate) fn log_failure(cfg: &Config, stage: &str, error: &anyhow::Error) {
    let failure = error.downcast_ref::<TargetFailure>();
    tracing::warn!(
        source = cfg
            .discovery_source
            .as_deref()
            .or(cfg.prefix.as_deref())
            .unwrap_or("standalone"),
        stage,
        reason = failure.map(|f| f.reason).unwrap_or("relay_unavailable"),
        http_status = failure.and_then(|f| f.status),
        "Curl relay fallback failed or unavailable; discovery fetch failed, no retry"
    );
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
        payers: Arc<std::sync::Mutex<Vec<String>>>,
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
        if let Some(signature) = headers.get("payment-signature") {
            let payload: Value =
                serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
            f.payers.lock().unwrap().push(
                payload["payload"]["authorization"]["from"]
                    .as_str()
                    .unwrap()
                    .to_ascii_lowercase(),
            );
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
            payers: Default::default(),
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
    fn second_wallet(first: &Relay) -> Relay {
        let payer = crate::payment::Payer::new(
            "0000000000000000000000000000000000000000000000000000000000000002",
            crate::payment::SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap();
        Relay {
            client: PaidClient::new(payer),
            endpoint: first.endpoint.clone(),
            response_limit: first.response_limit,
            key: "second-wallet".into(),
            sources: vec![],
            state: first.state.clone(),
        }
    }
    #[tokio::test]
    async fn wallet_scopes_do_not_share_paid_results_but_share_failure_breaker() {
        let (first, f, task) = fixture(envelope(TARGET), false, false).await;
        let second = second_wallet(&first);
        first
            .fetch(TARGET, 200, 4096, "max_spec_bytes")
            .await
            .unwrap();
        second
            .fetch(TARGET, 200, 4096, "max_spec_bytes")
            .await
            .unwrap();
        first
            .fetch(TARGET, 200, 4096, "max_spec_bytes")
            .await
            .unwrap();
        assert_eq!(f.signed.load(Ordering::SeqCst), 2);
        let payers = f.payers.lock().unwrap().clone();
        assert_eq!(payers.len(), 2);
        assert_ne!(payers[0], payers[1]);
        task.abort();
        let (first, f, task) = fixture(envelope(TARGET), true, false).await;
        let second = second_wallet(&first);
        let (a, b) = tokio::join!(
            first.fetch(TARGET, 200, 4096, "max_spec_bytes"),
            second.fetch(TARGET, 200, 4096, "max_spec_bytes")
        );
        assert!(a.is_err() && b.is_err());
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        task.abort();
    }
    #[tokio::test]
    async fn built_relays_share_wallet_clients_and_one_global_breaker() {
        let config: crate::deployment::MetaConfig =
            toml::from_str(include_str!("../examples/deployments/privacy.toml")).unwrap();
        let policy = config.discovery_relay.as_ref().unwrap();
        let bindings = resolve_wallets(
            policy,
            &config,
            &crate::rotation::assignment::resolve(&config).unwrap(),
            config.sources.keys().cloned(),
        )
        .unwrap();
        let clients = config
            .wallets
            .keys()
            .map(|w| (w.clone(), PaidClient::unsigned()))
            .collect();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/deployments/privacy.toml");
        let relays = build(policy, &path, &bindings, &clients).await.unwrap();
        assert!(Arc::ptr_eq(&relays["exa"], &relays["company_otto"]));
        assert!(!Arc::ptr_eq(&relays["social"], &relays["webinfo"]));
        assert!(Arc::ptr_eq(
            &relays["social"].state,
            &relays["webinfo"].state
        ));
        assert_ne!(relays["social"].key, relays["webinfo"].key);
    }
    #[test]
    fn default_wallet_selection_is_stable_scoped_and_honors_overrides() {
        let mut config: crate::deployment::MetaConfig =
            toml::from_str(include_str!("../examples/deployments/privacy.toml")).unwrap();
        let policy = config.discovery_relay.clone().unwrap();
        assert!(policy.wallet.is_none() && policy.serve && policy.warm);
        let resolve = |config: &crate::deployment::MetaConfig, policy: &Policy| {
            resolve_wallets(
                policy,
                config,
                &crate::rotation::assignment::resolve(config).unwrap(),
                config.sources.keys().cloned(),
            )
            .unwrap()
        };
        let first = resolve(&config, &policy);
        assert_eq!(first["exa"], "company");
        assert_eq!(first["social"], "social");
        assert_eq!(first["webinfo"], "web");
        let web = config.servers.remove("web").unwrap();
        config
            .servers
            .insert("renamed_listener".into(), web.clone());
        config.servers.insert("another_listener".into(), web);
        assert_eq!(first, resolve(&config, &policy));
        // Warming a subset chooses the same wallet as complete serving startup.
        let subset = resolve_wallets(
            &policy,
            &config,
            &crate::rotation::assignment::resolve(&config).unwrap(),
            ["exa".into()],
        )
        .unwrap();
        assert_eq!(subset.len(), 1);
        assert_eq!(subset["exa"], first["exa"]);
        let mut override_policy = policy.clone();
        override_policy.wallet = Some("web".into());
        assert!(
            resolve(&config, &override_policy)
                .values()
                .all(|w| w == "web")
        );
        config.sources.get_mut("exa").unwrap().wallet = Some("social".into());
        assert_eq!(resolve(&config, &policy)["exa"], "social");
        // Automatic assignments select existing generated profiles, not new pools.
        for server in config.servers.values_mut() {
            server.wallet = None;
        }
        for source in config.sources.values_mut() {
            source.wallet = None;
        }
        config
            .wallet_templates
            .insert("template".into(), config.wallets["web"].clone());
        config.wallet_assignment = Some(crate::rotation::assignment::Assignment {
            scope: crate::rotation::assignment::Scope::Binding,
            template: "template".into(),
        });
        let assignment = crate::rotation::assignment::resolve(&config).unwrap();
        let automatic = resolve_wallets(
            &policy,
            &config,
            &assignment,
            config.sources.keys().cloned(),
        )
        .unwrap();
        for (source, wallet) in automatic {
            assert!(assignment.generated.contains_key(&wallet));
            assert!(
                assignment
                    .bindings
                    .values()
                    .any(|b| b.get(&source).is_some_and(|b| b.wallet == wallet))
            );
        }
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
    async fn aliases_share_paid_success_and_invalid_catalog_is_not_retried() {
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
                .fetch(TARGET, 200, 4096, "max_spec_bytes")
                .await
                .is_err()
        );
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        task.abort();
    }
    #[tokio::test]
    async fn target_failures_are_not_retried_and_do_not_disable_other_providers() {
        for expected in [200, 402] {
            for failure in [
                "403",
                "429",
                "503",
                "reset",
                "invalid_content",
                "wrong_target",
            ] {
                let mut body = envelope(TARGET);
                body["statusCode"] = json!(expected);
                match failure {
                    "reset" => {
                        body["statusCode"] = json!(0);
                        body["error"] = json!("connection reset: private upstream details");
                    }
                    "invalid_content" => {
                        body["body"] = json!("invalid JSON");
                    }
                    "wrong_target" => {
                        body["url"] = json!("https://wrong.example/");
                    }
                    code => {
                        body["statusCode"] = json!(code.parse::<u16>().unwrap());
                    }
                }
                let (relay, fixture, task) = fixture(body, false, false).await;
                let results = futures_util::future::join_all(
                    (0..4).map(|_| relay.fetch(TARGET, expected, 4096, "max_spec_bytes")),
                )
                .await;
                assert!(results.iter().all(Result::is_err));
                assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
                let other_url = "https://other.example/spec";
                // Two local handlers model Curl's different responses per target;
                // both relay clients use the same wallet key and shared failure state.
                let (mut other, other_fixture, other_task) =
                    self::fixture(envelope(other_url), false, false).await;
                Arc::get_mut(&mut other).unwrap().state = relay.state.clone();
                let result = other.fetch(other_url, 200, 4096, "max_spec_bytes").await;
                if failure == "wrong_target" {
                    assert!(result.is_err());
                    assert_eq!(other_fixture.signed.load(Ordering::SeqCst), 0);
                } else {
                    assert!(result.is_ok(), "{expected}/{failure}");
                    assert_eq!(other_fixture.signed.load(Ordering::SeqCst), 1);
                }
                task.abort();
                other_task.abort();
            }
        }
    }
    #[tokio::test]
    async fn cancellation_after_signed_submission_disables_relay() {
        let (relay, f, task) = fixture(envelope(TARGET), false, true).await;
        let second = second_wallet(&relay);
        let r = relay.clone();
        let fetch = tokio::spawn(async move { r.fetch(TARGET, 200, 4096, "max_spec_bytes").await });
        tokio::time::timeout(std::time::Duration::from_secs(10), f.arrived.notified())
            .await
            .unwrap();
        fetch.abort();
        let _ = fetch.await;
        assert!(
            second
                .fetch(TARGET, 200, 4096, "max_spec_bytes")
                .await
                .is_err()
        );
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
            !crate::http_cache::Metadata::from_relay_headers(
                &response.headers,
                response.catalog_cache_age(),
                Some(600)
            )
            .unwrap()
            .fresh()
        );
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
        for status in [403, 429, 503, 500, 404, 200] {
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(log.reopen().unwrap())
                .finish();
            let _guard = tracing::subscriber::set_default(subscriber);
            let origin_calls = Arc::new(AtomicUsize::new(0));
            let count = origin_calls.clone();
            let app = Router::new().route(
                "/spec",
                axum::routing::get(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        // 200 deliberately returns invalid catalog text.
                        (StatusCode::from_u16(status).unwrap(), "invalid catalog")
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
                discovery_source: Some("test_provider".into()),
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
            let logs = std::fs::read_to_string(log.path()).unwrap();
            assert!(logs.contains("source=\"test_provider\""), "{logs}");
            assert!(logs.contains("stage=\"catalog\""), "{logs}");
            if status != 200 {
                assert!(logs.contains(&format!("http_status={status}")), "{logs}");
            } else {
                assert!(logs.contains("reason=\"parse\""), "{logs}");
            }
            assert!(!logs.contains(&url), "{logs}");
            task.abort();
            origin.abort();
        }
    }
}
