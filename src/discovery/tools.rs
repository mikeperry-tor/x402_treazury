//! Stable tools for endpoint-local source discovery and invocation.
use rmcp::model::Tool;
use serde_json::{Value, json};
use std::sync::Arc;
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn definitions(enabled: bool) -> Vec<Tool> {
    if !enabled {
        return vec![];
    }
    let browse_description = &super::directory::tool("/services").description;
    let best_description = &super::directory::tool("/best").description;
    let details = super::directory::tool("/services/{slug}");
    let directory_schema = super::directory::search_schema();
    [
        (super::directory::SEARCH, format!("Search the built-in x402 List API directory. mode=browse (default) lists and pages services; mode=best ranks recommendations for a stated need. Do not mix mode-specific arguments. max_price_usd filters directory recommendations; it does not set a payment cap. Results are leads, not executable schemas or payment guarantees; verify a published OpenAPI URL before adding. Uses this endpoint's source-management wallet and ordinary payment limits. Reads share an IP-level free quota and may require payment. Cite x402-list.com for its CC BY 4.0 data.\n\nBrowse:\n{browse_description}\n\nBest:\n{best_description}"), directory_schema),
        (super::directory::DETAILS, format!("Inspect a directory service before adding its API. Takes a directory slug from sources_search, not a registered source_id. Returns advertised endpoints, pricing and reliability information; a published OpenAPI URL is not guaranteed. Uses the endpoint's source-management wallet and normal payment limits. Cite x402-list.com for its CC BY 4.0 data.\n\n{}", details.description), super::directory::details_schema()),
        ("x402_treazury_source_add", "Register a public HTTPS OpenAPI 3 JSON source on this endpoint. Repeated additions of the same URL return the existing registration. Persistence and wallet selection belong to the operator. Registration never funds wallets. Use tools_search to inspect the resulting signatures.".into(), object(json!({"spec_url":{"type":"string"},"name":{"type":"string"}}), &["spec_url"])),
        ("x402_treazury_tools_search", "Inspect visible API tool descriptions and complete input schemas. Pass the returned tool_ref unchanged to x402_treazury_tool_call; this works even if your framework caches its original MCP tool list.".into(), object(json!({"source_id":{"type":"string"},"query":{"type":"string"},"cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}}), &[])),
        ("x402_treazury_tool_call", "Invoke a tool_ref returned by tools_search with its arguments. Uses normal payment admission. Stale references fail before HTTP; search again after an operator refresh or restart.".into(), object(json!({"tool_ref":{"type":"string"},"arguments":{"type":"object","additionalProperties":true}}), &["tool_ref","arguments"])),
    ].into_iter().map(|(name, description, schema)| Tool::new(name,description,Arc::new(schema.as_object().unwrap().clone()))).collect()
}

/// Listing mode never grants source registration or paid directory permissions.
pub fn listener_definitions(source_management: bool, discover_on_demand: bool) -> Vec<Tool> {
    definitions(source_management || discover_on_demand)
        .into_iter()
        .filter(|t| {
            source_management
                || matches!(
                    t.name.as_ref(),
                    "x402_treazury_tools_search" | "x402_treazury_tool_call"
                )
        })
        .collect()
}
