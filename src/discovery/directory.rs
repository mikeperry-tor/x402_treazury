//! Built-in directory operations; schemas load offline, calls use ordinary PaidClient.
use super::policy::{EndpointPolicy, RemoteDirectory};
use crate::catalog::{Config, RoutedRequest, ToolSpec, diagnostics::Problem};
use anyhow::{Result, bail, ensure};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

pub const BASE: &str = "https://x402-list.com/api/v1";
pub const AGENTS_TRUST_BASE: &str = "https://api.agents-trust.com";
pub const SEARCH: &str = "x402_treazury_providers_search_remote";
pub const DETAILS: &str = "x402_treazury_provider_details_remote";
const AGENTS_TRUST_QUERY_MAX_CHARS: usize = 300;
const AGENTS_TRUST_QUERY_MIN_ALPHANUMERIC: usize = 2;
const AGENTS_TRUST_MAX_RESULTS: u64 = 5;
pub(super) const DEFAULT_SEARCH_LIMIT: u64 = 5;

#[derive(Clone, Copy)]
pub(super) enum QueryRules {
    Optional,
    Coinbase,
    AgentsTrustOnly,
}

impl From<&EndpointPolicy> for QueryRules {
    fn from(policy: &EndpointPolicy) -> Self {
        if policy.directories == [RemoteDirectory::AgentsTrust] {
            Self::AgentsTrustOnly
        } else if policy.has_coinbase() {
            Self::Coinbase
        } else {
            Self::Optional
        }
    }
}

pub fn tool(path: &str) -> &'static ToolSpec {
    static TOOLS: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    TOOLS
        .get_or_init(|| {
            let mut config: Config = toml::from_str(include_str!("../../providers/x402-list.toml"))
                .expect("bundled directory provider is valid");
            config.help_url = None;
            config.recommended_tools.clear();
            config.include_operations = vec![
                "GET /services".into(),
                "GET /best".into(),
                "GET /services/{slug}".into(),
            ];
            let document = serde_json::from_str(include_str!(
                "../../providers/x402-list/directory.openapi.json"
            ))
            .expect("bundled directory schema is valid");
            crate::catalog::build_tools(&config, &document, "x402_list")
                .expect("bundled directory tools build")
        })
        .iter()
        .find(|tool| tool.path == path)
        .expect("known built-in directory operation")
}

pub(super) fn search_schema(rules: QueryRules) -> Value {
    let ranked = &tool("/best").input_schema["properties"];
    let mut schema = json!({"type":"object","properties":{
        "q":{"type":"string","description":"Describe what the API should do. Omit to browse where supported."},
        "limit":ranked["limit"], "max_price_usd":ranked["max_price_usd"]
    },"additionalProperties":false});
    schema["properties"]["limit"]["default"] = json!(DEFAULT_SEARCH_LIMIT);
    match rules {
        QueryRules::AgentsTrustOnly => {
            schema["required"] = json!(["q"]);
            schema["properties"]["q"]["description"] =
                json!("Describe what the API should do using at least two letters or digits.");
            schema["properties"]["q"]["minLength"] = json!(AGENTS_TRUST_QUERY_MIN_ALPHANUMERIC);
            schema["properties"]["q"]["maxLength"] = json!(AGENTS_TRUST_QUERY_MAX_CHARS);
            schema["properties"]["limit"]["maximum"] = json!(AGENTS_TRUST_MAX_RESULTS);
        }
        QueryRules::Coinbase => schema["properties"]["q"]["maxLength"] = json!(400),
        QueryRules::Optional => {}
    }
    schema["properties"]["limit"]["description"] =
        json!("How many results to request per directory. Defaults to 5.");
    schema["properties"]["max_price_usd"]["description"] = json!(
        "Filter advertised API prices in USD. Directory and relay search charges are separate."
    );
    schema
}

