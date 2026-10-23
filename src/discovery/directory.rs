//! Built-in directory operations; schemas load offline, calls use ordinary PaidClient.
use crate::catalog::{Config, ToolSpec};
use anyhow::{Result, bail, ensure};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

pub const BASE: &str = "https://x402-list.com/api/v1";
pub const SEARCH: &str = "x402_treazury_sources_search";
pub const DETAILS: &str = "x402_treazury_source_details";

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

/// One flat argument object, with explicit mode constraints for discoverability.
pub fn search_schema() -> Value {
    let browse = &tool("/services").input_schema;
    let best = &tool("/best").input_schema;
    let coinbase = super::coinbase::search_branch();
    let mut properties: Map<String, Value> = Map::new();
    for schema in [browse, best, &coinbase] {
        for (name, field) in schema["properties"].as_object().unwrap() {
            if matches!(
                name.as_str(),
                "network" | "directory" | "mode" | "local_only"
            ) {
                continue;
            }
            match properties.entry(name.clone()) {
                serde_json::map::Entry::Vacant(entry) => {
                    entry.insert(field.clone());
                }
                serde_json::map::Entry::Occupied(mut entry) => {
                    let shared = entry.get_mut();
                    if shared["type"] != field["type"] {
                        shared["type"] = json!([shared["type"].clone(), field["type"].clone()]);
                    }
                }
            }
        }
    }
    // Describe the choices once, without concatenating upstream implementation prose.
    let help = json!({
        "q":"Capability keywords. Coinbase ranks matches; x402 List browse and local search use case-insensitive substrings. Omit to browse or filter without keywords.",
        "category":"x402 List only: service category; omit for all categories.",
        "status":"x402 List browse only: service status.",
        "verified":"x402 List browse only: filter for its paid-delivery verification badge. Distinct from payment_ready; no guarantee for a future call.",
        "payment_ready":"x402 List browse only: filter for a recent valid payment challenge. Does not mean paid delivery was verified.",
        "signable":"x402 List browse only: filter by whether the last observed payment challenge had the required signing fields.",
        "source":"x402 List browse only: listing provenance, such as submitted or imported.",
        "sort":"x402 List browse only: result order.",
        "page":"Local or x402 List browse page, starting at 1. Coinbase has no pagination.",
        "per_page":"Local or x402 List browse page size; default 25. x402 List maximum 100.",
        "prefer":"x402 List best only: ranking preference; default balanced.",
        "max_price_usd":"Advisory USD price filter, not a payment cap. Coinbase takes a string such as \"0.05\"; x402 List best takes a number.",
        "require_verified":"x402 List best only: require its paid-delivery verification badge, not just payment_ready.",
        "include_facilitator_context":"x402 List best only: include separate ecosystem-level facilitator statistics.",
        "limit":"Ranked result count, 1–20. Defaults to 20 for Coinbase or 5 for x402 List best; no pagination."
    });
    // Shared fields must admit local sizes; remote branches retain upstream bounds.
    properties
        .get_mut("per_page")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("maximum");
    properties.insert("local_only".into(), json!({
        "type":"boolean","default":false,
        "description":"True searches bundled providers offline using q, page and per_page. Omit directory; mode must be omitted or browse. Default false searches a remote directory."
    }));
    properties.insert("mode".into(), json!({
        "type":"string","enum":["browse","best"],
        "description":"x402 List: browse (default, paginated) or best (ranked recommendations). Coinbase: omit or use best. Local: omit or use browse."
    }));
    for (name, field) in &mut properties {
        if let Some(description) = help.get(name) {
            field["description"] = description.clone();
        }
        if name != "local_only" {
            field.as_object_mut().unwrap().remove("default");
        }
    }
    properties.insert("directory".into(), json!({"type":"string","enum":["x402_list","coinbase"],"description":"coinbase: curated ranked resources (curated_only=false broadens results). x402_list: broader service directory, the default. Omit with local_only=true."}));
    let branch = |mode: &str, schema: &Value, required: Value| {
        let mut allowed: Map<String, Value> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|name| name.as_str() != "network")
            .map(|name| {
                let mut field = schema["properties"][name].clone();
                field["description"] = properties[name]["description"].clone();
                (name.clone(), field)
            })
            .collect();
        allowed.insert("mode".into(), json!({"const":mode}));
        allowed.insert("directory".into(), json!({"const":"x402_list"}));
        allowed.insert("local_only".into(), json!({"const":false}));
        json!({"properties":allowed,"required":required,"additionalProperties":false})
    };
    json!({
        "type":"object","properties":properties,"additionalProperties":false,
        "oneOf":[branch("browse",browse,json!([])),branch("best",best,json!(["mode"])),super::local::search_branch(),coinbase]
    })
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
        SEARCH => match args.remove("mode") {
            None => (tool("/services"), "browse"),
            Some(Value::String(mode)) if mode == "browse" => (tool("/services"), "browse"),
            Some(Value::String(mode)) if mode == "best" => (tool("/best"), "best"),
            _ => bail!("directory search mode must be browse or best"),
        },
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
