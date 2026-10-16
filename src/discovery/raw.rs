//! Direct, documented API calls under endpoint-local source-management authority.
use super::{Manager, import};
use crate::{catalog::RoutedRequest, output::ToolOutput};
use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub const GET: &str = "x402_treazury_raw_get";
pub const POST: &str = "x402_treazury_raw_post";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    url: String,
    #[serde(default)]
    query: BTreeMap<String, Value>,
    #[serde(default)]
    body: Value,
}

pub fn schema(post: bool) -> Value {
    let scalar = json!({"type":["string","number","boolean"]});
    let mut properties = json!({
        "url":{"type":"string","format":"uri","description":"Documented public HTTPS API endpoint. No URL credentials or fragments. Existing query parameters are preserved."},
        "query":{"type":"object","description":"Optional query parameters, appended to the URL and URL-encoded. Scalars become strings; arrays repeat the parameter name. Do not repeat parameters already in the URL unless the API requires it.","additionalProperties":{"oneOf":[scalar.clone(),{"type":"array","items":scalar}]}}
    });
    let mut required = vec!["url"];
    if post {
        properties["body"] = json!({"description":"Required JSON request body, sent as application/json. Follow the provider's documented structure; JSON null is allowed."});
        required.push("body");
    }
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

fn request(manager: &Manager, name: &str, args: &Map<String, Value>) -> Result<RoutedRequest> {
    let method = match name {
        GET => {
            ensure!(
                !args.contains_key("body"),
                "x402_treazury_raw_get does not accept a body"
            );
            "GET"
        }
        POST => {
            ensure!(
                args.contains_key("body"),
                "x402_treazury_raw_post requires a JSON body"
            );
            "POST"
        }
        _ => bail!("unknown raw request tool"),
    };
    let input: Arguments = serde_json::from_value(Value::Object(args.clone()))?;
    let url = import::endpoint(&manager.policy, &input.url)?;
    let scalar = |v: &Value| v.is_string() || v.is_number() || v.is_boolean();
    for value in input.query.values() {
        ensure!(
            scalar(value) || value.as_array().is_some_and(|vs| vs.iter().all(scalar)),
            "query values must be strings, numbers, booleans or arrays of these scalars"
        );
    }
    Ok(RoutedRequest {
        method: method.into(),
        url: url.into(),
        query: input.query,
        body: (method == "POST").then_some(input.body),
    })
}

impl Manager {
    pub async fn invoke_raw(
        &self,
        owner: &str,
        name: &str,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput> {
        ensure!(self.enabled(owner), "source_management_disabled");
        let route = request(self, name, args)?;
        self.source_client(owner)?
            .with_source_identity("builtin:raw")
            .public_destinations()
            .execute_response(route)
            .await?
            .render(None, &Default::default())
    }
}
