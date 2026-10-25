//! Static calls snapshot the payer before HTTP; managed calls lease a signer after
//! challenge validation and durable admission. Managed insufficient-funds failures
//! may traverse existing wallets; every authorization remains chargeable.
use crate::catalog::{RoutedRequest, arg_text};
use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use tracing::Instrument;
use x402_chain_eip155::{V2Eip155ExactClient, V2Eip155UptoClient};
use x402_reqwest::X402Client;
use x402_types::scheme::client::{PaymentCandidate, PaymentSelector};
mod document_identity;
mod v1;
pub(crate) use document_identity::Authority;
pub(crate) use v1::V1ExactClient;

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

#[derive(Debug)]
pub(crate) struct IncompleteRelayResponse;
impl std::fmt::Display for IncompleteRelayResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("relay service partial_response")
    }
}
impl std::error::Error for IncompleteRelayResponse {}
/// Operation-scoped refusal, with a fixed safe diagnostic reason.
#[derive(Debug)]
struct PrivatePayerHandoff;
impl std::fmt::Display for PrivatePayerHandoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private headers cannot follow another wallet")
    }
}
impl std::error::Error for PrivatePayerHandoff {}

fn complete_relay_response(response: &reqwest::Response, relay: bool) -> Result<()> {
    ensure!(
        !relay
            || (response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                && !response
                    .headers()
                    .contains_key(reqwest::header::CONTENT_RANGE)),
        IncompleteRelayResponse
    );
    Ok(())
}

#[cfg(test)]
mod best_effort_tests;
#[cfg(test)]
mod handoff_tests;
#[cfg(test)]
mod hint_tests;
#[cfg(test)]
mod tor_cover_tests;
#[cfg(test)]
mod tor_wait_tests;

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
            crate::rotation::test_capacity::two_reserves("5"),
            Some(crate::rotation::config::positive_usdc("5").unwrap()),
            0
        )
        .unwrap();
        let second = crate::rotation::test_capacity::declare_and_allocate!(
            state,
            "second",
            crate::rotation::test_capacity::two_reserves("5"),
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
                    BaseRpc::new("http://127.0.0.1:1", 12).unwrap(),
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
                BaseRpc::new("http://127.0.0.1:1", 12).unwrap(),
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
            .register(V1ExactClient::new(signer.clone()))
            .register(V2Eip155ExactClient::new(signer.clone()))
            .register(V2Eip155UptoClient::new(signer))
            .with_selector(policy);
        Ok(Self { client, address })
    }
}
mod submission;
pub(crate) use submission::{PossiblePayment, SubmissionProgress, with_payment_context};

pub(crate) mod diagnostics;
pub(crate) mod ownership;
#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod relay_tests;
#[cfg(test)]
mod saturation_tests;

