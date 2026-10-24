//! Ordered paid read relays; static cache/readiness ownership stays with discovery.
mod adapters;
pub(crate) mod api;
pub(crate) mod document;
mod policy;
mod sequence;
use crate::{
    catalog::{Config, RoutedRequest},
    payment::PaidClient,
};
use anyhow::{Context, Result, ensure};
pub use policy::Policy;
pub(crate) use sequence::SearchRelay;
/// Trusted acquisition purpose, never selected by agent arguments or target URLs.
#[derive(Clone, Copy, Default)]
pub(crate) enum Purpose {
    SourceDiscovery,
    #[default]
    ApiRequests,
    ProviderMetadata,
}
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Instant};
/// The stable logical profile, never a current rotating address or listener order.
pub(crate) fn resolve_wallets(
    policy: &Policy,
    config: &crate::deployment::MetaConfig,
    resolution: &crate::rotation::assignment::Resolution,
    selected: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>> {
    use sha2::{Digest, Sha256};
    if !policy.has_method("GET") {
        return Ok(BTreeMap::new());
    }
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
            }).with_context(|| format!("source {source}: discovery relay needs an assigned wallet; set sources.{source}.wallet or relay_fallback.wallet"))?
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

/// Deliberately has no source: upstream errors can contain credentials or prose.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RelayFailure {
    reason: &'static str,
    status: Option<u16>,
}
impl RelayFailure {
    pub(crate) fn http_status(status: u16) -> Self {
        Self {
            reason: "http_status",
            status: Some(status),
        }
    }

    pub(crate) fn rejected_response() -> Self {
        Self {
            reason: "response_rejected",
            status: None,
        }
    }

    pub(crate) fn from_error(error: &anyhow::Error) -> Self {
        if let Some(partial) = error.downcast_ref::<document::IncompleteDocument>() {
            return Self {
                reason: "partial_response",
                status: Some(partial.status),
            };
        }
        if let Some(failure) = error.downcast_ref::<TargetFailure>() {
            return Self {
                reason: failure.reason,
                status: failure.status,
            };
        }
        if let Some(failure) = error.downcast_ref::<Self>() {
            return *failure;
        }
        if error.is::<crate::qualification::RetryAuthorityDenied>() {
            return Self {
                reason: "qualification_authority",
                status: None,
            };
        }
        if error.is::<crate::payment::IncompleteRelayResponse>() {
            return Self {
                reason: "partial_response",
                status: None,
            };
        }
        if let Some(http) = error.downcast_ref::<reqwest::Error>() {
            let reason = if http.is_status() {
                "http_status"
            } else {
                crate::payment::diagnostics::transport_reason(http)
            };
            return Self {
                reason,
                status: http.status().map(|s| s.as_u16()),
            };
        }
        if let Some(http) = error.downcast_ref::<crate::payment::HttpFailure>() {
            return Self {
                reason: "http_status",
                status: Some(http.status.as_u16()),
            };
        }
        if error
            .downcast_ref::<crate::limits::DownloadLimit>()
            .is_some()
        {
            return Self {
                reason: "download_limit",
                status: None,
            };
        }
        if let Some(admission) = error.downcast_ref::<crate::rotation::error::AdmissionError>() {
            use crate::rotation::error::AdmissionError;
            let reason = match admission {
                AdmissionError::PayerChanged => "payer_changed",
                AdmissionError::UnsupportedPayment(_) => "unsupported_payment",
                AdmissionError::PriceLimit(_) => "price_limit",
                AdmissionError::PaymentPending(_) => "payment_pending",
                AdmissionError::WalletNotReady(_) => "wallet_not_ready",
                AdmissionError::FundingUnavailable(_) => "funding_unavailable",
                AdmissionError::FundingRestricted | AdmissionError::FundingPermitDenied => {
                    "funding_denied"
                }
                AdmissionError::OutcomeUnknown(_) => "payment_outcome_unknown",
                AdmissionError::ChainRecoveryRequired(_) => "chain_recovery_required",
            };
            return Self {
                reason,
                status: None,
            };
        }
        Self {
            reason: "payment_or_response",
            status: None,
        }
    }
    pub(crate) fn kind(&self) -> &'static str {
        self.reason
    }
    pub(crate) fn status(&self) -> Option<u16> {
        self.status
    }
    pub(crate) fn explanation(&self) -> &'static str {
        match self.reason {
            "http_status" => "The server returned an unsuccessful HTTP status",
            "tls_failure" => "The secure connection failed TLS validation or negotiation",
            "connection_timeout" => "The connection to the server or proxy timed out",
            "read_inactivity" => "The server stopped sending data before the response was complete",
            "connection" => "Could not connect to the server or configured proxy",
            "body" => "The response body could not be read completely",
            "decode" => "The server response could not be decoded",
            "request_construction" => "The HTTP request could not be constructed",
            "transport" => "The HTTP request or response failed during transport",
            "download_limit" => "The response exceeded the configured download limit",
            "partial_response" | "incomplete_delivery" => {
                "The server did not deliver a complete response"
            }
            "response_rejected" => {
                "The returned content did not meet the required response contract"
            }
            "invalid_relay_envelope" => "The response did not match the configured relay format",
            "catalog_parse" => "The relay returned content that is not valid catalog JSON",
            "origin_request" => {
                "The relay could not retrieve the requested content from its source"
            }
            "target_payment_required" => {
                "The target service requires a payment that this relay cannot perform"
            }
            "unsolicited_not_modified" => {
                "The server returned HTTP 304 when a complete response was required"
            }
            "unsupported_response" => "The relay cannot preserve the required response format",
            "no_representable_relay" => "No configured relay can represent this request",
            "provider_unavailable" => "A configured relay is unavailable",
            "qualification_authority" | "funding_denied" => {
                "This operation does not have authority to continue"
            }
            "payer_changed" => "The wallet assigned to the payment changed",
            "unsupported_payment" => {
                "The server requested an unsupported payment format or requirement"
            }
            "price_limit" => "The requested payment exceeds the authorized amount",
            "payment_pending" => "A payment is still pending",
            "wallet_not_ready" => "No eligible wallet is ready for this payment",
            "funding_unavailable" => "Wallet funding is unavailable",
            "payment_outcome_unknown" => {
                "The payment outcome is unknown; reconciliation is required"
            }
            "chain_recovery_required" => {
                "Blockchain reconciliation is required before this wallet can continue"
            }
            _ => {
                "The requested content could not be retrieved or processed; no further cause information is available"
            }
        }
    }
}
impl std::fmt::Display for RelayFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "discovery relay request failed: reason={}", self.reason)?;
        if let Some(status) = self.status {
            write!(f, " http_status={status}")?;
        }
        Ok(())
    }
}
impl std::error::Error for RelayFailure {}

