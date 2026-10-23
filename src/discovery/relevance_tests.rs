use super::*;
use crate::{
    catalog::ToolSpec,
    payment::{PaidClient, Payer, SpendPolicy},
};
use serde_json::json;

fn tool(name: &str, description: &str, parameters: &[&str]) -> BoundTool {
    let properties: serde_json::Map<String, serde_json::Value> = parameters
        .iter()
        .map(|name| ((*name).into(), json!({"type":"string"})))
        .collect();
    let spec: ToolSpec = serde_json::from_value(json!({
        "name":name,"description":description,"method":"GET","path":"/",
        "input_schema":{"type":"object","properties":properties},
        "param_routes":{},"has_body":false
    }))
    .unwrap();
    let payer = PaidClient::new(
        Payer::new(
            &format!("{:064x}", 1),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    );
    crate::catalog_state::bind(vec![(spec, payer, "https://api.example.com".into())]).remove(0)
}

fn names<'a>(tools: &'a [BoundTool], query: &str) -> Vec<&'a str> {
    rank(tools.iter().collect(), query)
        .into_iter()
        .map(|tool| tool.tool.name.as_str())
        .collect()
}

#[test]
fn multiword_queries_rank_nonadjacent_terms_and_retain_or_matches() {
    let tools = [
        tool("aaa_business", "Look up a Yelp business", &[]),
        tool(
            "zzz_reviews",
            "List public reviews for a Yelp business",
            &[],
        ),
        tool("other", "Read Telegram posts", &[]),
    ];
    assert_eq!(
        names(&tools, "YELP REVIEWS"),
        ["zzz_reviews", "aaa_business"]
    );
    assert_eq!(names(&tools, "reviews yelp"), names(&tools, "YELP REVIEWS"));
    assert_eq!(
        names(&tools, "reviews yelp yelp reviews"),
        names(&tools, "reviews yelp")
    );
    assert!(names(&tools, "nonexistent").is_empty());
}

#[test]
fn literal_phrases_and_partial_identifiers_keep_priority() {
    let tools = [
        tool("aaa", "reviews reviews from a Yelp business", &[]),
        tool("zzz", "Yelp reviews and additional descriptive detail", &[]),
        tool(
            "socialfetch_yelp_businesses_alias_reviews",
            "Read public feedback",
            &[],
        ),
    ];
    assert_eq!(names(&tools, "Yelp reviews")[0], "zzz");
    assert_eq!(
        names(&tools, "fetch_yelp_businesses_alias_reviews")[0],
        "socialfetch_yelp_businesses_alias_reviews"
    );
}

#[test]
fn tokenization_indexes_parameter_names_without_searching_schema_prose() {
    let tools = [
        tool(
            "lookup",
            "Resolve identity",
            &["business_id", "entityId", "地域"],
        ),
        tool("other", "Other operation", &[]),
    ];
    for query in ["business", "BUSINESS_ID", "entityId", "地域"] {
        assert_eq!(names(&tools, query), ["lookup"]);
    }
    let mut tools = tools;
    tools[1].tool.input_schema["properties"] = json!({"value":{"description":"sentinelword"}});
    assert!(names(&tools, "sentinelword").is_empty());
}

#[test]
fn blank_queries_preserve_enumeration_and_punctuation_uses_literal_matching() {
    let tools = [
        tool("z", "Punctuation 💥", &[]),
        tool("a", "Plain text", &[]),
    ];
    for query in ["", " ", "\t\n"] {
        assert_eq!(names(&tools, query), ["z", "a"]);
    }
    assert_eq!(names(&tools, "💥"), ["z"]);
    assert!(names(&tools, "***").is_empty());
    assert!(rank(vec![], "yelp reviews").is_empty());
    let punctuation = [tool("!", "?", &[])];
    assert!(names(&punctuation, "word").is_empty());
    assert_eq!(names(&punctuation, "?"), ["!"]);
}

#[test]
fn equal_scores_use_tool_names_and_document_length_affects_relevance() {
    let tied = [
        tool("zeta", "rare term", &[]),
        tool("beta", "rare term", &[]),
    ];
    assert_eq!(names(&tied, "rare"), ["beta", "zeta"]);
    let lengths = [
        tool(
            "aaa",
            "rare term with many additional words describing unrelated behavior",
            &[],
        ),
        tool("zzz", "rare term", &[]),
    ];
    assert_eq!(names(&lengths, "rare"), ["zzz", "aaa"]);
}