#[derive(Clone)]
pub struct PaidClient {
    work: crate::server::work::Work,
    source_identity: Arc<str>,
    cover: Option<(Arc<crate::cover::Config>, crate::cover::Scope)>,
    unsigned_only: bool,
    /// Relay exchanges use common unsigned transport recovery and never recurse.
    relay_call: bool,
    document_relay: Option<Arc<crate::discovery_relay::SearchRelay>>,
    relay_purpose: crate::discovery_relay::Purpose,
    attempt_capacity: Vec<Arc<tokio::sync::Semaphore>>,
    unavailable: Option<crate::rotation::error::AdmissionError>,
    selection: crate::network::SelectionPolicy,
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
            relay_call: false,
            document_relay: None,
            relay_purpose: Default::default(),
            attempt_capacity: Vec::new(),
            unavailable: None,
            selection: crate::network::SelectionPolicy::Participating,
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
            relay_call: false,
            document_relay: None,
            relay_purpose: Default::default(),
            attempt_capacity: Vec::new(),
            unavailable: None,
            selection: crate::network::SelectionPolicy::Participating,
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
            relay_call: false,
            document_relay: None,
            relay_purpose: Default::default(),
            attempt_capacity: Vec::new(),
            unavailable: None,
            selection: crate::network::SelectionPolicy::Participating,
            transport: Default::default(),
            public_only: false,
            max_response_bytes: crate::limits::RESPONSE_BYTES,
            max_help_bytes: crate::limits::HELP_BYTES,
            timeout: None,
            payer: None,
            managed: None,
        }
    }
    /// Preserve the configured tool binding without any transport or signing
    /// capability when its managed connection policy is unavailable.
    pub(crate) fn unavailable(reason: crate::rotation::error::AdmissionError) -> Self {
        let mut client = Self::unsigned();
        client.unsigned_only = false;
        client.unavailable = Some(reason);
        client
    }
    fn require_available(&self) -> Result<()> {
        if let Some(error) = self.unavailable {
            return Err(error.into());
        }
        Ok(())
    }
    pub(crate) async fn discovery_changes(&self) -> Result<Option<Arc<tokio::sync::Notify>>> {
        self.require_available()?;
        match &self.managed {
            Some(pool) => pool.discovery_changes().await.map(Some),
            None => Ok(None),
        }
    }
    pub(crate) async fn discovery_requires_creation(&self) -> Result<bool> {
        self.require_available()?;
        match &self.managed {
            Some(pool) => pool.discovery_requires_creation().await,
            None => Ok(false),
        }
    }
    pub(crate) async fn discovery_ready(&self) -> Result<bool> {
        self.require_available()?;
        match &self.managed {
            Some(pool) => pool.discovery_ready().await,
            None => {
                ensure!(self.payer.is_some(), "discovery relay requires a payer");
                Ok(true)
            }
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
    pub(crate) fn for_relay(mut self) -> Self {
        self.relay_call = true;
        self.document_relay = None;
        self.selection = crate::network::SelectionPolicy::Participating;
        self
    }
    /// Existing acquisition gates apply to active attempts, never retry delays.
    pub(crate) fn with_attempt_capacity(
        mut self,
        capacity: &[Arc<tokio::sync::Semaphore>],
    ) -> Self {
        self.attempt_capacity = capacity.to_vec();
        self
    }
    async fn acquire_attempt_capacity(&self) -> Result<Vec<tokio::sync::OwnedSemaphorePermit>> {
        let mut permits = Vec::new();
        for gate in &self.attempt_capacity {
            permits.push(
                crate::limits::acquire_owned_capacity(
                    gate.clone(),
                    "catalog_relay_attempts_busy",
                    None,
                    "startup.catalog_concurrency",
                )
                .await
                .context("relay acquisition capacity closed")?,
            );
        }
        Ok(permits)
    }
    #[cfg(test)]
    pub(crate) fn with_selection(mut self, selection: crate::network::SelectionPolicy) -> Self {
        self.selection = selection;
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
    /// Pin an existing wallet's isolation identity for unsigned documentation probes.
    /// This selects no spending authority and never prepares or funds a wallet.
    pub(crate) async fn documentation_identity(&self) -> Result<crate::network::IsolationId> {
        self.require_available()?;
        let address = if let Some(pool) = &self.managed {
            pool.documentation_address().await?
        } else {
            self.payer
                .as_ref()
                .context("documentation wallet unavailable")?
                .read()
                .expect("payer lock poisoned")
                .address
                .clone()
        };
        crate::network::IsolationId::evm(&address)
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
        self.execute_finished(
            route,
            Default::default(),
            &SubmissionProgress::default(),
            None,
            |output| {
                let text = output.render(None, &Default::default())?.into_text()?;
                Ok((text, true))
            },
        )
        .await
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
    pub(crate) fn run_owned<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = T> + Send + 'static,
    ) -> impl std::future::Future<Output = Result<T>> + Send + '_ {
        ownership::run(&self.work, work)
    }
    pub(crate) async fn execute_rendered_scoped(
        &self,
        route: RoutedRequest,
        operation: Option<crate::pricing::runtime::OperationKey>,
        mapping: Option<&crate::output::ResponseMapping>,
        limits: &crate::output::ImageLimits,
    ) -> Result<crate::output::ToolOutput> {
        let mapping = mapping.cloned();
        let limits = limits.clone();
        self.execute_finished(
            route,
            Default::default(),
            &SubmissionProgress::default(),
            operation,
            move |output| Ok((output.render(mapping.as_ref(), &limits)?, true)),
        )
        .await
    }
    pub(crate) async fn execute_response_scoped(
        &self,
        route: RoutedRequest,
        progress: &SubmissionProgress,
        operation: Option<crate::pricing::runtime::OperationKey>,
    ) -> Result<crate::output::HttpOutput> {
        self.execute_finished(route, Default::default(), progress, operation, |output| {
            let valid = output.is_valid_unmapped();
            Ok((output, valid))
        })
        .await
    }
    /// Manual raw input has no registered operation or managed-auth reference.
    pub(crate) async fn execute_manual_raw(
        &self,
        route: RoutedRequest,
        headers: crate::discovery::raw::ManualRawHeaders,
    ) -> Result<crate::output::ToolOutput> {
        ensure!(
            route.headers.is_empty(),
            "manual and routed headers cannot be combined"
        );
        self.execute_finished(
            route,
            headers,
            &SubmissionProgress::default(),
            None,
            |output| Ok((output.render(None, &Default::default())?, true)),
        )
        .await
    }

    async fn execute_finished<T: Send + 'static>(
        &self,
        route: RoutedRequest,
        headers: crate::discovery::raw::ManualRawHeaders,
        progress: &SubmissionProgress,
        operation: Option<crate::pricing::runtime::OperationKey>,
        finish: impl FnOnce(crate::output::HttpOutput) -> Result<(T, bool)> + Send + 'static,
    ) -> Result<T> {
        let client = self.clone();
        let progress = progress.clone();
        let span = diagnostics::span(&self.source_identity, &route.method);
        self.run_owned(
            diagnostics::scope(async move {
                let result = client
                    .execute_response_owned(route, headers, &progress, operation)
                    .await
                    .and_then(|(output, unsigned_promotion)| {
                        diagnostics::stage("output_conversion");
                        let (output, valid) = finish(output)?;
                        if valid && let Some((selected, transport)) = unsigned_promotion {
                            crate::network::global().promote_http(&selected, &transport);
                        }
                        Ok(output)
                    });
                diagnostics::finish(&result, &progress, client.relay_call);
                with_payment_context(progress.possible(), result)
            })
            .instrument(span),
        )
        .await?
    }
    async fn execute_response_owned(
        &self,
        route: RoutedRequest,
        headers: crate::discovery::raw::ManualRawHeaders,
        progress: &SubmissionProgress,
        operation: Option<crate::pricing::runtime::OperationKey>,
    ) -> Result<(
        crate::output::HttpOutput,
        Option<(
            Arc<crate::network::HttpTransport>,
            Arc<crate::network::HttpTransport>,
        )>,
    )> {
        self.require_available()?;
        // Pin static signer before any I/O; replacement affects only later calls.
        let payer = self
            .payer
            .as_ref()
            .map(|p| p.read().expect("payer lock poisoned").clone());
        let private = headers.is_private();
        let mut request = build_request(route)?;
        let padding = self.cover.as_ref().and_then(|(config, _)| {
            config
                .padding
                .as_ref()
                .filter(|padding| padding.on_api_requests)
                .map(|padding| padding.header_name.as_str())
        });
        headers.apply(&mut request, padding)?;
        let unsigned_only = self.unsigned_only || crate::qualification::unsigned_case();
        let through = match &self.managed {
            Some(pool) if !unsigned_only => pool.inventory_boundary().await?,
            _ => 0,
        };
        let mut attempted = Vec::new();
        let mut paid_submission = false;
        let mut extensions_omitted = false;
        // Only a pre-signing managed identity change can restart this loop.
        // The new unsigned challenge must use the new identity-bound transport.
        let mut candidate = match &self.managed {
            Some(pool) if !unsigned_only => Some(
                pool.next_candidate(
                    operation
                        .and_then(|key| crate::pricing::runtime::process_hints().get(key))
                        .unwrap_or_default(),
                    through,
                    &[],
                )
                .await?
                .context(
                    crate::rotation::error::AdmissionError::WalletNotReady(
                        "no eligible payment identity",
                    ),
                )?,
            ),
            _ => None,
        };
        let (response, mut cover, unsigned_promotion, _attempt_capacity) = loop {
            let stage = self
                .unsigned_stage(
                    &request,
                    private,
                    unsigned_only,
                    candidate.as_ref(),
                    payer.as_deref(),
                )
                .await;
            let UnsignedStage {
                selected,
                transport,
                mut response,
                retry,
                receipt,
                mut cover,
                mut attempt_capacity,
            } = match stage {
                Ok(stage) => stage,
                Err(failure) => {
                    if private {
                        return Err(failure.error);
                    }
                    return self
                        .relay_after_unsigned_failure(
                            failure,
                            &request,
                            payer.as_ref(),
                            progress,
                            unsigned_only,
                        )
                        .await
                        .map(|output| (output, None));
                }
            };
            let mut stage_paid = false;
            if response.status() == reqwest::StatusCode::PAYMENT_REQUIRED {
                diagnostics::stage("challenge_preparation");
                if unsigned_only {
                    crate::qualification::record_challenge(response.headers()).await?;
                    tracing::warn!(
                        code = "qualification_payment_denied",
                        "unsigned qualification case refused HTTP 402; no payment signed or retried"
                    );
                    bail!(
                        "qualification_payment_denied: unsigned-only mode; HTTP 402 received; no payment signed or retried"
                    );
                }
                let original_headers = response.headers().clone();
                let bounded = bound_challenge(response, self.max_response_bytes).await;
                crate::qualification::record_challenge(
                    bounded
                        .as_ref()
                        .map_or(&original_headers, |response| response.headers()),
                )
                .await?;
                response = bounded?;
                // The unsigned response is complete. The signed attempt gets its
                // own permits; no acquisition slot spans retry/handoff delays.
                attempt_capacity.clear();
                attempt_capacity = self.acquire_attempt_capacity().await?;
                if let Some(pool) = &self.managed {
                    match pool
                        .pay_with_cover_observed(
                            &transport.client,
                            candidate.clone().expect("managed candidate"),
                            retry,
                            response,
                            cover.as_ref(),
                            crate::rotation::manager::AdmissionObservation {
                                progress,
                                hint: operation.zip(receipt),
                                promotion: Some((&selected, &transport)),
                                through,
                                attempted: &attempted,
                            },
                        )
                        .await
                    {
                        Ok((paid, omitted, cost)) => {
                            extensions_omitted |= omitted;
                            paid_submission = true;
                            stage_paid = true;
                            if paid.status() == reqwest::StatusCode::PAYMENT_REQUIRED
                                && insufficient_funds(paid.headers())
                            {
                                let status = paid.status();
                                let version = paid.version();
                                // Structured headers classify this failure. Its
                                // unused body must not delay or veto fallback.
                                drop(paid);
                                attempt_capacity.clear();
                                if let Some(call) = &mut cover {
                                    call.protocol(version);
                                    call.rejected_padding(status);
                                    call.response_headers();
                                    call.complete();
                                }
                                let failed = candidate
                                    .as_ref()
                                    .expect("managed candidate")
                                    .wallet
                                    .clone();
                                pool.note_insufficient(failed.clone()).await?;
                                ensure!(!private, PrivatePayerHandoff);
                                attempted.push(failed);
                                candidate=Some(pool.next_candidate(cost,through,&attempted).await?
                                    .context("insufficient_funds: captured eligible wallets exhausted; earlier authorizations may still charge or execute")?);
                                continue;
                            }
                            response = paid;
                        }
                        Err(error) if error.is::<crate::rotation::manager::PayerHandoff>() => {
                            ensure!(!private, PrivatePayerHandoff);
                            attempted.push(
                                candidate
                                    .as_ref()
                                    .expect("managed candidate")
                                    .wallet
                                    .clone(),
                            );
                            candidate = Some(
                                error
                                    .downcast_ref::<crate::rotation::manager::PayerHandoff>()
                                    .expect("typed handoff")
                                    .0
                                    .clone(),
                            );
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    response = static_payment(
                        payer.as_deref(),
                        &selected,
                        &transport,
                        retry,
                        response,
                        cover.as_ref(),
                        progress,
                    )
                    .await?;
                    paid_submission = true;
                    stage_paid = true;
                }
            }
            crate::network::log_http(&response, "payment_result");
            diagnostics::response(&response);
            if let Some(c) = &cover {
                c.protocol(response.version());
                c.rejected_padding(response.status());
            }
            if let Some(call) = &mut cover {
                call.response_headers();
            }
            if stage_paid {
                complete_relay_response(&response, self.relay_call)?;
            }
            break (
                response,
                cover,
                (!stage_paid).then_some((selected, transport)),
                attempt_capacity,
            );
        };
        diagnostics::stage("response_body");
        let result = self
            .response_output(
                response,
                paid_submission,
                request.method() == reqwest::Method::HEAD,
            )
            .await;
        if let Some(call) = &mut cover
            && result.is_ok()
        {
            call.complete();
        }
        let result = result.map(|output| (output, unsigned_promotion));
        if extensions_omitted {
            result.context(crate::rotation::manager::OMITTED_EXTENSIONS)
        } else {
            result
        }
    }

    /// A failed unsigned phase can hand off without entering target signing.
    /// Captured payer and whole-call liability stay together at this boundary.
    async fn relay_after_unsigned_failure(
        &self,
        failure: crate::network::acquisition::Failure,
        request: &reqwest::Request,
        payer: Option<&Arc<Payer>>,
        progress: &SubmissionProgress,
        unsigned_only: bool,
    ) -> Result<crate::output::HttpOutput> {
        if self.relay_call {
            return Err(failure.into());
        }
        // An earlier signed wallet remains chargeable, even on a later wallet's
        // unsigned failure. Qualification's unsigned-only mode grants no fee.
        if unsigned_only || progress.possible() {
            return Err(failure.error);
        }
        let mut relay_client = self.clone();
        if let Some(payer) = payer {
            relay_client.payer = Some(Arc::new(RwLock::new(payer.clone())));
        }
        crate::discovery_relay::api::fallback(
            &relay_client,
            request,
            failure,
            progress,
            &self.attempt_capacity,
        )
        .await
    }

    async fn unsigned_stage(
        &self,
        request: &reqwest::Request,
        private: bool,
        unsigned_only: bool,
        candidate: Option<&crate::rotation::manager::PaymentCandidateHandle>,
        payer: Option<&Payer>,
    ) -> std::result::Result<UnsignedStage, crate::network::acquisition::Failure> {
        use crate::network::acquisition::{self, Failure, FallbackPolicy};
        let identity = if unsigned_only {
            crate::network::IsolationId::discovery(request.url().as_str())
        } else {
            let address = candidate
                .map(|c| c.address.as_str())
                .or_else(|| payer.map(|p| p.address.as_str()))
                .context("network_identity_missing: payer")
                .map_err(Failure::local)?;
            crate::network::IsolationId::evm(address)
        }
        .map_err(Failure::local)?;
        let factory = crate::network::global();
        diagnostics::stage("transport_selection");
        let selected = factory
            .http_transport(
                &identity,
                request.url().as_str(),
                self.timeout,
                self.public_only,
                self.transport,
                self.selection,
            )
            .map_err(Failure::local)?;
        let participating_tor = self.selection == crate::network::SelectionPolicy::Participating
            && factory.policy.mode == crate::network::Mode::Tor;
        // Manual writes can act without a payment gate. Only their TLS setup
        // failures retain ordinary recovery; never retry their status/body.
        let response_replay = matches!(request.method().as_str(), "GET" | "HEAD")
            || (!private && request.method() == reqwest::Method::POST);
        let runtime_relay = !private && response_replay && self.request_relay().is_some();
        let common_transport = self.relay_call || response_replay;
        let collect_unsigned = (runtime_relay || (participating_tor && common_transport))
            .then_some((self.max_response_bytes, unsigned_only));
        let acquired = acquisition::run(
            factory,
            selected.clone(),
            false,
            |transport, attempt, max_attempts| async move {
                diagnostics::stage("transport_selection");
                let mut cover = self.begin_cover(&transport).await;
                diagnostics::unsigned_attempt(attempt, max_attempts);
                diagnostics::stage("unsigned_request");
                let attempt_capacity = self
                    .acquire_attempt_capacity()
                    .await
                    .map_err(Failure::local)?;
                // A claim may expire while waiting for transport or capacity.
                // Check current authority at every new unsigned dispatch.
                crate::qualification::check_retry_authority()
                    .await
                    .map_err(Failure::local)?;
                let (response, retry, receipt) = unsigned_response(
                    &transport,
                    request,
                    cover.as_mut(),
                    self.relay_call,
                    collect_unsigned,
                )
                .await
                .map_err(|error| {
                    // TLS can fail after a write executes, even before response
                    // headers arrive. Only connection setup proves this attempt
                    // could not have delivered the request.
                    let ordinary_retry =
                        error.downcast_ref::<reqwest::Error>().is_some_and(|error| {
                            error.is_connect() && crate::network::tls_failure(error)
                        });
                    let mut failure = if error.is::<IncompleteRelayResponse>() {
                        Failure::validation(error)
                    } else {
                        Failure::body(error)
                    };
                    if !common_transport {
                        failure.retry &= ordinary_retry;
                        failure.fallback = false;
                    }
                    failure
                })?;
                diagnostics::response(&response);
                let status = response.status();
                let relay_rejected = self.relay_call
                    && !status.is_success()
                    && status != reqwest::StatusCode::PAYMENT_REQUIRED;
                if relay_rejected
                    || (participating_tor
                        && response_replay
                        && crate::network::eligible_retry_status(status))
                    || (runtime_relay && acquisition::blocking_status(status.as_u16()))
                {
                    let error = HttpFailure {
                        status,
                        detail: if participating_tor {
                            "unsigned Tor isolation attempts exhausted"
                        } else {
                            "unsigned HTTP rejection"
                        }
                        .into(),
                    };
                    let failure = Failure::status(
                        error.into(),
                        status,
                        response.headers(),
                        if self.relay_call {
                            FallbackPolicy::RemoteFailures
                        } else {
                            FallbackPolicy::BlockingFailures
                        },
                    );
                    drop(response);
                    drop(cover);
                    drop(attempt_capacity);
                    if attempt < max_attempts && failure.retry_after.is_some() {
                        crate::qualification::check_retry_authority()
                            .await
                            .map_err(Failure::local)?;
                    }
                    return Err(failure);
                }
                Ok((response, retry, receipt, cover, attempt_capacity))
            },
        )
        .await?;
        let (response, retry, receipt, cover, attempt_capacity) = acquired.value;
        Ok(UnsignedStage {
            selected,
            transport: acquired.current,
            response,
            retry,
            receipt,
            cover,
            attempt_capacity,
        })
    }

    async fn begin_cover(
        &self,
        transport: &crate::network::HttpTransport,
    ) -> Option<crate::cover::runtime::Call> {
        let (config, scope) = self.cover.as_ref()?;
        let engine = crate::network::global().cover.as_ref()?;
        let owner = crate::cover::registry::Owner::from_transport(transport);
        let call = match engine.begin(
            owner,
            config.clone(),
            scope.clone(),
            transport.client.clone(),
        ) {
            Ok(call) => call,
            Err(error) => {
                tracing::warn!(listener = %scope.listener, source = %scope.source, "optional cover unavailable: {error}");
                return None;
            }
        };
        call.prioritize().await;
        Some(call)
    }

    async fn response_output(
        &self,
        response: reqwest::Response,
        paid_submission: bool,
        head: bool,
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
            mime_type: if head {
                Some("text/plain".into())
            } else {
                mime_type
            },
            paid_submission,
            relay_payment_may_have_occurred: false,
        })
    }
}

