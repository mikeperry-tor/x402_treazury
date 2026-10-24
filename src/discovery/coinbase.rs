//! Curated resource search with shared ranked-search inputs.
use super::{Manager, import};
use crate::{network, output::ToolOutput};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(super) const SEARCH_URL: &str =
    "https://api.cdp.coinbase.com/platform/v2/x402/discovery/search";

pub(super) fn search_branch() -> Value {
    // Share ranked-search controls with x402 List; provider-specific choices stay internal.
    let ranked = &super::directory::tool("/best").input_schema["properties"];
    let mut schema = json!({
        "properties": {
            "q": {"type":"string", "maxLength":400, "description":"Optional capability keywords."},
            "limit": ranked["limit"],
            "max_price_usd": ranked["max_price_usd"],
            "directory": {"const":"coinbase"},
            "mode": {"const":"best"},
            "local_only": {"const":false}
        },
        "required":["directory"],
        "additionalProperties":false
    });
    schema["properties"]["limit"]["description"] =
        json!("Maximum ranked results, 1–20; default 5.");
    schema["properties"]["max_price_usd"]["description"] =
        json!("Advisory USD price filter, not a payment cap.");
    schema
}

/// Validate the exact branch before acquiring identities or making requests.
pub(super) fn request(args: &Map<String, Value>) -> Result<reqwest::Url> {
    let schema = search_branch();
    let properties = schema["properties"].as_object().unwrap();
    for (name, value) in args {
        let field = properties.get(name).with_context(|| {
            format!("coinbase_search argument {name} is unsupported in curated search")
        })?;
        validate(name, value, field)?;
    }
    let mut url = reqwest::Url::parse(SEARCH_URL)?;
    let mut query = url.query_pairs_mut();
    if let Some(value) = args.get("q") {
        query.append_pair("query", value.as_str().unwrap());
    }
    if let Some(value) = args.get("max_price_usd") {
        // Preserve the accepted JSON number when adapting to the upstream string field.
        query.append_pair("maxUsdPrice", &value.as_number().unwrap().to_string());
    }
    query.append_pair("curatedOnly", "true");
    query.append_pair(
        "limit",
        &args
            .get("limit")
            .unwrap_or(&schema["properties"]["limit"]["default"])
            .to_string(),
    );
    query.append_pair("network", "eip155:8453");
    query.append_pair("asset", "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
    query.append_pair("scheme", "exact");
    drop(query);
    Ok(url)
}

fn validate(name: &str, value: &Value, field: &Value) -> Result<()> {
    if let Some(expected) = field.get("const") {
        ensure!(
            value == expected,
            "coinbase_search argument {name} must be {expected}"
        );
        return Ok(());
    }
    let kind = field["type"].as_str().unwrap();
    let valid = match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_u64().is_some(),
        _ => false,
    };
    ensure!(valid, "coinbase_search argument {name} must be {kind}");
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        ensure!(
            field["minLength"].as_u64().is_none_or(|min| length >= min)
                && field["maxLength"].as_u64().is_none_or(|max| length <= max),
            "coinbase_search argument {name} is outside documented character bounds"
        );
    }
    if let Some(number) = value.as_f64() {
        ensure!(
            field["minimum"].as_f64().is_none_or(|min| number >= min)
                && field["maximum"].as_f64().is_none_or(|max| number <= max),
            "coinbase_search argument {name} is outside documented bounds"
        );
    }
    Ok(())
}

pub(super) fn parse_response(bytes: &[u8]) -> Result<Value> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        anyhow::Error::new(crate::discovery_relay::RelayFailure::rejected_response())
            .context("coinbase_search response invalid_json")
    })?;
    ensure!(
        value.is_object()
            && value["resources"]
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_object))
            && value["partialResults"].is_boolean(),
        crate::discovery_relay::RelayFailure::rejected_response()
    );
    Ok(value)
}

fn output(response: Value, via: &str) -> Result<ToolOutput> {
    Ok(ToolOutput::text(serde_json::to_string(&json!({
        "directory":"coinbase", "acquisition":{"via":via}, "response":response,
        "guidance":"Resource metadata, not registered tools. Use exact documented HTTP requests with raw_call, published OpenAPI URLs with source_add, or optional usage URLs with source_details. Never guess missing methods/arguments or infer API bases; MCP/unknown protocols remain metadata only. Partial/empty results do not prove absence. Price/curation does not guarantee delivery. Remote search may charge; do not automatically repeat failed paid calls."
    }))?))
}

