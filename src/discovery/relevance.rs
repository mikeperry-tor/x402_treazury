//! Project the caller's filtered API tools into the shared local search ranker.
use crate::catalog_state::BoundTool;

pub(super) fn rank<'a>(tools: Vec<&'a BoundTool>, query: &str) -> Vec<&'a BoundTool> {
    crate::relevance::rank(
        tools,
        query,
        |tool| {
            let literal = format!("{} {}", tool.tool.name, tool.tool.description);
            let parameters = tool.tool.input_schema["properties"]
                .as_object()
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
        |tool| tool.tool.name.as_str(),
    )
}

#[cfg(test)]
#[path = "relevance_tests.rs"]
mod tests;
