//! Explicit public or confidential foreign-chain EXACT_OUTPUT swaps. No signer or wallet keys enter
//! this client. Its credential cannot follow redirects or reach another origin.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
pub const ORIGIN: &str = "https://1click.chaindefuser.com";
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
}
#[derive(Clone)]
pub struct Limits {
    pub max_output: u64,
    pub max_input: u64,
    pub max_fee: u64,
    pub max_fee_bps: u32,
}
#[derive(Debug, PartialEq, Eq)]
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
        write!(f, "near_bridge_minimum_output_atomic:{}", self.0)
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
pub struct NearClient {
    headers: reqwest::header::HeaderMap,
    origin: String,
}
impl NearClient {
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
            Duration::from_secs(30),
        )
    }
    async fn response(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let mut response = request.send().await.map_err(|error| {
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
            "near_http_{}",
            status.as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
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
                    crate::network::discovery(&self.origin, Duration::from_secs(30))?
                        .get(format!("{}/v0/tokens", self.origin))
                        .headers(self.headers.clone()),
                )
                .await?,
        )
    }
    pub async fn quote(&self, request: Value, limits: &Limits, now: u64) -> Result<Quote> {
        let response = self
            .response(
                self.wallet_http(&request)?
                    .post(format!("{}/v0/quote", self.origin))
                    .headers(self.headers.clone())
                    .json(&request),
            )
            .await?;
        validate_quote(request, response, limits, now)
    }
    /// Only rejected, unsigned quotes may adjust output; every accepted quote
    /// still passes the complete route, echo, input-cost and fee-cap validation.
    pub async fn quote_with_minimum(
        &self,
        mut request: Value,
        limits: &Limits,
        now: u64,
    ) -> Result<Quote> {
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
                "funding_amount_limit_exceeded"
            );
            match self.quote(request.clone(), limits, now).await {
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
                        "funding_amount_limit_exceeded"
                    );
                    if attempt == 2 {
                        anyhow::bail!(
                            "near_bridge_minimum_unstable: three quote attempts exhausted; no funds submitted"
                        );
                    }
                    tracing::warn!(
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
        Ok(match v["status"].as_str() {
            Some("PENDING_DEPOSIT") => SwapStatus::PendingDeposit,
            Some("KNOWN_DEPOSIT_TX") => SwapStatus::KnownDeposit,
            Some("INCOMPLETE_DEPOSIT") => SwapStatus::IncompleteDeposit,
            Some("PROCESSING") => SwapStatus::Processing,
            Some("SUCCESS") => SwapStatus::Success,
            Some("REFUNDED") => SwapStatus::Refunded,
            Some("FAILED") => SwapStatus::Failed,
            _ => SwapStatus::Unknown,
        })
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
    ensure!(
        assets.origin == ZEC && assets.destination == USDC,
        "unsupported assets"
    );
    recipient
        .parse::<alloy_primitives::Address>()
        .context("invalid EVM recipient")?;
    transparent(refund)?;
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
fn transparent(address: &str) -> Result<()> {
    ensure!(
        matches!(
            zcash_keys::address::Address::decode(&zcash_protocol::consensus::MAIN_NETWORK, address),
            Some(zcash_keys::address::Address::Transparent(_))
        ),
        "requires mainnet transparent address"
    );
    Ok(())
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
    // Rebuild policy even when validating persisted data, not just fresh requests.
    let assets = Assets {
        origin: ZEC.into(),
        destination: USDC.into(),
    };
    let text = |k| request[k].as_str().context("missing quote binding");
    let rebuilt = self::request(
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
    ensure!(rebuilt == request, "unsupported quote request");
    check_echo(&request, &response["quoteRequest"])?;
    let q = &response["quote"];
    let input = atomic(q["amountIn"].as_str().context("missing input")?)?;
    let target = atomic(text("amount")?)?;
    ensure!(target <= limits.max_output, "funding_amount_limit_exceeded");
    ensure!(
        input > 0
            && input
                .checked_add(limits.max_fee)
                .is_some_and(|n| n <= limits.max_input),
        "quote input limit"
    );
    ensure!(
        atomic(q["amountOut"].as_str().context("missing output")?)? == target,
        "quote output mismatch"
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
    // Our authority to send always expires at the earlier local/provider deadline.
    let deadline = deadline.min(timestamp(&request["deadline"])?);
    ensure!(
        deadline.checked_sub(now).is_some_and(|s| s >= 300),
        "quote deadline too close"
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
    let deposit = if dry {
        ensure!(
            q.get("depositAddress").is_none_or(Value::is_null),
            "dry quote allocated deposit"
        );
        None
    } else {
        let address = q["depositAddress"].as_str().context("missing deposit")?;
        transparent(address)?;
        Some(address.into())
    };
    Ok(Quote {
        request,
        response,
        input,
        deadline,
        deposit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_socks as socks;

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
            assert!(validate_quote(request, response, &limits, 2_000_000_701).is_err());
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
        assert_eq!(quote.deadline, deadline); // Provider deadline is three days later.
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
