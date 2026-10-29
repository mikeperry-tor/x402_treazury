//! Explicit public or confidential foreign-chain swaps. No signer or wallet keys enter
//! this client. Its credential cannot follow redirects or reach another origin.
/// HTTP response class, without provider prose or credentials.
#[derive(Debug)]
pub(crate) struct HttpStatus(pub u16);
impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "near_http_{}", self.0)
    }
}
impl std::error::Error for HttpStatus {}

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub const ORIGIN: &str = "https://1click.chaindefuser.com";
#[derive(Debug)]
pub(crate) struct QuoteWindowExpired;
impl std::fmt::Display for QuoteWindowExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("quote deadline has expired")
    }
}
impl std::error::Error for QuoteWindowExpired {}
const ZEC: &str = "nep141:zec.omft.near";
pub(crate) const USDC: &str = "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
#[derive(Clone, Serialize, Deserialize)]
pub struct Assets {
    pub origin: String,
    pub destination: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Quote {
    pub request: Value,
    pub response: Value,
    pub input: u64,
    pub deadline: u64,
    pub deposit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sizing_preview: Option<SizingPreview>,
}
/// Exact provider inputs used to size a fixed-input swap, retained with its quote.
#[derive(Clone, Serialize, Deserialize)]
pub struct SizingPreview {
    pub request: Value,
    pub response: Value,
}
impl Quote {
    pub fn output_atomic(&self) -> Result<u64> {
        let value = if self.request["swapType"] == "EXACT_INPUT" {
            &self.response["quote"]["amountOut"]
        } else {
            &self.request["amount"]
        };
        atomic(value.as_str().context("missing output amount")?)
    }
    pub fn sizing_output_atomic(&self) -> Result<u64> {
        if self.request["swapType"] == "EXACT_INPUT" {
            atomic(
                self.sizing_preview
                    .as_ref()
                    .context("missing sizing preview")?
                    .request["amount"]
                    .as_str()
                    .context("missing sizing output amount")?,
            )
        } else {
            self.output_atomic()
        }
    }
}
#[derive(Clone)]
pub struct Limits {
    pub max_output: u64,
    pub max_input: u64,
    pub max_fee: u64,
    pub max_fee_bps: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapStatus {
    PendingDeposit,
    KnownDeposit,
    IncompleteDeposit,
    Processing,
    Success,
    Refunded,
    Failed,
    Unknown,
}
#[derive(Debug)]
struct BridgeMinimum(u64);
impl std::fmt::Display for BridgeMinimum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "near_bridge_minimum_atomic:{}", self.0)
    }
}
impl std::error::Error for BridgeMinimum {}
fn bridge_minimum(value: &Value) -> Option<u64> {
    let digits = value
        .get("message")?
        .as_str()?
        .strip_prefix("Amount is too low for bridge, try at least ")?;
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok().filter(|v| *v > 0)
}
#[derive(Clone)]
pub struct NearClient {
    headers: reqwest::header::HeaderMap,
    origin: String,
    address_network: AddressNetwork,
}
// Production bridge routes are mainnet-only. The synthetic chain is available
// solely in opted-in unit tests, and is scoped to one client (never global).
#[derive(Clone, Copy)]
enum AddressNetwork {
    Mainnet,
    #[cfg(all(test, feature = "zcash-testutils"))]
    Synthetic,
}
impl AddressNetwork {
    fn chain(self) -> zingolib::config::ChainType {
        match self {
            Self::Mainnet => crate::rotation::store::TreasuryNetwork::Mainnet.chain(),
            #[cfg(all(test, feature = "zcash-testutils"))]
            Self::Synthetic => crate::rotation::store::TreasuryNetwork::Synthetic.chain(),
        }
    }
    fn transparent(self, address: &str) -> Result<()> {
        ensure!(
            matches!(
                zcash_keys::address::Address::decode(&self.chain(), address),
                Some(zcash_keys::address::Address::Transparent(_))
            ),
            "Zcash source deposit requires a transparent address on the configured network"
        );
        Ok(())
    }
    fn refund(self, address: &str) -> Result<()> {
        use zcash_keys::address::Address;
        let supported = match Address::decode(&self.chain(), address) {
            Some(Address::Transparent(_)) => true,
            Some(Address::Unified(address)) => {
                address.orchard().is_some() || address.transparent().is_some()
            }
            _ => false,
        };
        ensure!(
            supported,
            "Zcash refund requires a transparent or Orchard receiver on the configured network"
        );
        Ok(())
    }
}
impl NearClient {
    #[cfg(all(test, feature = "zcash-testutils"))]
    pub(crate) fn synthetic_at(origin: &str) -> Result<Self> {
        let mut client = Self::test_at(origin)?;
        client.address_network = AddressNetwork::Synthetic;
        Ok(client)
    }
    pub(crate) fn validate_quote(
        &self,
        request: Value,
        response: Value,
        limits: &Limits,
        now: u64,
    ) -> Result<Quote> {
        validate_quote_on(self.address_network, request, response, limits, now)
    }
    pub(crate) fn validate_saved_quote(
        &self,
        quote: &Quote,
        limits: &Limits,
        now: u64,
    ) -> Result<Quote> {
        let mut checked =
            self.validate_quote(quote.request.clone(), quote.response.clone(), limits, now)?;
        ensure!(
            checked.input == quote.input
                && checked.deadline == quote.deadline
                && checked.deposit == quote.deposit,
            "saved quote facts mismatch"
        );
        if checked.request["swapType"] == "EXACT_INPUT" {
            let preview = quote
                .sizing_preview
                .as_ref()
                .context("missing sizing preview")?;
            ensure!(
                preview.request["dry"] == true
                    && preview.request["swapType"] == "EXACT_OUTPUT"
                    && preview.request["slippageTolerance"] == 0,
                "invalid sizing preview policy"
            );
            // This unsigned historical preview supplies sizing provenance, not a
            // second deadline for the accepted executable quote.
            let preview_time = timestamp(&preview.request["deadline"])?.saturating_sub(1);
            let sizing = validate_quote_structure(
                self.address_network,
                preview.request.clone(),
                preview.response.clone(),
                preview_time,
            )?;
            ensure!(
                sizing.output_atomic()? <= limits.max_output,
                super::error::FundingCondition::AmountLimit
            );
            let mut expected = sizing.request.clone();
            expected["dry"] = json!(false);
            expected["swapType"] = json!("EXACT_INPUT");
            expected["amount"] = json!(sizing.input.to_string());
            expected["slippageTolerance"] = checked.request["slippageTolerance"].clone();
            ensure!(
                expected == checked.request,
                "executable quote differs from sizing preview"
            );
            checked.sizing_preview = Some(preview.clone());
        } else {
            ensure!(quote.sizing_preview.is_none(), "unexpected sizing preview");
        }
        Ok(checked)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn request(
        &self,
        assets: &Assets,
        recipient: &str,
        refund: &str,
        amount: &str,
        confidentiality: &str,
        slippage: u32,
        deadline: u64,
        dry: bool,
    ) -> Result<Value> {
        request_on(
            self.address_network,
            assets,
            recipient,
            refund,
            amount,
            confidentiality,
            slippage,
            deadline,
            dry,
        )
    }

    #[cfg(test)]
    pub(crate) fn test_at(origin: &str) -> Result<Self> {
        Self::at(origin, None, None)
    }
    pub fn new(key: Option<&str>) -> Result<Self> {
        Self::with_session(key, None)
    }
    pub fn with_session(key: Option<&str>, session: Option<&str>) -> Result<Self> {
        Self::at(ORIGIN, key, session)
    }
    fn at(origin: &str, key: Option<&str>, session: Option<&str>) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = key {
            let mut value =
                reqwest::header::HeaderValue::from_str(key).context("invalid NEAR API key")?;
            value.set_sensitive(true);
            headers.insert("X-API-Key", value);
        }
        if let Some(session) = session {
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {session}"))
                .context("invalid NEAR user session")?;
            value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        Ok(Self {
            address_network: AddressNetwork::Mainnet,
            origin: origin.into(),
            headers,
        })
    }
    fn wallet_http(&self, request: &Value) -> Result<reqwest::Client> {
        let address = request["recipient"]
            .as_str()
            .context("network_identity_missing: quote recipient")?;
        crate::network::global().http(
            &crate::network::IsolationId::evm(address)?,
            &self.origin,
            None,
        )
    }
    async fn response(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let mut response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                tracing::debug!(
                    stage = "near_response_headers",
                    reason = if error.is_connect() {
                        "connection_timeout"
                    } else {
                        "read_inactivity"
                    },
                    "NEAR HTTP timeout; affected request failed"
                );
            }
            anyhow::anyhow!(
                "near_unavailable: {}",
                if error.is_timeout() {
                    "timeout"
                } else if error.is_connect() {
                    "connect"
                } else {
                    "transport"
                }
            )
        })?;
        let status = response.status();
        ensure!(
            status.is_success() || status.as_u16() == 400,
            HttpStatus(status.as_u16())
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            if error.is_timeout() {
                tracing::debug!(
                    stage = "near_response_body",
                    reason = "read_inactivity",
                    "NEAR HTTP timeout; affected request failed"
                );
            }
            anyhow::anyhow!(
                "near_response_failed: {}",
                if error.is_timeout() {
                    "timeout"
                } else {
                    "body transport"
                }
            )
        })? {
            if chunk.len() > 2_000_000usize.saturating_sub(bytes.len()) {
                tracing::warn!(
                    limit_bytes = 2_000_000,
                    "NEAR response exceeds fixed byte limit; content rejected"
                );
                anyhow::bail!(
                    "near_response_too_large: fixed 2000000-byte limit exceeded; content rejected"
                );
            }
            bytes.extend_from_slice(&chunk);
        }
        if status.as_u16() == 400 {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes)
                && let Some(minimum) = bridge_minimum(&value)
            {
                return Err(BridgeMinimum(minimum).into());
            }
            anyhow::bail!("near_http_400");
        }
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid NEAR JSON"))
    }
    pub async fn assets(&self) -> Result<Assets> {
        validate_assets(
            &self
                .response(
                    crate::network::discovery(&self.origin, None)?
                        .get(format!("{}/v0/tokens", self.origin))
                        .headers(self.headers.clone()),
                )
                .await?,
        )
    }
    pub async fn quote(&self, request: Value, limits: &Limits, now: u64) -> Result<Quote> {
        let started = std::time::Instant::now();
        let response = self
            .response(
                self.wallet_http(&request)?
                    .post(format!("{}/v0/quote", self.origin))
                    .headers(self.headers.clone())
                    .json(&request),
            )
            .await
            .inspect_err(|error| {
                if let Some(minimum) = error.downcast_ref::<BridgeMinimum>() {
                    tracing::debug!(
                        swap_type = if request["swapType"] == "EXACT_INPUT" {
                            "EXACT_INPUT"
                        } else {
                            "EXACT_OUTPUT"
                        },
                        minimum_atomic = minimum.0,
                        minimum_unit = if request["swapType"] == "EXACT_INPUT" {
                            "zatoshi"
                        } else {
                            "usdc_atomic"
                        },
                        "NEAR rejected the quote below its route minimum; no funds submitted"
                    );
                }
            })?;
        let quote = self.validate_quote(
            request,
            response,
            limits,
            now.saturating_add(started.elapsed().as_secs()),
        )?;
        log_quote(&quote, "acquisition");
        Ok(quote)
    }
    /// Size ZEC through an unsigned output preview, then obtain a fixed-input
    /// offer. Slippage is applied only to the executable offer.
    pub async fn quote_exact_input(
        &self,
        mut request: Value,
        limits: &Limits,
        now: u64,
        slippage: u32,
    ) -> Result<Quote> {
        let started = std::time::Instant::now();
        request["dry"] = json!(true);
        request["slippageTolerance"] = json!(0);
        let preview = self.quote_with_minimum(request, limits, now).await?;
        let mut request = preview.request.clone();
        request["dry"] = json!(false);
        request["swapType"] = json!("EXACT_INPUT");
        request["amount"] = json!(preview.input.to_string());
        request["slippageTolerance"] = json!(slippage);
        let now = now.saturating_add(started.elapsed().as_secs());
        let mut quote = self.quote(request, limits, now).await?;
        quote.sizing_preview = Some(SizingPreview {
            request: preview.request,
            response: preview.response,
        });
        self.validate_saved_quote(&quote, limits, now)
    }
    /// Only rejected, unsigned quotes may adjust output; every accepted quote
    /// still passes the complete route, echo, input-cost and fee-cap validation.
    pub async fn quote_with_minimum(
        &self,
        mut request: Value,
        limits: &Limits,
        now: u64,
    ) -> Result<Quote> {
        let started = std::time::Instant::now();
        ensure!(
            request["swapType"] == "EXACT_OUTPUT" && request["destinationAsset"] == USDC,
            "minimum adjustment requires exact-output Base USDC"
        );
        for attempt in 0..3 {
            ensure!(
                atomic(
                    request["amount"]
                        .as_str()
                        .context("missing output amount")?
                )? <= limits.max_output,
                super::error::FundingCondition::AmountLimit
            );
            match self
                .quote(
                    request.clone(),
                    limits,
                    now.saturating_add(started.elapsed().as_secs()),
                )
                .await
            {
                Ok(quote) => return Ok(quote),
                Err(error) => {
                    let Some(minimum) = error.downcast_ref::<BridgeMinimum>() else {
                        return Err(error);
                    };
                    let previous = atomic(
                        request["amount"]
                            .as_str()
                            .context("missing output amount")?,
                    )?;
                    ensure!(minimum.0 > previous, "near_bridge_minimum_not_increasing");
                    ensure!(
                        minimum.0 <= limits.max_output,
                        super::error::FundingCondition::AmountLimit
                    );
                    if attempt == 2 {
                        anyhow::bail!(
                            "near_bridge_minimum_unstable: three quote attempts exhausted; no funds submitted"
                        );
                    }
                    tracing::debug!(
                        previous_atomic = previous,
                        minimum_atomic = minimum.0,
                        "NEAR bridge minimum increased wallet funding target; source and fee caps still apply"
                    );
                    request["amount"] = json!(minimum.0.to_string());
                }
            }
        }
        unreachable!()
    }
    pub async fn status(&self, quote: &Quote) -> Result<SwapStatus> {
        let deposit = quote
            .deposit
            .as_deref()
            .context("dry quote has no status")?;
        let v = self
            .response(
                self.wallet_http(&quote.request)?
                    .get(format!("{}/v0/status", self.origin))
                    .headers(self.headers.clone())
                    .query(&[("depositAddress", deposit)]),
            )
            .await?;
        ensure!(
            v["quoteResponse"]["quote"]["depositAddress"].as_str() == Some(deposit),
            "status deposit mismatch"
        );
        check_echo(&quote.request, &v["quoteResponse"]["quoteRequest"])?;
        let status = match v["status"].as_str() {
            Some("PENDING_DEPOSIT") => SwapStatus::PendingDeposit,
            Some("KNOWN_DEPOSIT_TX") => SwapStatus::KnownDeposit,
            Some("INCOMPLETE_DEPOSIT") => SwapStatus::IncompleteDeposit,
            Some("PROCESSING") => SwapStatus::Processing,
            Some("SUCCESS") => SwapStatus::Success,
            Some("REFUNDED") => SwapStatus::Refunded,
            Some("FAILED") => SwapStatus::Failed,
            _ => SwapStatus::Unknown,
        };
        log_status(&v, status);
        Ok(status)
    }
}
pub fn validate_assets(v: &Value) -> Result<Assets> {
    let tokens = v.as_array().context("invalid token catalog")?;
    for (id, chain, decimals, symbol) in [(ZEC, "zec", 8, "ZEC"), (USDC, "base", 6, "USDC")] {
        let matches: Vec<_> = tokens.iter().filter(|t| t["assetId"] == id).collect();
        ensure!(matches.len() == 1, "required asset missing or ambiguous");
        let token = matches[0];
        ensure!(
            token["blockchain"] == chain
                && token["decimals"] == decimals
                && token["symbol"] == symbol,
            "asset metadata mismatch"
        );
        if chain == "base" {
            ensure!(
                token["contractAddress"]
                    .as_str()
                    .is_some_and(|a| a.eq_ignore_ascii_case(crate::payment::USDC)),
                "USDC contract mismatch"
            );
        } else {
            ensure!(
                token.get("contractAddress").is_none_or(Value::is_null),
                "ZEC must be native"
            );
        }
    }
    Ok(Assets {
        origin: ZEC.into(),
        destination: USDC.into(),
    })
}
#[allow(clippy::too_many_arguments)]
pub fn request(
    assets: &Assets,
    recipient: &str,
    refund: &str,
    amount: &str,
    confidentiality: &str,
    slippage: u32,
    deadline: u64,
    dry: bool,
) -> Result<Value> {
    request_on(
        AddressNetwork::Mainnet,
        assets,
        recipient,
        refund,
        amount,
        confidentiality,
        slippage,
        deadline,
        dry,
    )
}
#[allow(clippy::too_many_arguments)]
fn request_on(
    network: AddressNetwork,
    assets: &Assets,
    recipient: &str,
    refund: &str,
    amount: &str,
    confidentiality: &str,
    slippage: u32,
    deadline: u64,
    dry: bool,
) -> Result<Value> {
    ensure!(
        assets.origin == ZEC && assets.destination == USDC,
        "unsupported assets"
    );
    recipient
        .parse::<alloy_primitives::Address>()
        .context("invalid EVM recipient")?;
    network.refund(refund)?;
    ensure!(
        atomic(amount)? > 0
            && matches!(confidentiality, "public" | "basic" | "advanced")
            && slippage <= 1000,
        "invalid quote policy"
    );
    let deadline = chrono::DateTime::from_timestamp(i64::try_from(deadline)?, 0)
        .context("invalid deadline")?
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok(
        json!({"dry":dry,"swapType":"EXACT_OUTPUT","originAsset":assets.origin,"destinationAsset":assets.destination,"depositType":"ORIGIN_CHAIN","recipientType":"DESTINATION_CHAIN","refundType":"ORIGIN_CHAIN","depositMode":"SIMPLE","amount":amount,"recipient":recipient,"refundTo":refund,"confidentiality":confidentiality,"slippageTolerance":slippage,"deadline":deadline}),
    )
}
fn atomic(s: &str) -> Result<u64> {
    ensure!(
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
        "invalid atomic amount"
    );
    Ok(s.parse()?)
}
fn timestamp(v: &Value) -> Result<u64> {
    Ok(u64::try_from(
        chrono::DateTime::parse_from_rfc3339(v.as_str().context("missing deadline")?)?.timestamp(),
    )?)
}
fn check_echo(request: &Value, echo: &Value) -> Result<()> {
    for (key, value) in request.as_object().context("invalid request")? {
        let matches = if key == "deadline" {
            timestamp(value)? == timestamp(&echo[key])?
        } else {
            echo.get(key) == Some(value)
        };
        ensure!(matches, "quote binding mismatch: {key}");
    }
    for key in [
        "rebates",
        "virtualChainRecipient",
        "virtualChainRefundRecipient",
        "customRecipientMsg",
    ] {
        ensure!(
            echo.get(key)
                .is_none_or(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty)),
            "unsupported quote routing or fees"
        );
    }
    // 1Click injects its own platform fee even when we request no app fees.
    // Accept only the observed platform collector, not arbitrary commissions.
    if let Some(fees) = echo.get("appFees").filter(|v| !v.is_null()) {
        let fees = fees.as_array().context("invalid platform fees")?;
        ensure!(fees.len() <= 1, "unsupported fee recipients");
        for fee in fees {
            ensure!(
                fee["recipient"]
                    == "5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd"
                    && fee["fee"].as_u64().is_some_and(|n| n <= 10000)
                    && fee.get("limitOrderId").is_none_or(Value::is_null),
                "unsupported platform fee"
            );
        }
    }
    Ok(())
}
pub fn validate_quote(request: Value, response: Value, limits: &Limits, now: u64) -> Result<Quote> {
    validate_quote_on(AddressNetwork::Mainnet, request, response, limits, now)
}
fn validate_quote_on(
    network: AddressNetwork,
    request: Value,
    response: Value,
    limits: &Limits,
    now: u64,
) -> Result<Quote> {
    let quote = validate_quote_structure(network, request, response, now)?;
    validate_quote_costs(&quote, limits)?;
    Ok(quote)
}
fn validate_quote_structure(
    network: AddressNetwork,
    request: Value,
    response: Value,
    now: u64,
) -> Result<Quote> {
    // Rebuild policy even when validating persisted data, not just fresh requests.
    let assets = Assets {
        origin: ZEC.into(),
        destination: USDC.into(),
    };
    let text = |k| request[k].as_str().context("missing quote binding");
    let mut rebuilt = request_on(
        network,
        &assets,
        text("recipient")?,
        text("refundTo")?,
        text("amount")?,
        text("confidentiality")?,
        u32::try_from(
            request["slippageTolerance"]
                .as_u64()
                .context("missing slippage")?,
        )?,
        timestamp(&request["deadline"])?,
        request["dry"].as_bool().context("missing dry flag")?,
    )?;
    let exact_input = request["swapType"] == "EXACT_INPUT";
    if exact_input {
        rebuilt["swapType"] = json!("EXACT_INPUT");
    }
    ensure!(rebuilt == request, "unsupported quote request");
    check_echo(&request, &response["quoteRequest"])?;
    let q = &response["quote"];
    let input = atomic(q["amountIn"].as_str().context("missing input")?)?;
    let amount = atomic(text("amount")?)?;
    let target = atomic(q["amountOut"].as_str().context("missing output")?)?;
    ensure!(target > 0, "zero quote output");
    if exact_input {
        ensure!(input == amount, "quote input mismatch");
        let minimum = atomic(
            q["minAmountOut"]
                .as_str()
                .context("missing minimum output")?,
        )?;
        let slippage = request["slippageTolerance"]
            .as_u64()
            .context("missing slippage")?;
        let floor = u128::from(target) * u128::from(10000 - slippage) / 10000;
        ensure!(
            minimum > 0 && minimum <= target && u128::from(minimum) >= floor,
            "quote minimum output exceeds slippage"
        );
    } else {
        ensure!(target == amount, "quote output mismatch");
    }
    ensure!(input > 0, "zero quote input");
    ensure!(
        q.get("depositMemo").is_none_or(Value::is_null),
        "deposit memo unsupported"
    );
    let dry = request["dry"] == true;
    let deadline = if dry {
        timestamp(&request["deadline"])?
    } else {
        timestamp(&q["deadline"])?
    };
    // The provider can keep the deposit address active longer than requested.
    ensure!(deadline > now, QuoteWindowExpired);
    let deposit = if dry {
        ensure!(
            q.get("depositAddress").is_none_or(Value::is_null),
            "dry quote allocated deposit"
        );
        None
    } else {
        let address = q["depositAddress"].as_str().context("missing deposit")?;
        network.transparent(address)?;
        Some(address.into())
    };
    Ok(Quote {
        request,
        response,
        input,
        deadline,
        deposit,
        sizing_preview: None,
    })
}

