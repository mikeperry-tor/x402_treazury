//! Stable tools for endpoint-local source discovery and invocation.
use rmcp::model::Tool;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn search_schema() -> Value {
    object(
        json!({
            "source_name": {"type":"string", "description":"Optional registered source name, preserved exactly and case-sensitive. Restricts search to this source on this endpoint. Omit to search configured and registered APIs together."},
            "query": {"type":"string", "description":"Optional BM25 keyword search over tool names, descriptions and top-level parameter names. Lowercase Unicode alphanumeric terms match with OR semantics, so 'yelp reviews' need not be adjacent. Literal name/description substring matches rank first, then BM25 relevance. No semantic, fuzzy, wildcard or Boolean query syntax. Omit query or pass blank text to enumerate all tools in the selected scope; follow next_cursor for more results."},
            "cursor": {"type":"string", "description":"Opaque next_cursor from the preceding result. Reuse the same query and source filter; catalog changes invalidate cursors."},
            "limit": {"type":"integer", "minimum":1, "maximum":100, "description":"Maximum results per page, from 1 to 100. Defaults to 5 with on-demand discovery, otherwise 20."}
        }),
        &[],
    )
}

pub fn definitions(enabled: bool) -> Vec<Tool> {
    if !enabled {
        return vec![];
    }
    let browse_description = &super::directory::tool("/services").description;
    let best_description = &super::directory::tool("/best").description;
    let directory_schema = super::directory::search_schema();
    let raw_guidance = "Use for a documented endpoint when no OpenAPI specification is available or a one-off call does not need registration. Use concise provider documentation to establish the exact endpoint, method and arguments; never guess paid request arguments. Do not use raw calls to download OpenAPI specifications or full API catalogs into context; use x402_treazury_source_details to locate them and x402_treazury_source_add to import them server-side. Directory verification and a payment challenge do not establish the request schema or guarantee useful results. This call can spend Base USDC: the server handles challenge/sign/retry using this endpoint's source-management wallet and normal payment checks and configured direct/Tor transport. Do not supply credentials, headers, wallet selectors or payment settings. A failure, timeout or ambiguous response may follow a completed payment or side effect. Do not automatically repeat the call; each invocation may authorize another payment. Returns bounded text or supported image content; does not register tools. Prefer x402_treazury_source_add with a published OpenAPI specification for reusable schemas and tool discovery.";
    [
        (super::directory::SEARCH, format!("Find APIs in the built-in x402 List directory; searches are automatically restricted to Base. Describe the capability you need in q. mode=browse (default) uses upstream case-insensitive substring matching across service name, description, category and base URL, with paginated results. mode=best uses upstream relevance and quality ranking, matching service metadata plus AI-derived capability tags and summaries. Neither mode uses our endpoint-local tools_search BM25 ranking. Do not mix mode-specific arguments. max_price_usd filters directory recommendations; it does not set a payment cap. Next, use x402_treazury_source_details with a promising result's slug to inspect its listing and discover working specification and documentation URLs in one call. Results are leads, not executable schemas or payment guarantees. Uses this endpoint's source-management wallet and ordinary payment limits. Reads share an IP-level free quota and may require payment. Cite x402-list.com for its CC BY 4.0 data.\n\nBrowse:\n{browse_description}\n\nBest:\n{best_description}"), directory_schema),
        (super::source_details::NAME, "Inspect a service and discover working OpenAPI JSON, x402 discovery and llms.txt documentation URLs. Supply exactly one of slug (the service identifier returned by x402_treazury_sources_search) or base_url (any public HTTPS API base URL, optionally with a path). With slug, fetches directory listing metadata once and checks common document URLs; base_url skips the directory lookup. Returns listing plus url_discovery with per-URL status, HTTP status and content type. Directory reads may charge through normal payment admission; provider probes are unsigned and never pay. Uses the endpoint's existing source-management wallet and direct/Tor transport. Redirects are reported without following them. verified_openapi means OpenAPI 3 JSON structure was checked; pass its URL directly to x402_treazury_source_add for full import validation. documentation_found identifies concise usage documents; directory_json_unverified is only a lead. Probe failures retain any directory listing and report discovery status; no match does not prove the API is unusable. Do not download large specifications or full API catalogs into agent context. If you already know a published specification URL, call source_add directly. Otherwise use concise provider documentation to find it, or use raw_call with documented methods and arguments. Cite x402-list.com when using its CC BY 4.0 directory data.".into(), super::source_details::schema()),
        ("x402_treazury_source_add", "Register a provider using its public HTTPS OpenAPI 3 JSON specification URL (spec_url), not its API endpoint, documentation page or directory detail URL. Use a URL returned by x402_treazury_source_details or a published specification link from concise provider documentation. Pass the URL directly: this tool fetches and fully validates the document server-side. Do not download, inspect or verify the specification yourself; it can be very large. The specification must describe the intended provider API; importing the directory's own OpenAPI document only registers directory tools. Repeated additions of the same URL return the existing registration. Persistence and wallet selection belong to the operator. Registration never funds wallets. Choose a short, descriptive name when adding a source. Names are preserved exactly, are case-sensitive, and allow up to 64 ASCII letters, digits, underscores or hyphens; double underscores are reserved and rejected. It determines the tool prefix: name=netintel produces dyn_netintel__* tools. Use that prefix for source-specific tool filters. After registration, use x402_treazury_tools_search with source_name set to the returned name to inspect complete tool descriptions and input schemas, then invoke the chosen tool_id through x402_treazury_tool_call.".into(), object(json!({"spec_url":{"type":"string","description":"Public HTTPS URL of the provider OpenAPI 3 JSON specification, not an API endpoint or documentation page. Pass the URL directly; the server fetches and validates it."},"name":{"type":"string","description":"Optional endpoint-local registration name, preserved exactly and case-sensitive. Up to 64 ASCII letters, digits, underscores or hyphens; double underscores are reserved. Determines the dyn_<name>__ tool prefix and source_name search filter. Defaults to a readable hostname-derived name, with a numeric suffix on collision."}}), &["spec_url"])),

        (super::raw::NAME, format!("Call any public HTTPS x402 API URL using its documented HTTP method, optional query parameters and optional JSON body, without registering a source or needing an OpenAPI specification. The server handles x402 payment automatically. {raw_guidance}"), super::raw::schema()),

        ("x402_treazury_tools_search", "Search only this MCP endpoint's available API tools and inspect their complete descriptions and input schemas. This search is MCP-endpoint-local: identically named wrappers on other endpoints search different catalogs. Use your client's MCP server prefix (for example social versus toolbox) to choose the intended endpoint. Omit source_name to search configured APIs and all registered sources together, or supply a registered source name to narrow the search. Keyword queries use BM25; see query help for matching and enumeration. Results include available (visible tools in scope), total (matches before pagination), and next_cursor. Invoke the returned readable tool_id through this endpoint's x402_treazury_tool_call, even across agent turns or server restarts, provided the tool remains available.".into(), search_schema()),
        ("x402_treazury_tool_call", "Invoke an available API tool by its readable tool_id and JSON arguments. This call is MCP-endpoint-local: it resolves the current tool on this endpoint, with its current route and payment wallet. Use the matching MCP server prefix assigned by your client (for example social versus toolbox); identically named wrappers on other endpoints are separate. Tool IDs remain usable across turns and restarts while available. Search first when you need the tool's current description or input schema. A refreshed or re-added tool with the same ID uses its current definition. Uses normal payment admission.".into(), object(json!({"tool_id":{"type":"string","description":"Exact readable tool_id (tool name) on this MCP endpoint, such as telesint_darkweb. Can be reused across turns and restarts while the tool remains available."},"arguments":{"type":"object","additionalProperties":true,"description":"JSON argument object matching the selected tool input_schema. Use {} for a tool with no arguments; payment and wallet selection are managed by the server."}}), &["tool_id","arguments"])),
    ].into_iter().map(|(name, description, schema)| Tool::new(name,description,Arc::new(schema.as_object().unwrap().clone()))).collect()
}

