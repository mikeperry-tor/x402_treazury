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
            "limit": {"type":"integer", "minimum":1, "default":crate::limits::DEFAULT_PAGE_SIZE, "description":"Maximum results per page. Any positive integer; defaults to 25."}
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
    let mut add_schema = object(
        json!({
            "provider_id":{"type":"string","minLength":1,"description":"Stable bundled provider ID returned by providers_list, local-only sources_search or source_details. Uses its reviewed configuration and remote or embedded schema. Supply exactly one of provider_id or spec_url."},
            "spec_url":{"type":"string","description":"Public HTTPS URL of an OpenAPI 3 JSON specification. A unique exact known URL uses its bundled provider definition; ambiguous matches require provider_id. An existing generic URL registration is returned unchanged. Other URLs use generic import. Supply exactly one of spec_url or provider_id."},
            "name":{"type":"string","description":"Optional endpoint-local registration name, preserved exactly and case-sensitive. Up to 64 ASCII letters, digits, underscores or hyphens; double underscores are reserved. Determines dyn_<name>__ tool names and the source_name filter. Defaults to the short provider ID for curated sources or a readable hostname-derived name for generic sources, with a numeric suffix on collision."}
        }),
        &[],
    );
    add_schema["oneOf"] = json!([{"required":["provider_id"]},{"required":["spec_url"]}]);
    let raw_guidance = "Use for a documented endpoint when no OpenAPI specification is available or a one-off call does not need registration. Use concise provider documentation to establish the exact endpoint, method and arguments; never guess paid request arguments. Do not use raw calls to download OpenAPI specifications or full API catalogs into context; use x402_treazury_source_details to locate them and x402_treazury_source_add to import them server-side. Directory verification and a payment challenge do not establish the request schema or guarantee useful results. This call can spend Base USDC: the server handles challenge/sign/retry using this endpoint's source-management wallet and normal payment checks and configured direct/Tor transport. Do not supply credentials, headers, wallet selectors or payment settings. A failure, timeout or ambiguous response may follow a completed payment or side effect. Do not automatically repeat the call; each invocation may authorize another payment. Returns bounded text or supported image content; does not register tools. Prefer x402_treazury_source_add with a published OpenAPI specification for reusable schemas and tool discovery.";
    [
        (super::directory::SEARCH, format!("Find APIs in the bundled provider catalog or built-in x402 List directory. Set local_only=true for offline substring search across bundled provider IDs, names, capability descriptions and declared API bases, ordered by provider ID. Blank q enumerates the bundle; page/per_page share browse pagination. Local-only supports omitted mode or browse only, rejects directory filters/ranking arguments and never performs wallet work, network requests or remote fallback. Local results contain provider_id for source_add and are metadata leads, not registered tools, funding or reachability claims. False or omitted local_only searches the external directory, automatically restricted to Base. Describe the capability you need in q. mode=browse (default) uses upstream case-insensitive substring matching across service name, description, category and base URL, with paginated results. mode=best uses upstream relevance and quality ranking, matching service metadata plus AI-derived capability tags and summaries. Neither mode uses our endpoint-local tools_search BM25 ranking. Bundled local matching also uses no BM25 or semantic ranking. Do not mix mode-specific arguments. max_price_usd filters directory recommendations; it does not set a payment cap. Next, use x402_treazury_source_details with a promising result's slug to inspect its listing and discover working specification and documentation URLs in one call. Results are leads, not executable schemas or payment guarantees. Remote reads use this endpoint's source-management wallet and ordinary payment limits. Reads share an IP-level free quota and may require payment. Cite x402-list.com for its CC BY 4.0 data.\n\nBrowse:\n{browse_description}\n\nBest:\n{best_description}"), directory_schema),
        (super::source_details::NAME, "Inspect a service and resolve bundled provider definitions or discover working OpenAPI JSON, x402 discovery and llms.txt URLs. Supply exactly one of slug (an external directory identifier) or base_url (a public HTTPS API base, optionally with a path). With slug, fetches listing metadata once; base_url skips the directory lookup. Returns provider_matches with selectable provider_id, name, description, API base and schema provenance. Exact bundled API-base matches skip speculative provider probes and do not establish current reachability or live qualification; ambiguous companions remain separate candidates. Unknown API bases retain unsigned document discovery. The listing is preserved when optional discovery cannot proceed. Directory reads may charge through normal payment admission; provider probes never sign or pay and redirects are reported without following them. verified_openapi checks structure only; use source_add for full validation. Pass a selected provider_id or known specification URL directly to source_add. Do not download large specifications or full API catalogs into agent context. Cite x402-list.com when using its CC BY 4.0 directory data.".into(), super::source_details::schema()),
        ("x402_treazury_source_add", "Register an API using exactly one bundled provider_id or public HTTPS OpenAPI 3 JSON spec_url, plus optional readable name. Get bundled IDs from providers_list, sources_search(local_only=true) or source_details; IDs use reviewed provider settings and can load embedded schemas without vendor requests. A unique exact known specification URL selects its curated definition; ambiguous known URLs require a provider_id. Other specification URLs use generic import, without trusting a document's servers claim to select a curated provider. Pass the selector directly: the server obtains and fully validates the schema; do not download large specifications into agent context. Repeated provider IDs return their existing endpoint-local registration; a repeated legacy generic URL returns that accepted generic source unchanged. Registration does not implicitly refresh a source. Persistence and wallets belong to the operator; registration never funds wallets or probes prices. Names are case-sensitive, up to 64 ASCII letters, digits, underscores or hyphens; double underscores are reserved. Curated names default to the short configuration ID: exa produces dyn_exa__search. An explicit name replaces that default without repeating the provider prefix. After registration, use x402_treazury_tools_search with source_name set to the returned name to inspect complete descriptions and schemas, then x402_treazury_tool_call to invoke a selected tool_id.".into(), add_schema),

        (super::local::NAME, "List bundled provider configurations offline, ordered by stable provider ID. Returns provider_id, display name, capability description, declared API base where available, schema provenance and pagination. Uses the same metadata and pagination as sources_search(local_only=true) with blank q. The catalog is independent of this endpoint's registered or loaded tools. Listing does no network or wallet work, allocates no funds and does not register sources. Bundled metadata does not prove current reachability or live qualification. Pass a selected provider_id to source_add; use tools_search for APIs already callable on this endpoint.".into(), super::local::list_schema()),

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