/// Current spending policy belongs to the executable quote. Historical sizing
/// previews retain their bindings and nominal amount, not a second price gate.
fn validate_quote_costs(quote: &Quote, limits: &Limits) -> Result<()> {
    let request = &quote.request;
    let response = &quote.response;
    let q = &response["quote"];
    let input = quote.input;
    let target = quote.output_atomic()?;
    if request["swapType"] == "EXACT_OUTPUT" {
        ensure!(
            target <= limits.max_output,
            super::error::FundingCondition::AmountLimit
        );
    }
    ensure!(
        input > 0
            && input
                .checked_add(limits.max_fee)
                .is_some_and(|n| n <= limits.max_input),
        super::error::FundingCondition::InputLimit
    );
    // USD uses exact rational arithmetic (up to 18 fractional digits), never f64.
    let usd = q["amountInUsd"].as_str().context("missing USD input")?;
    let (whole, fraction) = usd.split_once('.').unwrap_or((usd, ""));
    ensure!(
        fraction.len() <= 18 && fraction.bytes().all(|b| b.is_ascii_digit()),
        "invalid USD amount"
    );
    let scale = 10u128.pow(fraction.len() as u32);
    let value = u128::from(atomic(whole)?)
        .checked_mul(scale)
        .and_then(|n| {
            n.checked_add(if fraction.is_empty() {
                0
            } else {
                fraction.parse().unwrap_or(u128::MAX)
            })
        })
        .context("USD overflow")?;
    let left = value
        .checked_mul(1_000_000)
        .and_then(|n| n.checked_mul(10000))
        .context("USD overflow")?;
    let right = u128::from(target)
        .checked_mul(scale)
        .and_then(|n| n.checked_mul(10000 + u128::from(limits.max_fee_bps)))
        .context("USD overflow")?;
    ensure!(
        limits.max_fee_bps <= 10000 && left <= right,
        "quote overhead limit"
    );
    if let Some(fees) = response["quoteRequest"]["appFees"].as_array() {
        let total: u64 = fees
            .iter()
            .map(|fee| fee["fee"].as_u64().unwrap_or(u64::MAX))
            .sum();
        ensure!(
            total <= u64::from(limits.max_fee_bps),
            "platform fee exceeds overhead cap"
        );
    }
    Ok(())
}

