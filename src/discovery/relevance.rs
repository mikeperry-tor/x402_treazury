//! Rank borrowed API and first-party definitions using their canonical text.
use crate::catalog_state::BoundTool;
use rmcp::model::Tool;
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub(super) enum SearchCandidate<'a> {
    Api(&'a BoundTool),
    FirstParty(&'a Tool),
}
impl SearchCandidate<'_> {
    fn name(&self) -> &str {
        match self {
            Self::Api(tool) => &tool.tool.name,
            Self::FirstParty(tool) => &tool.name,
        }
    }
    fn canonical_description(&self) -> &str {
        match self {
            Self::Api(tool) => &tool.tool.description,
            Self::FirstParty(tool) => tool.description.as_deref().unwrap_or_default(),
        }
    }
    fn parameters(&self) -> Option<&serde_json::Map<String, Value>> {
        let properties = match self {
            Self::Api(tool) => tool.tool.input_schema.get("properties"),
            Self::FirstParty(tool) => tool.input_schema.get("properties"),
        };
        properties.and_then(Value::as_object)
    }
    pub(super) fn render(&self) -> Value {
        match self {
            Self::Api(tool) => {
                json!({"tool_id":tool.tool.name,"description":tool.description(),"input_schema":tool.tool.input_schema})
            }
            Self::FirstParty(tool) => {
                json!({"tool_id":tool.name,"description":tool.description,"input_schema":tool.input_schema})
            }
        }
    }
}

pub(super) fn rank_candidates<'a>(
    tools: Vec<SearchCandidate<'a>>,
    query: &str,
) -> Vec<SearchCandidate<'a>> {
    crate::relevance::rank(
        tools,
        query,
        |tool| {
            let literal = format!("{} {}", tool.name(), tool.canonical_description());
            let parameters = tool
                .parameters()
                .map(|properties| {
                    properties
                        .keys()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            (literal, parameters)
        },
        |tool| tool.name(),
    )
}

#[cfg(test)]
fn rank<'a>(tools: Vec<&'a BoundTool>, query: &str) -> Vec<&'a BoundTool> {
    rank_candidates(tools.into_iter().map(SearchCandidate::Api).collect(), query)
        .into_iter()
        .map(|tool| match tool {
            SearchCandidate::Api(tool) => tool,
            SearchCandidate::FirstParty(_) => unreachable!(),
        })
        .collect()
}

#[cfg(test)]
#[path = "relevance_tests.rs"]
mod tests;
