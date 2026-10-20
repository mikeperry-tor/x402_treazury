//! Direct, documented API calls under endpoint-local source-management authority.
use super::{Manager, import};
use crate::{catalog::RoutedRequest, output::ToolOutput};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub const NAME: &str = "x402_treazury_raw_call";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    method: String,
    url: String,
    #[serde(default)]
    query: BTreeMap<String, Value>,
    #[serde(default)]
    body: Value,
}

pub fn schema() -> Value {
    let scalar = json!({"type":["string","number","boolean"]});
    json!({"type":"object","properties":{
        "method":{"type":"string","description":"Documented HTTP method, such as GET, POST, PUT, PATCH, DELETE, HEAD or OPTIONS. Use the provider's exact method."},
        "url":{"type":"string","format":"uri","description":"Any documented public HTTPS x402 API URL. No URL credentials or fragments. Existing query parameters are preserved."},
        "query":{"type":"object","description":"Optional query parameters, appended to the URL and URL-encoded. Scalars become strings; arrays repeat the parameter name. Do not repeat parameters already in the URL unless the API requires it.","additionalProperties":{"oneOf":[scalar.clone(),{"type":"array","items":scalar}]}},
        "body":{"description":"Optional JSON request body, sent as application/json exactly as supplied. Omit when the documented request has no body; explicit null sends JSON null."}
    },"required":["method","url"],"additionalProperties":false})
}

fn request(manager: &Manager, args: &Map<String, Value>) -> Result<RoutedRequest> {
    let input: Arguments = serde_json::from_value(Value::Object(args.clone()))?;
    let method =
        reqwest::Method::from_bytes(input.method.as_bytes()).context("invalid HTTP method")?;
    let url = import::endpoint(&manager.policy, &input.url)?;
    let scalar = |v: &Value| v.is_string() || v.is_number() || v.is_boolean();
    for value in input.query.values() {
        ensure!(
            scalar(value) || value.as_array().is_some_and(|vs| vs.iter().all(scalar)),
            "query values must be strings, numbers, booleans or arrays of these scalars"
        );
    }
    Ok(RoutedRequest {
        method: method.to_string(),
        url: url.into(),
        query: input.query,
        body: args.contains_key("body").then_some(input.body),
    })
}

impl Manager {
    pub async fn invoke_raw(&self, owner: &str, args: &Map<String, Value>) -> Result<ToolOutput> {
        ensure!(self.enabled(owner), "source_management_disabled");
        let route = request(self, args)?;
        self.source_client(owner)?
            .with_source_identity("builtin:raw")
            .public_destinations()
            .execute_rendered_scoped(route, None, None, &Default::default())
            .await
    }
}
