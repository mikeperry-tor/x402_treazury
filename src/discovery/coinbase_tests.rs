use super::*;

fn args(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn exact_translation_preserves_shared_query_and_server_payment_filters() {
    let url = request(
        &args(json!({
            "q":" Weather & clouds ","limit":2,"max_price_usd":0.05
        })),
        true,
    )
    .unwrap();
    let pairs: Vec<_> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert!(pairs.contains(&("query".into(), " Weather & clouds ".into())));
    assert!(pairs.contains(&("maxUsdPrice".into(), "0.05".into())));
    for (key, value) in [
        ("curatedOnly", "true"),
        ("limit", "2"),
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
    assert_eq!(
        pairs.len(),
        7,
        "only shared arguments and fixed payment/curation filters are sent"
    );
    let defaults = request(&Map::new(), true).unwrap();
    let query: std::collections::BTreeMap<_, _> = defaults.query_pairs().collect();
    assert_eq!(query["curatedOnly"], "true");
    assert_eq!(query["limit"], "5");
    assert!(!query.contains_key("query"));
    assert!(!query.contains_key("maxUsdPrice"));
    let empty = request(&args(json!({"q":""})), true).unwrap();
    assert!(
        empty
            .query_pairs()
            .any(|(k, v)| k == "query" && v.is_empty())
    );
    let unicode = "é🌧".repeat(200);
    let encoded = request(&args(json!({"q":unicode})), true).unwrap();
    assert_eq!(
        encoded
            .query_pairs()
            .find(|(key, _)| key == "query")
            .unwrap()
            .1,
        unicode
    );
}

#[test]
fn numeric_price_uses_json_number_without_a_float_roundtrip() {
    // These exact integers lose precision if converted through f64 before encoding.
    for price in [
        json!(0),
        json!(0.05),
        json!(9_007_199_254_740_993_u64),
        json!(u64::MAX),
    ] {
        let url = request(&args(json!({"max_price_usd":price})), true).unwrap();
        let translated = url
            .query_pairs()
            .find(|(key, _)| key == "maxUsdPrice")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(translated, price.as_number().unwrap().to_string());
    }
}

#[test]
fn branch_validation_rejects_removed_fields_and_respects_shared_bounds() {
    for invalid in [
        json!({"mode":"best"}),
        json!({"directory":"coinbase"}),
        json!({"local_only":false}),
        json!({"mode":"browse"}),
        json!({"mode":null}),
        json!({"local_only":null}),
        json!({"page":1}),
        json!({"per_page":1}),
        json!({"cursor":"next"}),
        json!({"network":"BSE"}),
        json!({"asset":"usdc"}),
        json!({"scheme":"exact"}),
        json!({"max_price_usd":"0.05"}),
        json!({"max_price_usd":-0.05}),
        json!({"max_price_usd":null}),
        json!({"curated_only":true}),
        json!({"curated_only":false}),
        json!({"tags":["Weather"]}),
        json!({"bundle_slugs":["weather-kit"]}),
        json!({"extensions":["bazaar"]}),
        json!({"pay_to":"merchant"}),
        json!({"url_substring":"api/weather"}),
        json!({"limit":0}),
        json!({"limit":21}),
        json!({"limit":1.5}),
        json!({"q":"é".repeat(401)}),
        json!({"category":"Weather"}),
    ] {
        assert!(
            request(&args(invalid.clone()), true).is_err(),
            "accepted {invalid}"
        );
    }
    for valid in [
        json!({"q":"é".repeat(400)}),
        json!({"q":"🌧".repeat(400)}),
        json!({"limit":1}),
        json!({"limit":20}),
        json!({"max_price_usd":0}),
        json!({"max_price_usd":0.05}),
    ] {
        assert!(
            request(&args(valid.clone()), true).is_ok(),
            "rejected {valid}"
        );
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
        let output = output(upstream.clone(), via, true).unwrap();
        assert!(output.payment_may_have_occurred);
        assert_eq!(output.advisories.is_empty(), via == "origin");
        let text = output.text;
        let result: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(result, upstream);
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
    for invalid in [json!([]), json!(null), json!("not an object")] {
        let error = parse_response(&serde_json::to_vec(&invalid).unwrap()).unwrap_err();
        assert_eq!(
            crate::discovery_relay::RelayFailure::from_error(&error).explanation(),
            "The returned content did not meet the required response contract"
        );
    }
    let error = parse_response(b"{incomplete").unwrap_err();
    assert_eq!(
        crate::discovery_relay::RelayFailure::from_error(&error).explanation(),
        "The returned content did not meet the required response contract"
    );
}

#[test]
fn flat_schema_uses_only_shared_arguments_and_endpoint_specific_query_bound() {
    for coinbase in [false, true] {
        let schema = super::super::directory::search_schema(coinbase);
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(
            properties
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["q", "limit", "max_price_usd"])
        );
        assert!(schema.get("oneOf").is_none());
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            properties["q"].get("maxLength").cloned(),
            coinbase.then(|| json!(400))
        );
        assert_eq!(properties["max_price_usd"]["type"], "number");
        assert_eq!(properties["max_price_usd"]["minimum"], 0);
        assert_eq!(properties["limit"]["default"], 5);
        assert_eq!(properties["limit"]["minimum"], 1);
        assert_eq!(properties["limit"]["maximum"], 20);
        let long_query = args(json!({"q":"é".repeat(401)}));
        assert_eq!(
            super::super::directory::validate_search(&long_query, coinbase).is_err(),
            coinbase
        );
    }
    for curated in [false, true] {
        let url = request(&Map::new(), curated).unwrap();
        assert_eq!(
            url.query_pairs()
                .find(|(key, _)| key == "curatedOnly")
                .unwrap()
                .1,
            curated.to_string()
        );
    }
    for native in [
        json!({}),
        json!({"resources":[],"partialResults":null}),
        json!({"newNativeField":[null,1]}),
    ] {
        assert_eq!(
            parse_response(&serde_json::to_vec(&native).unwrap()).unwrap(),
            native
        );
    }
}

#[tokio::test]
async fn origin_transport_failure_leaves_warning_to_terminal_search() {
    use tracing::instrument::WithSubscriber;
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    let url =
        reqwest::Url::parse(&format!("http://{address}/PRIVATE_PATH?key=PRIVATE_KEY")).unwrap();
    let selected = network::global()
        .http_participating(
            &network::IsolationId::bootstrap(),
            url.as_str(),
            None,
            false,
            network::HttpPolicy::default(),
        )
        .unwrap();
    let logs = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_env_filter("warn,x402_treazury=debug")
        .with_writer(logs.reopen().unwrap())
        .finish();
    let result = origin(&url, &selected, 1024)
        .with_subscriber(subscriber)
        .await;
    assert!(matches!(
        result.unwrap_err().kind,
        network::acquisition::FailureKind::Transport
    ));
    let text = std::fs::read_to_string(logs.path()).unwrap();
    assert!(
        !text.contains("WARN") && text.contains("DEBUG") && text.contains("coinbase_search_origin"),
        "{text}"
    );
    assert!(
        !text.contains("PRIVATE_") && !text.contains(&address.to_string()),
        "{text}"
    );
}