/// Dynamic tool names use dyn_<verbatim source name>__<slugged operation>.
/// Slugged operation/help suffixes contain no double underscores; splitting from
/// the right preserves valid source names ending in a single underscore.
pub(crate) fn registered_source_names(tools: &[crate::catalog_state::BoundTool]) -> Vec<&str> {
    tools
        .iter()
        .filter(|tool| tool.source.is_some())
        .filter_map(|tool| {
            tool.tool
                .name
                .strip_prefix("dyn_")?
                .rsplit_once("__")
                .map(|(name, _)| name)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Listing mode never grants source registration or paid directory permissions.
pub fn listener_definitions(
    source_management: bool,
    discover_on_demand: bool,
    discovery_instructions: Option<&str>,
    sources_instructions: Option<&str>,
    registered_sources: &[&str],
) -> Vec<Tool> {
    definitions(source_management || discover_on_demand)
        .into_iter()
        .filter(|t| {
            source_management
                || matches!(
                    t.name.as_ref(),
                    "x402_treazury_tools_search" | "x402_treazury_tool_call"
                )
        })
        .map(|mut tool| {
            if source_management && tool.name == "x402_treazury_tools_search" {
                let sources = if registered_sources.is_empty() {
                    "none".to_owned()
                } else {
                    registered_sources.join(", ")
                };
                tool.description = Some(format!(
                    "{}\n\nRegistered sources with searchable tools on this endpoint in this description snapshot: {sources}. This list is not updated dynamically in your context and clients may cache it; search results reflect current availability.",
                    tool.description.as_deref().unwrap_or_default()
                ).into());
            }
            let instructions = match tool.name.as_ref() {
                "x402_treazury_tools_search" => discovery_instructions,
                "x402_treazury_sources_search" => sources_instructions,
                _ => None,
            };
            if let Some(text) = instructions.filter(|text| !text.is_empty()) {
                tool.description = Some(
                    format!(
                        "{}\n\n{text}",
                        tool.description.as_deref().unwrap_or_default()
                    )
                    .into(),
                );
            }
            tool
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_help_is_endpoint_local_and_empty_append_preserves_definitions() {
        for (management, on_demand) in [(false, false), (true, false), (false, true), (true, true)]
        {
            let default = listener_definitions(management, on_demand, None, None, &[]);
            let empty = listener_definitions(management, on_demand, Some(""), Some(""), &[]);
            assert_eq!(
                serde_json::to_value(&default).unwrap(),
                serde_json::to_value(empty).unwrap()
            );
            let appended = listener_definitions(
                management,
                on_demand,
                Some("Discovery guidance"),
                Some("Directory guidance"),
                &["example"],
            );
            for (tool, decorated) in default.iter().zip(&appended) {
                assert_eq!(tool.name, decorated.name);
                assert_eq!(tool.input_schema, decorated.input_schema);
                for (name, parameter) in tool.input_schema["properties"].as_object().unwrap() {
                    assert!(
                        parameter["description"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty()),
                        "{} parameter {name} lacks help",
                        tool.name
                    );
                }
                if tool.name == super::super::directory::SEARCH {
                    let help = tool.description.as_deref().unwrap();
                    assert!(help.contains("upstream case-insensitive substring"));
                    assert!(help.contains("upstream relevance and quality"));
                    assert!(help.contains("Neither mode uses"));
                }
                if tool.name == "x402_treazury_tools_search" {
                    let properties = &tool.input_schema["properties"];
                    for parameter in ["query", "source_name", "cursor", "limit"] {
                        assert!(
                            !properties[parameter]["description"]
                                .as_str()
                                .unwrap()
                                .is_empty()
                        );
                    }
                    assert!(
                        properties["query"]["description"]
                            .as_str()
                            .unwrap()
                            .contains("BM25")
                    );
                    assert!(
                        properties["query"]["description"]
                            .as_str()
                            .unwrap()
                            .contains("enumerate")
                    );
                    assert_eq!(tool.input_schema["required"], json!([]));
                }
            }
            for tool in default.iter().filter(|tool| {
                matches!(
                    tool.name.as_ref(),
                    "x402_treazury_tools_search" | "x402_treazury_tool_call"
                )
            }) {
                let description = tool.description.as_deref().unwrap();
                assert!(description.contains("MCP-endpoint-local"));
                assert!(description.contains("MCP server prefix"));
                assert!(description.contains("social versus toolbox"));
            }
        }
    }
}
