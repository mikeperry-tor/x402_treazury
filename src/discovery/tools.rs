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
            "source_name": {"type":"string", "description":"Optional case-sensitive registration name returned by source_add. Omit to search all APIs on this endpoint."},
            "query": {"type":"string", "description":"BM25 keywords over names, descriptions and parameter names; terms match with OR semantics, literal substrings rank first. No semantic/fuzzy/Boolean syntax. Omit or pass blank to enumerate."},
            "cursor": {"type":"string", "description":"next_cursor from the preceding result; keep the same query/source filter. Restart if the catalog changed."},
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
            "provider_id":{"type":"string","minLength":1,"description":"Bundled ID from providers_list, local_only search or source_details. Supply this or spec_url."},
            "spec_url":{"type":"string","description":"Public HTTPS OpenAPI 3 JSON URL, not an API base or usage document. Known URLs may use bundled settings; existing registrations are reused. Ambiguous matches require provider_id. Supply this or provider_id."},
            "name":{"type":"string","description":"Optional case-sensitive registration name: up to 64 ASCII letters/digits/underscores/hyphens; no double underscores. Used by source_name and dyn_<name>__ tools. Defaults to provider ID or hostname, with a suffix on collision."}
        }),
        &[],
    );
    add_schema["oneOf"] = json!([{"required":["provider_id"]},{"required":["spec_url"]}]);
    [
        (super::directory::SEARCH, "Find APIs in three collections:\n- Curated: directory=\"coinbase\" (no pagination).\n- Broader directory: x402 List, directory=\"x402_list\" (default); mode=browse pages results, mode=best ranks recommendations.\n- Local: local_only=true searches bundled providers offline with the same BM25 matching as tools_search.\nRegister a known provider_id/spec_url with source_add, or use raw_call for documented requests. Use tools_search for already callable tools. Remote searches may charge; do not automatically repeat failed paid calls. Cite x402-list.com for its CC BY 4.0 data.".into(), directory_schema),
        (super::source_details::NAME, "Optional assistance: use an x402 List slug or exact API base_url to locate bundled providers, specifications and help. Use documentation_url to read complete plain/Markdown usage text; HTML/non-text is rejected. Known provider_id/spec_url goes directly to source_add. Returns locations or text, not registered tools or proof of reachability. Missing information does not make an API unusable. Lookups may charge; do not automatically repeat failed paid lookups.".into(), super::source_details::schema()),
        ("x402_treazury_source_add", "Register directly with one bundled provider_id or public HTTPS OpenAPI 3 JSON spec_url, plus optional name. Pass specification URLs here instead of downloading large OpenAPI documents into context. Repeated registrations reuse the existing source. When visible tools are returned, inspect them with tools_search(source_name=returned name), then use tool_call on this same MCP endpoint. Help is optional. Unknown cost is not free; registration does not prove paid delivery.".into(), add_schema),

        (super::local::NAME, "List bundled providers offline; pass a provider_id directly to source_add. Returns addable metadata and pagination, independently of registered tools. Use tools_search for currently callable APIs. An absent api_base means no configured base override: import uses the schema server, not the specification URL origin. Listing makes no network requests and does not establish reachability or paid delivery.".into(), super::local::list_schema()),

        (super::raw::NAME, "Call an exact documented public HTTPS endpoint without registration. Supply its method, URL and arguments; never guess paid request arguments. For reusable tools, pass a provider_id or OpenAPI URL directly to source_add; do not download large specifications or API catalogs with raw_call. May spend Base USDC. A failed call may have paid or completed a side effect; do not automatically repeat it. Returns text or supported images.".into(), super::raw::schema()),

        ("x402_treazury_tools_search", "Find currently callable API tools and complete input schemas on this MCP endpoint. Omit source_name to search all configured and registered APIs; use a returned registration name to narrow results. Omit query to enumerate; follow next_cursor for more. Invoke a returned tool_id with tool_call on this same MCP endpoint. Help is optional; unknown cost is not free and metadata does not prove paid delivery.".into(), search_schema()),
        ("x402_treazury_tool_call", "Invoke a currently available tool_id with arguments matching its input schema. Use this same MCP endpoint as tools_search; wrappers on other endpoints have different catalogs. Reuse IDs across turns or restarts while available; search again when you need the current schema. Calls may charge. Do not automatically repeat a failed paid call.".into(), object(json!({"tool_id":{"type":"string","description":"Exact tool_id returned by tools_search on this MCP endpoint. Reusable while available."},"arguments":{"type":"object","additionalProperties":true,"description":"Object matching the selected tool input_schema; use {} when it takes no arguments."}}), &["tool_id","arguments"])),
    ].into_iter().map(|(name, description, schema): (&'static str, String, Value)| Tool::new(name,description,Arc::new(schema.as_object().unwrap().clone()))).collect()
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
                    for choice in ["coinbase", "x402 List", "local_only=true"] {
                        assert!(help.contains(choice), "missing directory choice {choice}");
                    }
                    assert!(help.contains("no pagination"));
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
                assert!(description.contains("same MCP endpoint"));
                assert!(description.contains("tool_id"));
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
        let local = description(super::super::local::NAME);
        assert!(local.contains("provider_id directly to source_add"));
        assert!(local.contains("no configured base override"));
        assert!(local.contains("schema server, not the specification URL origin"));
        let details = description(super::super::source_details::NAME);
        assert!(details.starts_with("Optional assistance"));
        assert!(details.contains("x402 List slug"));
        assert!(details.contains("documentation_url"));
        assert!(details.contains("HTML/non-text"));
        assert!(description("x402_treazury_source_add").starts_with("Register directly"));
        for name in ["x402_treazury_source_add", "x402_treazury_tools_search"] {
            let help = description(name);
            assert!(help.to_lowercase().contains("unknown cost is not free"));
            assert!(help.to_lowercase().contains("help is optional"));
            assert!(help.contains("paid delivery"));
        }
        let raw = description(super::super::raw::NAME);
        assert!(raw.contains("exact documented"));
        assert!(raw.contains("never guess paid request arguments"));
        assert!(raw.contains("do not automatically repeat"));
        assert!(raw.contains("do not download large specifications"));
        assert!(tools.iter().all(|tool| {
            !tool
                .description
                .as_deref()
                .unwrap()
                .contains("description snapshot")
        }));
    }
}
