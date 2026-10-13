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
        (super::directory::SEARCH, format!("Find APIs in the built-in x402 List directory; searches are automatically restricted to Base. Describe the capability you need in q. mode=browse (default) lists and pages services; mode=best ranks recommendations for a stated need. Do not mix mode-specific arguments. max_price_usd filters directory recommendations; it does not set a payment cap. Next, pass a promising result's slug to x402_treazury_source_details to inspect its endpoints and documentation. Results are leads, not executable schemas or payment guarantees. Uses this endpoint's source-management wallet and ordinary payment limits. Reads share an IP-level free quota and may require payment. Cite x402-list.com for its CC BY 4.0 data.\n\nBrowse:\n{browse_description}\n\nBest:\n{best_description}"), directory_schema),
        (super::directory::DETAILS, format!("Inspect a directory service before adding its API. Takes a directory slug from sources_search, not a registered source_id. Returns advertised endpoints, pricing and reliability information. To obtain an importable specification, follow the provider's documentation or website links and look for a published OpenAPI JSON link; its llms.txt may point to API documentation. Use available web tools to inspect those pages. Conventional paths such as /openapi.json or /.well-known/openapi.json are only candidates: verify that the URL returns an OpenAPI 3 JSON document describing the intended API, not HTML or a directory listing. A published specification is not guaranteed. Never invent a specification URL or infer request schemas from prose. If no specification can be found, report that the API cannot yet be registered. Pass a verified specification URL to x402_treazury_source_add. Uses the endpoint's source-management wallet and normal payment limits. Cite x402-list.com for its CC BY 4.0 data.\n\n{}", details.description), super::directory::details_schema()),
        ("x402_treazury_source_add", "Register a provider using its verified public HTTPS OpenAPI 3 JSON specification URL (spec_url), not its API endpoint, documentation page or directory detail URL. Obtain the published specification using source_details and the provider's documentation first. The specification must describe the intended provider API; importing the directory's own OpenAPI document only registers directory tools. Repeated additions of the same URL return the existing registration. Persistence and wallet selection belong to the operator. Registration never funds wallets. After registration, use x402_treazury_tools_search with the returned source_id to inspect complete tool descriptions and input schemas, then invoke the chosen tool_ref through x402_treazury_tool_call.".into(), object(json!({"spec_url":{"type":"string"},"name":{"type":"string"}}), &["spec_url"])),
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
