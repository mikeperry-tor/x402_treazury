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
mod receipt;
mod transport;
pub use receipt::{TransferExpectation, TransferProof};
pub(crate) use transport::RpcFailure;
#[derive(Debug)]
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

#[derive(Clone, Debug, serde::Serialize)]
pub struct Anchor {
    pub height: u64,
    pub hash: String,
}
#[derive(Clone)]
pub struct PendingAuthorization {
    pub id: String,
    pub wallet: String,
    pub payer: String,
    pub nonce: String,
    pub valid_before: u64,
}
#[derive(Clone)]
pub struct ChainQuery {
    pub wallets: Vec<(String, String)>,
    pub pending: Vec<PendingAuthorization>,
    pub anchor: Option<Anchor>,
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
pub struct ChainView {
    /// Admission authority expires independently of retained canonical evidence.
    /// Derived from the balance block timestamp, never from completion time.
    pub admission_valid_until: u64,
    pub anchor: Anchor,
    pub balances: BTreeMap<String, U256>,
    pub released: Vec<String>,
    pub resolutions: BTreeMap<String, AuthorizationResolution>,
}
#[derive(Clone)]
pub struct BaseRpc {
    http: reqwest::Client,
    url: reqwest::Url,
    confirmations: u64,
    max_age: u64,
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
impl BaseRpc {
    pub fn new(url: &str, confirmations: u64, max_age: u64) -> Result<Self> {
        ensure!(confirmations > 0 && max_age > 0, "invalid Base RPC limits");
        Ok(Self {
            http: crate::network::discovery(url, None)?,
            url: secure_endpoint(url)?,
            confirmations,
            max_age,
            fallbacks: vec![],
        })
    }
    /// Only operator-selected endpoints; every attempt constructs a whole chain view.
    pub fn with_fallbacks(urls: &[String], confirmations: u64, max_age: u64) -> Result<Self> {
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
            .map(|url| Self::new(url, confirmations, max_age))
            .collect::<Result<Vec<_>>>()?;
        let mut primary = endpoints.remove(0);
        primary.fallbacks = endpoints;
        Ok(primary)
    }
    fn for_address(&self, address: &str) -> Result<Self> {
        Ok(Self {
            http: crate::network::global().http(
                &crate::network::IsolationId::evm(address)?,
                self.url.as_str(),
                None,
            )?,
            url: self.url.clone(),
            confirmations: self.confirmations,
            max_age: self.max_age,
            fallbacks: vec![],
        })
    }
    async fn rpc(&self, method: &'static str, params: Value) -> Result<Value> {
        transport::rpc(&self.http, &self.url, method, params).await
    }

