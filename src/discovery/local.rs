//! Offline bundled provider browsing; metadata lookup never acquires a payer identity.
use super::{Manager, directory};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};

pub const NAME: &str = "x402_treazury_providers_list";

fn pagination_properties() -> Map<String, Value> {
    let properties = &directory::tool("/services").input_schema["properties"];
    ["page", "per_page"]
        .into_iter()
        .map(|name| (name.into(), properties[name].clone()))
        .collect()
}

pub fn list_schema() -> Value {
    json!({"type":"object","properties":pagination_properties(),"required":[],"additionalProperties":false})
}

pub fn search_branch() -> Value {
    let mut properties = pagination_properties();
    properties.insert("local_only".into(), json!({"const":true}));
    properties.insert("mode".into(), json!({"const":"browse"}));
    properties.insert("q".into(), json!({"type":"string","description":"Optional case-insensitive substring across bundled provider IDs, names, capability descriptions and declared API bases. Omit or pass blank text to enumerate all bundled providers."}));
    json!({"properties":properties,"required":["local_only"],"additionalProperties":false})
}

pub fn list(args: &Map<String, Value>) -> Result<Value> {
    browse(None, args)
}

fn search(args: &Map<String, Value>) -> Result<Value> {
    let mut args = args.clone();
    if let Some(mode) = args.remove("mode") {
        ensure!(
            mode.as_str() == Some("browse"),
            "local provider search supports only browse mode"
        );
    }
    let query = args
        .remove("q")
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .context("local provider q must be a string")
        })
        .transpose()?;
    browse(query.as_deref(), &args)
}

fn browse(query: Option<&str>, args: &Map<String, Value>) -> Result<Value> {
    ensure!(
        args.keys()
            .all(|name| matches!(name.as_str(), "page" | "per_page")),
        "local provider browsing accepts only q, page, per_page and optional browse mode; provider listing accepts only page and per_page"
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
                    .expect("directory pagination default"))
            })
    };
    crate::providers::browse(query, integer("page")?, integer("per_page")?)
}

impl Manager {
    pub async fn invoke_sources_search(
        &self,
        owner: &str,
        args: &Map<String, Value>,
    ) -> Result<crate::output::ToolOutput> {
        ensure!(self.enabled(owner), "source_management_disabled");
        let local = args
            .get("local_only")
            .map(|value| value.as_bool().context("local_only must be a boolean"))
            .transpose()?
            .unwrap_or(false);
        let mut args = args.clone();
        args.remove("local_only");
        if local {
            Ok(crate::output::ToolOutput::text(serde_json::to_string(
                &search(&args)?,
            )?))
        } else {
            self.invoke_directory(owner, directory::SEARCH, &args).await
        }
    }
}
