//! Curated resource search with shared ranked-search inputs.
use super::{Manager, import};
use crate::{network, output::ToolOutput};
use anyhow::{Context, Result, bail, ensure};
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
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("coinbase_search response invalid_json"))?;
    ensure!(
        value.is_object()
            && value["resources"]
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_object))
            && value["partialResults"].is_boolean(),
        "coinbase_search response invalid_contract"
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
        import::endpoint(&self.policy, url.as_str())
            .context("coinbase_search policy rejected; Curl unused")?;
        let client = self.source_client(owner)?;
        let manager = self.clone();
        let owned_client = client.clone();
        client.run_owned(async move {
            let identity = owned_client.documentation_identity().await.map_err(|_| anyhow::anyhow!("coinbase_search identity unavailable; Curl unused"))?;
            let factory = network::global();
            let selected = factory.http_participating(&identity, url.as_str(), import::read_timeout(&manager.policy, None), true, owned_client.transport())
                .map_err(|_| anyhow::anyhow!("coinbase_search transport selection failed; Curl unused"))?;
            match origin(&url, &selected, manager.policy.max_response_bytes, manager.search_relay.is_some()).await {
                Ok(value) => output(value, "origin"),
                Err(error) if error.downcast_ref::<crate::limits::DownloadLimit>().is_some() => Err(error.context("coinbase_search origin max_response_bytes rejected; Curl unused")),
                Err(error) => {
                    let Some(relay) = &manager.search_relay else {
                        return Err(error.context("coinbase_search origin failed; Curl disabled"));
                    };
                    let bytes = relay.fetch(owned_client, url.as_str(), &manager.policy).await.context("coinbase_search Curl attempted and failed; do not automatically repeat: payment or side effects may have occurred")?;
                    let value = parse_response(&bytes).context("coinbase_search Curl attempted but target response invalid; do not automatically repeat: payment may have occurred")?;
                    output(value, "curl")
                }
            }
        }).await?
    }
}

async fn origin(
    url: &reqwest::Url,
    selected: &Arc<network::HttpTransport>,
    limit: usize,
    relay_enabled: bool,
) -> Result<Value> {
    let factory = network::global();
    let attempts = factory.max_http_attempts(false);
    for attempt in 1..=attempts {
        let transport = factory
            .http_attempt(selected, attempt)
            .map_err(|_| anyhow::anyhow!("coinbase_search origin transport unavailable"))?;
        let result = transport.client.get(url.clone()).send().await;
        let response = match result {
            Ok(response) => response,
            Err(error) if attempt < attempts && network::eligible_retry_transport(&error) => {
                continue;
            }
            Err(_) => bail!("coinbase_search origin transport failed"),
        };
        network::log_http(&response, "coinbase_search");
        let status = response.status();
        if status != reqwest::StatusCode::OK {
            let retry = network::eligible_retry_status(status);
            let delay = network::parse_retry_after(status, response.headers())
                .map(network::RetryAfter::after);
            drop(response);
            if (retry && attempt < attempts) || relay_enabled {
                network::sleep_retry_after(delay).await;
            }
            if retry && attempt < attempts {
                continue;
            }
            bail!("coinbase_search origin http_status {}", status.as_u16());
        }
        ensure!(
            !response
                .headers()
                .contains_key(reqwest::header::CONTENT_RANGE),
            "coinbase_search origin partial_response"
        );
        let bytes =
            match crate::limits::read(response, limit, "Coinbase search", "max_response_bytes")
                .await
            {
                Ok(bytes) => bytes,
                Err(error)
                    if attempt < attempts
                        && error
                            .downcast_ref::<reqwest::Error>()
                            .is_some_and(network::eligible_retry_transport) =>
                {
                    continue;
                }
                Err(error)
                    if error
                        .downcast_ref::<crate::limits::DownloadLimit>()
                        .is_some() =>
                {
                    return Err(error);
                }
                Err(_) => bail!("coinbase_search origin response transport failed"),
            };
        let value = parse_response(&bytes)?;
        factory.promote_http(selected, &transport);
        return Ok(value);
    }
    unreachable!("at least one origin attempt")
}

#[cfg(test)]
#[path = "coinbase_tests.rs"]
mod tests;
