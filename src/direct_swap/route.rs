//! General deposit-funded 1Click routes; managed funding retains its own policy.
use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{network::IsolationId, rotation::near::NearClient};

pub(crate) const ZEC: &str = "nep141:zec.omft.near";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Asset {
    #[serde(rename = "assetId")]
    pub asset_id: String,
    pub blockchain: String,
    pub decimals: u32,
    pub symbol: String,
    #[serde(flatten)]
    pub metadata: serde_json::Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum SwapType {
    #[serde(rename = "EXACT_INPUT")]
    ExactInput,
    #[serde(rename = "EXACT_OUTPUT")]
    ExactOutput,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DirectQuote {
    pub request: Value,
    pub response: Value,
    pub deposit: Option<String>,
    pub deposit_memo: Option<String>,
    pub deadline: u64,
    pub input: String,
    pub output: String,
    pub min_output: String,
}

pub(crate) fn atomic(value: &str) -> Result<U256> {
    ensure!(
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        "invalid atomic amount"
    );
    U256::from_str_radix(value, 10).map_err(|_| anyhow::anyhow!("atomic amount exceeds 256 bits"))
}

pub(crate) fn validate_assets(value: Value) -> Result<Vec<Asset>> {
    let assets: Vec<Asset> = serde_json::from_value(value)
        .map_err(|_| anyhow::anyhow!("invalid direct swap asset metadata"))?;
    let mut ids = std::collections::HashSet::new();
    for asset in &assets {
        ensure!(
            !asset.asset_id.is_empty()
                && !asset.blockchain.is_empty()
                && !asset.symbol.is_empty()
                && ids.insert(&asset.asset_id),
            "missing or ambiguous direct swap asset identity"
        );
        if asset.asset_id == ZEC {
            ensure!(
                asset.blockchain == "zec" && asset.decimals == 8,
                "ZEC metadata mismatch"
            );
        }
    }
    Ok(assets)
}

pub(crate) fn select_asset<'a>(assets: &'a [Asset], id: &str, chain: &str) -> Result<&'a Asset> {
    let mut matches = assets
        .iter()
        .filter(|a| a.asset_id == id && a.blockchain == chain);
    let asset = matches.next().context("unsupported asset ID and chain")?;
    ensure!(matches.next().is_none(), "ambiguous asset ID and chain");
    Ok(asset)
}

