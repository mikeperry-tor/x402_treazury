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
    timeout: std::time::Duration,
    payer: Option<Arc<RwLock<Arc<Payer>>>>,
    managed: Option<Arc<crate::rotation::manager::ManagedPool>>,
}
impl PaidClient {
    pub fn new(payer: Payer) -> Self {
        Self {
            timeout: std::time::Duration::from_secs(60),
            payer: Some(Arc::new(RwLock::new(Arc::new(payer)))),
            managed: None,
        }
    }
    pub fn managed(pool: Arc<crate::rotation::manager::ManagedPool>) -> Self {
        Self {
            timeout: std::time::Duration::from_secs(60),
            payer: None,
            managed: Some(pool),
        }
    }
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn timeout(&self) -> std::time::Duration {
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
        let payer = self
            .payer
            .as_ref()
            .map(|p| p.read().expect("payer lock poisoned").clone());
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
        let idempotent = matches!(request.method().as_str(), "GET" | "HEAD");
        let mut attempt = 0;
        let response = loop {
            let candidate = match &self.managed {
                Some(pool) => Some(pool.candidate().await?),
                None => None,
            };
            let address = candidate
                .as_ref()
                .map(|c| c.address.as_str())
                .or_else(|| payer.as_ref().map(|p| p.address.as_str()))
                .context("network_identity_missing: payer")?;
            let http = crate::network::global().http(
                &crate::network::IsolationId::evm(address)?,
                request.url().as_str(),
                self.timeout,
            )?;
            let unsigned = request
                .try_clone()
                .context("request body cannot be retried")?;
            let retry = request
                .try_clone()
                .context("request body cannot be retried")?;
            let mut response = http.execute(unsigned).await?;
            if response.status() == reqwest::StatusCode::PAYMENT_REQUIRED {
                if let Some(pool) = &self.managed {
                    match pool
                        .pay(
                            &http,
                            candidate.expect("managed candidate"),
                            retry,
                            response,
                        )
                        .await
                    {
                        Ok(paid) => response = paid,
                        Err(error)
                            if error.downcast_ref::<crate::rotation::error::AdmissionError>()
                                == Some(&crate::rotation::error::AdmissionError::PayerChanged)
                                && idempotent
                                && attempt == 0 =>
                        {
                            attempt += 1;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                } else {
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
                        .as_ref()
                        .expect("static payer")
                        .client
                        .make_payment_headers(response)
                        .await
                        .context("x402 challenge rejected or signing failed")?;
                    let mut retry = retry;
                    retry.headers_mut().extend(headers);
                    response = http.execute(retry).await?;
                }
            }
            break response;
        };
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
