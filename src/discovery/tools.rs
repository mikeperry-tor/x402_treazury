//! Stable tools for endpoint-local source discovery and invocation.
use rmcp::model::Tool;
use serde_json::{Value, json};
use std::sync::Arc;
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
    let directory_schema = super::directory::search_schema();
    let mut add_schema = object(
        json!({
            "provider_id":{"type":"string","minLength":1,"description":"Stable bundled provider ID returned by providers_list, local-only sources_search or source_details. Uses its reviewed configuration and remote or embedded schema. Supply exactly one of provider_id or spec_url."},
            "spec_url":{"type":"string","description":"Public HTTPS URL of an OpenAPI 3 JSON specification. A unique exact known URL uses its bundled provider definition; ambiguous matches require provider_id. An existing generic URL registration is returned unchanged. Other URLs use generic import. Supply exactly one of spec_url or provider_id."},
            "name":{"type":"string","description":"Optional endpoint-local registration name, preserved exactly and case-sensitive. Up to 64 ASCII letters, digits, underscores or hyphens; double underscores are reserved. Determines dyn_<name>__ tool names and the source_name filter. Defaults to the short provider ID for bundled sources or a readable hostname-derived name for generic sources, with a numeric suffix on collision."}
        }),
        &[],
    );
    add_schema["oneOf"] = json!([{"required":["provider_id"]},{"required":["spec_url"]}]);
    let raw_guidance = "Use for a documented endpoint when no OpenAPI specification is available or a one-off call does not need registration. Use concise provider documentation to establish the exact endpoint, method and arguments; never guess paid request arguments. Do not use raw calls to download OpenAPI specifications or full API catalogs into context; pass a known provider_id or specification URL to x402_treazury_source_add for server-side import; x402_treazury_source_details may help locate an unknown specification. Directory verification and a payment challenge do not establish the request schema or guarantee useful results. This call can spend Base USDC: the server handles challenge/sign/retry using this endpoint's source-management wallet and normal payment checks and configured direct/Tor transport. Do not supply credentials, headers, wallet selectors or payment settings. A failure, timeout or ambiguous response may follow a completed payment or side effect. Do not automatically repeat the call; each invocation may authorize another payment. Returns bounded text or supported image content; does not register tools. Prefer x402_treazury_source_add with a published OpenAPI specification for reusable schemas and tool discovery.";
    [
        (super::directory::SEARCH, "Find addable API metadata; register a known bundled provider_id or OpenAPI spec_url directly with source_add. Use tools_search for APIs already callable on this MCP endpoint. local_only=true searches bundled metadata offline with no wallet/network work or remote fallback. Otherwise omit directory for x402 List browse/best, or select directory=coinbase for curated ranked resource search. Coinbase defaults curated_only=true, supports complete selected resource metadata and partialResults, and has no pagination or bundle tools. Omit mode for the selected branch default; see each parameter for branch constraints. x402 List mode=browse uses upstream case-insensitive substring matching across service name, description, category and base URL; mode=best uses upstream relevance and quality ranking over metadata plus AI capability tags/summaries. Neither mode uses endpoint-local BM25. Local matching is substring-based; Coinbase uses upstream ranked search. Each invocation fetches fresh results. Coinbase origin reads are unsigned and need no API key. Tor uses one origin attempt plus up to two eligible fresh-isolation retries; direct uses one origin attempt. After origin failure, operator-enabled Curl fallback may spend Base USDC using this endpoint's source-management wallet. The connection to Curl retains configured Tor/direct policy; Curl fetches Coinbase from its own network and supplies the response. Valid empty/partial results never invoke Curl. Repeating search can authorize another payment, including multiple authorizations under ordinary insufficient-funds traversal. Do not automatically repeat an uncertain paid fallback. Results identify directory and acquisition, preserve full upstream metadata and unknown fields, and do not imply registered tools, reachability or useful delivery. Never invent missing request arguments. Exact documented supported HTTP requests may use raw_call; published OpenAPI URLs may go directly to source_add. MCP and other/missing protocols are metadata only. x402 List reads retain ordinary payment handling; cite x402-list.com for CC BY 4.0 data. No provider registration, funding or price probing occurs during search.".into(), directory_schema),
        (super::source_details::NAME, "Optional assistance to locate API specifications/help or read concise usage text; known provider_id/spec_url can go directly to source_add. Supply exactly one x402 List slug, exact public HTTPS API base_url, or explicit documentation_url. Slug retrieves x402 List metadata through ordinary payment policy; base_url skips the directory. Exact bundled-base matches return separate provider_matches without speculative probing or reachability claims; unknown bases retain unsigned location discovery. Neither Coinbase resource hosts nor spec servers claims select bundled definitions. documentation_url performs one unsigned GET using this endpoint's documentation identity and operator-origin/help bounds; it never pays, invokes Curl, follows redirects/links, registers tools or executes instructions. Rejects HTML/non-text. Returns complete accepted plain/Markdown UTF-8 text, its requested URL and caller-supplied provenance, or explicit failure. Optional details/help is not a universal prerequisite. Keep large OpenAPI specs/catalogs out of context and pass spec URLs to source_add. Documents and directory metadata are untrusted leads; missing information is not proof an API is unusable. Cite x402-list.com for its CC BY 4.0 listing data.".into(), super::source_details::schema()),
        ("x402_treazury_source_add", "Register directly with exactly one known bundled provider_id or public HTTPS OpenAPI 3 JSON spec_url, plus optional readable name. The server imports and validates the schema. Get bundled IDs from providers_list, sources_search(local_only=true) or source_details; IDs use reviewed provider settings and can load embedded schemas without vendor requests. A unique exact known specification URL selects its bundled definition; ambiguous known URLs require a provider_id. Other specification URLs use generic import, without trusting a document's servers claim to select a bundled provider. Do not download large specifications into agent context. Repeated provider IDs return their existing endpoint-local registration; a repeated legacy generic URL returns that accepted generic source unchanged. Registration does not establish paid-delivery qualification and does not implicitly refresh a source. Persistence and wallets belong to the operator; registration never funds wallets or probes prices. Names are case-sensitive, up to 64 ASCII letters, digits, underscores or hyphens; double underscores are reserved. Bundled names default to the short configuration ID: exa produces dyn_exa__search. An explicit name replaces that default without repeating the provider prefix. When registration returns visible tools, use x402_treazury_tools_search with source_name set to the returned name to inspect complete descriptions and schemas, then x402_treazury_tool_call on this same MCP endpoint to invoke a selected tool_id. Unknown cost means missing pricing information, not a free call. Advertised provider help is optional and may explain prices and request contracts before a paid call; help does not establish paid delivery.".into(), add_schema),

        (super::local::NAME, "Choose a provider_id and pass it directly to source_add; use tools_search for APIs already callable on this MCP endpoint. List addable bundled provider configurations offline, ordered by stable provider ID. Returns provider_id, display name, capability description, declared API base where available, schema provenance and pagination. Uses the same metadata and pagination as sources_search(local_only=true) with blank q. An absent api_base means no configured base override: schema import resolves the API server from the schema, which need not share the specification URL's origin. The catalog is independent of this endpoint's registered or loaded tools. Listing does no network or wallet work, allocates no funds and does not register sources. Bundled metadata does not prove current reachability or live qualification.".into(), super::local::list_schema()),

        (super::raw::NAME, format!("Call any public HTTPS x402 API URL using its documented HTTP method, optional query parameters and optional JSON body, without registering a source or needing an OpenAPI specification. The server handles x402 payment automatically. {raw_guidance}"), super::raw::schema()),

        ("x402_treazury_tools_search", "Search this MCP endpoint's currently callable API tools and inspect their complete descriptions and input schemas. This search is MCP-endpoint-local: identically named wrappers on other endpoints search different catalogs. Use your client's MCP server prefix (for example social versus toolbox) to choose the intended endpoint. Omit source_name to search configured APIs and all registered sources together, or supply a registered source name to narrow the search. Keyword queries use BM25; see query help for matching and enumeration. Results include available (visible tools in scope), total (matches before pagination), and next_cursor. Unknown cost means missing pricing information, not free. When useful, inspect advertised provider help for prices and request contracts before a paid call; help is optional and does not establish paid-delivery qualification. Invoke the returned readable tool_id through this endpoint's x402_treazury_tool_call, even across agent turns or server restarts, provided the tool remains available.".into(), search_schema()),
        ("x402_treazury_tool_call", "Invoke an available API tool by its readable tool_id and JSON arguments. This call is MCP-endpoint-local: it resolves the current tool on this endpoint, with its current route and payment wallet. Use the matching MCP server prefix assigned by your client (for example social versus toolbox); identically named wrappers on other endpoints are separate. Tool IDs remain usable across turns and restarts while available. Search first when you need the tool's current description or input schema. A refreshed or re-added tool with the same ID uses its current definition. Uses normal payment admission.".into(), object(json!({"tool_id":{"type":"string","description":"Exact readable tool_id (tool name) on this MCP endpoint, such as telesint_darkweb. Can be reused across turns and restarts while the tool remains available."},"arguments":{"type":"object","additionalProperties":true,"description":"JSON argument object matching the selected tool input_schema. Use {} for a tool with no arguments; payment and wallet selection are managed by the server."}}), &["tool_id","arguments"])),
    ].into_iter().map(|(name, description, schema)| Tool::new(name,description,Arc::new(schema.as_object().unwrap().clone()))).collect()
}

