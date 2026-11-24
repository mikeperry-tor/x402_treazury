//! Built-in directory operations; schemas load offline, calls use ordinary PaidClient.
use crate::catalog::{Config, RoutedRequest, ToolSpec, diagnostics::Problem};
use anyhow::{Result, bail, ensure};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

pub const BASE: &str = "https://x402-list.com/api/v1";
pub const AGENTS_TRUST_BASE: &str = "https://api.agents-trust.com";
pub const SEARCH: &str = "x402_treazury_providers_search_remote";
pub const DETAILS: &str = "x402_treazury_provider_details_remote";

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

pub fn search_schema(coinbase: bool) -> Value {
    let ranked = &tool("/best").input_schema["properties"];
    let mut schema = json!({"type":"object","properties":{
        "q":{"type":"string","description":"Describe what the API should do. Omit to browse where supported."},
        "limit":ranked["limit"], "max_price_usd":ranked["max_price_usd"]
    },"additionalProperties":false});
    if coinbase {
        schema["properties"]["q"]["maxLength"] = json!(400);
    }
    schema["properties"]["limit"]["description"] =
        json!("How many results to request per directory. Defaults to 5.");
    schema["properties"]["max_price_usd"]["description"] =
        json!("Filter results by advertised API price in USD. Omit to search at any price.");
    schema
}

pub fn agents_trust_request(base: &str, args: &Map<String, Value>) -> Result<RoutedRequest> {
    let need = args
        .get("q")
        .and_then(Value::as_str)
        .filter(|q| {
            q.chars().count() <= 300
                && q.chars().filter(|c| c.is_alphanumeric()).take(2).count() == 2
        })
        .ok_or(Problem(
            "provide a query of up to 300 characters with at least two letters or digits",
        ))?;
    let mut query = std::collections::BTreeMap::from([
        ("need".into(), json!(need)),
        (
            "limit".into(),
            json!(
                args.get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(5)
                    .min(5)
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

pub fn validate_search(args: &Map<String, Value>, coinbase: bool) -> Result<()> {
    let schema = search_schema(coinbase);
    for (name, value) in args {
        let field = schema["properties"]
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unsupported remote search argument {name}"))?;
        super::coinbase::validate(name, value, field)?;
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
    let mut args = args.clone();
    let (selected, mode) = match name {
        SEARCH => {
            validate_search(&args, false)?;
            args.insert("prefer".into(), json!("balanced"));
            args.insert("require_verified".into(), json!(false));
            args.entry("limit").or_insert(json!(5));
            (tool("/best"), "best")
        }
        DETAILS => (tool("/services/{slug}"), "details"),
        _ => bail!("unknown directory tool"),
    };
    if name == SEARCH {
        args.insert("network".into(), json!("BSE"));
    }
    let schema = &selected.input_schema;
    let properties = schema["properties"]
        .as_object()
        .expect("directory object schema");
    for (name, value) in &args {
        let Some(field) = properties.get(name) else {
            bail!(
                "argument {name} is not supported in directory {mode}; use fields for the selected mode"
            );
        };
        let valid = match field["type"].as_str() {
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            Some("integer") => value.is_u64() || value.is_i64(),
            Some("number") => value.is_number(),
            _ => false,
        };
        ensure!(valid, "invalid type for directory argument {name}");
        if let Some(allowed) = field["enum"].as_array() {
            ensure!(
                allowed.contains(value),
                "invalid value for directory argument {name}"
            );
        }
        if let Some(n) = value.as_f64() {
            ensure!(
                field["minimum"].as_f64().is_none_or(|min| n >= min)
                    && field["maximum"].as_f64().is_none_or(|max| n <= max),
                "directory argument {name} is outside its documented bounds"
            );
        }
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let key = required.as_str().expect("required field name");
        ensure!(args.contains_key(key), "missing directory argument {key}");
    }
    if name == DETAILS {
        ensure!(
            args.get("slug")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty() && s != "." && s != ".."),
            "directory slug must be a nonempty string from a search result"
        );
    }
    Ok((selected, args))
}