// PaidClient challenges and direct body-only challenges are collected here once.
// Normalize legacy body-only envelopes so admission and qualification share the
// same challenge. Header-based v1 providers can have unrelated wrapper bodies.
pub(crate) async fn bound_challenge(
    response: reqwest::Response,
    limit: usize,
) -> Result<reqwest::Response> {
    let mut envelope = axum::http::Response::builder().status(response.status());
    *envelope.headers_mut().expect("valid status") = response.headers().clone();
    let bytes = crate::limits::read(
        response,
        limit,
        "payment challenge body",
        "max_response_bytes",
    )
    .await
    .context("payment challenge rejected before signing")?;
    if !envelope
        .headers_ref()
        .expect("valid status")
        .contains_key("payment-required")
        && let Ok(challenge) = serde_json::from_slice::<Value>(&bytes)
        && challenge["x402Version"] == 1
    {
        envelope.headers_mut().expect("valid status").insert(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge)?).parse()?,
        );
    }
    Ok(envelope.body(bytes)?.into())
}

// Capture receipt time before challenge handling, and release unsigned padding
// before recording response protocol observations.
async fn unsigned_response(
    transport: &crate::network::HttpTransport,
    request: &reqwest::Request,
    mut cover: Option<&mut crate::cover::runtime::Call>,
    relay: bool,
    collect_unsigned: Option<(usize, bool)>,
) -> Result<(
    reqwest::Response,
    reqwest::Request,
    Option<crate::pricing::runtime::Receipt>,
)> {
    let mut unsigned = request
        .try_clone()
        .context("request body cannot be retried")?;
    let retry = request
        .try_clone()
        .context("request body cannot be retried")?;
    let mut padding = cover.as_deref().and_then(|c| c.pad(&mut unsigned, true));
    if let Some(p) = &mut padding {
        p.dispatched();
    }
    let response = transport
        .client
        .execute(unsigned)
        .await
        .map_err(reqwest::Error::without_url)?;
    let receipt = crate::pricing::runtime::received();
    drop(padding);
    if let Some(c) = cover.as_deref_mut() {
        c.protocol(response.version());
        c.rejected_padding(response.status());
    }
    crate::network::log_http(&response, "payment_challenge");
    complete_relay_response(&response, relay)?;
    // Collect successful unsigned bodies inside the same finite attempt sequence. A body
    // failure cannot restart payer selection or enter the signed dispatch path.
    let response = if let Some((limit, unsigned_only)) = collect_unsigned
        && (response.status().is_success()
            || response.status() == reqwest::StatusCode::PAYMENT_REQUIRED)
        && !(unsigned_only && response.status() == reqwest::StatusCode::PAYMENT_REQUIRED)
    {
        if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED
            && let Some(c) = cover
        {
            c.response_headers();
        }
        let mut buffered = axum::http::Response::builder()
            .status(response.status())
            .version(response.version());
        *buffered.headers_mut().expect("valid status") = response.headers().clone();
        let bytes =
            crate::limits::read(response, limit, "API response", "max_response_bytes").await?;
        buffered.body(bytes)?.into()
    } else {
        response
    };
    Ok((response, retry, receipt))
}

