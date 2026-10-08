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
    let mut properties = Map::new();
    for (mode, schema, other) in [("browse", browse, best), ("best", best, browse)] {
        for (name, field) in schema["properties"].as_object().unwrap() {
            if properties.contains_key(name) {
                continue;
            }
            let mut field = field.clone();
            let description = field["description"].as_str().unwrap_or("");
            field["description"] = if let Some(other) = other["properties"].get(name) {
                json!(format!(
                    "Browse: {description} Best: {}",
                    other["description"].as_str().unwrap_or("")
                ))
            } else {
                json!(format!("{mode} mode only. {description}"))
            };
            properties.insert(name.clone(), field);
        }
    }
    properties.insert("mode".into(), json!({
        "type":"string","enum":["browse","best"],"default":"browse",
        "description":"browse: paginated directory search. best: ranked recommendations for a stated need, without pagination. Only send fields supported by the selected mode."
    }));
    let branch = |mode: &str, schema: &Value, required: Value| {
        let mut allowed: Map<String, Value> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(|name| (name.clone(), json!({})))
            .collect();
        allowed.insert("mode".into(), json!({"const":mode}));
        json!({"properties":allowed,"required":required,"additionalProperties":false})
    };
    json!({
        "type":"object","properties":properties,"additionalProperties":false,
        "oneOf":[branch("browse",browse,json!([])),branch("best",best,json!(["mode"]))]
    })
}

pub fn details_schema() -> Value {
    let mut schema = tool("/services/{slug}").input_schema.clone();
    schema["additionalProperties"] = json!(false);
    schema["properties"]["slug"]["minLength"] = json!(1);
    schema["properties"]["slug"]["pattern"] = json!("\\S");
    schema["properties"]["slug"]["not"] = json!({"enum":[".", ".."]});
    schema
}

/// Validate before provider I/O, strip the local selector, and dispatch exactly once.
pub fn request(
    name: &str,
    args: &Map<String, Value>,
) -> Result<(&'static ToolSpec, Map<String, Value>)> {
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
