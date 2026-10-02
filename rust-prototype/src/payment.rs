//! A request snapshots its payer before the first HTTP attempt. Replacing the
//! payer affects only later calls; ambiguous paid failures are never replayed.
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

pub const USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
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
#[derive(Clone)]
pub struct PaidClient {
    pub http: reqwest::Client,
    payer: Arc<RwLock<Arc<Payer>>>,
}
impl PaidClient {
    pub fn new(http: reqwest::Client, payer: Payer) -> Self {
        Self {
            http,
            payer: Arc::new(RwLock::new(Arc::new(payer))),
        }
    }
    /// Share the wallet identity/policy while using a source-specific transport.
    pub fn with_http(&self, http: reqwest::Client) -> Self {
        Self {
            http,
            payer: self.payer.clone(),
        }
    }
    pub fn replace_payer(&self, payer: Payer) {
        *self.payer.write().expect("payer lock poisoned") = Arc::new(payer);
    }
    pub async fn execute(&self, route: RoutedRequest) -> Result<String> {
        let payer = self.payer.read().expect("payer lock poisoned").clone();
        let mut request = self.http.request(route.method.parse()?, &route.url);
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
        request = request.query(&query);
        if let Some(body) = route.body {
            request = request.json(&body);
        }
        let request = request.build()?;
        let retry = request
            .try_clone()
            .context("request body cannot be retried")?;
        let mut response = self.http.execute(request).await?;
        if response.status() == reqwest::StatusCode::PAYMENT_REQUIRED {
            if let Some(header) = response.headers().get("payment-required") {
                let mut challenge: Value =
                    serde_json::from_slice(&STANDARD.decode(header.as_bytes())?)?;
                if let Some(desc) = challenge.pointer_mut("/resource/description")
                    && let Some(text) = desc.as_str()
                {
                    *desc = Value::String(text.chars().take(500).collect());
                }
                response.headers_mut().insert(
                    "payment-required",
                    STANDARD.encode(serde_json::to_vec(&challenge)?).parse()?,
                );
            }
            // Selection (asset/network/cap) happens inside the SDK before signing.
            let headers = payer
                .client
                .make_payment_headers(response)
                .await
                .context("x402 challenge rejected or signing failed")?;
            let mut retry = retry;
            retry.headers_mut().extend(headers);
            response = self.http.execute(retry).await?;
        }
        let status = response.status();
        let detail = response
            .headers()
            .get("payment-required")
            .and_then(|h| STANDARD.decode(h.as_bytes()).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.get("error").cloned());
        let body = response.text().await?;
        if !status.is_success() {
            bail!("HTTP {status}: {} {body}", detail.unwrap_or(Value::Null));
        }
        Ok(body)
    }
}