struct UnsignedStage {
    selected: Arc<crate::network::HttpTransport>,
    transport: Arc<crate::network::HttpTransport>,
    response: reqwest::Response,
    retry: reqwest::Request,
    receipt: Option<crate::pricing::runtime::Receipt>,
    cover: Option<crate::cover::runtime::Call>,
    attempt_capacity: Vec<tokio::sync::OwnedSemaphorePermit>,
}

async fn static_payment(
    payer: Option<&Payer>,
    selected: &Arc<crate::network::HttpTransport>,
    transport: &Arc<crate::network::HttpTransport>,
    retry: reqwest::Request,
    mut response: reqwest::Response,
    cover: Option<&crate::cover::runtime::Call>,
    progress: &SubmissionProgress,
) -> Result<reqwest::Response> {
    diagnostics::stage("challenge_preparation");
    sanitize_challenge(&mut response)?;
    // Selection (asset/network/cap) happens inside the SDK before signing.
    diagnostics::stage("signing");
    let headers = payer
        .expect("static payer")
        .client
        .make_payment_headers(response)
        .await
        .context("x402 challenge rejected or signing failed")?;
    crate::network::global().promote_http(selected, transport);
    let mut retry = retry;
    retry.headers_mut().extend(headers);
    let mut padding = cover.and_then(|c| c.pad(&mut retry, true));
    if let Some(p) = &mut padding {
        p.dispatched();
    }
    progress.mark_possible();
    diagnostics::stage("signed_request");
    let response = transport
        .client
        .execute(retry)
        .await
        .map_err(reqwest::Error::without_url)?;
    Ok(response)
}