fn diagnostic_atomic(value: &Value) -> Option<u64> {
    atomic(value.as_str()?).ok()
}
fn diagnostic_id(value: &Value) -> Option<String> {
    uuid::Uuid::parse_str(value.as_str()?)
        .ok()
        .map(|id| id.to_string())
}
fn diagnostic_time(value: &Value) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}
fn refund_reason(value: &Value) -> Option<&'static str> {
    Some(match value.as_str()? {
        "AMOUNT_LESS_THAN_MIN_AMOUNT_OUT" => "AMOUNT_LESS_THAN_MIN_AMOUNT_OUT",
        "AMOUNT_MORE_THAN_BALANCE" => "AMOUNT_MORE_THAN_BALANCE",
        "INTENT_SUBMIT_FAILED" => "INTENT_SUBMIT_FAILED",
        "PARTIAL_DEPOSIT" => "PARTIAL_DEPOSIT",
        "CIRCUIT_BREAKER_BLOCKED" => "CIRCUIT_BREAKER_BLOCKED",
        "FEE_ESTIMATION_FAILED" => "FEE_ESTIMATION_FAILED",
        "NO_LIQUIDITY" => "NO_LIQUIDITY",
        "AMOUNT_LESS_THAN_MIN_WITHDRAWABLE" => "AMOUNT_LESS_THAN_MIN_WITHDRAWABLE",
        "INSUFFICIENT_BALANCE" => "INSUFFICIENT_BALANCE",
        "REFUND_ADD_FAILED" => "REFUND_ADD_FAILED",
        _ => "UNKNOWN",
    })
}
pub(crate) fn log_quote(quote: &Quote, stage: &'static str) {
    let response = &quote.response;
    let q = &response["quote"];
    let age = timestamp(&response["timestamp"]).ok().map(|time| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|now| now.as_secs().saturating_sub(time))
            .unwrap_or(0)
    });
    tracing::debug!(
        stage,
        correlation_id = ?diagnostic_id(&response["correlationId"]),
        provider_timestamp = ?diagnostic_time(&response["timestamp"]),
        requested_deadline = ?diagnostic_time(&quote.request["deadline"]),
        provider_deadline = ?diagnostic_time(&q["deadline"]),
        inactive_at = ?diagnostic_time(&q["timeWhenInactive"]),
        refund_fee_zatoshi = ?diagnostic_atomic(&q["refundFee"]),
        withdrawal_fee_usdc_atomic = ?diagnostic_atomic(&q["withdrawFee"]),
        quote_age_seconds = ?age,
        swap_type = if quote.request["swapType"] == "EXACT_INPUT" {"EXACT_INPUT"} else {"EXACT_OUTPUT"},
        dry = quote.request["dry"] == true,
        input_zatoshi = quote.input,
        output_usdc_atomic = ?diagnostic_atomic(&q["amountOut"]),
        minimum_output_usdc_atomic = ?diagnostic_atomic(&q["minAmountOut"]),
        slippage_bps = ?quote.request["slippageTolerance"].as_u64(),
        estimated_seconds = ?q["timeEstimate"].as_u64(),
        "NEAR quote response validated"
    );
}
fn log_status(response: &Value, status: SwapStatus) {
    let details = &response["swapDetails"];
    tracing::debug!(
        near_status = ?status,
        correlation_id = ?diagnostic_id(&response["quoteResponse"]["correlationId"]),
        provider_updated_at = ?diagnostic_time(&response["updatedAt"]),
        refund_reason = ?refund_reason(&details["refundReason"]),
        deposited_zatoshi = ?diagnostic_atomic(&details["depositedAmount"]),
        refunded_zatoshi = ?diagnostic_atomic(&details["refundedAmount"]),
        "NEAR provider status and refund diagnostics validated"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_socks as socks;

    #[tokio::test]
    async fn direct_near_timeouts_log_debug_without_private_url() {
        if crate::test_log_capture::run_in_child(
            "rotation::near::tests::direct_near_timeouts_log_debug_without_private_url",
        ) {
            return;
        }
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tracing::instrument::WithSubscriber;
        for body in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!(
                "http://{}/PRIVATE_PATH?key=PRIVATE_KEY",
                listener.local_addr().unwrap()
            );
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                }
                if body {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\n\r\n{")
                        .await
                        .unwrap();
                }
                std::future::pending::<()>().await;
            });
            let client = NearClient::test_at(&url).unwrap();
            let http =
                crate::network::discovery(&url, std::time::Duration::from_millis(100)).unwrap();
            let logs = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(logs.reopen().unwrap())
                .finish();
            let result = client
                .response(http.get(&url))
                .with_subscriber(subscriber)
                .await;
            let expected = if body {
                "near_response_failed: timeout"
            } else {
                "near_unavailable: timeout"
            };
            assert!(result.unwrap_err().to_string().contains(expected));
            let text = std::fs::read_to_string(logs.path()).unwrap();
            let stage = if body {
                "near_response_body"
            } else {
                "near_response_headers"
            };
            let event = text.lines().find(|line| line.contains(stage)).unwrap();
            assert!(
                event.contains("DEBUG") && event.contains("read_inactivity"),
                "{text}"
            );
            assert!(
                !text.contains("WARN") && !text.contains("PRIVATE_"),
                "{text}"
            );
            server.abort();
        }
    }

    #[cfg(feature = "zcash-testutils")]
    #[test]
    fn synthetic_address_codec_is_scoped_to_its_test_client() {
        let assets = Assets {
            origin: ZEC.into(),
            destination: USDC.into(),
        };
        let synthetic = NearClient::synthetic_at("http://127.0.0.1:1").unwrap();
        let ordinary = NearClient::test_at("http://127.0.0.1:1").unwrap();
        let recipient = "0x0000000000000000000000000000000000000003";
        let refund = "t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8";
        let request = synthetic
            .request(
                &assets,
                recipient,
                refund,
                "5000000",
                "public",
                100,
                2_000_007_200,
                false,
            )
            .unwrap();
        assert!(
            ordinary
                .request(
                    &assets,
                    recipient,
                    refund,
                    "5000000",
                    "public",
                    100,
                    2_000_007_200,
                    false
                )
                .is_err()
        );
        assert!(
            super::request(
                &assets,
                recipient,
                refund,
                "5000000",
                "public",
                100,
                2_000_007_200,
                false
            )
            .is_err()
        );
        let response = json!({"quoteRequest":request,"quote":{"amountIn":"50000","amountOut":"5000000","amountInUsd":"5","depositAddress":refund,"deadline":request["deadline"]}});
        let limits = Limits {
            max_input: 100_000,
            max_fee: 30_000,
            max_output: 5_000_000,
            max_fee_bps: 500,
        };
        synthetic
            .validate_quote(request.clone(), response.clone(), &limits, 2_000_000_000)
            .unwrap();
        assert!(
            ordinary
                .validate_quote(request.clone(), response.clone(), &limits, 2_000_000_000)
                .is_err()
        );
        assert!(validate_quote(request, response, &limits, 2_000_000_000).is_err());
        let addresses: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/state/sdk_v6_addresses.json"
        ))
        .unwrap();
        let address = zcash_keys::address::Address::decode(
            &zcash_protocol::consensus::Network::MainNetwork,
            addresses["after_next"][1]["encoded_address"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let regtest = address.encode(&crate::rotation::store::TreasuryNetwork::Synthetic.chain());
        let testnet = address.encode(&zcash_protocol::consensus::Network::TestNetwork);
        for (client, refund, accepted) in [
            (&synthetic, regtest.as_str(), true),
            (&synthetic, testnet.as_str(), false),
            (&ordinary, regtest.as_str(), false),
        ] {
            assert_eq!(
                client
                    .request(
                        &assets,
                        recipient,
                        refund,
                        "5000000",
                        "public",
                        100,
                        2_000_007_200,
                        false,
                    )
                    .is_ok(),
                accepted,
            );
        }
    }

    #[test]
    fn near_requests_use_recipient_and_discovery_isolation() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "rotation::near::tests::proxied_near_child",
            ])
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "*")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    #[ignore = "isolated process network policy; exercised by parent test"]
    async fn proxied_near_child() {
        use crate::network::{IsolationId, Mode, NetworkPolicy};
        let (request, response, limits) = fixture(false);
        let quote_response = response.clone();
        let status_response = json!({"status":"SUCCESS", "quoteResponse":response});
        let app = axum::Router::new()
            .route(
                "/v0/tokens",
                axum::routing::get(|| async {
                    axum::Json(
                        serde_json::from_str::<Value>(include_str!(
                            "../../tests/fixtures/near/tokens.json"
                        ))
                        .unwrap(),
                    )
                }),
            )
            .route(
                "/v0/quote",
                axum::routing::post(move || {
                    let v = quote_response.clone();
                    async move { axum::Json(v) }
                }),
            )
            .route(
                "/v0/status",
                axum::routing::get(move || {
                    let v = status_response.clone();
                    async move { axum::Json(v) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = socks::Socks::start(
            std::collections::BTreeMap::from([(
                "near.invalid".into(),
                listener.local_addr().unwrap(),
            )]),
            socks::Fault::None,
        )
        .await;
        crate::network::install(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            ..Default::default()
        })
        .unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = NearClient::at("http://near.invalid", None, None).unwrap();
        client.assets().await.unwrap();
        let quote = client
            .quote(request.clone(), &limits, 2_000_000_000)
            .await
            .unwrap();
        assert_eq!(client.status(&quote).await.unwrap(), SwapStatus::Success);
        // Reconstruct the client and quote as recovery does; recipient identity must persist.
        let recovered: Quote =
            serde_json::from_value(serde_json::to_value(&quote).unwrap()).unwrap();
        assert_eq!(
            NearClient::at("http://near.invalid", None, None)
                .unwrap()
                .status(&recovered)
                .await
                .unwrap(),
            SwapStatus::Success
        );
        let records = proxy.records.lock().unwrap();
        assert_eq!(records.len(), 2);
        for (record, id) in records.iter().zip([
            IsolationId::discovery("http://near.invalid").unwrap(),
            IsolationId::evm(request["recipient"].as_str().unwrap()).unwrap(),
        ]) {
            assert_eq!(record.password, crate::network::global().credentials(&id).1);
            assert_eq!(record.address_type, 3);
        }
        server.abort();
    }
    fn fixture(dry: bool) -> (Value, Value, Limits) {
        let assets = validate_assets(
            &serde_json::from_str(include_str!("../../tests/fixtures/near/tokens.json")).unwrap(),
        )
        .unwrap();
        let request = request(
            &assets,
            "0x0000000000000000000000000000000000000001",
            "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F",
            "5000000",
            "basic",
            100,
            2_000_001_000,
            dry,
        )
        .unwrap();
        let mut response = json!({"quoteRequest":request,"quote":{"amountIn":"100000","amountOut":"5000000","amountInUsd":"5.25"}});
        if !dry {
            response["quote"]["depositAddress"] = json!("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F");
            response["quote"]["deadline"] = request["deadline"].clone();
        }
        (
            request,
            response,
            Limits {
                max_output: u64::MAX,
                max_input: 200000,
                max_fee: 10000,
                max_fee_bps: 500,
            },
        )
    }

    fn input_fixture() -> (Quote, Limits) {
        let (mut request, mut response, mut limits) = fixture(true);
        request["slippageTolerance"] = json!(0);
        response["quoteRequest"] = request.clone();
        limits.max_output = 5_000_000;
        let preview = SizingPreview {
            request: request.clone(),
            response,
        };
        request["swapType"] = json!("EXACT_INPUT");
        request["dry"] = json!(false);
        request["amount"] = json!("100000");
        request["slippageTolerance"] = json!(250);
        let response = json!({"quoteRequest":request,"quote":{
            "amountIn":"100000","amountOut":"5000000","minAmountOut":"4875000",
            "amountInUsd":"5.25","depositAddress":"t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F",
            "deadline":request["deadline"]}});
        let mut quote = validate_quote(request, response, &limits, 2_000_000_000).unwrap();
        quote.sizing_preview = Some(preview);
        (quote, limits)
    }

    #[test]
    fn input_quotes_preserve_sizing_and_apply_slippage_once() {
        let client = NearClient::new(None).unwrap();
        let (quote, limits) = input_fixture();
        let checked = client
            .validate_saved_quote(&quote, &limits, 2_000_000_000)
            .unwrap();
        assert_eq!(checked.output_atomic().unwrap(), 5_000_000);
        assert_eq!(checked.sizing_output_atomic().unwrap(), 5_000_000);
        assert_eq!(checked.input, 100000);
        let mut better = quote.clone();
        better.response["quote"]["amountOut"] = json!("6000000");
        better.response["quote"]["minAmountOut"] = json!("5850000");
        assert!(
            client
                .validate_saved_quote(&better, &limits, 2_000_000_000)
                .is_ok(),
            "more USDC for the same authorized ZEC does not exceed sizing authority"
        );
        for (pointer, value) in [
            ("/response/quote/amountIn", json!("100001")),
            ("/response/quote/minAmountOut", json!("4874999")),
            ("/response/quote/minAmountOut", json!("5000001")),
            ("/response/quote/minAmountOut", Value::Null),
            ("/sizing_preview/request/amount", json!("6000000")),
            ("/sizing_preview/request/slippageTolerance", json!(250)),
        ] {
            let mut bad = serde_json::to_value(&quote).unwrap();
            *bad.pointer_mut(pointer).unwrap() = value;
            let bad = serde_json::from_value(bad).unwrap();
            assert!(
                client
                    .validate_saved_quote(&bad, &limits, 2_000_000_000)
                    .is_err(),
                "{pointer}"
            );
        }
        let mut missing = quote;
        missing.sizing_preview = None;
        assert!(
            client
                .validate_saved_quote(&missing, &limits, 2_000_000_000)
                .is_err()
        );
    }

    #[test]
    fn sizing_preview_does_not_extend_or_shorten_executable_authority() {
        let client = NearClient::new(None).unwrap();
        let (mut quote, limits) = input_fixture();
        let extended = 2_000_002_000;
        quote.response["quote"]["deadline"] = json!(
            chrono::DateTime::from_timestamp(extended, 0)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
        quote.deadline = extended as u64;
        assert!(
            client
                .validate_saved_quote(&quote, &limits, 2_000_001_500)
                .is_ok()
        );
        assert!(
            client
                .validate_saved_quote(&quote, &limits, extended as u64)
                .is_err()
        );
        let (request, response, limits) = fixture(false);
        let old = validate_quote(request, response, &limits, 2_000_000_000).unwrap();
        let value = serde_json::to_value(&old).unwrap();
        assert!(value.get("sizing_preview").is_none());
        let recovered: Quote = serde_json::from_value(value).unwrap();
        let checked = client
            .validate_saved_quote(&recovered, &limits, 2_000_000_000)
            .unwrap();
        assert_eq!(checked.request["slippageTolerance"], 100);
        assert_eq!(checked.request["swapType"], "EXACT_OUTPUT");
    }

    #[test]
    fn historical_sizing_cost_does_not_override_current_executable_policy() {
        let (mut quote, mut limits) = input_fixture();
        quote.sizing_preview.as_mut().unwrap().response["quoteRequest"]["appFees"] = json!([{
            "recipient":"5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd",
            "fee":500
        }]);
        // The preview cost 5.25 USD for 5 USDC under the former 5% policy.
        // The same fixed ZEC now returns 6 USDC and meets the current 3% policy.
        quote.response["quote"]["amountOut"] = json!("6000000");
        quote.response["quote"]["minAmountOut"] = json!("5850000");
        limits.max_fee_bps = 300;
        let client = NearClient::new(None).unwrap();
        assert!(
            client
                .validate_saved_quote(&quote, &limits, 2_000_000_000)
                .is_ok()
        );
        let mut bad = quote.clone();
        bad.response["quote"]["amountInUsd"] = json!("6.1800001");
        assert!(
            client
                .validate_saved_quote(&bad, &limits, 2_000_000_000)
                .is_err()
        );
        let mut bad = quote;
        bad.response["quoteRequest"]["appFees"] = json!([{
            "recipient":"5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd",
            "fee":301
        }]);
        assert!(
            client
                .validate_saved_quote(&bad, &limits, 2_000_000_000)
                .is_err()
        );
    }

    #[test]
    fn optional_diagnostics_are_typed_and_cannot_change_quote_authority() {
        let (mut quote, limits) = input_fixture();
        quote.response["correlationId"] = json!("PRIVATE_PROVIDER_TEXT");
        quote.response["timestamp"] = json!("PRIVATE_PROVIDER_TEXT");
        quote.response["quote"]["timeEstimate"] = json!("PRIVATE_PROVIDER_TEXT");
        assert!(
            NearClient::new(None)
                .unwrap()
                .validate_saved_quote(&quote, &limits, 2_000_000_000)
                .is_ok()
        );
        assert!(diagnostic_id(&quote.response["correlationId"]).is_none());
        assert!(diagnostic_time(&quote.response["timestamp"]).is_none());
        assert!(diagnostic_atomic(&json!("PRIVATE_PROVIDER_TEXT")).is_none());
        assert_eq!(
            refund_reason(&json!("PRIVATE_PROVIDER_TEXT")),
            Some("UNKNOWN")
        );
        assert_eq!(refund_reason(&json!("NO_LIQUIDITY")), Some("NO_LIQUIDITY"));
        assert_eq!(refund_reason(&Value::Null), None);
    }

    #[test]
    fn provider_diagnostics_log_safe_refund_codes_without_provider_prose() {
        if crate::test_log_capture::run_in_child(
            "rotation::near::tests::provider_diagnostics_log_safe_refund_codes_without_provider_prose",
        ) {
            return;
        }
        let logs = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(logs.reopen().unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let (mut quote, _) = input_fixture();
            quote.response["correlationId"] = json!("PRIVATE_PROVIDER_TEXT");
            quote.response["timestamp"] = json!("PRIVATE_PROVIDER_TEXT");
            quote.response["quote"]["timeEstimate"] = json!("PRIVATE_PROVIDER_TEXT");
            log_quote(&quote, "acquisition");
            let mut response = json!({"updatedAt":"PRIVATE_PROVIDER_TEXT","swapDetails":{
                "refundReason":"NO_LIQUIDITY","refundedAmount":"321000","depositedAmount":"321000",
                "refundAddress":"PRIVATE_PROVIDER_ADDRESS","refundTx":"PRIVATE_PROVIDER_TEXT"}});
            log_status(&response, SwapStatus::Refunded);
            response["swapDetails"]["refundReason"] = json!("PRIVATE_PROVIDER_TEXT");
            log_status(&response, SwapStatus::Refunded);
        });
        let text = std::fs::read_to_string(logs.path()).unwrap();
        assert!(text.contains("NO_LIQUIDITY"));
        assert!(text.contains("UNKNOWN"));
        assert!(text.contains("321000"));
        assert!(!text.contains("PRIVATE_PROVIDER"));
        assert!(!text.contains("t1XVX"));
        assert!(!text.contains(" WARN ") && !text.contains(" ERROR "));
    }

    #[tokio::test]
    async fn input_quote_sizes_each_offer_from_an_unbuffered_provider_preview() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests = seen.clone();
        let app = axum::Router::new().route(
            "/v0/quote",
            axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
                requests.lock().unwrap().push(request.clone());
                async move {
                    let mut response = json!({"quoteRequest":request,"quote":{
                        "amountIn":"321000","amountOut":request["amount"],"amountInUsd":"5.0"}});
                    if request["dry"] == false {
                        response["quote"]["amountOut"] = json!("4990000");
                        response["quote"]["minAmountOut"] = json!("4865250");
                        response["quote"]["depositAddress"] =
                            json!("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F");
                        response["quote"]["deadline"] = request["deadline"].clone();
                    }
                    axum::Json(response)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client =
            NearClient::test_at(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (request, _, mut limits) = fixture(false);
        limits.max_input = 400000;
        let quote = client
            .quote_exact_input(request, &limits, 2_000_000_000, 250)
            .await
            .unwrap();
        assert_eq!(quote.input, 321000);
        assert_eq!(quote.output_atomic().unwrap(), 4990000);
        assert_eq!(quote.sizing_output_atomic().unwrap(), 5000000);
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["swapType"], "EXACT_OUTPUT");
        assert_eq!(requests[0]["dry"], true);
        assert_eq!(requests[0]["amount"], "5000000");
        assert_eq!(requests[0]["slippageTolerance"], 0);
        assert_eq!(requests[1]["swapType"], "EXACT_INPUT");
        assert_eq!(requests[1]["dry"], false);
        assert_eq!(requests[1]["amount"], "321000");
        assert_eq!(requests[1]["slippageTolerance"], 250);
        server.abort();
    }

    #[tokio::test]
    async fn input_minimum_rejection_never_reinterprets_zatoshi_as_a_usdc_target() {
        if crate::test_log_capture::run_in_child(
            "rotation::near::tests::input_minimum_rejection_never_reinterprets_zatoshi_as_a_usdc_target",
        ) {
            return;
        }
        use tracing::instrument::WithSubscriber;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests = seen.clone();
        let app = axum::Router::new().route(
            "/v0/quote",
            axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
                requests.lock().unwrap().push(request.clone());
                async move {
                    if request["swapType"] == "EXACT_INPUT" {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::Json(json!({
                                "message":"Amount is too low for bridge, try at least 400000",
                                "detail":"PRIVATE_PROVIDER_TEXT"
                            })),
                        );
                    }
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(json!({"quoteRequest":request,"quote":{
                            "amountIn":"321000","amountOut":request["amount"],"amountInUsd":"5.0"
                        }})),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client =
            NearClient::test_at(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let logs = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(logs.reopen().unwrap())
            .finish();
        let (request, _, mut limits) = fixture(false);
        limits.max_input = 500000;
        let error = client
            .quote_exact_input(request, &limits, 2_000_000_000, 250)
            .with_subscriber(subscriber)
            .await
            .err()
            .unwrap();
        assert_eq!(error.downcast_ref::<BridgeMinimum>().unwrap().0, 400000);
        assert!(!error.to_string().contains("output"));
        let requests = seen.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "a rejected input offer cannot retry with a new USDC amount or allocate another deposit"
        );
        assert_eq!(requests[0]["dry"], true);
        assert_eq!(requests[0]["amount"], "5000000");
        assert_eq!(requests[1]["amount"], "321000");
        assert_eq!(requests[1]["swapType"], "EXACT_INPUT");
        let text = std::fs::read_to_string(logs.path()).unwrap();
        assert!(
            text.contains("EXACT_INPUT") && text.contains("zatoshi") && text.contains("400000")
        );
        assert!(text.contains("no funds submitted"));
        assert!(!text.contains("PRIVATE_PROVIDER_TEXT"));
        assert!(!text.contains(" WARN ") && !text.contains(" ERROR "));
        server.abort();
    }

    #[test]
    fn refund_receivers_preserve_exact_quote_binding_and_transparent_deposits() {
        use zcash_keys::address::{Address, UnifiedAddress};
        use zcash_protocol::consensus::Network::{MainNetwork, TestNetwork};
        let addresses: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/state/sdk_v6_addresses.json"
        ))
        .unwrap();
        let shielded = addresses["after_next"][1]["encoded_address"]
            .as_str()
            .unwrap();
        let Address::Unified(ua) = Address::decode(&MainNetwork, shielded).unwrap() else {
            panic!("public unified fixture")
        };
        assert!(ua.orchard().is_some() && ua.sapling().is_some());
        let orchard_only =
            UnifiedAddress::from_receivers(ua.orchard().cloned(), None, None, None, None)
                .unwrap()
                .encode(&MainNetwork);
        let transparent = "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F";
        let Address::Transparent(t) = Address::decode(&MainNetwork, transparent).unwrap() else {
            panic!("public transparent fixture")
        };
        let mixed_orchard =
            UnifiedAddress::from_receivers(ua.orchard().cloned(), None, Some(t), None, None)
                .unwrap()
                .encode_receiver_preserving(&MainNetwork);
        let mixed_sapling =
            UnifiedAddress::from_receivers(None, ua.sapling().cloned(), Some(t), None, None)
                .unwrap()
                .encode_receiver_preserving(&MainNetwork);
        for mixed in [&mixed_orchard, &mixed_sapling] {
            let Address::Unified(decoded) = Address::decode(&MainNetwork, mixed).unwrap() else {
                panic!("mixed unified fixture")
            };
            assert_eq!(decoded.transparent(), Some(&t));
        }
        for refund in [
            transparent,
            orchard_only.as_str(),
            shielded,
            &mixed_orchard,
            &mixed_sapling,
        ] {
            let (mut request, mut response, limits) = fixture(false);
            request["refundTo"] = json!(refund);
            response["quoteRequest"] = request.clone();
            let quote = validate_quote(request.clone(), response.clone(), &limits, 2_000_000_000)
                .unwrap_or_else(|error| panic!("public refund fixture {refund}: {error}"));
            assert_eq!(quote.request, request);
            let mut mismatched = response.clone();
            mismatched["quoteRequest"]["refundTo"] = json!("different refund");
            assert!(validate_quote(request.clone(), mismatched, &limits, 2_000_000_000).is_err());
            response["quote"]["depositAddress"] = json!(shielded);
            assert!(
                validate_quote(request, response, &limits, 2_000_000_000).is_err(),
                "shielded refund support cannot widen source deposits"
            );
        }
        let sapling_only =
            UnifiedAddress::from_receivers(None, ua.sapling().cloned(), None, None, None)
                .unwrap()
                .encode(&MainNetwork);
        for unsupported in [
            ua.encode(&TestNetwork),
            Address::Sapling(Box::new(ua.sapling().cloned().unwrap())).encode(&MainNetwork),
            sapling_only,
            "invalid".into(),
        ] {
            let (mut request, mut response, limits) = fixture(false);
            request["refundTo"] = json!(unsupported);
            response["quoteRequest"] = request.clone();
            assert!(validate_quote(request, response, &limits, 2_000_000_000).is_err());
        }
    }
    #[test]
    fn quotes_fail_closed_on_cost_route_and_binding_changes() {
        for dry in [true, false] {
            let (request, response, limits) = fixture(dry);
            let quote =
                validate_quote(request.clone(), response.clone(), &limits, 2_000_000_000).unwrap();
            assert_eq!(quote.input, 100000);
            for (pointer, value) in [
                ("/quoteRequest/confidentiality", json!("none")),
                (
                    "/quoteRequest/recipient",
                    json!("0x0000000000000000000000000000000000000002"),
                ),
                ("/quoteRequest/refundTo", json!("other")),
                ("/quote/amountInUsd", json!("5.250000000000000001")),
                ("/quote/amountIn", json!("190001")),
                ("/quote/amountOut", json!("4999999")),
            ] {
                let mut bad = response.clone();
                *bad.pointer_mut(pointer).unwrap() = value;
                assert!(
                    validate_quote(request.clone(), bad, &limits, 2_000_000_000).is_err(),
                    "{pointer}"
                );
            }
            let mut bad = response.clone();
            bad["quote"]["depositMemo"] = json!("1");
            assert!(validate_quote(request.clone(), bad, &limits, 2_000_000_000).is_err());
            assert!(validate_quote(request, response, &limits, 2_000_001_000).is_err());
        }
    }
    #[test]
    fn captured_public_quote_accepts_normalization_but_preserves_policy() {
        let response: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/near/public-quote.json"))
                .unwrap();
        let assets = Assets {
            origin: ZEC.into(),
            destination: USDC.into(),
        };
        let echo = &response["quoteRequest"];
        let deadline = timestamp(&echo["deadline"]).unwrap();
        let req = request(
            &assets,
            echo["recipient"].as_str().unwrap(),
            echo["refundTo"].as_str().unwrap(),
            "5000000",
            "public",
            100,
            deadline,
            false,
        )
        .unwrap();
        assert!(req.get("appFees").is_none());
        let limits = Limits {
            max_output: u64::MAX,
            max_input: 2_000_000,
            max_fee: 100_000,
            max_fee_bps: 500,
        };
        let quote =
            validate_quote(req.clone(), response.clone(), &limits, deadline - 1800).unwrap();
        assert_eq!(
            quote.deadline,
            timestamp(&response["quote"]["deadline"]).unwrap()
        );
        assert!(quote.deadline > deadline); // Returned preference may be longer.
        assert_eq!(quote.input, 369795);
        for (pointer, value) in [
            ("/quoteRequest/confidentiality", json!("basic")),
            (
                "/quoteRequest/appFees/0/recipient",
                json!("arbitrary-commission.near"),
            ),
            ("/quoteRequest/appFees/0/fee", json!(501)),
            ("/quoteRequest/deadline", json!("2099-01-01T00:00:00Z")),
        ] {
            let mut bad = response.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(
                validate_quote(req.clone(), bad, &limits, deadline - 1800).is_err(),
                "{pointer}"
            );
        }
        let mut confidential = req.clone();
        confidential["confidentiality"] = json!("basic");
        assert!(validate_quote(confidential, response, &limits, deadline - 1800).is_err());
    }
    #[tokio::test]
    async fn auth_errors_are_sanitized_and_redirects_are_not_followed() {
        let app = axum::Router::new()
            .route(
                "/v0/quote",
                axum::routing::post(|| async {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        "DUMMY_NEAR_KEY DUMMY_NEAR_SESSION DUMMY_UPSTREAM_DETAIL",
                    )
                }),
            )
            .route(
                "/v0/tokens",
                axum::routing::get(|| async {
                    axum::response::Redirect::temporary("http://127.0.0.1:1/credential-leak")
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = NearClient::at(
            &format!("http://{}", listener.local_addr().unwrap()),
            Some("DUMMY_NEAR_KEY"),
            Some("DUMMY_NEAR_SESSION"),
        )
        .unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (request, _, limits) = fixture(true);
        assert_eq!(
            client
                .quote(request, &limits, 2_000_000_000)
                .await
                .err()
                .unwrap()
                .to_string(),
            "near_http_401"
        );
        assert_eq!(
            client.assets().await.err().unwrap().to_string(),
            "near_http_307"
        );
        server.abort();
    }
    #[test]
    fn bridge_minimum_parser_accepts_only_positive_atomic_decimal_messages() {
        for text in [
            "0",
            "-1",
            "1.2",
            "+1",
            "1 trailing",
            "18446744073709551616",
            "",
        ] {
            assert_eq!(
                bridge_minimum(
                    &json!({"message":format!("Amount is too low for bridge, try at least {text}")})
                ),
                None
            );
        }
        assert_eq!(
            bridge_minimum(
                &json!({"message":"Amount is too low for bridge, try at least 2100000"})
            ),
            Some(2100000)
        );
    }
    #[tokio::test]
    async fn floating_minimum_quotes_are_bounded_and_revalidate_every_accepted_offer() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        for mode in [
            "floor",
            "raise",
            "twice",
            "unstable",
            "lower",
            "malformed",
            "input_cap",
            "output_cap",
            "fee_cap",
            "binding",
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let seen = calls.clone();
            let app = axum::Router::new().route("/v0/quote", axum::routing::post(move |axum::Json(req): axum::Json<Value>| {
                let index = seen.fetch_add(1, Ordering::SeqCst);
                async move {
                    let reject = mode != "floor" && (index == 0 || mode == "unstable" || (mode == "twice" && index == 1));
                    if reject {
                        let minimum = match mode {
                            "lower" => "4000000".to_string(),
                            "malformed" => "6000000 untrusted text".to_string(),
                            _ => ((6 + index) * 1_000_000).to_string(),
                        };
                        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({"message":format!("Amount is too low for bridge, try at least {minimum}")})));
                    }
                    let mut response = json!({"quoteRequest":req,"quote":{"amountIn":if mode=="input_cap" {"200001"} else {"100000"},"amountOut":req["amount"],"amountInUsd":if mode=="fee_cap" {"9.00"} else {"5.25"}}});
                    if mode == "binding" { response["quoteRequest"]["recipient"] = json!("0x0000000000000000000000000000000000000002"); }
                    (axum::http::StatusCode::OK, axum::Json(response))
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = NearClient::at(
                &format!("http://{}", listener.local_addr().unwrap()),
                None,
                None,
            )
            .unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let (req, _, mut limits) = fixture(true);
            if mode == "output_cap" {
                limits.max_output = 5_000_000;
            }
            let result = client.quote_with_minimum(req, &limits, 2_000_000_000).await;
            match mode {
                "floor" | "raise" | "twice" => assert_eq!(
                    result.unwrap().request["amount"],
                    match mode {
                        "floor" => "5000000",
                        "raise" => "6000000",
                        _ => "7000000",
                    }
                ),
                _ => assert!(result.is_err(), "{mode}"),
            }
            assert_eq!(
                calls.load(Ordering::SeqCst),
                match mode {
                    "floor" | "lower" | "malformed" | "output_cap" => 1,
                    "twice" | "unstable" => 3,
                    _ => 2,
                }
            );
            server.abort();
        }
    }
    mod boundaries {
        include!("../../tests/support/near_boundaries.rs");
    }
}