#[derive(Default)]
pub(crate) struct State {
    #[cfg(test)]
    dependency_wait: Option<Arc<tokio::sync::Notify>>,
    changes: Arc<tokio::sync::Notify>,
    providers: BTreeMap<String, Arc<tokio::sync::Mutex<()>>>,
    stopped: std::collections::BTreeSet<String>,
    outcomes: BTreeMap<(String, String), std::result::Result<Arc<Response>, TargetFailure>>,
}
/// Redacted definitive target/provider facts retained by the static acquisition owner.
#[derive(Clone, Copy, Debug)]
struct TargetFailure {
    reason: &'static str,
    status: Option<u16>,
    limit: Option<(&'static str, usize)>,
    advance: bool,
    possible: bool,
    retry_after: Option<crate::network::RetryAfter>,
}
impl TargetFailure {
    fn new(reason: &'static str, status: Option<u16>, advance: bool) -> Self {
        Self {
            reason,
            status,
            limit: None,
            advance,
            possible: false,
            retry_after: None,
        }
    }
    fn error(self, possible: bool) -> anyhow::Error {
        let error = match self.limit {
            Some((setting, limit)) => {
                crate::limits::exceeded("relayed discovery body", setting, limit).context(self)
            }
            None => self.into(),
        };
        if possible || self.possible {
            error.context(document::PossiblePayment)
        } else {
            error
        }
    }
}
impl std::fmt::Display for TargetFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "relay target failed: {}", self.reason)?;
        if let Some(status) = self.status {
            write!(f, "; HTTP {status}")?;
        }
        Ok(())
    }
}
impl std::error::Error for TargetFailure {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDisposition {
    DependencyWait,
    Terminal,
}
impl RetryDisposition {
    pub fn from_error(error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<Self>()
            .copied()
            .unwrap_or(Self::Terminal)
    }
}
impl std::fmt::Display for RetryDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DependencyWait => {
                "discovery relay payment identity unavailable; waiting for capacity"
            }
            Self::Terminal => "discovery relay retry unavailable",
        })
    }
}
impl std::error::Error for RetryDisposition {}