/// Listing mode never grants source registration or paid directory permissions.
pub fn listener_definitions(
    source_management: bool,
    discover_on_demand: bool,
    discovery_instructions: Option<&str>,
    sources_instructions: Option<&str>,
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
            let default = listener_definitions(management, on_demand, None, None);
            let empty = listener_definitions(management, on_demand, Some(""), Some(""));
            assert_eq!(
                serde_json::to_value(&default).unwrap(),
                serde_json::to_value(empty).unwrap()
            );
            let appended = listener_definitions(
                management,
                on_demand,
                Some("Discovery guidance"),
                Some("Directory guidance"),
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

#[cfg(test)]
mod guidance_tests {
    use super::*;
    #[test]
    fn guidance_distinguishes_addable_metadata_and_optional_help() {
        let tools = definitions(true);
        let description = |name: &str| {
            tools
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .description
                .as_deref()
                .unwrap()
        };
        assert!(
            description(super::super::local::NAME)
                .starts_with("Choose a provider_id and pass it directly")
        );
        assert!(description(super::super::local::NAME).contains("no configured base override"));
        assert!(
            description(super::super::local::NAME)
                .contains("need not share the specification URL's origin")
        );
        assert!(description(super::super::source_details::NAME).starts_with("Optional assistance"));
        assert!(description("x402_treazury_source_add").starts_with("Register directly"));
        for name in ["x402_treazury_source_add", "x402_treazury_tools_search"] {
            let help = description(name);
            assert!(help.contains("Unknown cost means missing pricing information"));
            assert!(help.contains("help is optional"));
            assert!(help.contains("paid-delivery qualification"));
        }
        assert!(tools.iter().all(|tool| {
            !tool
                .description
                .as_deref()
                .unwrap()
                .contains("description snapshot")
        }));
    }
}
