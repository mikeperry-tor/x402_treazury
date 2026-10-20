//! Trusted Base RPC evidence at pinned blocks; no vendor response releases exposure.
use super::error::AdmissionError;
use crate::payment::USDC;
use alloy_primitives::{Address, B256, U256, keccak256};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};
pub mod evidence;
mod receipt;
mod resources;
mod shared;
#[cfg(test)]
mod tor_inactivity_tests;
#[cfg(test)]
mod tor_ownership_tests;
#[cfg(test)]
mod tor_tests;
mod transport;
mod verification;
pub use receipt::{TransferExpectation, TransferProof};
pub(crate) use transport::RpcFailure;
pub use verification::{BalanceCertificate, EvidenceCapture, ResolutionCertificate};
pub(crate) use verification::{EndpointMismatch, EvidenceConflict};
#[derive(Clone, Debug)]
pub(crate) struct VerificationStage(pub &'static str);
impl std::fmt::Display for VerificationStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for VerificationStage {}
pub fn safe_diagnostic(error: &anyhow::Error) -> String {
    let stage = error
        .downcast_ref::<VerificationStage>()
        .map_or("chain view", |s| s.0);
    format!("{stage}: {}", safe_reason(error))
}
fn safe_reason(error: &anyhow::Error) -> String {
    if let Some(failure) = error.downcast_ref::<EvidenceConflict>() {
        return failure.to_string();
    }
    if let Some(failure) = error.downcast_ref::<EndpointMismatch>() {
        return failure.to_string();
    }
    if let Some(failure) = error.downcast_ref::<evidence::EvidenceFailure>() {
        return failure.to_string();
    }
    if let Some(rpc) = error.downcast_ref::<RpcFailure>() {
        return rpc.to_string();
    }
    for cause in error.chain() {
        let message = cause.to_string();
        if matches!(
            message.as_str(),
            "wrong Base chain"
                | "invalid confirmed block"
                | "Base view changed during reconciliation"
                | "stale Base block"
                | "invalid contract result length"
                | "invalid authorizationState result"
                | "missing Base RPC result"
                | "invalid block hash"
                | "invalid RPC quantity"
                | "insufficient Base confirmations"
                | "Base block is in the future"
                | "receipt block height mismatch"
                | "receipt transaction reverted"
                | "receipt transaction/block mismatch"
                | "receipt log limit exceeded"
                | "receipt log provenance mismatch"
                | "receipt lacks unique matching USDC transfer and authorization nonce"
                | "receipt event topics malformed"
                | "authorization event data malformed"
        ) {
            return message;
        }
    }
    "Base chain verification failed; unclassified validation error".into()
}

/// Per-attempt, content-free progress survives cancellation of the view future.
struct ViewProgress(std::sync::Mutex<(&'static str, std::time::Instant, usize)>);
impl ViewProgress {
    fn new() -> Self {
        Self(std::sync::Mutex::new((
            "not_started",
            std::time::Instant::now(),
            0,
        )))
    }
    async fn step<T>(
        &self,
        stage: &'static str,
        work: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        {
            let mut state = self.0.lock().expect("view progress");
            state.0 = stage;
            state.1 = std::time::Instant::now();
        }
        let result = work.await;
        if result.is_ok() {
            self.0.lock().expect("view progress").2 += 1;
        }
        result
    }
    fn snapshot(&self) -> (&'static str, u64, usize) {
        let state = self.0.lock().expect("view progress");
        (state.0, state.1.elapsed().as_millis() as u64, state.2)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Anchor {
    pub height: u64,
    pub hash: String,
}
#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PendingAuthorization {
    pub id: String,
    pub wallet: String,
    pub payer: String,
    pub nonce: String,
    pub valid_before: u64,
}
#[derive(Clone, PartialEq, Eq)]
pub struct ChainQuery {
    pub wallets: Vec<(String, String)>,
    pub pending: Vec<PendingAuthorization>,
    pub anchor: Option<Anchor>,
    pub captures: BTreeMap<String, EvidenceCapture>,
}
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuthorizationOutcome {
    Used,
    ExpiredUnused,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct AuthorizationResolution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    pub outcome: AuthorizationOutcome,
    pub block_time: u64,
}
#[derive(Clone)]
pub struct ChainView {
    pub anchor: Anchor,
    pub balances: BTreeMap<String, U256>,
    /// Complete exact reads, retained separately from the conservative balance.
    pub observations: BTreeMap<String, evidence::BalanceObservation>,
    pub balance_certificates: BTreeMap<String, BalanceCertificate>,
    pub resolution_certificates: BTreeMap<String, ResolutionCertificate>,
    pub released: Vec<String>,
    pub resolutions: BTreeMap<String, AuthorizationResolution>,
}
#[derive(Clone)]
pub struct BaseRpc {
    identity: crate::network::IsolationId,
    url: reqwest::Url,
    confirmations: u64,
    fallbacks: Vec<BaseRpc>,
}
pub fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
pub fn secure_endpoint(text: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(text).map_err(|_| anyhow::anyhow!("invalid endpoint URL"))?;
    let local = url.host_str().is_some_and(|h| {
        h == "localhost"
            || h.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && local),
        "endpoint requires TLS (HTTP allowed only on loopback)"
    );
    Ok(url)
}
struct ViewHeaders {
    latest: Anchor,
    confirmed: Anchor,
    confirmed_time: u64,
}
struct ResolvedHistory {
    released: Vec<String>,
    resolutions: BTreeMap<String, AuthorizationResolution>,
    resolution_certificates: BTreeMap<String, ResolutionCertificate>,
}

impl BaseRpc {
    pub fn new(url: &str, confirmations: u64) -> Result<Self> {
        ensure!(confirmations > 0, "invalid Base RPC limits");
        Ok(Self {
            identity: crate::network::IsolationId::discovery(url)?,
            url: secure_endpoint(url)?,
            confirmations,
            fallbacks: vec![],
        })
    }
    /// Only operator-selected endpoints; every attempt constructs a whole chain view.
    pub fn with_fallbacks(urls: &[String], confirmations: u64) -> Result<Self> {
        ensure!(
            !urls.is_empty() && urls.len() <= 3,
            "configure one to three Base RPC endpoints; list is never truncated"
        );
        let mut seen = std::collections::BTreeSet::new();
        for url in urls {
            ensure!(
                seen.insert(secure_endpoint(url)?.to_string()),
                "duplicate Base RPC endpoint"
            );
        }
        let mut endpoints = urls
            .iter()
            .map(|url| Self::new(url, confirmations))
            .collect::<Result<Vec<_>>>()?;
        let mut primary = endpoints.remove(0);
        primary.fallbacks = endpoints;
        Ok(primary)
    }
    fn for_address(&self, address: &str) -> Result<Self> {
        Ok(Self {
            identity: crate::network::IsolationId::evm(address)?,
            url: self.url.clone(),
            confirmations: self.confirmations,
            fallbacks: vec![],
        })
    }
    fn endpoint_id(&self) -> String {
        use sha2::{Digest, Sha256};
        alloy_primitives::hex::encode(Sha256::digest(self.url.as_str().as_bytes()))
    }
    /// Share canonical header observations only within one verification phase on
    /// this endpoint. Initial headers read before state acquisition are excluded.
    async fn recheck_anchor(
        &self,
        height: u64,
        rechecked: &mut BTreeMap<u64, Anchor>,
        progress: &ViewProgress,
        stage: &'static str,
    ) -> Result<Anchor> {
        if let Some(anchor) = rechecked.get(&height) {
            return Ok(anchor.clone());
        }
        let (anchor, _) = progress
            .step(stage, self.block(&format!("0x{height:x}")))
            .await?;
        rechecked.insert(height, anchor.clone());
        Ok(anchor)
    }
    async fn verify_capture(
        &self,
        capture: &EvidenceCapture,
        endpoint: &str,
        progress: &ViewProgress,
        rechecked: &mut BTreeMap<u64, Anchor>,
    ) -> Result<()> {
        let mut anchors: BTreeMap<(u64, String), (bool, bool)> = BTreeMap::new();
        if let Some(checkpoint) = &capture.checkpoint {
            anchors.insert((checkpoint.height, checkpoint.hash.clone()), (false, true));
        }
        if let Some(previous) = &capture.previous {
            if !capture.previous_confirmed_covered {
                anchors
                    .entry((
                        previous.confirmed.anchor.height,
                        previous.confirmed.anchor.hash.clone(),
                    ))
                    .or_insert((false, false));
            }
            anchors
                .entry((
                    previous.latest.anchor.height,
                    previous.latest.anchor.hash.clone(),
                ))
                .or_insert((true, false));
        }
        for anchor in &capture.durable_balance_anchors {
            anchors
                .entry((anchor.height, anchor.hash.clone()))
                .or_insert((false, false));
        }
        for ((height, hash), (latest_only, financial_checkpoint)) in anchors {
            let canonical = self
                .recheck_anchor(height, rechecked, progress, "wallet_checkpoint")
                .await?;
            if canonical.height != height || canonical.hash != hash {
                return Err(EvidenceConflict {
                    wallet: capture.wallet.clone(),
                    endpoint: endpoint.into(),
                    anchor: Anchor { height, hash },
                    latest_only,
                    financial_checkpoint,
                }
                .into());
            }
        }
        Ok(())
    }
    async fn rpc<T>(
        &self,
        method: &'static str,
        params: Value,
        validate: impl FnOnce(Value) -> Result<(T, bool)>,
    ) -> Result<T> {
        let network = crate::network::global();
        let selected = network.http_participating(
            &self.identity,
            self.url.as_str(),
            None,
            false,
            crate::network::HttpPolicy::COMPATIBLE,
        )?;
        transport::rpc(network, selected, &self.url, method, params, validate).await
    }

    async fn block(&self, tag: &str) -> Result<(Anchor, u64)> {
        self.rpc("eth_getBlockByNumber", json!([tag, false]), |v| {
            let malformed = || RpcFailure::malformed("eth_getBlockByNumber");
            let height = quantity(&v["number"]).map_err(|_| malformed())?;
            let timestamp = quantity(&v["timestamp"]).map_err(|_| malformed())?;
            let hash = v["hash"].as_str().ok_or_else(malformed)?;
            let hash = hash.parse::<B256>().map_err(|_| malformed())?;
            // Preserve the caller's canonical-conflict classification for an
            // unexpected height, without selecting that response's transport.
            let matches_request =
                tag == "latest" || quantity(&json!(tag)).is_ok_and(|requested| requested == height);
            Ok((
                (
                    Anchor {
                        height,
                        hash: hash.to_string(),
                    },
                    timestamp,
                ),
                matches_request,
            ))
        })
        .await
    }
    async fn call<T>(
        &self,
        data: String,
        block: &Anchor,
        validate: impl FnOnce(U256) -> Result<T>,
    ) -> Result<T> {
        // EIP-1898 binds balance and nonce evidence to exactly the same canonical block.
        self.rpc(
            "eth_call",
            json!([{"to":USDC,"data":data},{"blockHash":block.hash,"requireCanonical":true}]),
            |v| Ok((validate(contract_value(v)?)?, true)),
        )
        .await
    }
    /// One ordinary balanceOf read; an advisory result needs no historical proof,
    /// confirmed/latest pair, host-clock check or canonical certificate.
    pub(crate) async fn current_balance(&self, address: &str) -> Result<U256> {
        let _owner = resources::query()?;
        let address: Address = address.parse()?;
        let data = format!("0x70a08231{:0>64}", format!("{address:x}"));
        let mut failure = None;
        let provider_count = self.fallbacks.len() + 1;
        for (index, endpoint) in std::iter::once(self).chain(&self.fallbacks).enumerate() {
            let started = std::time::Instant::now();
            let scoped = endpoint.for_address(&address.to_string())?;
            let result = scoped
                .rpc("eth_call", json!([{"to":USDC,"data":data},"latest"]), |v| {
                    Ok((contract_value(v)?, true))
                })
                .await;
            match result {
                Ok(value) => {
                    tracing::debug!(
                        provider_index = index + 1,
                        provider_count,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Base balance read succeeded on configured provider"
                    );
                    return Ok(value);
                }
                Err(error) => {
                    tracing::debug!(provider_index = index + 1, provider_count,
                        failover = index + 1 < provider_count,
                        category = %safe_diagnostic(&error),
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Base balance RPC provider unavailable");
                    failure = Some(error);
                }
            }
        }
        Err(failure.context("Base RPC endpoint unavailable")?)
    }
    pub async fn view(&self, query: ChainQuery) -> Result<ChainView> {
        let owner = resources::query()?;
        self.view_with_owner(query, &owner).await
    }
    async fn view_with_owner(
        &self,
        query: ChainQuery,
        _owner: &tokio::sync::SemaphorePermit<'static>,
    ) -> Result<ChainView> {
        // Process-local correlation only; never derived from wallet/pool identities.
        static NEXT_VIEW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let view_sequence = NEXT_VIEW.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let count = self.fallbacks.len() + 1;
        let wallet_count = query.wallets.len();
        let pending_count = query.pending.len();
        let expected_rpc_calls = 5usize
            .saturating_add(usize::from(query.anchor.is_some()))
            .saturating_add(wallet_count.saturating_mul(2))
            .saturating_add(pending_count);
        for (index, endpoint) in std::iter::once(self).chain(&self.fallbacks).enumerate() {
            let started = std::time::Instant::now();
            let progress = ViewProgress::new();
            // Required evidence failures restart on a configured fallback.
            // Foreground balance queries contain no authorization history;
            // maintenance owns its independent historical acquisitions.
            let result = endpoint.view_inner(query.clone(), &progress).await;
            match result {
                Ok(view) => {
                    if index > 0 {
                        tracing::debug!(
                            view_sequence,
                            provider_index = index + 1,
                            provider_count = count,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            completed_rpc_calls = progress.snapshot().2,
                            "Base verification succeeded on configured fallback; balances and canonical anchors reverified"
                        );
                    }
                    return Ok(view);
                }
                Err(error) => {
                    let failover = index + 1 < count
                        && error
                            .downcast_ref::<RpcFailure>()
                            .is_some_and(RpcFailure::can_failover);
                    let (verification_stage, stage_elapsed_ms, completed_rpc_calls) =
                        progress.snapshot();
                    if failover {
                        tracing::debug!(view_sequence, verification_stage, stage_elapsed_ms, completed_rpc_calls, expected_rpc_calls,
                            wallet_count, pending_count, category = %safe_diagnostic(&error), provider_index = index + 1, provider_count = count,
                            failover, elapsed_ms = started.elapsed().as_millis() as u64,
                            "Base verification provider failed; trying configured fallback with a fresh view");
                    } else {
                        tracing::warn!(view_sequence, verification_stage, stage_elapsed_ms, completed_rpc_calls, expected_rpc_calls,
                            wallet_count, pending_count, category = %safe_diagnostic(&error), provider_index = index + 1, provider_count = count,
                            failover, elapsed_ms = started.elapsed().as_millis() as u64,
                            "Base credit/payment chain verification failed; no usable configured fallback");
                    }
                    if !failover {
                        return Err(error);
                    }
                }
            }
        }
        unreachable!("primary Base RPC endpoint always exists")
    }
    async fn view_inner(&self, query: ChainQuery, progress: &ViewProgress) -> Result<ChainView> {
        let headers = self.initial_view_headers(&query, progress).await?;
        let endpoint = self.endpoint_id();
        let read_history = !query.pending.is_empty();
        let ResolvedHistory {
            released,
            resolutions,
            resolution_certificates,
        } = self
            .resolve_authorizations(
                query.pending,
                &query.captures,
                &headers,
                &endpoint,
                progress,
            )
            .await?;
        // Historical resolution and fresh admission retain distinct anchors.
        let resolution_anchor = headers.confirmed.clone();
        let mut rechecked = BTreeMap::new();
        if query.wallets.is_empty() {
            self.verify_history_anchor(
                &resolution_anchor,
                &query.captures,
                &endpoint,
                progress,
                &mut rechecked,
            )
            .await?;
            for capture in query.captures.values() {
                let mut historical = capture.clone();
                historical.previous = None;
                self.verify_capture(&historical, &endpoint, progress, &mut rechecked)
                    .await?;
            }
            return Ok(ChainView {
                anchor: resolution_anchor,

                balances: BTreeMap::new(),
                observations: BTreeMap::new(),
                balance_certificates: BTreeMap::new(),
                resolution_certificates,
                released,
                resolutions,
            });
        }
        // Balance-only queries reuse the original coherent pair; a nonce sweep
        // obtains fresh admission headers after retaining historical resolution.
        let headers = if read_history {
            self.admission_view_headers(&resolution_anchor, progress)
                .await?
        } else {
            headers
        };
        let observations = self
            .balance_observations(&query.wallets, &headers, &endpoint, progress)
            .await?;
        self.recheck_view_anchors(
            [&resolution_anchor, &headers.confirmed, &headers.latest],
            &query.captures,
            &endpoint,
            progress,
            &mut rechecked,
        )
        .await?;
        // The financial checkpoint and captured same-wallet facts must still be
        // canonical on the very endpoint that supplied these exact reads.
        for capture in query.captures.values() {
            self.verify_capture(capture, &endpoint, progress, &mut rechecked)
                .await?;
        }
        let balance_certificates = observations
            .iter()
            .filter_map(|(wallet, observation)| {
                query.captures.get(wallet).map(|capture| {
                    (
                        wallet.clone(),
                        BalanceCertificate::verified(
                            capture.clone(),
                            observation.clone(),
                            resolution_anchor.clone(),
                        ),
                    )
                })
            })
            .collect();
        // Preserve completed evidence even if balance acquisition outlasted its
        // admission window. The store checks that window before new authority;
        // retrying this entire sweep here could hold the pool gate forever.
        Ok(ChainView {
            anchor: headers.confirmed,
            balances: observations
                .iter()
                .map(|(id, observation)| (id.clone(), observation.available()))
                .collect(),
            observations,
            balance_certificates,
            resolution_certificates,
            released,
            resolutions,
        })
    }
    async fn initial_view_headers(
        &self,
        query: &ChainQuery,
        progress: &ViewProgress,
    ) -> Result<ViewHeaders> {
        progress
            .step(
                "chain_id",
                self.rpc("eth_chainId", json!([]), |value| {
                    ensure!(
                        quantity(&value).map_err(|_| RpcFailure::malformed("eth_chainId"))? == 8453,
                        EndpointMismatch {
                            endpoint: self.endpoint_id()
                        }
                    );
                    Ok(((), true))
                }),
            )
            .await?;
        if let Some(previous) = &query.anchor {
            let (canonical, _) = progress
                .step(
                    "previous_anchor",
                    self.block(&format!("0x{:x}", previous.height)),
                )
                .await?;
            ensure!(
                canonical.height == previous.height && canonical.hash == previous.hash,
                AdmissionError::ChainRecoveryRequired("confirmed anchor changed")
            );
        }
        let (latest, timestamp) = progress
            .step("latest_block", self.block("latest"))
            .await
            .context(VerificationStage("latest block"))?;
        let height = latest
            .height
            .checked_sub(self.confirmations)
            .context("insufficient Base confirmations")?;
        let (confirmed, confirmed_time) = progress
            .step("confirmed_block", self.block(&format!("0x{height:x}")))
            .await
            .context(VerificationStage("confirmed block"))?;
        ensure!(
            confirmed.height == height
                && confirmed_time <= timestamp
                && query.anchor.as_ref().is_none_or(|a| height >= a.height),
            "invalid confirmed block"
        );
        Ok(ViewHeaders {
            latest,
            confirmed,
            confirmed_time,
        })
    }
    async fn resolve_authorizations(
        &self,
        pending: Vec<PendingAuthorization>,
        captures: &BTreeMap<String, EvidenceCapture>,
        headers: &ViewHeaders,
        endpoint: &str,
        progress: &ViewProgress,
    ) -> Result<ResolvedHistory> {
        let mut released = Vec::new();
        let mut resolutions = BTreeMap::new();
        let mut resolution_certificates = BTreeMap::new();
        let selector = &keccak256("authorizationState(address,bytes32)")[..4];
        let selector = alloy_primitives::hex::encode(selector);
        for auth in pending {
            let payer: Address = auth.payer.parse()?;
            let nonce: B256 = auth.nonce.parse()?;
            let data = format!(
                "0x{selector}{:0>64}{}",
                format!("{payer:x}"),
                alloy_primitives::hex::encode(nonce)
            );
            let scoped = self.for_address(&auth.payer)?;
            let used = progress
                .step(
                    "authorization_nonce",
                    scoped.call(data, &headers.confirmed, |used| {
                        ensure!(used <= U256::from(1), RpcFailure::malformed("eth_call"));
                        Ok(used)
                    }),
                )
                .await
                .context(VerificationStage("authorization nonce"))?;
            if used == U256::from(1) || headers.confirmed_time > auth.valid_before {
                resolutions.insert(
                    auth.id.clone(),
                    AuthorizationResolution {
                        anchor: Some(headers.confirmed.clone()),
                        outcome: if used == U256::from(1) {
                            AuthorizationOutcome::Used
                        } else {
                            AuthorizationOutcome::ExpiredUnused
                        },
                        block_time: headers.confirmed_time,
                    },
                );
                if let Some(capture) = captures.get(&auth.wallet) {
                    let resolution = &resolutions[&auth.id];
                    resolution_certificates.insert(
                        auth.id.clone(),
                        ResolutionCertificate::verified(
                            capture.clone(),
                            endpoint.to_owned(),
                            auth.clone(),
                            headers.confirmed.clone(),
                            resolution.outcome.clone(),
                            headers.confirmed_time,
                        ),
                    );
                }
                released.push(auth.id);
            }
        }
        Ok(ResolvedHistory {
            released,
            resolutions,
            resolution_certificates,
        })
    }
    async fn verify_history_anchor(
        &self,
        resolution_anchor: &Anchor,
        captures: &BTreeMap<String, EvidenceCapture>,
        endpoint: &str,
        progress: &ViewProgress,
        rechecked: &mut BTreeMap<u64, Anchor>,
    ) -> Result<()> {
        let canonical = self
            .recheck_anchor(
                resolution_anchor.height,
                rechecked,
                progress,
                "history_anchor_recheck",
            )
            .await?;
        if canonical != *resolution_anchor {
            if let Some(capture) = captures.values().find(|capture| {
                capture.checkpoint.as_ref() == Some(resolution_anchor)
                    || capture.durable_balance_anchors.contains(resolution_anchor)
            }) {
                let financial_checkpoint = capture.checkpoint.as_ref() == Some(resolution_anchor);
                return Err(EvidenceConflict {
                    wallet: capture.wallet.clone(),
                    endpoint: endpoint.to_owned(),
                    anchor: resolution_anchor.clone(),
                    latest_only: false,
                    financial_checkpoint,
                }
                .into());
            }
            anyhow::bail!("Base view changed during reconciliation");
        }
        Ok(())
    }
    async fn admission_view_headers(
        &self,
        resolution_anchor: &Anchor,
        progress: &ViewProgress,
    ) -> Result<ViewHeaders> {
        let (latest, timestamp) = progress
            .step("admission_latest_block", self.block("latest"))
            .await?;
        let height = latest
            .height
            .checked_sub(self.confirmations)
            .context("insufficient Base confirmations")?;
        let (confirmed, confirmed_time) = progress
            .step(
                "admission_confirmed_block",
                self.block(&format!("0x{height:x}")),
            )
            .await?;
        ensure!(
            confirmed.height == height
                && confirmed_time <= timestamp
                && confirmed.height >= resolution_anchor.height,
            "invalid admission block"
        );
        Ok(ViewHeaders {
            latest,
            confirmed,
            confirmed_time,
        })
    }
    async fn balance_observations(
        &self,
        wallets: &[(String, String)],
        headers: &ViewHeaders,
        endpoint: &str,
        progress: &ViewProgress,
    ) -> Result<BTreeMap<String, evidence::BalanceObservation>> {
        use futures_util::{StreamExt, TryStreamExt};
        let observations: BTreeMap<_, _> =
            futures_util::stream::iter(wallets.iter().cloned().map(|(id, address)| {
                let confirmed = headers.confirmed.clone();
                let latest = headers.latest.clone();
                let rpc = self.clone();
                let endpoint = endpoint.to_owned();
                async move {
                    let address: Address = address.parse()?;
                    let data = format!("0x70a08231{:0>64}", format!("{address:x}"));
                    let scoped = rpc.for_address(&address.to_string())?;
                    let stable = progress
                        .step(
                            "confirmed_balance",
                            scoped.call(data.clone(), &confirmed, Ok),
                        )
                        .await
                        .context(VerificationStage("confirmed balance"))?;
                    let current = progress
                        .step("latest_balance", scoped.call(data, &latest, Ok))
                        .await
                        .context(VerificationStage("latest balance"))?;
                    let observation = evidence::BalanceObservation {
                        wallet: id.clone(),
                        address: address.to_string(),
                        network: "eip155:8453".into(),
                        asset: USDC.into(),
                        endpoint,
                        confirmations: rpc.confirmations,
                        confirmed: evidence::BalancePoint {
                            anchor: confirmed,
                            amount: stable,
                        },
                        latest: evidence::BalancePoint {
                            anchor: latest,
                            amount: current,
                        },
                    };
                    observation.validate()?;
                    Ok::<_, anyhow::Error>((id, observation))
                }
            }))
            .buffer_unordered(8)
            .try_collect()
            .await?;
        Ok(observations)
    }
    async fn recheck_view_anchors(
        &self,
        anchors: [&Anchor; 3],
        captures: &BTreeMap<String, EvidenceCapture>,
        endpoint: &str,
        progress: &ViewProgress,
        rechecked: &mut BTreeMap<u64, Anchor>,
    ) -> Result<()> {
        for anchor in anchors {
            let end = self
                .recheck_anchor(anchor.height, rechecked, progress, "anchor_recheck")
                .await?;
            if end.height != anchor.height || end.hash != anchor.hash {
                // A contradiction of captured accepted evidence has its scoped
                // integrity treatment even when that anchor was also a new read.
                for capture in captures.values() {
                    let financial_checkpoint = capture.checkpoint.as_ref() == Some(anchor);
                    let previous_confirmed = capture
                        .previous
                        .as_ref()
                        .is_some_and(|observation| &observation.confirmed.anchor == anchor);
                    let previous_latest = capture
                        .previous
                        .as_ref()
                        .is_some_and(|observation| &observation.latest.anchor == anchor);
                    let durable = capture.durable_balance_anchors.contains(anchor);
                    if financial_checkpoint || previous_confirmed || previous_latest || durable {
                        return Err(EvidenceConflict {
                            wallet: capture.wallet.clone(),
                            endpoint: endpoint.to_owned(),
                            anchor: anchor.clone(),
                            latest_only: !financial_checkpoint
                                && !previous_confirmed
                                && previous_latest,
                            financial_checkpoint,
                        }
                        .into());
                    }
                }
                anyhow::bail!("Base view changed during reconciliation");
            }
        }
        Ok(())
    }
}
fn contract_value(value: Value) -> Result<U256> {
    let s = value
        .as_str()
        .filter(|s| s.len() == 66 && s.starts_with("0x"))
        .ok_or_else(|| RpcFailure::malformed("eth_call"))?;
    U256::from_str_radix(&s[2..], 16).map_err(|_| RpcFailure::malformed("eth_call").into())
}

fn quantity(v: &Value) -> Result<u64> {
    let s = v.as_str().context("invalid RPC quantity")?;
    Ok(u64::from_str_radix(
        s.strip_prefix("0x").context("invalid RPC quantity")?,
        16,
    )?)
}

#[cfg(test)]
mod tests {
    #[derive(Clone, Default)]
    pub(super) struct Logs(pub(super) std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn view_progress_retains_active_stage_after_deadline_cancellation() {
        use super::*;
        let progress = ViewProgress::new();
        progress.step("chain_id", async { Ok(()) }).await.unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(20),
            progress.step("latest_block", std::future::pending::<Result<()>>()),
        )
        .await;
        assert!(result.is_err());
        let (stage, elapsed, completed) = progress.snapshot();
        assert_eq!(stage, "latest_block");
        assert!(elapsed >= 20);
        assert_eq!(completed, 1);
    }
    #[tokio::test]
    async fn rpc_failures_report_codes_without_upstream_secrets_in_errors_or_logs() {
        use super::*;
        use axum::{Router, http::StatusCode, routing::post};
        use tracing::instrument::WithSubscriber;
        for (status, body, expected) in [
            (403, "private-address secret-token".to_owned(), "HTTP 403"),
            (429, "private-address secret-token".to_owned(), "HTTP 429"),
            (200, json!({"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"private-address secret-token"}}).to_string(), "RPC code -32005"),
            (200, "private-address secret-token".to_owned(), "invalid JSON body"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/secret-url", listener.local_addr().unwrap());
            let app = Router::new().route("/secret-url",post(move || {let body=body.clone();async move {(StatusCode::from_u16(status).unwrap(),body)}}));
            let server = tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
            let logs = Logs::default();
            let writer = logs.clone();
            let subscriber = tracing_subscriber::fmt().without_time().with_ansi(false).with_writer(move ||writer.clone()).finish();
            let rpc = BaseRpc::new(&url, 12).unwrap();
            let error = rpc.view(ChainQuery{wallets:vec![],pending:vec![],anchor:None,captures:Default::default()}).with_subscriber(subscriber).await.err().unwrap();
            let public = safe_diagnostic(&error);
            let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
            for text in [&public, &logged, &error.to_string()] {
                assert!(text.contains(expected), "{text}");
                for secret in ["private-address","secret-token","secret-url"] { assert!(!text.contains(secret), "{text}"); }
            }
            assert!(logged.contains("Base credit/payment chain verification failed"));
            server.abort();
        }
    }
}

#[cfg(test)]
mod view_tests;