pub(crate) async fn assets(client: &NearClient, identity: &IsolationId) -> Result<Vec<Asset>> {
    validate_assets(
        client
            .direct_request(identity, reqwest::Method::GET, "/v0/tokens", None, &[])
            .await?,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn request(
    origin: &Asset,
    destination: &Asset,
    recipient: &str,
    refund: &str,
    amount: &str,
    swap_type: SwapType,
    slippage: u32,
    deadline: u64,
    dry: bool,
) -> Result<Value> {
    request_on(
        &zingolib::config::ChainType::Mainnet,
        origin,
        destination,
        recipient,
        refund,
        amount,
        swap_type,
        slippage,
        deadline,
        dry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn request_on(
    chain: &zingolib::config::ChainType,
    origin: &Asset,
    destination: &Asset,
    recipient: &str,
    refund: &str,
    amount: &str,
    swap_type: SwapType,
    slippage: u32,
    deadline: u64,
    dry: bool,
) -> Result<Value> {
    ensure!(
        origin.asset_id != destination.asset_id,
        "swap requires distinct assets"
    );
    let deadline = chrono::DateTime::from_timestamp(i64::try_from(deadline)?, 0)
        .context("invalid deadline")?
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let request = json!({"dry":dry,"swapType":swap_type,"originAsset":origin.asset_id,
        "destinationAsset":destination.asset_id,"depositType":"ORIGIN_CHAIN",
        "recipientType":"DESTINATION_CHAIN","refundType":"ORIGIN_CHAIN",
        "depositMode":if origin.blockchain == "stellar" { "MEMO" } else { "SIMPLE" },
        "amount":amount,"recipient":recipient,"refundTo":refund,"confidentiality":"public",
        "slippageTolerance":slippage,"deadline":deadline});
    validate_request(chain, &request)?;
    Ok(request)
}

fn shielded(chain: &zingolib::config::ChainType, address: &str) -> Result<()> {
    crate::rotation::near::require_shielded_refund(chain, address)
}

fn validate_request(chain: &zingolib::config::ChainType, request: &Value) -> Result<()> {
    let object = request
        .as_object()
        .context("invalid direct quote request")?;
    const FIELDS: &[&str] = &[
        "dry",
        "swapType",
        "originAsset",
        "destinationAsset",
        "depositType",
        "recipientType",
        "refundType",
        "depositMode",
        "amount",
        "recipient",
        "refundTo",
        "confidentiality",
        "slippageTolerance",
        "deadline",
    ];
    ensure!(
        object.len() == FIELDS.len() && FIELDS.iter().all(|k| object.contains_key(*k)),
        "unsupported direct quote fields; destination tags and messages are unsupported"
    );
    for field in ["originAsset", "destinationAsset", "recipient", "refundTo"] {
        ensure!(
            !text(request, field)?.trim().is_empty(),
            "missing direct quote address or asset"
        );
    }
    ensure!(
        request["originAsset"] != request["destinationAsset"],
        "swap requires distinct assets"
    );
    ensure!(
        request["depositType"] == "ORIGIN_CHAIN"
            && request["recipientType"] == "DESTINATION_CHAIN"
            && request["refundType"] == "ORIGIN_CHAIN"
            && request["depositMode"]
                .as_str()
                .is_some_and(|mode| matches!(mode, "SIMPLE" | "MEMO"))
            && request["confidentiality"]
                .as_str()
                .is_some_and(|mode| matches!(mode, "public" | "basic" | "advanced"))
            && request["dry"].is_boolean(),
        "unsupported direct quote policy"
    );
    ensure!(
        atomic(text(request, "amount")?)? > U256::ZERO,
        "zero swap amount"
    );
    ensure!(
        request["slippageTolerance"]
            .as_u64()
            .is_some_and(|n| n <= 10000),
        "invalid slippage"
    );
    timestamp(&request["deadline"])?;
    match request["swapType"].as_str() {
        Some("EXACT_INPUT") => {
            ensure!(
                request["destinationAsset"] == ZEC && request["originAsset"] != ZEC,
                "receive requires a foreign asset to ZEC route"
            );
            shielded(chain, text(request, "recipient")?)?;
        }
        Some("EXACT_OUTPUT") => {
            ensure!(
                request["originAsset"] == ZEC && request["destinationAsset"] != ZEC,
                "send requires a ZEC to foreign asset route"
            );
            shielded(chain, text(request, "refundTo")?)?;
        }
        _ => anyhow::bail!("unsupported direct swap type"),
    }
    Ok(())
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .with_context(|| format!("missing direct quote {field}"))
}
fn timestamp(value: &Value) -> Result<u64> {
    Ok(u64::try_from(
        chrono::DateTime::parse_from_rfc3339(
            value.as_str().context("missing direct quote deadline")?,
        )
        .map_err(|_| anyhow::anyhow!("invalid direct quote deadline"))?
        .timestamp(),
    )?)
}

fn check_echo(request: &Value, echo: &Value, accepting_quote: bool) -> Result<()> {
    let object = echo.as_object().context("missing direct quote echo")?;
    for (key, value) in request.as_object().context("invalid direct request")? {
        let matches = if key == "deadline" {
            // The service echoes RFC3339 at millisecond precision, including
            // requests supplied with a more precise timestamp.
            chrono::DateTime::parse_from_rfc3339(value.as_str().context("missing deadline")?)?
                .timestamp_millis()
                == chrono::DateTime::parse_from_rfc3339(text(echo, key)?)?.timestamp_millis()
        } else {
            object.get(key) == Some(value)
        };
        ensure!(matches, "direct quote binding mismatch: {key}");
    }
    for (key, value) in object {
        if request.get(key).is_some() {
            continue;
        }
        if !accepting_quote && matches!(key.as_str(), "appFees" | "insured" | "quoteWaitingTimeMs")
        {
            continue;
        }
        match key.as_str() {
            "quoteWaitingTimeMs" => ensure!(
                value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0),
                "invalid quote waiting metadata"
            ),
            "insured" => ensure!(value.is_boolean(), "invalid quote insurance metadata"),
            "appFees" => {
                let fees = value.as_array().context("invalid direct quote fees")?;
                for fee in fees {
                    ensure!(
                        fee["recipient"]
                            .as_str()
                            .is_some_and(|recipient| !recipient.is_empty())
                            && fee["fee"]
                                .as_f64()
                                .is_some_and(|n| n.is_finite() && n >= 0.0),
                        "invalid direct quote fee metadata"
                    );
                }
            }
            _ => ensure!(value.is_null(), "unsupported direct quote routing metadata"),
        }
    }
    Ok(())
}

// Avoid overflowing U256 when computing floor(amount * basis_points / 10000).
fn fraction(amount: U256, basis_points: u64) -> U256 {
    let scale = U256::from(10000);
    amount / scale * U256::from(basis_points) + amount % scale * U256::from(basis_points) / scale
}

#[cfg(test)]
pub(crate) fn validate_quote(request: Value, response: Value, now: u64) -> Result<DirectQuote> {
    validate_quote_on(
        &zingolib::config::ChainType::Mainnet,
        request,
        response,
        now,
    )
}

pub(crate) fn validate_quote_on(
    chain: &zingolib::config::ChainType,
    request: Value,
    response: Value,
    now: u64,
) -> Result<DirectQuote> {
    validate_request(chain, &request)?;
    check_echo(&request, &response["quoteRequest"], true)?;
    let q = &response["quote"];
    let input = text(q, "amountIn")?.to_owned();
    let output = text(q, "amountOut")?.to_owned();
    let min_output = text(q, "minAmountOut")?.to_owned();
    let (input_n, output_n, min_n) = (atomic(&input)?, atomic(&output)?, atomic(&min_output)?);
    let amount = atomic(text(&request, "amount")?)?;
    ensure!(
        input_n > U256::ZERO && output_n > U256::ZERO && min_n > U256::ZERO && min_n <= output_n,
        "invalid direct quote amounts"
    );
    if request["swapType"] == "EXACT_INPUT" {
        ensure!(input_n == amount, "direct quote input mismatch");
        let slippage = request["slippageTolerance"]
            .as_u64()
            .context("missing slippage")?;
        ensure!(
            min_n >= fraction(output_n, 10000 - slippage),
            "direct quote minimum exceeds slippage"
        );
    } else {
        ensure!(
            output_n == amount && min_n == amount,
            "direct quote exact output mismatch"
        );
        let min_input = atomic(text(q, "minAmountIn")?)?;
        ensure!(
            min_input > U256::ZERO && min_input <= input_n,
            "invalid direct quote minimum input"
        );
    }
    for field in [
        "virtualChainRecipient",
        "virtualChainRefundRecipient",
        "customRecipientMsg",
        "recipientMemo",
        "destinationMemo",
        "destinationTag",
    ] {
        ensure!(
            q.get(field).is_none_or(Value::is_null),
            "unsupported destination tag or message"
        );
    }
    let dry = request["dry"] == true;
    let (deposit, deposit_memo, deadline) = if dry {
        ensure!(
            q.get("depositAddress").is_none_or(Value::is_null)
                && q.get("depositMemo").is_none_or(Value::is_null),
            "dry quote allocated deposit instructions"
        );
        (None, None, timestamp(&request["deadline"])?)
    } else {
        let address = text(q, "depositAddress")?;
        ensure!(!address.trim().is_empty(), "empty direct quote deposit");
        let memo = q
            .get("depositMemo")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .context("invalid direct quote deposit memo")
                    .map(str::to_owned)
            })
            .transpose()?;
        if request["depositMode"] == "MEMO" {
            ensure!(
                memo.as_deref().is_some_and(|memo| !memo.is_empty()),
                "memo deposit quote omitted mandatory memo"
            );
        }
        if request["originAsset"] == ZEC {
            ensure!(
                matches!(
                    zcash_keys::address::Address::decode(chain, address),
                    Some(zcash_keys::address::Address::Transparent(_))
                ),
                "ZEC source deposit requires a transparent address on the configured network"
            );
            ensure!(memo.is_none(), "ZEC source deposit memo unsupported");
        }
        (Some(address.to_owned()), memo, timestamp(&q["deadline"])?)
    };
    ensure!(deadline > now, crate::rotation::near::QuoteWindowExpired);
    Ok(DirectQuote {
        request,
        response,
        deposit,
        deposit_memo,
        deadline,
        input,
        output,
        min_output,
    })
}

pub(crate) async fn quote(
    client: &NearClient,
    identity: &IsolationId,
    request: Value,
    now: u64,
) -> Result<DirectQuote> {
    validate_request(&client.chain(), &request)?;
    let started = std::time::Instant::now();
    let response = client
        .direct_request(
            identity,
            reqwest::Method::POST,
            "/v0/quote",
            Some(&request),
            &[],
        )
        .await?;
    validate_quote_on(
        &client.chain(),
        request,
        response,
        now.saturating_add(started.elapsed().as_secs()),
    )
}

pub(crate) fn validate_status(quote: &DirectQuote, value: &Value) -> Result<()> {
    let deposit = quote
        .deposit
        .as_deref()
        .context("dry quote has no status")?;
    let response = &value["quoteResponse"];
    ensure!(
        response["quote"]["depositAddress"].as_str() == Some(deposit),
        "direct status deposit mismatch"
    );
    let memo = response["quote"]
        .get("depositMemo")
        .filter(|v| !v.is_null());
    ensure!(
        memo.and_then(Value::as_str) == quote.deposit_memo.as_deref()
            && memo.is_none_or(Value::is_string),
        "direct status deposit memo mismatch"
    );
    check_echo(&quote.request, &response["quoteRequest"], false)?;
    // Amount and fee reports are advisory after acceptance. They cannot veto a
    // bound operation's outcome or authorize a replacement deposit.
    Ok(())
}

pub(crate) async fn status(
    client: &NearClient,
    identity: &IsolationId,
    quote: &DirectQuote,
) -> Result<Value> {
    let deposit = quote
        .deposit
        .as_deref()
        .context("dry quote has no status")?;
    let mut query = vec![("depositAddress", deposit)];
    if let Some(memo) = quote.deposit_memo.as_deref() {
        query.push(("depositMemo", memo));
    }
    let response = client
        .direct_request(identity, reqwest::Method::GET, "/v0/status", None, &query)
        .await?;
    validate_status(quote, &response)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::near::PUBLIC_REFUND_UA;

    fn asset(id: &str, chain: &str, decimals: u32) -> Asset {
        Asset {
            asset_id: id.into(),
            blockchain: chain.into(),
            decimals,
            symbol: "TEST".into(),
            metadata: Default::default(),
        }
    }
    fn receive(dry: bool) -> Value {
        request(
            &asset("foreign:asset", "foreign", 18),
            &asset(ZEC, "zec", 8),
            PUBLIC_REFUND_UA,
            "foreign-refund",
            "184467440737095516160",
            SwapType::ExactInput,
            100,
            2000,
            dry,
        )
        .unwrap()
    }
    fn response(request: &Value) -> Value {
        let mut echo = request.clone();
        echo["insured"] = json!(false);
        echo["quoteWaitingTimeMs"] = json!(0);
        echo["appFees"] = json!([{"recipient":"5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd","fee":20}]);
        let mut value = json!({"quoteRequest":echo,"quote":{
            "amountIn":request["amount"],"amountOut":"100000", "minAmountOut":"99000"}});
        if request["dry"] == false {
            value["quote"]["depositAddress"] = json!("service-validated-foreign-address");
            value["quote"]["depositMemo"] = json!("mandatory-123");
            value["quote"]["deadline"] = json!("1970-01-01T00:40:00Z");
        }
        value
    }
    #[test]
    fn service_millisecond_deadline_echo_and_default_metadata_are_accepted() {
        // Public unfunded 2026-11-07 dry quote, reduced to the authored
        // millisecond-normalization/default-metadata compatibility regression.
        let request: Value = serde_json::from_str(r#"{"dry":true,"swapType":"EXACT_INPUT","originAsset":"nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near","destinationAsset":"nep141:zec.omft.near","depositType":"ORIGIN_CHAIN","recipientType":"DESTINATION_CHAIN","refundType":"ORIGIN_CHAIN","depositMode":"SIMPLE","amount":"10000000","recipient":"u1twhzfnwvps8vlv20jak2lud8x3j5y9p42unjrq69kvhlsu3yp88e8m4jn9r53369su3mx2e9hjan5kl4pwqkfgetam7v7rzwfduu2q6dp90um6al5zjyk3akns2923va5xyn92f6f5neq9amdshzpj67n2e9ez94qj36hgagpv5ltwk7","refundTo":"0x1111111111111111111111111111111111111111","confidentiality":"public","slippageTolerance":100,"deadline":"2026-11-07T01:11:25.017787+00:00"}"#).unwrap();
        let response: Value = serde_json::from_str(r#"{"quote":{"amountIn":"10000000","amountOut":"719790","minAmountOut":"712592"},"quoteRequest":{"dry":true,"depositMode":"SIMPLE","swapType":"EXACT_INPUT","slippageTolerance":100,"originAsset":"nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near","depositType":"ORIGIN_CHAIN","destinationAsset":"nep141:zec.omft.near","amount":"10000000","refundTo":"0x1111111111111111111111111111111111111111","refundType":"ORIGIN_CHAIN","recipient":"u1twhzfnwvps8vlv20jak2lud8x3j5y9p42unjrq69kvhlsu3yp88e8m4jn9r53369su3mx2e9hjan5kl4pwqkfgetam7v7rzwfduu2q6dp90um6al5zjyk3akns2923va5xyn92f6f5neq9amdshzpj67n2e9ez94qj36hgagpv5ltwk7","recipientType":"DESTINATION_CHAIN","deadline":"2026-11-07T01:11:25.017Z","confidentiality":"public","quoteWaitingTimeMs":0,"insured":false,"appFees":[{"recipient":"5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd","fee":20}]}}"#).unwrap();
        let quote = validate_quote(request, response, 0).unwrap();
        assert_eq!(quote.output, "719790");
        assert_eq!(quote.response["quoteRequest"]["appFees"][0]["fee"], 20);
        assert!(quote.deposit.is_none());
    }

    #[test]
    fn service_fee_metadata_is_advisory_and_stellar_requires_a_deposit_memo() {
        let mut request = receive(false);
        request["depositMode"] = json!("MEMO");
        request["confidentiality"] = json!("advanced");
        let mut response = response(&request);
        response["quoteRequest"]["appFees"] = json!([
            {"recipient":"service-fee-a","fee":12000,"limitOrderId":null},
            {"recipient":"service-fee-b","fee":0.5}]);
        response["quoteRequest"]["insured"] = json!(true);
        response["quoteRequest"]["quoteWaitingTimeMs"] = json!(3000);
        let quote = validate_quote(request.clone(), response.clone(), 1000).unwrap();
        let mut status = json!({"status":"SUCCESS", "quoteResponse":response.clone()});
        status["quoteResponse"]["quoteRequest"]["appFees"] = json!([]);
        assert!(validate_status(&quote, &status).is_ok());
        status["quoteResponse"]["quoteRequest"]["appFees"] = json!("unavailable report");
        assert!(validate_status(&quote, &status).is_ok());
        response["quote"]["depositMemo"] = Value::Null;
        assert!(validate_quote(request.clone(), response.clone(), 1000).is_err());
        response["quote"]["depositMemo"] = json!("memo");
        response["quoteRequest"]["appFees"][0]["fee"] = json!("wrong-type");
        assert!(validate_quote(request, response, 1000).is_err());
        let request = super::request(
            &asset("foreign:stellar", "stellar", 7),
            &asset(ZEC, "zec", 8),
            PUBLIC_REFUND_UA,
            "service-valid-refund",
            "10000000",
            SwapType::ExactInput,
            100,
            2000,
            false,
        )
        .unwrap();
        assert_eq!(request["depositMode"], "MEMO");
    }

    #[test]
    fn wide_amounts_are_decimal_checked_and_fraction_does_not_overflow() {
        assert_eq!(atomic(&U256::MAX.to_string()).unwrap(), U256::MAX);
        assert!(atomic(&format!("{}0", U256::MAX)).is_err());
        for invalid in ["", "-1", "+1", "0x10", "1.2", " 1"] {
            assert!(atomic(invalid).is_err());
        }
        assert_eq!(fraction(U256::MAX, 10000), U256::MAX);
        let request = receive(true);
        assert!(validate_quote(request.clone(), response(&request), 1000).is_ok());
    }
    #[test]
    fn catalog_retains_chain_decimals_and_rejects_ambiguity() {
        let value = json!([
            {"assetId":"asset:a","blockchain":"a","decimals":18,"symbol":"USDC","price":1},
            {"assetId":"asset:b","blockchain":"b","decimals":6,"symbol":"USDC"}]);
        let assets = validate_assets(value.clone()).unwrap();
        assert_eq!(select_asset(&assets, "asset:a", "a").unwrap().decimals, 18);
        assert_eq!(assets[0].metadata["price"], 1);
        assert!(select_asset(&assets, "asset:a", "b").is_err());
        assert!(validate_assets(json!([value[0], value[0]])).is_err());
    }
    #[test]
    fn receive_memo_and_fee_metadata_are_preserved_and_status_bound() {
        let request = receive(false);
        let response = response(&request);
        let quote = validate_quote(request, response.clone(), 1000).unwrap();
        assert_eq!(quote.deposit_memo.as_deref(), Some("mandatory-123"));
        assert_eq!(quote.response["quoteRequest"]["appFees"][0]["fee"], 20);
        let mut status = json!({"status":"SUCCESS", "quoteResponse":response});
        status["quoteResponse"]["quote"]["amountOut"] = json!("different-provider-report");
        assert!(validate_status(&quote, &status).is_ok());
        status["quoteResponse"]["quote"]["depositMemo"] = Value::Null;
        assert!(validate_status(&quote, &status).is_err());
        status["quoteResponse"]["quote"]["depositMemo"] = json!("mandatory-123");
        status["quoteResponse"]["quoteRequest"]["refundTo"] = json!("other-refund");
        assert!(validate_status(&quote, &status).is_err());
    }
    #[test]
    fn quote_rejects_substitutions_unsupported_tags_and_expired_disclosure() {
        let request = receive(false);
        let mut changed = response(&request);
        changed["quoteRequest"]["recipient"] = json!("other");
        assert!(validate_quote(request.clone(), changed, 1000).is_err());
        let mut changed = response(&request);
        changed["quoteRequest"]["customRecipientMsg"] = json!("tag");
        assert!(validate_quote(request.clone(), changed, 1000).is_err());
        let mut changed = response(&request);
        changed["quote"]["minAmountOut"] = json!("98999");
        assert!(validate_quote(request.clone(), changed, 1000).is_err());
        assert!(validate_quote(request.clone(), response(&request), 2400).is_err());
        let mut tagged = request.clone();
        tagged["recipientMemo"] = json!("tag");
        assert!(validate_request(&zingolib::config::ChainType::Mainnet, &tagged).is_err());
        let dry = receive(true);
        let mut changed = response(&dry);
        changed["quote"]["depositMemo"] = json!("memo");
        assert!(validate_quote(dry, changed, 1000).is_err());
    }
    #[test]
    fn send_exact_output_fixes_destination_and_uses_bounded_zec_deposit_contract() {
        let request = request(
            &asset(ZEC, "zec", 8),
            &asset("foreign:asset", "foreign", 18),
            "service-validated-recipient",
            PUBLIC_REFUND_UA,
            "184467440737095516160",
            SwapType::ExactOutput,
            100,
            2000,
            true,
        )
        .unwrap();
        let mut response = response(&request);
        response["quote"]["amountIn"] = json!("100000");
        response["quote"]["minAmountIn"] = json!("99000");
        response["quote"]["amountOut"] = request["amount"].clone();
        response["quote"]["minAmountOut"] = request["amount"].clone();
        assert!(validate_quote(request.clone(), response.clone(), 1000).is_ok());
        let mut executable = request.clone();
        executable["dry"] = json!(false);
        let mut executable_response = response.clone();
        executable_response["quoteRequest"]["dry"] = json!(false);
        executable_response["quote"]["deadline"] = json!("1970-01-01T00:40:00Z");
        executable_response["quote"]["depositAddress"] =
            json!("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F");
        assert!(validate_quote(executable.clone(), executable_response.clone(), 1000).is_ok());
        executable_response["quote"]["depositMemo"] = json!("memo");
        assert!(validate_quote(executable.clone(), executable_response.clone(), 1000).is_err());
        executable_response["quote"]["depositMemo"] = Value::Null;
        executable_response["quote"]["depositAddress"] = json!(PUBLIC_REFUND_UA);
        assert!(validate_quote(executable, executable_response, 1000).is_err());
        response["quote"]["amountOut"] = json!("1");
        assert!(validate_quote(request, response, 1000).is_err());
    }
}
