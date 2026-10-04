//! Trusted Base RPC evidence at pinned blocks; no vendor response releases exposure.
use super::error::AdmissionError;
use crate::payment::USDC;
use alloy_primitives::{Address, B256, U256, keccak256};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
mod transport;
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
        ) {
            return message;
        }
    }
    "Base chain verification failed; unclassified validation error".into()
}

#[derive(Clone, Debug)]
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
pub struct ChainQuery {
    pub wallets: Vec<(String, String)>,
    pub pending: Vec<PendingAuthorization>,
    pub anchor: Option<Anchor>,
}
pub struct ChainView {
    pub anchor: Anchor,
    pub balances: BTreeMap<String, U256>,
    pub released: Vec<String>,
}
#[derive(Clone)]
pub struct BaseRpc {
    http: reqwest::Client,
    url: reqwest::Url,
    confirmations: u64,
    max_age: u64,
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
            http: crate::network::discovery(url, Duration::from_secs(15))?,
            url: secure_endpoint(url)?,
            confirmations,
            max_age,
        })
    }
    fn for_address(&self, address: &str) -> Result<Self> {
        Ok(Self {
            http: crate::network::global().http(
                &crate::network::IsolationId::evm(address)?,
                self.url.as_str(),
                Duration::from_secs(15),
            )?,
            url: self.url.clone(),
            confirmations: self.confirmations,
            max_age: self.max_age,
        })
    }
    async fn rpc(&self, method: &'static str, params: Value) -> Result<Value> {
        transport::rpc(
            &self.http,
            &self.url,
            method,
            params,
            crate::network::global().request_timeout(Duration::from_secs(15)),
        )
        .await
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
        let started = std::time::Instant::now();
        let result = self.view_inner(query).await;
        if let Err(error) = &result {
            tracing::warn!(category = %safe_diagnostic(error), elapsed_ms = started.elapsed().as_millis() as u64,
                "Base credit/payment chain verification failed");
        }
        result
    }
    async fn view_inner(&self, query: ChainQuery) -> Result<ChainView> {
        ensure!(
            quantity(&self.rpc("eth_chainId", json!([])).await?)? == 8453,
            "wrong Base chain"
        );
        if let Some(previous) = &query.anchor {
            let (canonical, _) = self.block(&format!("0x{:x}", previous.height)).await?;
            ensure!(
                canonical.height == previous.height && canonical.hash == previous.hash,
                AdmissionError::ChainRecoveryRequired("confirmed anchor changed")
            );
        }
        let (latest, timestamp) = self
            .block("latest")
            .await
            .context(VerificationStage("latest block"))?;
        let clock = now()?;
        validate_timestamp(timestamp, clock, self.max_age)?;
        let height = latest
            .height
            .checked_sub(self.confirmations)
            .context("insufficient Base confirmations")?;
        let (confirmed, confirmed_time) = self
            .block(&format!("0x{height:x}"))
            .await
            .context(VerificationStage("confirmed block"))?;
        ensure!(
            confirmed.height == height
                && confirmed_time <= timestamp
                && query.anchor.as_ref().is_none_or(|a| height >= a.height),
            "invalid confirmed block"
        );
        let mut balances = BTreeMap::new();
        for (id, address) in query.wallets {
            let address: Address = address.parse()?;
            let data = format!("0x70a08231{:0>64}", format!("{address:x}"));
            let scoped = self.for_address(&address.to_string())?;
            let stable = scoped
                .call(data.clone(), &confirmed)
                .await
                .context(VerificationStage("confirmed balance"))?;
            let current = scoped
                .call(data, &latest)
                .await
                .context(VerificationStage("latest balance"))?;
            balances.insert(id, stable.min(current));
        }
        let mut released = Vec::new();
        let selector = &keccak256("authorizationState(address,bytes32)")[..4];
        let selector = alloy_primitives::hex::encode(selector);
        for auth in query.pending {
            let payer: Address = auth.payer.parse()?;
            let nonce: B256 = auth.nonce.parse()?;
            let data = format!(
                "0x{selector}{:0>64}{}",
                format!("{payer:x}"),
                alloy_primitives::hex::encode(nonce)
            );
            let used = self
                .for_address(&auth.payer)?
                .call(data, &confirmed)
                .await
                .context(VerificationStage("authorization nonce"))?;
            ensure!(used <= U256::from(1), "invalid authorizationState result");
            if used == U256::from(1) || confirmed_time > auth.valid_before {
                released.push(auth.id);
            }
        }
        for anchor in [&confirmed, &latest] {
            let (end, _) = self.block(&format!("0x{:x}", anchor.height)).await?;
            ensure!(
                end.height == anchor.height && end.hash == anchor.hash,
                "Base view changed during reconciliation"
            );
        }
        ensure!(
            now()?.saturating_sub(timestamp) <= self.max_age,
            "stale Base block"
        );
        Ok(ChainView {
            anchor: confirmed,
            balances,
            released,
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
