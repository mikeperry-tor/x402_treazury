//! Stable tools for endpoint-local source discovery and invocation.
use rmcp::model::Tool;
use serde_json::{Value, json};
use std::sync::Arc;
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn definitions(enabled: bool, directory: Option<&crate::catalog::ToolSpec>) -> Vec<Tool> {
    if !enabled {
        return vec![];
    }
    let directory_description = directory
        .map_or("Configured directory is unavailable".to_owned(), |t| {
            t.description.clone()
        });
    let directory_schema =
        directory.map_or_else(|| object(json!({}), &[]), |t| t.input_schema.clone());
    [
        ("x402_treazury_sources_search", format!("Search the operator-configured API directory. Results are leads, not executable schemas or payment guarantees; verify a published OpenAPI URL before adding. Ordinary payment limits apply.\n\n{directory_description}"), directory_schema),
        ("x402_treazury_source_add", "Register a public HTTPS OpenAPI 3 JSON source on this endpoint. Repeated additions of the same URL return the existing registration. Persistence and wallet selection belong to the operator. Registration never funds wallets. Use tools_search to inspect the resulting signatures.".into(), object(json!({"spec_url":{"type":"string"},"name":{"type":"string"}}), &["spec_url"])),
        ("x402_treazury_tools_search", "Inspect visible API tool descriptions and complete input schemas. Pass the returned tool_ref unchanged to x402_treazury_tool_call; this works even if your framework caches its original MCP tool list.".into(), object(json!({"source_id":{"type":"string"},"query":{"type":"string"},"cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}}), &[])),
        ("x402_treazury_tool_call", "Invoke a tool_ref returned by tools_search with its arguments. Uses normal payment admission. Stale references fail before HTTP; search again after an operator refresh or restart.".into(), object(json!({"tool_ref":{"type":"string"},"arguments":{"type":"object","additionalProperties":true}}), &["tool_ref","arguments"])),
    ].into_iter().map(|(name, description, schema)| Tool::new(name,description,Arc::new(schema.as_object().unwrap().clone()))).collect()
}
