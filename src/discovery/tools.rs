//! Endpoint capabilities share one advertising and dispatch policy.
use super::policy::EndpointPolicy;
use rmcp::model::Tool;
use serde_json::{Value, json};
use std::sync::Arc;
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn search_schema() -> Value {
    object(
        json!({
            "source_name":{"type":"string","description":"Registration name to search; omit to search all APIs on this endpoint."},
            "query":{"type":"string","description":"Capability keywords. Omit or pass blank text to enumerate callable tools."},
            "cursor":{"type":"string","description":"Returned next_cursor; keep the same query and source filter. Restart after catalog changes."},
            "limit":{"type":"integer","minimum":1,"default":25,"description":"Results per page; defaults to 25."}
        }),
        &[],
    )
}

pub fn listener_definitions(
    policy: EndpointPolicy,
    discover_on_demand: bool,
    discovery_instructions: Option<&str>,
    remote_instructions: Option<&str>,
) -> Vec<Tool> {
    let mut definitions = Vec::new();
    if policy.local {
        definitions.push((super::local::NAME, "Find locally bundled provider definitions, reviewed, filtered and repaired for compatible API use. Listing is offline. Use a provider_id to add one.".to_owned(), super::local::list_schema()));
        definitions.push(("x402_treazury_providers_add_local", "Add a locally bundled provider using its reviewed, filtered and repaired API definition. Find its callable tools with tools_search.".into(), object(json!({
            "provider_id":{"type":"string","minLength":1,"description":"Exact local definition ID returned by providers_list_local."},
            "name":registration_name()
        }), &["provider_id"])));
    }
    if policy.remote_search() {
        let mut description = "Find APIs across the operator-selected remote directories. Returns native JSON objects keyed by directory; local_error marks a failed directory. Empty or partial results do not prove absence. One search may authorize multiple directory or relay charges; do not automatically repeat failed paid searches.".to_owned();
        if policy.has_x402_list() {
            description.push_str(" Cite x402-list.com for its CC BY 4.0 data.");
        }
        definitions.push((
            super::directory::SEARCH,
            description,
            super::directory::search_schema(
                policy
                    .directories
                    .iter()
                    .any(|d| *d != super::policy::RemoteDirectory::X402List),
            ),
        ));
    }
    if policy.remote {
        definitions.push(("x402_treazury_providers_add_remote", "Add a provider from a public HTTPS OpenAPI 3 JSON specification URL. Pass the URL directly for server-side import, then find callable tools with tools_search. Repeated registrations reuse the accepted provider.".into(), object(json!({
            "spec_url":{"type":"string","format":"uri","description":"Published public HTTPS OpenAPI 3 JSON URL; an API base or usage document is insufficient."},
            "name":registration_name()
        }), &["spec_url"])));
        definitions.push((super::source_details::NAME, "Locate provider specifications and help from a public HTTPS base_url, or read complete plain/Markdown text from documentation_url. HTML/non-text is rejected. Add a known specification with providers_add_remote. Lookups may charge; do not automatically repeat failed paid lookups.".into(), super::source_details::schema(policy.has_x402_list())));
    }
    if policy.raw {
        definitions.push((super::raw::NAME, "Call an exact documented public HTTPS endpoint without registration. Supply its documented method, URL and arguments. May spend Base USDC; a failed call may have paid or completed a side effect. Do not automatically repeat it. Returns text or supported images.".into(), super::raw::schema()));
    }
    if policy.management() || discover_on_demand {
        definitions.push(("x402_treazury_tools_search", "Find callable API tools and complete input schemas on this MCP endpoint. Omit query to enumerate and follow next_cursor for more. Invoke a returned tool_id with tool_call on this same endpoint. Unknown cost is not free.".into(), search_schema()));
        definitions.push(("x402_treazury_tool_call", "Invoke a tool_id using its discovered input schema on this same MCP endpoint. Calls may charge; do not automatically repeat a failed paid call.".into(), object(json!({
            "tool_id":{"type":"string","description":"Exact tool_id returned by tools_search on this endpoint; reusable while available."},
            "arguments":{"type":"object","additionalProperties":true,"description":"Arguments matching the tool input_schema; use {} when no arguments are needed."}
        }), &["tool_id","arguments"])));
    }
    definitions
        .into_iter()
        .map(|(name, mut description, schema)| {
            let instructions = match name {
                "x402_treazury_tools_search" => discovery_instructions,
                super::directory::SEARCH => remote_instructions,
                _ => None,
            };
            if let Some(text) = instructions.filter(|t| !t.is_empty()) {
                description.push_str("\n\n");
                description.push_str(text);
            }
            Tool::new(
                name,
                description,
                Arc::new(schema.as_object().unwrap().clone()),
            )
        })
        .collect()
}
fn registration_name() -> Value {
    json!({"type":"string","description":"Optional registration name: up to 64 ASCII letters, digits, underscores or hyphens; no double underscores. Defaults to provider ID or hostname with a collision suffix."})
}
