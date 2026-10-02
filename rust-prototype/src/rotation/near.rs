//! Foreign-chain confidential EXACT_OUTPUT swaps. No signer or wallet keys enter
//! this client. Its credential cannot follow redirects or reach another origin.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
const ORIGIN: &str = "https://1click.chaindefuser.com";
const ZEC: &str = "nep141:zec.omft.near";
const USDC: &str = "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
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
pub struct NearClient {
    http: reqwest::Client,
    origin: String,
}
impl NearClient {
    pub fn new(key: Option<&str>) -> Result<Self> {
        Self::at(ORIGIN, key)
    }
    fn at(origin: &str, key: Option<&str>) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = key {
            let mut value =
                reqwest::header::HeaderValue::from_str(key).context("invalid NEAR API key")?;
            value.set_sensitive(true);
            headers.insert("X-API-Key", value);
        }
        Ok(Self {
            origin: origin.into(),
            http: reqwest::Client::builder()
                .default_headers(headers)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()?,
        })
    }
    async fn response(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("near_unavailable"))?;
        ensure!(
            response.status().is_success(),
            "near_http_{}",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("near_response_failed"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= 2_000_000,
                "near_response_too_large"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid NEAR JSON"))
    }
    pub async fn assets(&self) -> Result<Assets> {
        validate_assets(
            &self
                .response(self.http.get(format!("{}/v0/tokens", self.origin)))
                .await?,
        )
    }
    pub async fn quote(&self, request: Value, limits: &Limits, now: u64) -> Result<Quote> {
        let response = self
            .response(
                self.http
                    .post(format!("{}/v0/quote", self.origin))
                    .json(&request),
            )
            .await?;
        validate_quote(request, response, limits, now)
    }
    pub async fn status(&self, quote: &Quote) -> Result<SwapStatus> {
        let deposit = quote
            .deposit
            .as_deref()
            .context("dry quote has no status")?;
        let v = self
            .response(
                self.http
                    .get(format!("{}/v0/status", self.origin))
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
        atomic(amount)? > 0 && matches!(confidentiality, "basic" | "advanced") && slippage <= 1000,
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
        ensure!(
            echo.get(key) == Some(value),
            "quote binding mismatch: {key}"
        );
    }
    for key in [
        "appFees",
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
    ensure!(
        deadline <= timestamp(&request["deadline"])?
            && deadline.checked_sub(now).is_some_and(|s| s >= 300),
        "quote deadline too close or extended"
    );
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
    #[tokio::test]
    async fn auth_errors_are_sanitized_and_redirects_are_not_followed() {
        let app = axum::Router::new()
            .route(
                "/v0/quote",
                axum::routing::post(|| async {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        include_str!("../../tests/fixtures/near/confidential-unauthorized.json"),
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
            Some("test-only-key"),
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
}