fn agents_trust_query(args: &Map<String, Value>) -> Result<&str, Problem> {
    args.get("q")
        .and_then(Value::as_str)
        .filter(|q| {
            q.chars().count() <= AGENTS_TRUST_QUERY_MAX_CHARS
                && q.chars()
                    .filter(|c| c.is_alphanumeric())
                    .take(AGENTS_TRUST_QUERY_MIN_ALPHANUMERIC)
                    .count()
                    == AGENTS_TRUST_QUERY_MIN_ALPHANUMERIC
        })
        .ok_or(Problem(
            "provide a query of up to 300 characters with at least two letters or digits",
        ))
}

pub fn agents_trust_request(base: &str, args: &Map<String, Value>) -> Result<RoutedRequest> {
    let need = agents_trust_query(args)?;
    let mut query = std::collections::BTreeMap::from([
        ("need".into(), json!(need)),
        (
            "limit".into(),
            json!(
                args.get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_SEARCH_LIMIT)
                    .min(AGENTS_TRUST_MAX_RESULTS)
            ),
        ),
        ("chain".into(), json!("eip155:8453")),
        ("view".into(), json!("compact")),
    ]);
    if let Some(price) = args.get("max_price_usd") {
        query.insert("max_price_usd".into(), price.clone());
    }
    Ok(RoutedRequest {
        method: "GET".into(),
        url: format!("{base}/recommend"),
        query,
        body: None,
        headers: Default::default(),
        json_media_type: None,
    })
}

pub(super) fn validate_search(args: &Map<String, Value>, rules: QueryRules) -> Result<()> {
    if matches!(rules, QueryRules::AgentsTrustOnly) {
        agents_trust_query(args)?;
    }
    let schema = search_schema(rules);
    for (name, value) in args {
        let field = schema["properties"]
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unsupported remote search argument {name}"))?;
        validate_search_field(name, value, field)?;
    }
    Ok(())
}

fn validate_search_field(name: &str, value: &Value, field: &Value) -> Result<()> {
    let kind = field["type"].as_str().expect("search field type");
    let valid = match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_u64().is_some(),
        _ => false,
    };
    ensure!(valid, "Remote search argument {name} must be {kind}");
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if let Some(max) = field["maxLength"].as_u64() {
            ensure!(
                length <= max,
                "Remote search argument {name} must be at most {max} characters"
            );
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(min) = field["minimum"].as_f64() {
            ensure!(
                number >= min,
                "Remote search argument {name} must be at least {min}"
            );
        }
        if let Some(max) = field["maximum"].as_f64() {
            ensure!(
                number <= max,
                "Remote search argument {name} must be at most {max}"
            );
        }
    }
    Ok(())
}

/// Validate before provider I/O, strip the local selector, and dispatch exactly once.
pub fn request(
    name: &str,
    args: &Map<String, Value>,
) -> Result<(&'static ToolSpec, Map<String, Value>)> {
    ensure!(
        !args.contains_key("network"),
        "directory network is selected by the server; omit network"
    );
    match name {
        SEARCH => {
            validate_search(args, QueryRules::Optional)?;
            let mut args = args.clone();
            args.insert("prefer".into(), json!("balanced"));
            args.insert("require_verified".into(), json!(false));
            args.entry("limit").or_insert(json!(DEFAULT_SEARCH_LIMIT));
            args.insert("network".into(), json!("BSE"));
            Ok((tool("/best"), args))
        }
        DETAILS => {
            for (name, value) in args {
                ensure!(
                    name == "slug",
                    "argument {name} is not supported in directory details; use fields for the selected mode"
                );
                ensure!(
                    value.is_string(),
                    "invalid type for directory argument {name}"
                );
            }
            let slug = args
                .get("slug")
                .ok_or_else(|| anyhow::anyhow!("missing directory argument slug"))?;
            ensure!(
                slug.as_str()
                    .is_some_and(|s| !s.trim().is_empty() && s != "." && s != ".."),
                "directory slug must be a nonempty string from a search result"
            );
            Ok((tool("/services/{slug}"), args.clone()))
        }
        _ => bail!("unknown directory tool"),
    }
}