#[derive(Clone)]
pub struct Relay {
    client: PaidClient,
    sequence: SearchRelay,
    pub key: String,
    sources: Vec<String>,
    state: Arc<tokio::sync::Mutex<State>>,
}
impl std::fmt::Debug for Relay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscoveryRelay")
    }
}
#[derive(Clone)]
pub struct Response {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
    pub final_url: reqwest::Url,
    pub delay: std::time::Duration,
    received: Instant,
    pub cache_key: String,
    pub via: String,
    pub payment_may_have_occurred: bool,
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
        let sequence = SearchRelay::load(policy, path).await?;
        Ok(Arc::new(Self {
            client,
            sequence,
            key: serde_json::to_string(&policy.wallet)?,
            sources: policy.sources.clone(),
            state,
        }))
    }
    pub fn allows(&self, source: &str) -> bool {
        self.sources.is_empty() || self.sources.iter().any(|s| s == source)
    }
    pub async fn drain(&self) {
        self.client.drain().await;
    }
    pub async fn fetch_when_ready(
        &self,
        url: &str,
        limit: usize,
        setting: &'static str,
    ) -> Result<Arc<Response>> {
        self.fetch_when_ready_bounded(url, limit, setting, None, None, None)
            .await
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn fetch_when_ready_bounded(
        &self,
        url: &str,
        limit: usize,
        setting: &'static str,
        gate: Option<Arc<tokio::sync::Semaphore>>,
        operator_status: Option<tokio::sync::watch::Sender<bool>>,
        retry_after: Option<crate::network::RetryAfter>,
    ) -> Result<Arc<Response>> {
        let capacity = self.client.discovery_changes().await?;
        let relay_changes = self.state.lock().await.changes.clone();
        loop {
            let relay_changed = relay_changes.notified();
            tokio::pin!(relay_changed);
            relay_changed.as_mut().enable();
            let capacity_notice = capacity
                .as_ref()
                .map(|changes| changes.clone().notified_owned());
            let mut capacity_notice = capacity_notice.map(Box::pin);
            if let Some(notice) = &mut capacity_notice {
                notice.as_mut().enable();
            }
            if let Some(status) = &operator_status {
                let required = self.client.discovery_requires_creation().await?;
                status.send_if_modified(|old| {
                    let changed = *old != required;
                    *old = required;
                    changed
                });
            }
            let result = self
                .fetch_with_capacity(
                    url,
                    limit,
                    setting,
                    gate.iter().cloned().collect(),
                    retry_after,
                )
                .await;
            match result {
                Err(error)
                    if RetryDisposition::from_error(&error) == RetryDisposition::DependencyWait =>
                {
                    #[cfg(test)]
                    if let Some(signal) = &self.state.lock().await.dependency_wait {
                        signal.notify_one();
                    }
                    let notice = async {
                        match capacity_notice {
                            Some(notice) => notice.await,
                            None => std::future::pending().await,
                        }
                    };
                    tokio::select! { _ = notice => {}, _ = relay_changed => {} }
                }
                outcome => return outcome,
            }
        }
    }
    pub async fn fetch(
        &self,
        url: &str,
        limit: usize,
        setting: &'static str,
    ) -> Result<Arc<Response>> {
        self.fetch_with_capacity(url, limit, setting, Vec::new(), None)
            .await
    }
    pub(crate) async fn fetch_with_capacity(
        &self,
        url: &str,
        limit: usize,
        setting: &'static str,
        capacity: Vec<Arc<tokio::sync::Semaphore>>,
        retry_after: Option<crate::network::RetryAfter>,
    ) -> Result<Arc<Response>> {
        let target = adapters::TargetRequest {
            method: reqwest::Method::GET,
            url: url.parse()?,
            headers: Default::default(),
            body: None,
            allowed_origins: Vec::new(),
        };
        let request = StaticFetch {
            prepared: self.sequence.prepare(&target, limit)?,
            target,
            limit,
            setting,
            scope: static_scope(&self.key, "catalog"),
        };
        if let Some(cached) = request
            .prepared
            .cached(
                self.state.clone(),
                &request.scope,
                &request.target,
                limit,
                setting,
            )
            .await
        {
            return cached;
        }
        if !self
            .client
            .discovery_ready()
            .await
            .map_err(|e| RelayFailure::from_error(&e))?
        {
            return Err(RetryDisposition::DependencyWait.into());
        }
        // The origin's handoff delay precedes accepted relay work. Cancellation
        // here cannot abandon a relay authorization or retain serialization.
        crate::network::sleep_retry_after(retry_after).await;
        let relay = self.clone();
        self.client
            .run_owned(async move { relay.fetch_owned(request, capacity).await })
            .await?
    }
    async fn fetch_owned(
        &self,
        request: StaticFetch,
        capacity: Vec<Arc<tokio::sync::Semaphore>>,
    ) -> Result<Arc<Response>> {
        request
            .prepared
            .acquire(
                self.client.clone(),
                &request.target,
                sequence::Contract::Catalog,
                request.limit,
                request.setting,
                None,
                crate::limits::HELP_BYTES,
                None,
                &capacity,
                Some((self.state.clone(), &request.scope)),
                None,
            )
            .await
    }
}
struct StaticFetch {
    prepared: sequence::Prepared,
    target: adapters::TargetRequest,
    scope: String,
    limit: usize,
    setting: &'static str,
}

