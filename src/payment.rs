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
        // Pin static signer before any I/O; replacement affects only later calls.
        let payer = self
            .payer
            .as_ref()
            .map(|p| p.read().expect("payer lock poisoned").clone());
        let request = build_request(route)?;
        let unsigned_only = self.unsigned_only || crate::qualification::unsigned_case();
        let idempotent = matches!(request.method().as_str(), "GET" | "HEAD");
        let mut attempt = 0;
        let mut paid_submission = false;
        let mut extensions_omitted = false;
        // Only a pre-signing managed identity change can restart this loop.
        // The new unsigned challenge must use the new identity-bound transport.
        let (response, mut cover) = loop {
            let candidate = match &self.managed {
                Some(pool) if !unsigned_only => Some(pool.candidate().await?),
                _ => None,
            };
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
                        .pay_with_cover(
                            &http,
                            candidate.expect("managed candidate"),
                            retry,
                            response,
                            cover.as_ref(),
                        )
                        .await
                    {
                        Ok((paid, omitted)) => {
                            extensions_omitted = omitted;
                            paid_submission = true;
                            response = paid;
                        }
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
            bail!("HTTP {status}: {} {body}", detail.unwrap_or(Value::Null));
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