    async fn block(&self, tag: &str) -> Result<(Anchor, u64)> {
        let v = self
            .rpc("eth_getBlockByNumber", json!([tag, false]))
            .await?;
        let height = quantity(&v["number"])?;
        let timestamp = quantity(&v["timestamp"])?;
        let hash = v["hash"].as_str().context("missing block hash")?;
        hash.parse::<B256>().context("invalid block hash")?;
        Ok((
            Anchor {
                height,
                hash: hash.into(),
            },
            timestamp,
        ))
    }
    async fn call(&self, data: String, block: &Anchor) -> Result<U256> {
        // EIP-1898 binds balance and nonce evidence to exactly the same canonical block.
        let v = self
            .rpc(
                "eth_call",
                json!([{"to":USDC,"data":data},{"blockHash":block.hash,"requireCanonical":true}]),
            )
            .await?;
        let s = v.as_str().context("invalid contract result")?;
        ensure!(
            s.len() == 66 && s.starts_with("0x"),
            "invalid contract result length"
        );
        Ok(U256::from_str_radix(&s[2..], 16)?)
    }
    pub async fn view(&self, query: ChainQuery) -> Result<ChainView> {
        self.view_with_policy(query, false).await
    }
    /// Nonce read outages cannot erase liability or veto spending of unrelated
    /// remaining capacity. Malformed evidence, chain/balance/anchor failures and
    /// ordinary background reconciliation retain their strict failure behavior.
    pub(crate) async fn payment_view(&self, query: ChainQuery) -> Result<ChainView> {
        self.view_with_policy(query, true).await
    }
    async fn view_with_policy(&self, query: ChainQuery, payment: bool) -> Result<ChainView> {
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
            // Optional nonce outages retain liability on this endpoint instead
            // of making another provider's availability a payment prerequisite.
            let result = endpoint.view_inner(query.clone(), &progress, payment).await;
            match result {
                Ok(view) => {
                    if index > 0 {
                        tracing::warn!(
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
                    tracing::warn!(view_sequence, verification_stage, stage_elapsed_ms, completed_rpc_calls, expected_rpc_calls,
                        wallet_count, pending_count, category = %safe_diagnostic(&error), provider_index = index + 1, provider_count = count,
                        failover, elapsed_ms = started.elapsed().as_millis() as u64,
                        "Base credit/payment chain verification failed; partial view discarded; only configured read-only fallback permitted");
                    if !failover {
                        return Err(error);
                    }
                }
            }
        }
        unreachable!("primary Base RPC endpoint always exists")
    }
    async fn view_inner(
        &self,
        query: ChainQuery,
        progress: &ViewProgress,
        retain_unavailable: bool,
    ) -> Result<ChainView> {
        ensure!(
            quantity(
                &progress
                    .step("chain_id", self.rpc("eth_chainId", json!([])))
                    .await?
            )? == 8453,
            "wrong Base chain"
        );
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
        let clock = now()?;
        validate_timestamp(timestamp, clock, self.max_age)?;
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
        let mut released = Vec::new();
        let mut resolutions = BTreeMap::new();
        let selector = &keccak256("authorizationState(address,bytes32)")[..4];
        let selector = alloy_primitives::hex::encode(selector);
        let mut unavailable = 0usize;
        for auth in query.pending {
            let payer: Address = auth.payer.parse()?;
            let nonce: B256 = auth.nonce.parse()?;
            let data = format!(
                "0x{selector}{:0>64}{}",
                format!("{payer:x}"),
                alloy_primitives::hex::encode(nonce)
            );
            let scoped = self.for_address(&auth.payer)?;
            let used = match progress
                .step("authorization_nonce", scoped.call(data, &confirmed))
                .await
                .context(VerificationStage("authorization nonce"))
            {
                Ok(used) => used,
                Err(error)
                    if retain_unavailable
                        && error
                            .downcast_ref::<RpcFailure>()
                            .is_some_and(RpcFailure::can_failover) =>
                {
                    // In particular, expiry alone cannot release a nonce whose
                    // canonical state was not observed. Its full amount stays
                    // reserved by the store, including across restart.
                    unavailable += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            ensure!(used <= U256::from(1), "invalid authorizationState result");
            if used == U256::from(1) || confirmed_time > auth.valid_before {
                resolutions.insert(
                    auth.id.clone(),
                    AuthorizationResolution {
                        anchor: Some(confirmed.clone()),
                        outcome: if used == U256::from(1) {
                            AuthorizationOutcome::Used
                        } else {
                            AuthorizationOutcome::ExpiredUnused
                        },
                        block_time: confirmed_time,
                    },
                );
                released.push(auth.id);
            }
        }
        if unavailable > 0 {
            tracing::warn!(
                unavailable_authorizations = unavailable,
                category = "authorization_read_unavailable",
                "Payment nonce observations unavailable; retaining full liabilities and requiring fresh balance evidence"
            );
        }
        // Authorization resolution is historical canonical evidence. Its sweep
        // cannot age the independently acquired admission balance view.
        let resolution_anchor = confirmed;
        let (latest, timestamp) = progress
            .step("admission_latest_block", self.block("latest"))
            .await?;
        validate_timestamp(timestamp, now()?, self.max_age)?;
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
        use futures_util::{StreamExt, TryStreamExt};
        let balances: BTreeMap<_, _> =
            futures_util::stream::iter(query.wallets.clone().into_iter().map(|(id, address)| {
                let confirmed = confirmed.clone();
                let latest = latest.clone();
                let rpc = self.clone();
                async move {
                    let address: Address = address.parse()?;
                    let data = format!("0x70a08231{:0>64}", format!("{address:x}"));
                    let scoped = rpc.for_address(&address.to_string())?;
                    let stable = progress
                        .step("confirmed_balance", scoped.call(data.clone(), &confirmed))
                        .await
                        .context(VerificationStage("confirmed balance"))?;
                    let current = progress
                        .step("latest_balance", scoped.call(data, &latest))
                        .await
                        .context(VerificationStage("latest balance"))?;
                    Ok::<_, anyhow::Error>((id.clone(), stable.min(current)))
                }
            }))
            .buffer_unordered(8)
            .try_collect()
            .await?;
        for anchor in [&resolution_anchor, &confirmed, &latest] {
            let (end, _) = progress
                .step(
                    "anchor_recheck",
                    self.block(&format!("0x{:x}", anchor.height)),
                )
                .await?;
            ensure!(
                end.height == anchor.height && end.hash == anchor.hash,
                "Base view changed during reconciliation"
            );
        }
        // Preserve completed evidence even if balance acquisition outlasted its
        // admission window. The store checks that window before new authority;
        // retrying this entire sweep here could hold the pool gate forever.
        Ok(ChainView {
            admission_valid_until: timestamp.saturating_add(self.max_age),
            anchor: confirmed,
            balances,
            released,
            resolutions,
        })
    }
}
fn quantity(v: &Value) -> Result<u64> {
    let s = v.as_str().context("invalid RPC quantity")?;
    Ok(u64::from_str_radix(
        s.strip_prefix("0x").context("invalid RPC quantity")?,
        16,
    )?)
}

fn validate_timestamp(timestamp: u64, clock: u64, max_age: u64) -> Result<()> {
    ensure!(
        timestamp <= clock.saturating_add(30) && clock.saturating_sub(timestamp) <= max_age,
        "stale Base block"
    );
    Ok(())
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
            let rpc = BaseRpc::new(&url,12,120).unwrap();
            let error = rpc.view(ChainQuery{wallets:vec![],pending:vec![],anchor:None}).with_subscriber(subscriber).await.err().unwrap();
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
    #[test]
    fn timestamp_boundaries_are_exact() {
        for (stamp, allowed) in [
            (1030, true),
            (1031, false),
            (880, true),
            (879, false),
            (1000, true),
        ] {
            assert_eq!(super::validate_timestamp(stamp, 1000, 120).is_ok(), allowed);
        }
    }
}
