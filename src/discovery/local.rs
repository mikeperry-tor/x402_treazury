//! Offline bundled provider browsing; metadata lookup never acquires a payer identity.
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};

pub const NAME: &str = "x402_treazury_providers_list_local";

fn pagination_properties() -> Map<String, Value> {
    json!({
        "page":{"type":"integer","minimum":1,"default":1,"description":"Page number, starting at 1."},
        "per_page":{"type":"integer","minimum":1,"default":crate::limits::DEFAULT_PAGE_SIZE,"description":"Results per page. Defaults to 25; accepts any positive integer."}
    }).as_object().unwrap().clone()
}

pub fn list_schema() -> Value {
    let mut properties = pagination_properties();
    properties.insert("q".into(), json!({"type":"string","description":"Capability keywords. Omit or pass blank text to enumerate bundled definitions."}));
    json!({"type":"object","properties":properties,"additionalProperties":false})
}

pub fn list(args: &Map<String, Value>) -> Result<Value> {
    let mut args = args.clone();
    let query = args
        .remove("q")
        .map(|v| v.as_str().map(str::to_owned).context("q must be a string"))
        .transpose()?;
    browse(query.as_deref(), &args)
}

fn browse(query: Option<&str>, args: &Map<String, Value>) -> Result<Value> {
    ensure!(
        args.keys()
            .all(|name| matches!(name.as_str(), "page" | "per_page")),
        "local provider listing accepts only q, page and per_page"
    );
    let properties = pagination_properties();
    let integer = |name: &str| -> Result<u64> {
        args.get(name)
            .map(|value| {
                value
                    .as_u64()
                    .context(format!("local provider {name} must be a positive integer"))
            })
            .unwrap_or_else(|| {
                Ok(properties[name]["default"]
                    .as_u64()
                    .expect("local pagination default"))
            })
    };
    crate::providers::browse(query, integer("page")?, integer("per_page")?)
}