impl Manager {
    pub(super) async fn invoke_coinbase_search(
        self: &Arc<Self>,
        owner: &str,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput> {
        ensure!(self.enabled(owner), "source_management_disabled");
        let url = request(args)?;
        #[cfg(test)]
        let url = {
            let fixture = self.fixture_coinbase.lock().unwrap().clone();
            if let Some(fixture) = fixture {
                let mut target = reqwest::Url::parse(&fixture)?;
                target.set_query(url.query());
                target
            } else {
                url
            }
        };
        import::endpoint(url.as_str()).context("coinbase_search policy rejected")?;
        let client = self.source_client(owner)?;
        let manager = self.clone();
        let owned_client = client.clone();
        let listener = owner.to_owned();
        client.run_owned(async move {
            let result = async {
            let identity = owned_client.documentation_identity().await.map_err(|_| anyhow::anyhow!("coinbase_search identity unavailable"))?;
            let factory = network::global();
            let selected = factory.http_participating(&identity, url.as_str(), import::read_timeout(None), true, owned_client.transport())
                .map_err(|_| anyhow::anyhow!("coinbase_search transport selection failed"))?;
            match origin(&url, &selected, crate::limits::RESPONSE_BYTES).await {
                Ok(value) => output(value, "origin"),
                Err(failure) if !failure.fallback => Err(failure.error.context("coinbase_search origin response rejected")),
                Err(failure) => {
                    let Some(relay) = manager.search_relay.as_ref().filter(|relay| relay.enabled(crate::discovery_relay::Purpose::SourceDiscovery)) else {
                        return Err(failure.error.context("coinbase_search origin failed; fallback unavailable"));
                    };
                    tracing::info!("Configured paid relay selected as fallback for the Coinbase directory search");
                    let response = relay.fetch(owned_client, url.as_str(), validate_response, failure.retry_after).await
                        .context("coinbase_search fallback failed; do not automatically repeat: payment or side effects may have occurred")?;
                    let value = parse_response(&response.body)
                        .context("coinbase_search target response invalid; do not automatically repeat: payment may have occurred")?;
                    output(value, &response.via)
                }
            }
            }.await;
            result.inspect_err(|error| {
                let failure = crate::discovery_relay::RelayFailure::from_error(error);
                let limit = error.downcast_ref::<crate::limits::DownloadLimit>();
                let authorization_may_charge = error.is::<crate::payment::PossiblePayment>();
                tracing::warn!(
                    listener,
                    stage = "coinbase_search",
                    failure_kind = failure.kind(),
                    reason = failure.explanation(),
                    http_status = failure.status(),
                    authorization_may_charge,
                    resource = limit.map(|limit| limit.kind),
                    setting = limit.map(|limit| limit.setting),
                    limit_bytes = limit.map(|limit| limit.limit),
                    "{}", if authorization_may_charge {
                        "Coinbase directory search failed; a payment authorization may still charge; do not automatically repeat the search"
                    } else {
                        "Coinbase directory search failed; no valid result was obtained"
                    }
                );
            })
        }).await?
    }
}

fn validate_response(bytes: &[u8]) -> Result<()> {
    parse_response(bytes).map(drop)
}

async fn origin(
    url: &reqwest::Url,
    selected: &Arc<network::HttpTransport>,
    limit: usize,
) -> std::result::Result<Value, network::acquisition::Failure> {
    use network::acquisition::{self, Failure, FallbackPolicy};
    let factory = network::global();
    let acquired = acquisition::run(
        factory,
        selected.clone(),
        false,
        |transport, _, _| async move {
            crate::qualification::check_retry_authority()
                .await
                .map_err(Failure::local)?;
            let response = transport
                .client
                .get(url.clone())
                .send()
                .await
                .map_err(|error| Failure::transport(error.without_url()))?;
            network::log_http(&response, "coinbase_search");
            let status = response.status();
            if status == reqwest::StatusCode::PARTIAL_CONTENT
                || response
                    .headers()
                    .contains_key(reqwest::header::CONTENT_RANGE)
            {
                return Err(Failure::validation(
                    crate::discovery_relay::document::IncompleteDocument {
                        status: status.as_u16(),
                    }
                    .into(),
                ));
            }
            if status != reqwest::StatusCode::OK {
                let error =
                    crate::discovery_relay::RelayFailure::http_status(status.as_u16()).into();
                // An unpaid target challenge is terminal; a relay must not turn it
                // into another purchase or acquire target signing authority.
                if status == reqwest::StatusCode::PAYMENT_REQUIRED {
                    return Err(Failure::validation(error));
                }
                return Err(Failure::status(
                    error,
                    status,
                    response.headers(),
                    FallbackPolicy::RemoteFailures,
                ));
            }
            let bytes =
                crate::limits::read(response, limit, "Coinbase search", "max_response_bytes")
                    .await
                    .map_err(Failure::body)?;
            parse_response(&bytes)
                .map_err(|error| Failure::remote_content(error, FallbackPolicy::RemoteFailures))
        },
    )
    .await
    .inspect_err(|failure| {
        // Origin exhaustion can still recover through the configured relay.
        // Report attempt details without upstream prose or a premature warning.
        if matches!(
            failure.kind,
            acquisition::FailureKind::Transport | acquisition::FailureKind::HttpStatus(_)
        ) {
            let error = failure.error.downcast_ref::<reqwest::Error>();
            let reason = if error.is_some_and(|error| error.is_timeout() && error.is_connect()) {
                "connection_timeout"
            } else if error.is_some_and(reqwest::Error::is_timeout) {
                "read_inactivity"
            } else if error.is_some() {
                "transport"
            } else {
                "http_status"
            };
            tracing::debug!(
                stage = "coinbase_search_origin",
                reason,
                "Unsigned Coinbase origin acquisition failed"
            );
        }
    })?;
    factory.promote_http(selected, &acquired.current);
    Ok(acquired.value)
}

#[cfg(test)]
#[path = "coinbase_tests.rs"]
mod tests;
