//! Static calls snapshot the payer before HTTP; managed calls lease a signer after
//! challenge validation and durable admission. Paid failures are never replayed.
use crate::catalog::{RoutedRequest, arg_text};
use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use x402_chain_eip155::{V1Eip155ExactClient, V2Eip155ExactClient, V2Eip155UptoClient};
use x402_reqwest::X402Client;
use x402_types::scheme::client::{PaymentCandidate, PaymentSelector};

/// Typed HTTP status for callers that must discard untrusted response prose.
#[derive(Debug)]
pub(crate) struct HttpFailure {
    pub status: reqwest::StatusCode,
    detail: String,
}
impl std::fmt::Display for HttpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.detail)
    }
}
impl std::error::Error for HttpFailure {}

#[cfg(test)]
mod handoff_tests;
#[cfg(test)]
mod hint_tests;

pub const USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
#[cfg(test)]
mod hint_binding_tests {
    use super::*;
    use crate::rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{Store, StoreHandle},
    };

    #[tokio::test]
    async fn trusted_binding_distinguishes_sources_revisions_groups_and_store_lifetimes() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let identity = state.id().to_owned();
        let first = crate::rotation::test_capacity::declare_and_allocate!(
            state,
            "first",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(crate::rotation::config::positive_usdc("5").unwrap()),
            0
        )
        .unwrap();
        let second = crate::rotation::test_capacity::declare_and_allocate!(
            state,
            "second",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(crate::rotation::config::positive_usdc("5").unwrap()),
            0
        )
        .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let client = |group: String| {
            PaidClient::managed(Arc::new(
                ManagedPool::new(
                    store.clone(),
                    group,
                    BaseRpc::new("http://127.0.0.1:1", 12, 120).unwrap(),
                    SpendPolicy::dollars("5").unwrap(),
                )
                .unwrap(),
            ))
            .with_source_identity("source-a")
        };
        let a = client(first.clone());
        let same = client(first.clone());
        let other_group = client(second);
        let key = |client: &PaidClient, source| {
            client
                .operation_key(source, "GET", "/items/{id}", "https://example.com")
                .unwrap()
        };
        let original = key(&a, None);
        assert_eq!(original, key(&same, None));
        assert_ne!(original, key(&other_group, None));
        assert_ne!(
            original,
            key(&a.clone().with_source_identity("source-b"), None)
        );
        assert_eq!(
            key(&a, Some(("dynamic", 1))),
            key(&same, Some(("dynamic", 1)))
        );
        assert_ne!(key(&a, Some(("dynamic", 1))), key(&a, Some(("dynamic", 2))));
        assert_ne!(original, key(&a.clone().public_destinations(), None));
        assert_ne!(
            original,
            a.operation_key(None, "GET", "/items/{id}", "https://other.example.com")
                .unwrap()
        );
        assert!(
            PaidClient::unsigned()
                .operation_key(None, "GET", "/items/{id}", "https://example.com")
                .is_none()
        );
        drop(a);
        drop(same);
        drop(other_group);
        drop(store);
        worker.await.unwrap();
        // Reopening the exact same group must not acquire the previous runtime's hints.
        let independent = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &identity,
        )
        .unwrap();
        let group = first;
        let (independent, independent_worker) = StoreHandle::spawn(independent);
        let separate = PaidClient::managed(Arc::new(
            ManagedPool::new(
                independent.clone(),
                group,
                BaseRpc::new("http://127.0.0.1:1", 12, 120).unwrap(),
                SpendPolicy::dollars("5").unwrap(),
            )
            .unwrap(),
        ))
        .with_source_identity("source-a");
        assert_ne!(original, key(&separate, None));
        drop(separate);
        drop(independent);
        independent_worker.await.unwrap();
    }
}
#[derive(Clone)]
pub struct SpendPolicy {
    pub max_atomic: Option<U256>,
}
impl SpendPolicy {
    pub fn dollars(text: &str) -> Result<Self> {
        if matches!(text.to_ascii_lowercase().as_str(), "none" | "off" | "") {
            return Ok(Self { max_atomic: None });
        }
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        ensure!(
            !whole.is_empty()
                && whole.bytes().all(|c| c.is_ascii_digit())
                && fraction.bytes().all(|c| c.is_ascii_digit())
                && fraction.len() <= 6,
            "price must be a nonnegative decimal with at most six decimal places"
        );
        let digits = format!("{whole}{fraction:0<6}");
        Ok(Self {
            max_atomic: Some(U256::from_str_radix(&digits, 10)?),
        })
    }
}
impl PaymentSelector for SpendPolicy {
    fn select<'a>(&self, candidates: &'a [PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        candidates.iter().find(|c| {
            c.chain_id.to_string() == "eip155:8453"
                && c.asset.eq_ignore_ascii_case(USDC)
                && self.max_atomic.is_none_or(|max| c.amount <= max)
        })
    }
}
pub struct Payer {
    client: X402Client<SpendPolicy>,
    pub address: String,
}
impl Payer {
    pub fn new(key: &str, policy: SpendPolicy) -> Result<Self> {
        let signer: PrivateKeySigner = key
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid EVM private key"))?;
        let address = signer.address().to_string();
        let signer = Arc::new(signer);
        let client = X402Client::new()
            .register(V1Eip155ExactClient::new(signer.clone()))
            .register(V2Eip155ExactClient::new(signer.clone()))
            .register(V2Eip155UptoClient::new(signer))
            .with_selector(policy);
        Ok(Self { client, address })
    }
}
/// Monotonic evidence set before durable signed intent or a static paid send.
/// It is deliberately independent of response decoding and error text.
#[derive(Clone, Default)]
pub(crate) struct SubmissionProgress(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl SubmissionProgress {
    pub(crate) fn mark_possible(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
    pub(crate) fn possible(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

pub(crate) mod ownership;
#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod saturation_tests;

#[derive(Clone)]
pub struct PaidClient {
    work: crate::server::work::Work,
    source_identity: Arc<str>,
    cover: Option<(Arc<crate::cover::Config>, crate::cover::Scope)>,
    unsigned_only: bool,
    transport: crate::network::HttpPolicy,
    public_only: bool,
    timeout: Option<std::time::Duration>,
    max_response_bytes: usize,
    max_help_bytes: usize,
    payer: Option<Arc<RwLock<Arc<Payer>>>>,
    managed: Option<Arc<crate::rotation::manager::ManagedPool>>,
}
impl PaidClient {
    pub fn new(payer: Payer) -> Self {
        Self {
            work: Default::default(),
            source_identity: Arc::from("standalone"),
            cover: None,
            unsigned_only: false,
            transport: Default::default(),
            public_only: false,
            max_response_bytes: crate::limits::RESPONSE_BYTES,
            max_help_bytes: crate::limits::HELP_BYTES,
            timeout: None,
            payer: Some(Arc::new(RwLock::new(Arc::new(payer)))),
            managed: None,
        }
    }
    pub fn managed(pool: Arc<crate::rotation::manager::ManagedPool>) -> Self {
        Self {
            work: Default::default(),
            source_identity: Arc::from("standalone"),
            cover: None,
            unsigned_only: false,
            transport: Default::default(),
            public_only: false,
            max_response_bytes: crate::limits::RESPONSE_BYTES,
            max_help_bytes: crate::limits::HELP_BYTES,
            timeout: None,
            payer: None,
            managed: Some(pool),
        }
    }
    /// Keyless qualification client. Challenges are never parsed for signing or retried.
    pub fn unsigned() -> Self {
        Self {
            work: Default::default(),
            source_identity: Arc::from("standalone"),
            cover: None,
            unsigned_only: true,
            transport: Default::default(),
            public_only: false,
            max_response_bytes: crate::limits::RESPONSE_BYTES,
            max_help_bytes: crate::limits::HELP_BYTES,
            timeout: None,
            payer: None,
            managed: None,
        }
    }
    pub fn with_cover(
        mut self,
        config: Option<crate::cover::Config>,
        scope: crate::cover::Scope,
    ) -> Self {
        if let (Some(config), Some(_)) = (config, &crate::network::global().cover) {
            self.cover = Some((Arc::new(config), scope));
        }
        self
    }
    /// Trusted runtime identity, separate from reusable provider tool metadata.
    pub(crate) fn with_source_identity(mut self, source: &str) -> Self {
        self.source_identity = Arc::from(source);
        self
    }
    pub(crate) fn operation_key(
        &self,
        source: Option<(&str, u64)>,
        method: &str,
        template: &str,
        base: &str,
    ) -> Option<crate::pricing::runtime::OperationKey> {
        use sha2::{Digest, Sha256};
        let (namespace, group) = self.managed.as_ref()?.hint_group_scope();
        let (source, revision) = source.unwrap_or((&self.source_identity, 0));
        // The fixed digest separates store lifetimes, configured network policy,
        // destination binding and provider transport choices. No URL or network
        // credential is retained in the cache, and arguments never enter this key.
        let mut transport = Sha256::new();
        let policy = serde_json::to_vec(&crate::network::global().policy).ok()?;
        for field in [
            namespace.as_bytes().as_slice(),
            policy.as_slice(),
            base.as_bytes(),
        ] {
            transport.update((field.len() as u64).to_be_bytes());
            transport.update(field);
        }
        transport.update([
            u8::from(self.transport.allow_http1),
            u8::from(self.transport.allow_tls12),
            u8::from(self.public_only),
        ]);
        Some(crate::pricing::runtime::OperationKey::new(
            source,
            revision,
            method,
            template,
            group,
            &transport.finalize(),
        ))
    }
    pub fn with_transport(mut self, transport: crate::network::HttpPolicy) -> Self {
        self.transport = transport;
        self
    }
    pub fn transport(&self) -> crate::network::HttpPolicy {
        self.transport
    }
    /// Override HTTP read inactivity. None inherits the network setting.
    pub fn with_timeout(mut self, timeout: impl Into<Option<std::time::Duration>>) -> Self {
        self.timeout = timeout.into();
        self
    }
    pub fn with_download_limits(mut self, response: usize, help: usize) -> Self {
        self.max_response_bytes = response;
        self.max_help_bytes = help;
        self
    }
    pub fn max_help_bytes(&self) -> usize {
        self.max_help_bytes
    }
    pub fn public_destinations(mut self) -> Self {
        self.public_only = true;
        self
    }
    #[cfg(test)]
    pub(crate) fn shares_profile_with(&self, other: &Self) -> bool {
        match (&self.payer, &other.payer, &self.managed, &other.managed) {
            (Some(a), Some(b), _, _) => Arc::ptr_eq(a, b),
            (_, _, Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn timeout(&self) -> Option<std::time::Duration> {
        self.timeout
    }
    pub fn replace_payer(&self, payer: Payer) {
        *self
            .payer
            .as_ref()
            .expect("static payer replacement only")
            .write()
            .expect("payer lock poisoned") = Arc::new(payer);
    }
    pub async fn execute(&self, route: RoutedRequest) -> Result<String> {
        self.execute_response(route)
            .await?
            .render(None, &Default::default())?
            .into_text()
    }
    pub async fn execute_response(
        &self,
        route: RoutedRequest,
    ) -> Result<crate::output::HttpOutput> {
        self.execute_response_observed(route, &SubmissionProgress::default())
            .await
    }
    pub(crate) async fn execute_response_observed(
        &self,
        route: RoutedRequest,
        progress: &SubmissionProgress,
    ) -> Result<crate::output::HttpOutput> {
        self.execute_response_scoped(route, progress, None).await
    }
    /// Stop accepting calls and wait for already accepted response/output work.
    /// Clones share the same lifecycle; cancellation of a waiter does not stop it.
    pub async fn drain(&self) {
        self.work.drain().await;
    }
    /// Retain an encompassing payment workflow through its outcome publication.
    /// Nested payment execution reuses this owner and its foreground permit.
    pub(crate) async fn run_owned<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = T> + Send + 'static,
    ) -> Result<T> {
        ownership::run(&self.work, work).await
    }
    pub(crate) async fn execute_response_scoped(
        &self,
        route: RoutedRequest,
        progress: &SubmissionProgress,
        operation: Option<crate::pricing::runtime::OperationKey>,
    ) -> Result<crate::output::HttpOutput> {
        let client = self.clone();
        let progress = progress.clone();
        self.run_owned(async move {
            client
                .execute_response_owned(route, &progress, operation)
                .await
        })
        .await?
    }
    async fn execute_response_owned(
        &self,
        route: RoutedRequest,
        progress: &SubmissionProgress,
        operation: Option<crate::pricing::runtime::OperationKey>,
    ) -> Result<crate::output::HttpOutput> {
        // Pin static signer before any I/O; replacement affects only later calls.
        let payer = self
            .payer
            .as_ref()
            .map(|p| p.read().expect("payer lock poisoned").clone());
        let request = build_request(route)?;
        let unsigned_only = self.unsigned_only || crate::qualification::unsigned_case();
        let mut attempt = 0;
        let mut recovery = crate::rotation::manager::RecoveryAllowance::default();
        let mut paid_submission = false;
        let mut extensions_omitted = false;
        // Only a pre-signing managed identity change can restart this loop.
        // The new unsigned challenge must use the new identity-bound transport.
        let mut candidate = match &self.managed {
            Some(pool) if !unsigned_only => Some(
                pool.candidate_for(
                    operation.and_then(|key| crate::pricing::runtime::process_hints().get(key)),
                )
                .await?,
            ),
            _ => None,
        };
        let (response, mut cover) = loop {
            let identity = if unsigned_only {
                crate::network::IsolationId::discovery(request.url().as_str())?
            } else {
                let address = candidate
                    .as_ref()
                    .map(|c| c.address.as_str())
                    .or_else(|| payer.as_ref().map(|p| p.address.as_str()))
                    .context("network_identity_missing: payer")?;
                crate::network::IsolationId::evm(address)?
            };
            let factory = crate::network::global();
            let http = factory.http_policy(
                &identity,
                request.url().as_str(),
                self.timeout,
                self.public_only,
                self.transport,
            )?;
            let mut cover = self.cover.as_ref().and_then(|(config, scope)| {
                let engine = factory.cover.as_ref()?;
                let owner = crate::cover::registry::Owner {
                    runtime: tokio::runtime::Handle::current().id(),
                    identity: identity.clone(),
                    origin: request.url().origin().ascii_serialization(),
                    transport: self.transport,
                    public_only: self.public_only,
                    timeout_ms: factory.read_timeout(self.timeout).as_millis() as u64,
                };
                match engine.begin(owner, config.clone(), scope.clone(), http.clone()) {
                    Ok(call) => Some(call),
                    Err(error) => {
                        tracing::warn!(listener = %scope.listener, source = %scope.source, "optional cover unavailable: {error}");
                        None
                    }
                }
            });
            if let Some(call) = &cover {
                call.prioritize().await;
            }
            let mut unsigned = request
                .try_clone()
                .context("request body cannot be retried")?;
            let retry = request
                .try_clone()
                .context("request body cannot be retried")?;
            let mut padding = cover.as_ref().and_then(|c| c.pad(&mut unsigned, true));
            if let Some(p) = &mut padding {
                p.dispatched();
            }
            let mut response = http
                .execute(unsigned)
                .await
                .map_err(reqwest::Error::without_url)?;
            let receipt = crate::pricing::runtime::received();
            drop(padding);
            if let Some(c) = &cover {
                c.protocol(response.version());
                c.rejected_padding(response.status());
            }
            crate::network::log_http(&response, "payment_challenge");
            if response.status() == reqwest::StatusCode::PAYMENT_REQUIRED {
                crate::qualification::record_challenge(response.headers()).await?;
                if unsigned_only {
                    tracing::warn!(
                        code = "qualification_payment_denied",
                        "unsigned qualification case refused HTTP 402; no payment signed or retried"
                    );
                    bail!(
                        "qualification_payment_denied: unsigned-only mode; HTTP 402 received; no payment signed or retried"
                    );
                }
                response = self.bound_challenge(response).await?;
                if let Some(pool) = &self.managed {
                    match pool
                        .pay_with_cover_observed(
                            &http,
                            candidate.clone().expect("managed candidate"),
                            retry,
                            response,
                            cover.as_ref(),
                            crate::rotation::manager::AdmissionObservation {
                                progress,
                                recovery: &mut recovery,
                                hint: operation.zip(receipt),
                                fixed_payer: attempt > 0,
                            },
                        )
                        .await
                    {
                        Ok((paid, omitted)) => {
                            extensions_omitted = omitted;
                            paid_submission = true;
                            response = paid;
                        }
                        Err(error)
                            if error.is::<crate::rotation::manager::PayerHandoff>()
                                && attempt == 0
                                && !progress.possible() =>
                        {
                            candidate = Some(
                                error
                                    .downcast_ref::<crate::rotation::manager::PayerHandoff>()
                                    .expect("typed handoff")
                                    .0
                                    .clone(),
                            );
                            attempt += 1;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    sanitize_challenge(&mut response)?;
                    // Selection (asset/network/cap) happens inside the SDK before signing.
                    let headers = payer
                        .as_ref()
                        .expect("static payer")
                        .client
                        .make_payment_headers(response)
                        .await
                        .context("x402 challenge rejected or signing failed")?;
                    let mut retry = retry;
                    retry.headers_mut().extend(headers);
                    let mut padding = cover.as_ref().and_then(|c| c.pad(&mut retry, true));
                    if let Some(p) = &mut padding {
                        p.dispatched();
                    }
                    progress.mark_possible();
                    response = http
                        .execute(retry)
                        .await
                        .map_err(reqwest::Error::without_url)?;
                    paid_submission = true;
                }
            }
            crate::network::log_http(&response, "payment_result");
            if let Some(c) = &cover {
                c.protocol(response.version());
                c.rejected_padding(response.status());
            }
            if let Some(call) = &mut cover {
                call.response_headers();
            }
            break (response, cover);
        };
        let result = self.response_output(response, paid_submission).await;
        if let Some(call) = &mut cover
            && result.is_ok()
        {
            call.complete();
        }
        if extensions_omitted {
            result.context(crate::rotation::manager::OMITTED_EXTENSIONS)
        } else {
            result
        }
    }

    async fn bound_challenge(&self, response: reqwest::Response) -> Result<reqwest::Response> {
        // Bound legacy JSON-body challenges before the SDK can collect them.
        let mut envelope = axum::http::Response::builder().status(response.status());
        *envelope.headers_mut().expect("valid status") = response.headers().clone();
        let bytes = crate::limits::read(
            response,
            self.max_response_bytes,
            "payment challenge body",
            "max_response_bytes",
        )
        .await
        .context("payment challenge rejected before signing")?;
        Ok(envelope.body(bytes)?.into())
    }

    async fn response_output(
        &self,
        response: reqwest::Response,
        paid_submission: bool,
    ) -> Result<crate::output::HttpOutput> {
        let status = response.status();
        let mime_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap().trim().to_ascii_lowercase());
        let detail = response
            .headers()
            .get("payment-required")
            .and_then(|h| STANDARD.decode(h.as_bytes()).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.get("error").cloned());
        let bytes = crate::limits::read(response, self.max_response_bytes, "API response", "max_response_bytes")
            .await.map_err(|error| if paid_submission {
                error.context("API response unavailable; a payment may already have settled. Do not automatically retry a paid request")
            } else { error })?;
        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            return Err(HttpFailure {
                status,
                detail: format!("{} {body}", detail.unwrap_or(Value::Null)),
            }
            .into());
        }
        Ok(crate::output::HttpOutput {
            advisories: vec![],
            bytes,
            mime_type,
            paid_submission,
        })
    }
}

fn build_request(route: RoutedRequest) -> Result<reqwest::Request> {
    let mut request =
        reqwest::Request::new(route.method.parse()?, reqwest::Url::parse(&route.url)?);
    let mut query = Vec::new();
    for (key, value) in route.query {
        if let Some(values) = value.as_array() {
            for v in values {
                query.push((key.clone(), arg_text(v)));
            }
        } else {
            query.push((key, arg_text(&value)));
        }
    }
    if !query.is_empty() {
        request.url_mut().query_pairs_mut().extend_pairs(query);
    }
    if let Some(body) = route.body {
        *request.body_mut() = Some(serde_json::to_vec(&body)?.into());
        request.headers_mut().insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }
    Ok(request)
}

fn sanitize_challenge(response: &mut reqwest::Response) -> Result<()> {
    if let Some(header) = response.headers().get("payment-required") {
        let mut challenge: Value = serde_json::from_slice(&STANDARD.decode(header.as_bytes())?)?;
        if let Some(desc) = challenge.pointer_mut("/resource/description")
            && let Some(text) = desc.as_str()
        {
            if text.chars().count() > 500 {
                tracing::warn!(
                    limit_chars = 500,
                    "x402 challenge description truncated for facilitator protocol compatibility"
                );
            }
            *desc = Value::String(text.chars().take(500).collect());
        }
        response.headers_mut().insert(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge)?).parse()?,
        );
    }
    Ok(())
}
