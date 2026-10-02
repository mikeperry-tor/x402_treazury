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
    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let response = self
            .http
            .post(self.url.clone())
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("base_rpc_unavailable"))?;
        ensure!(response.status().is_success(), "base_rpc_unavailable");
        let value: Value = response
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("invalid Base RPC response"))?;
        ensure!(
            value.get("error").is_none() && value["id"] == 1 && value["jsonrpc"] == "2.0",
            "Base RPC request failed"
        );
        value
            .get("result")
            .filter(|v| !v.is_null())
            .cloned()
            .context("missing Base RPC result")
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
        let (latest, timestamp) = self.block("latest").await?;
        let clock = now()?;
        ensure!(
            timestamp <= clock.saturating_add(30)
                && clock.saturating_sub(timestamp) <= self.max_age,
            "stale Base block"
        );
        let height = latest
            .height
            .checked_sub(self.confirmations)
            .context("insufficient Base confirmations")?;
        let (confirmed, confirmed_time) = self.block(&format!("0x{height:x}")).await?;
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
            let stable = scoped.call(data.clone(), &confirmed).await?;
            let current = scoped.call(data, &latest).await?;
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
                .await?;
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