fn static_scope(wallet: &str, kind: &str) -> String {
    serde_json::to_string(&(wallet, kind)).expect("serializable static request scope")
}
pub(crate) fn slots(
    cfg: &Config,
    url: &str,
    kind: &str,
    limit: usize,
) -> Vec<crate::http_cache::Slot> {
    let Some(relay) = &cfg.relay_fallback else {
        return Vec::new();
    };
    relay
        .sequence
        .cache_keys(&static_scope(&relay.key, kind))
        .iter()
        .filter_map(|key| slot_for(cfg, url, kind, limit, key))
        .collect()
}
pub(crate) fn slot_for(
    cfg: &Config,
    url: &str,
    kind: &str,
    limit: usize,
    key: &str,
) -> Option<crate::http_cache::Slot> {
    crate::http_cache::Slot::new(cfg, url, kind, limit).map(|slot| slot.relay(key))
}
#[cfg(test)]
pub(crate) fn slot(
    cfg: &Config,
    url: &str,
    kind: &str,
    limit: usize,
) -> Option<crate::http_cache::Slot> {
    slots(cfg, url, kind, limit).into_iter().next()
}

#[cfg(test)]
fn decode(
    bytes: &[u8],
    url: &str,
    limit: usize,
    setting: &'static str,
    delay: std::time::Duration,
) -> Result<Response> {
    let target = adapters::TargetRequest {
        method: reqwest::Method::GET,
        url: url.parse()?,
        headers: Default::default(),
        body: None,
        allowed_origins: Vec::new(),
    };
    let decoded = adapters::Format::Curl.decode(bytes, &target, limit, setting)?;
    // Legacy decoder fixture compatibility; runtime callers always use the common traversal.
    ensure!(
        matches!(
            sequence::Contract::Catalog.decision(&decoded),
            sequence::Decision::Accept
        ),
        "relay response rejected"
    );
    Ok(Response {
        status: decoded.status,
        headers: decoded.headers,
        body: decoded.body,
        final_url: decoded.final_url,
        delay,
        received: Instant::now(),
        cache_key: "fixture".into(),
        via: "curl".into(),
        payment_may_have_occurred: false,
    })
}
pub(crate) fn log_fallback(cfg: &Config, stage: &str, reason: &str, http_status: Option<u16>) {
    tracing::info!(
        source = cfg
            .discovery_source
            .as_deref()
            .or(cfg.prefix.as_deref())
            .unwrap_or("standalone"),
        stage,
        reason,
        http_status,
        connect_timeout_seconds = crate::network::global().policy.connect_timeout().as_secs(),
        read_timeout_seconds = crate::network::global()
            .read_timeout(
                cfg.read_timeout_seconds
                    .map(std::time::Duration::from_secs_f64)
            )
            .as_secs_f64(),
        "Catalog could not be loaded from its source; configured paid relay selected as fallback"
    );
}
pub(crate) fn log_failure(cfg: &Config, stage: &str, error: &anyhow::Error) {
    let failure = RelayFailure::from_error(error);
    let limit = error.downcast_ref::<crate::limits::DownloadLimit>();
    let possible_submission = error.is::<document::PossiblePayment>()
        || error
            .downcast_ref::<TargetFailure>()
            .is_some_and(|failure| failure.possible);
    tracing::warn!(
        source = cfg
            .discovery_source
            .as_deref()
            .or(cfg.prefix.as_deref())
            .unwrap_or("standalone"),
        stage,
        reason = failure.kind(),
        cause = failure.explanation(),
        http_status = failure.status(),
        possible_submission,
        resource = limit.map(|limit| limit.kind),
        setting = limit.map(|limit| limit.setting),
        limit_bytes = limit.map(|limit| limit.limit),
        "{}",
        if possible_submission {
            "Configured paid relay fallback failed; the catalog is unavailable; a payment authorization may still charge; do not automatically repeat"
        } else {
            "Configured paid relay fallback failed; the catalog is unavailable; correct the cause and restart to retry"
        }
    );
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
    pub(super) const TARGET: &str = "https://example.com/openapi.json";
    pub(super) fn envelope(url: &str) -> Value {
        json!({"url":url,"method":"GET","statusCode":200,"headers":{"cache-control":"max-age=600"},"body":"{\"openapi\":\"3.0.0\",\"paths\":{}}","redirectChain":[],"error":null})
    }
    #[derive(Clone)]
    pub(crate) struct Fixture {
        pub(crate) calls: Arc<AtomicUsize>,
        pub(crate) signed: Arc<AtomicUsize>,
        payers: Arc<std::sync::Mutex<Vec<String>>>,
        body: Value,
        reject: bool,
        pub(super) arrived: Arc<tokio::sync::Notify>,
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
                return (StatusCode::BAD_GATEWAY, "PRIVATE_UPSTREAM_DETAIL").into_response();
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
                sequence: SearchRelay::fixture(endpoint, Config::default()),
                key: "fixture".into(),
                sources: vec![],
                state: Default::default(),
            }),
            f,
            task,
        )
    }
    pub(crate) fn set_endpoint(relay: &mut Arc<Relay>, endpoint: String) {
        Arc::get_mut(relay)
            .expect("unshared fixture")
            .sequence
            .entries
            .get_mut("GET")
            .unwrap()[0]
            .endpoint = endpoint;
    }
    pub(crate) fn provider_key(relay: &Relay) -> String {
        relay.sequence.entries["GET"][0].provider_key.clone()
    }
    pub(crate) async fn set_stopped(relay: &Relay, stopped: bool) {
        if stopped {
            relay.state.lock().await.stopped.insert(provider_key(relay));
        } else {
            relay.state.lock().await.stopped.clear();
        }
    }
    pub(crate) async fn dependency_wait_signal(relay: &Relay) -> Arc<tokio::sync::Notify> {
        let signal = Arc::new(tokio::sync::Notify::new());
        relay.state.lock().await.dependency_wait = Some(signal.clone());
        signal
    }
    pub(crate) fn set_client(relay: &mut Arc<Relay>, client: PaidClient) {
        Arc::get_mut(relay)
            .expect("fixture relay is unshared")
            .client = client;
    }
    pub(crate) fn is_idle(relay: &Relay) -> bool {
        relay
            .state
            .try_lock()
            .is_ok_and(|state| state.providers.values().all(|gate| gate.try_lock().is_ok()))
    }
    pub(crate) fn set_timeout(relay: &mut Arc<Relay>, timeout: std::time::Duration) {
        let inner = Arc::get_mut(relay).expect("fixture relay is unshared");
        inner.client = inner.client.clone().with_timeout(timeout);
        for entry in inner.sequence.entries.values_mut().flatten() {
            entry.cfg.read_timeout_seconds = Some(timeout.as_secs_f64());
        }
    }
    fn second_wallet(first: &Relay) -> Relay {
        let payer = crate::payment::Payer::new(
            "0000000000000000000000000000000000000000000000000000000000000002",
            crate::payment::SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap();
        Relay {
            client: PaidClient::new(payer),
            sequence: first.sequence.clone(),
            key: "second-wallet".into(),
            sources: vec![],
            state: first.state.clone(),
        }
    }
    #[tokio::test]
    async fn wallet_scopes_do_not_share_paid_results_but_share_failure_breaker() {
        let (first, f, task) = fixture(envelope(TARGET), false, false).await;
        let second = second_wallet(&first);
        first.fetch(TARGET, 4096, "max_spec_bytes").await.unwrap();
        second.fetch(TARGET, 4096, "max_spec_bytes").await.unwrap();
        first.fetch(TARGET, 4096, "max_spec_bytes").await.unwrap();
        assert_eq!(f.signed.load(Ordering::SeqCst), 2);
        let payers = f.payers.lock().unwrap().clone();
        assert_eq!(payers.len(), 2);
        assert_ne!(payers[0], payers[1]);
        task.abort();
        let (first, f, task) = fixture(envelope(TARGET), true, false).await;
        let second = second_wallet(&first);
        let (a, b) = tokio::join!(
            first.fetch(TARGET, 4096, "max_spec_bytes"),
            second.fetch(TARGET, 4096, "max_spec_bytes")
        );
        assert!(a.is_err() && b.is_err());
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        task.abort();
    }
    #[tokio::test]
    async fn built_relays_share_wallet_clients_and_one_global_breaker() {
        let config: crate::deployment::MetaConfig =
            toml::from_str(include_str!("../examples/deployments/comprehensive.toml")).unwrap();
        let policy = config.relay_fallback.as_ref().unwrap();
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
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/deployments/comprehensive.toml");
        let relays = build(policy, &path, &bindings, &clients).await.unwrap();
        assert!(Arc::ptr_eq(&relays["social"], &relays["company_otto"]));
        assert!(Arc::ptr_eq(&relays["exa"], &relays["webinfo"]));
        assert!(!Arc::ptr_eq(&relays["social"], &relays["webinfo"]));
        assert!(Arc::ptr_eq(
            &relays["social"].state,
            &relays["webinfo"].state
        ));
        assert_ne!(relays["social"].key, relays["webinfo"].key);
    }
    #[test]
    fn absent_get_list_does_not_select_static_discovery_wallets() {
        let config: crate::deployment::MetaConfig =
            toml::from_str(include_str!("../examples/deployments/comprehensive.toml")).unwrap();
        let mut policy = config.relay_fallback.clone().unwrap();
        policy.relays.remove("GET");
        policy
            .relays
            .insert("POST".into(), vec!["unused.toml".into()]);
        assert!(
            resolve_wallets(
                &policy,
                &config,
                &crate::rotation::assignment::resolve(&config).unwrap(),
                config.sources.keys().cloned()
            )
            .unwrap()
            .is_empty()
        );
    }
    #[test]
    fn default_wallet_selection_is_stable_scoped_and_honors_overrides() {
        let mut config: crate::deployment::MetaConfig =
            toml::from_str(include_str!("../examples/deployments/comprehensive.toml")).unwrap();
        let policy = config.relay_fallback.clone().unwrap();
        assert!(
            policy.wallet.is_none()
                && policy.provider_metadata
                && policy.source_discovery
                && !policy.api_requests
        );
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
        assert_eq!(first["exa"], "web");
        // The authored example deliberately shares social/company identities,
        // while the web listener uses a separate payer and Tor scope.
        assert_eq!(first["social"], "company");
        assert_eq!(first["webinfo"], "web");
        let assignment = crate::rotation::assignment::resolve(&config).unwrap();
        for (source, wallet) in &first {
            assert!(assignment.wallets.contains_key(wallet));
            assert!(assignment.bindings.values().any(|bindings| {
                bindings
                    .get(source)
                    .is_some_and(|binding| &binding.wallet == wallet)
            }));
        }
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
        config.sources.get_mut("exa").unwrap().wallet = Some("missing".into());
        assert!(crate::rotation::assignment::resolve(&config).is_err());
        config.sources.get_mut("exa").unwrap().wallet = Some("web".into());
        assert_eq!(resolve(&config, &policy)["exa"], "web");
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
    async fn pre_submission_failure_leaves_other_targets_available() {
        let (mut relay, _, task) = fixture(envelope(TARGET), false, false).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        set_endpoint(
            &mut relay,
            format!("http://{}/", listener.local_addr().unwrap()),
        );
        drop(listener);
        assert!(relay.fetch(TARGET, 4096, "max_spec_bytes").await.is_err());
        let other_url = "https://other.example.com/spec";
        let (mut other, f, other_task) = fixture(envelope(other_url), false, false).await;
        Arc::get_mut(&mut other).unwrap().state = relay.state.clone();
        assert!(other.fetch(other_url, 4096, "max_spec_bytes").await.is_ok());
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        task.abort();
        other_task.abort();
    }

    #[test]
    fn relay_diagnostics_discard_untrusted_causes() {
        let error = anyhow::anyhow!("PRIVATE_UPSTREAM_DETAIL");
        assert_eq!(
            RelayFailure::from_error(&error).to_string(),
            "discovery relay request failed: reason=payment_or_response"
        );
        let error = anyhow::Error::new(crate::rotation::error::AdmissionError::PriceLimit(
            "PRIVATE_UPSTREAM_DETAIL",
        ));
        assert_eq!(
            RelayFailure::from_error(&error).to_string(),
            "discovery relay request failed: reason=price_limit"
        );
    }

    #[test]
    fn terminal_relay_warning_retains_target_cause_and_prior_payment_uncertainty() {
        for prior_payment in [false, true] {
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(log.reopen().unwrap())
                .finish();
            let mut failure = TargetFailure::new("http_status", Some(503), false);
            failure.possible = !prior_payment;
            let error = if prior_payment {
                failure.error(true)
            } else {
                anyhow::Error::new(failure)
            };
            tracing::subscriber::with_default(subscriber, || {
                log_failure(&Config::default(), "catalog", &error);
            });
            let logs = std::fs::read_to_string(log.path()).unwrap();
            for expected in [
                "WARN",
                "unsuccessful HTTP status",
                "http_status=503",
                "possible_submission=true",
                "catalog is unavailable",
                "payment authorization may still charge; do not automatically repeat",
            ] {
                assert!(logs.contains(expected), "missing {expected}: {logs}");
            }
            assert!(!logs.contains("restart to retry"), "{logs}");
        }
    }

    #[test]
    fn fallback_selection_is_info_and_does_not_claim_a_paid_request_started() {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let cfg = Config {
            spec: "https://private-token@example.invalid/private-path".into(),
            discovery_source: Some("fixture".into()),
            ..Default::default()
        };
        tracing::subscriber::with_default(subscriber, || {
            log_fallback(&cfg, "catalog", "read_inactivity", None);
        });
        let logs = std::fs::read_to_string(log.path()).unwrap();
        for expected in [
            "INFO",
            "fixture",
            "catalog",
            "read_inactivity",
            "Catalog could not be loaded from its source",
            "configured paid relay selected as fallback",
        ] {
            assert!(logs.contains(expected), "missing {expected}: {logs}");
        }
        for excluded in [
            "WARN",
            "trying",
            "private-token",
            "example.invalid",
            "private-path",
        ] {
            assert!(!logs.contains(excluded), "{logs}");
        }
    }

    #[tokio::test]
    async fn relay_errors_preserve_safe_causes_without_authorizing_paid_retries() {
        for (reason, reject, stall, limit) in [
            ("http_status", true, false, 4096),
            ("read_inactivity", false, true, 4096),
            ("download_limit", false, false, 16),
        ] {
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(log.reopen().unwrap())
                .finish();
            let _guard = tracing::subscriber::set_default(subscriber);
            let (mut relay, f, task) = fixture(envelope(TARGET), reject, stall).await;
            let inner = Arc::get_mut(&mut relay).unwrap();
            inner.client = inner
                .client
                .clone()
                .with_timeout(std::time::Duration::from_millis(200))
                .with_download_limits(limit, 4096);
            for entry in inner.sequence.entries.values_mut().flatten() {
                entry.cfg.max_response_bytes = limit;
                entry.cfg.read_timeout_seconds = Some(0.2);
            }
            let error = relay
                .fetch(TARGET, 4096, "max_spec_bytes")
                .await
                .err()
                .unwrap();
            let failure = error.downcast_ref::<TargetFailure>().unwrap();
            assert_eq!(failure.reason, reason);
            assert_eq!(failure.status, reject.then_some(502));
            log_failure(&Config::default(), "catalog", &error);
            assert!(!format!("{error:#}").contains("PRIVATE_UPSTREAM_DETAIL"));
            assert!(
                relay
                    .fetch("https://example.com/other", 4096, "max_spec_bytes")
                    .await
                    .is_err()
            );
            assert_eq!(f.signed.load(Ordering::SeqCst), 1);
            assert_eq!(f.calls.load(Ordering::SeqCst), 2);
            let logs = std::fs::read_to_string(log.path()).unwrap();
            assert!(logs.contains(&format!("reason=\"{reason}\"")), "{logs}");
            assert!(logs.contains("possible_submission=true"), "{logs}");
            assert!(!logs.contains("PRIVATE_UPSTREAM_DETAIL"));
            assert!(!logs.contains(TARGET));
            task.abort();
        }
    }

    #[tokio::test]
    async fn paid_failure_stops_all_queued_targets_and_never_replays() {
        let (relay, f, task) = fixture(envelope(TARGET), true, false).await;
        let results = futures_util::future::join_all((0..16).map(|i| {
            let relay = relay.clone();
            async move {
                relay
                    .fetch(&format!("https://example.com/{i}"), 4096, "max_spec_bytes")
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
            relay.fetch(TARGET, 4096, "max_spec_bytes"),
            relay.fetch(TARGET, 4096, "max_spec_bytes")
        );
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        task.abort();
        let mut invalid = envelope(TARGET);
        invalid["body"] = json!("broken JSON");
        let (relay, f, task) = fixture(invalid, false, false).await;
        assert!(relay.fetch(TARGET, 4096, "max_spec_bytes").await.is_err());
        assert!(relay.fetch(TARGET, 4096, "max_spec_bytes").await.is_err());
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
                    (0..4).map(|_| relay.fetch(TARGET, 4096, "max_spec_bytes")),
                )
                .await;
                assert!(results.iter().all(Result::is_err));
                assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
                let other_url = "https://other.example.com/spec";
                // Two local handlers model Curl's different responses per target;
                // both relay clients use the same wallet key and shared failure state.
                let (mut other, other_fixture, other_task) =
                    self::fixture(envelope(other_url), false, false).await;
                Arc::get_mut(&mut other).unwrap().state = relay.state.clone();
                let result = other.fetch(other_url, 4096, "max_spec_bytes").await;
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
    mod ownership_tests;

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
                4096,
                "max_response_bytes",
                Default::default()
            )
            .is_err()
        );
    }
    #[test]
    fn sharing_a_response_does_not_renew_origin_freshness() {
        let mut response = decode(
            &serde_json::to_vec(&envelope(TARGET)).unwrap(),
            TARGET,
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
        for status in [
            400, 403, 408, 429, 500, 502, 503, 504, 501, 404, 200, 302, 307,
        ] {
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(log.reopen().unwrap())
                .finish();
            let _guard = tracing::subscriber::set_default(subscriber);
            let origin_calls = Arc::new(AtomicUsize::new(0));
            let followed = Arc::new(AtomicUsize::new(0));
            let count = origin_calls.clone();
            let followed_route = followed.clone();
            let app = Router::new()
                .route(
                    "/spec",
                    axum::routing::get(move || {
                        let count = count.clone();
                        async move {
                            count.fetch_add(1, Ordering::SeqCst);
                            // 200 deliberately returns invalid catalog text.
                            (
                                StatusCode::from_u16(status).unwrap(),
                                [("location", "/must-not-follow")],
                                "invalid catalog",
                            )
                        }
                    }),
                )
                .route(
                    "/must-not-follow",
                    axum::routing::get(move || {
                        followed_route.fetch_add(1, Ordering::SeqCst);
                        async { "{\"openapi\":\"3.0.0\",\"paths\":{}}" }
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
            let (relay, f, task) = fixture(envelope(&url), false, false).await;
            let dir = tempfile::tempdir().unwrap();
            let mut cfg = Config {
                spec: url.clone(),
                discovery_source: Some("test_provider".into()),
                relay_fallback: Some(relay),
                http_cache_directory: Some(dir.path().join("http-cache")),
                ..Default::default()
            };
            let expected_attempts =
                if matches!(status, 400 | 403 | 408 | 429 | 500 | 502 | 503 | 504) {
                    3
                } else {
                    1
                };
            let identity = crate::network::IsolationId::discovery(&url).unwrap();
            let selected = context
                .http_participating(&identity, &url, None, false, cfg.transport())
                .unwrap();
            let pending = match crate::catalog::prepare_json_discovery_with(&cfg, &context)
                .await
                .unwrap()
            {
                crate::catalog::CatalogLoad::RelayPending(pending) => pending,
                crate::catalog::CatalogLoad::Ready(_) => panic!("blocked origin must defer relay"),
            };
            assert_eq!(origin_calls.load(Ordering::SeqCst), expected_attempts);
            assert_eq!(followed.load(Ordering::SeqCst), 0);
            assert!(
                Arc::ptr_eq(
                    &selected,
                    &context
                        .http_participating(&identity, &url, None, false, cfg.transport())
                        .unwrap()
                ),
                "invalid origin bodies never promote, including redirects"
            );
            assert_eq!(f.calls.load(Ordering::SeqCst), 0);
            {
                let records = _socks.records.lock().unwrap();
                assert_eq!(records.len(), expected_attempts);
                let passwords: std::collections::BTreeSet<_> =
                    records.iter().map(|r| r.password.clone()).collect();
                assert_eq!(passwords.len(), expected_attempts);
            }
            // The continuation retains its original target/cache/relay after the
            // caller changes its configuration. Concurrent aliases share paid work.
            let original = cfg.clone();
            cfg.spec = "https://other.example.com/spec".into();
            cfg.relay_fallback = None;
            let (a, b) = tokio::join!(pending.resume(), pending.resume());
            assert_eq!(a.unwrap(), b.unwrap());
            cfg = original;
            assert!(matches!(
                crate::catalog::prepare_json_discovery_with(&cfg, &context)
                    .await
                    .unwrap(),
                crate::catalog::CatalogLoad::Ready(_)
            ));
            assert_eq!(origin_calls.load(Ordering::SeqCst), expected_attempts);
            assert_eq!(f.signed.load(Ordering::SeqCst), 1);
            assert!(
                Arc::ptr_eq(
                    &selected,
                    &context
                        .http_participating(&identity, &url, None, false, cfg.transport())
                        .unwrap()
                ),
                "successful paid relay never promotes the origin transport"
            );
            assert!(
                crate::http_cache::Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes)
                    .unwrap()
                    .read()
                    .await
                    .is_none()
            );
            let without = Config {
                relay_fallback: None,
                ..cfg
            };
            assert!(
                crate::catalog::load_json_discovery_with(&without, &context)
                    .await
                    .is_err()
            );
            assert_eq!(origin_calls.load(Ordering::SeqCst), expected_attempts * 2);
            let logs = std::fs::read_to_string(log.path()).unwrap();
            assert!(logs.contains("source=\"test_provider\""), "{logs}");
            assert!(logs.contains("stage=\"catalog\""), "{logs}");
            if !matches!(status, 200 | 302 | 307) {
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

#[cfg(test)]
#[path = "discovery_relay/readiness_tests.rs"]
pub(crate) mod readiness_tests;

#[cfg(test)]
mod list_tests;