fn build_request(route: RoutedRequest) -> Result<reqwest::Request> {
    let mut request =
        reqwest::Request::new(route.method.parse()?, reqwest::Url::parse(&route.url)?);
    let mut query = Vec::new();
    for (name, value) in route.headers {
        let name = crate::catalog::headers::business_name(&name)?;
        ensure!(
            !request.headers().contains_key(&name),
            "duplicate API header"
        );
        request
            .headers_mut()
            .insert(name, value.parse().context("invalid API header value")?);
    }
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
        let encoded = serde_json::to_vec(&challenge)?;
        response
            .headers_mut()
            .insert("payment-required", STANDARD.encode(&encoded).parse()?);
        if challenge["x402Version"] == 1 {
            let mut envelope = axum::http::Response::builder().status(response.status());
            *envelope.headers_mut().expect("valid status") = response.headers().clone();
            *response = envelope.body(encoded)?.into();
        }
    }
    Ok(())
}

/// Structured forms emitted by the pinned x402 HTTP resource server. Call only
/// after this payer's signed dispatch; an unsigned challenge never earns retry.
fn insufficient_funds(headers: &reqwest::header::HeaderMap) -> bool {
    // Both protocol fields carry one record. Ambiguous duplicates cannot grant
    // the narrowly scoped permission to repeat a possibly chargeable request.
    let receipts = ["payment-response", "x-payment-response"];
    if receipts
        .iter()
        .map(|name| headers.get_all(*name).iter().count())
        .sum::<usize>()
        > 1
        || headers.get_all("payment-required").iter().count() > 1
    {
        return false;
    }
    let decoded = |name: &str| -> Option<serde_json::Value> {
        serde_json::from_slice(&STANDARD.decode(headers.get(name)?.as_bytes()).ok()?).ok()
    };
    // A receipt describes settlement of this signed attempt. A simultaneous
    // challenge cannot override success, another failure or an unreadable receipt.
    if let Some(name) = receipts
        .into_iter()
        .find(|name| headers.contains_key(*name))
    {
        let network = if name == "x-payment-response" {
            "base"
        } else {
            "eip155:8453"
        };
        return decoded(name).is_some_and(|v| {
            v["success"] == false
                && v["errorReason"] == "insufficient_funds"
                && v["network"] == network
                && v["transaction"].is_string()
        });
    }
    decoded("payment-required").is_some_and(|v| {
        (v["x402Version"] == 1 || v["x402Version"] == 2)
            && v["error"] == "insufficient_funds"
            && v["accepts"].is_array()
    })
}

#[cfg(test)]
#[path = "payment/relay_retry_tests.rs"]
mod relay_retry_tests;

#[cfg(test)]
#[path = "payment/header_tests.rs"]
mod header_tests;
