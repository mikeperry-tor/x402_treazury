use super::*;

fn args(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn exact_translation_preserves_query_values_and_server_payment_filters() {
    let url = request(&args(json!({
        "q":" Weather & clouds ","curated_only":false,"limit":2,
        "max_price_usd":"upstream validates syntax", "pay_to":"merchant",
        "url_substring":"API/Weather", "tags":["Weather","Weather","cloud & sky"],
        "bundle_slugs":["weather-kit","kit2"],"extensions":["bazaar","unknown"]
    })))
    .unwrap();
    let pairs: Vec<_> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert!(pairs.contains(&("query".into(), " Weather & clouds ".into())));
    assert!(pairs.contains(&("curatedOnly".into(), "false".into())));
    assert!(pairs.contains(&("maxUsdPrice".into(), "upstream validates syntax".into())));
    assert_eq!(
        pairs
            .iter()
            .filter(|(k, _)| k == "tags")
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>(),
        ["Weather", "Weather", "cloud & sky"]
    );
    assert_eq!(
        pairs
            .iter()
            .filter(|(k, _)| k == "bundleSlugs")
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>(),
        ["weather-kit", "kit2"]
    );
    for (key, value) in [
        ("network", "eip155:8453"),
        ("scheme", "exact"),
        ("asset", "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
    ] {
        assert_eq!(
            pairs
                .iter()
                .filter(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .collect::<Vec<_>>(),
            [value]
        );
    }
    let defaults = request(&Map::new()).unwrap();
    let query: std::collections::BTreeMap<_, _> = defaults.query_pairs().collect();
    assert_eq!(query["curatedOnly"], "true");
    assert_eq!(query["limit"], "20");
    assert!(!query.contains_key("query"));
    let empty = request(&args(
        json!({"q":"","tags":[],"bundle_slugs":[],"extensions":[]}),
    ))
    .unwrap();
    assert!(
        empty
            .query_pairs()
            .any(|(k, v)| k == "query" && v.is_empty())
    );
    assert!(
        !empty
            .query_pairs()
            .any(|(k, _)| matches!(k.as_ref(), "tags" | "bundleSlugs" | "extensions"))
    );
}

#[test]
fn branch_validation_rejects_mixed_fields_and_respects_upstream_bounds() {
    for invalid in [
        json!({"mode":"browse"}),
        json!({"mode":null}),
        json!({"local_only":null}),
        json!({"page":1}),
        json!({"per_page":1}),
        json!({"cursor":"next"}),
        json!({"network":"BSE"}),
        json!({"asset":"usdc"}),
        json!({"scheme":"exact"}),
        json!({"max_price_usd":0.1}),
        json!({"curated_only":null}),
        json!({"limit":0}),
        json!({"limit":21}),
        json!({"limit":1.5}),
        json!({"q":"é".repeat(401)}),
        json!({"tags":["1","2","3","4","5","6"]}),
        json!({"tags":[null]}),
        json!({"extensions":[1]}),
        json!({"bundle_slugs":["Bad"]}),
        json!({"bundle_slugs":["a--b"]}),
        json!({"bundle_slugs":[""]}),
        json!({"bundle_slugs":["x".repeat(65)]}),
        json!({"pay_to":""}),
        json!({"pay_to":"x".repeat(129)}),
        json!({"url_substring":"ab"}),
        json!({"url_substring":"x".repeat(2049)}),
        json!({"category":"Weather"}),
    ] {
        assert!(
            request(&args(invalid.clone())).is_err(),
            "accepted {invalid}"
        );
    }
    for valid in [
        json!({"q":"é".repeat(400)}),
        json!({"limit":1}),
        json!({"limit":20}),
        json!({"bundle_slugs":["a-2"]}),
        json!({"pay_to":"x".repeat(128)}),
        json!({"url_substring":"abc"}),
        json!({"url_substring":"x".repeat(2048)}),
        json!({"extensions":vec!["x"; 100]}),
    ] {
        assert!(request(&args(valid.clone())).is_ok(), "rejected {valid}");
    }
}

#[test]
fn complete_unknown_partial_resource_contracts_are_preserved() {
    let upstream = json!({"resources":[
        {"resource":"https://example.com/a","method":"GET","protocol":"http","unknown":{"x":1}},
        {"resource":"https://example.com/a","method":"POST","protocol":"http","input":{"example":"not a schema"}},
        {"protocol":"mcp","offers":[{"network":"unknown"}]},{}
    ],"partialResults":true,"x402Version":"future","searchMethod":{"new":true},"extra":[1,2]});
    let bytes = serde_json::to_vec(&upstream).unwrap();
    assert_eq!(parse_response(&bytes).unwrap(), upstream);
    for via in ["origin", "curl"] {
        let text = output(upstream.clone(), via).unwrap().into_text().unwrap();
        let result: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(result["response"], upstream);
        assert_eq!(result["acquisition"]["via"], via);
        assert_eq!(result["directory"], "coinbase");
        assert!(result.get("total").is_none());
        assert!(result.get("next_cursor").is_none());
    }
    let missing_version = json!({"resources":[],"partialResults":false});
    assert_eq!(
        parse_response(&serde_json::to_vec(&missing_version).unwrap()).unwrap(),
        missing_version
    );
}

#[test]
fn malformed_top_level_is_failure_without_invented_empty_results() {
    for invalid in [
        json!([]),
        json!({}),
        json!({"resources":[],"partialResults":null}),
        json!({"resources":[null],"partialResults":false}),
        json!({"resources":{},"partialResults":false}),
    ] {
        assert!(parse_response(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    assert!(parse_response(b"{incomplete").is_err());
}

#[test]
fn flat_schema_has_four_exclusive_branches_and_safe_root_defaults() {
    let schema = super::super::directory::search_schema();
    let properties = schema["properties"].as_object().unwrap();
    for (name, field) in properties {
        assert!(
            name == "local_only" || field.get("default").is_none(),
            "unsafe root default for {name}"
        );
    }
    assert_eq!(schema["oneOf"].as_array().unwrap().len(), 4);
    let branches = schema["oneOf"].as_array().unwrap();
    assert!(branches[2]["properties"].get("directory").is_none());
    assert_eq!(branches[3]["required"], json!(["directory"]));
    assert_eq!(branches[3]["properties"]["max_price_usd"]["type"], "string");
    assert_eq!(branches[1]["properties"]["max_price_usd"]["type"], "number");
    assert_eq!(
        properties["max_price_usd"]["type"],
        json!(["number", "string"])
    );
}
