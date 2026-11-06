//! Independent witnesses for the authored Strale methods and 2s CSV schemas.
use serde_json::{Value, json};
use x402_treazury::{catalog, providers};

fn bundled(id: &str) -> (Value, Vec<catalog::ToolSpec>) {
    let provider = providers::get(id).unwrap();
    assert_eq!(provider.metadata()["schema"]["kind"], "bundled");
    let document: Value = serde_json::from_str(provider.local_schema.unwrap()).unwrap();
    let tools = catalog::build_tools(
        &provider.config,
        &document,
        provider.config.prefix.as_deref().unwrap(),
    )
    .unwrap();
    (document, tools)
}

#[test]
fn strale_post_repairs_preserve_complete_catalog_inputs_and_nested_json_requests() {
    let witness: Value =
        serde_json::from_str(include_str!("fixtures/catalogs/strale_post_inputs.json")).unwrap();
    let (document, tools) = bundled("strale");
    let cases = [
        (
            "api-health-check",
            json!({"url":"https://example.com", "headers":{"X-Probe":"retained"}, "expected_schema":{"type":"object"}, "body":[1,null,true]}),
        ),
        (
            "api-mock-response",
            json!({"url":"https://example.com", "response_schema":{"type":"object","properties":{"n":{"type":"integer"}}}}),
        ),
        (
            "fake-data-generate",
            json!({"count":1,"fields":[{"name":"n","type":"integer"}],"schema":{"type":"object"}}),
        ),
        (
            "html-to-pdf",
            json!({"html":"<p>Example</p>","margins":{"top":"1cm"},"landscape":true}),
        ),
        (
            "http-to-curl",
            json!({"url":"https://example.com/contract-probe","method":"POST","headers":{"X-Probe":"retained"},"auth":{"type":"bearer","token":"public-test-token"},"body":{"nested":[1,null,true]}}),
        ),
        (
            "json-schema-validate",
            json!({"data":{"n":"wrong"},"schema":{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}}),
        ),
        (
            "llm-output-validate",
            json!({"llm_output":"{\"n\":1}","expected_schema":{"type":"object"},"strict_mode":true}),
        ),
        (
            "risk-narrative-generate",
            json!({"context":"kyb","check_results":{"registry":{"found":true,"missing":null}}}),
        ),
        (
            "tool-call-validate",
            json!({"tool_name":"example","tool_input":{"nested":[1,null,true]},"expected_schema":{"type":"object"}}),
        ),
    ];
    assert_eq!(
        witness["capabilities"].as_array().unwrap().len(),
        cases.len()
    );
    for (slug, args) in cases {
        let captured = witness["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["slug"] == slug)
            .unwrap();
        assert_eq!(captured["advertised_method"], "GET");
        let path = format!("/x402/v2/{slug}");
        let item = &document["paths"][&path];
        assert!(item.get("get").is_none(), "{slug}");
        assert!(item["post"].get("parameters").is_none(), "{slug}");
        assert_eq!(item["post"]["requestBody"]["required"], true);
        assert_eq!(
            item["post"]["requestBody"]["content"]["application/json"]["schema"],
            captured["input_schema"],
            "{slug} must retain the complete catalog body schema"
        );
        let tool = tools.iter().find(|tool| tool.path == path).unwrap();
        assert_eq!(tool.name, format!("strale_{}", slug.replace('-', "_")));
        assert_eq!(tool.method, "POST");
        let restored: catalog::ToolSpec =
            serde_json::from_value(serde_json::to_value(tool).unwrap()).unwrap();
        let request = restored
            .route("https://api.strale.io", args.as_object().unwrap())
            .unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, captured["endpoint"].as_str().unwrap());
        assert_eq!(request.json_media_type.as_deref(), Some("application/json"));
        assert!(request.query.is_empty());
        assert_eq!(request.body, Some(args), "{slug}");
    }
}

#[test]
fn twos_csv_repairs_preserve_domains_counts_defaults_and_scalar_wire_values() {
    let witness: Value =
        serde_json::from_str(include_str!("fixtures/catalogs/twos_csv_parameters.json")).unwrap();
    let (document, tools) = bundled("2s");
    let cases = [
        (
            "/api/climate/station-history",
            "dataTypes",
            "TMAX,PRCP",
            "TMAX,INVALID",
            "TMAX",
            10,
            None,
            None,
            Some("TMAX,TMIN,PRCP"),
        ),
        (
            "/api/crypto/kimchi-premium",
            "symbol",
            "BTC,ETH",
            "btc,ETH",
            "BTC",
            10,
            None,
            Some(120),
            Some("BTC"),
        ),
        (
            "/api/crypto/token-price",
            "ids",
            "bitcoin,ethereum",
            "Bitcoin,ethereum",
            "btc",
            25,
            Some(2),
            Some(600),
            None,
        ),
        (
            "/api/crypto/token-price",
            "vs",
            "usd,eur",
            "usd,EUR",
            "usd",
            5,
            None,
            Some(60),
            Some("usd"),
        ),
    ];
    assert_eq!(witness["parameters"].as_array().unwrap().len(), cases.len());
    for (path, name, valid, invalid, element, count, min, max, default) in cases {
        let original = witness["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["path"] == path && item["parameter"]["name"] == name)
            .unwrap();
        assert!(original["parameter"]["schema"]["allOf"].is_array());
        let parameter = document["paths"][path]["get"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == name)
            .unwrap();
        for key in ["name", "in", "description", "required"] {
            assert_eq!(parameter[key], original["parameter"][key]);
        }
        let schema = &parameter["schema"];
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["minLength"].as_u64(), min);
        assert_eq!(schema["maxLength"].as_u64(), max);
        assert_eq!(schema["default"].as_str(), default);
        let pattern = regex::Regex::new(schema["pattern"].as_str().unwrap()).unwrap();
        assert!(pattern.is_match(valid));
        assert!(!pattern.is_match(invalid));
        assert!(!pattern.is_match(""));
        assert!(!pattern.is_match(&vec![element; count + 1].join(",")));
        assert!(pattern.is_match(&vec![element; count].join(",")));
        let tool = tools.iter().find(|tool| tool.path == path).unwrap();
        let mut expected = schema.clone();
        expected["description"] = parameter["description"].clone();
        assert_eq!(tool.input_schema["properties"][name], expected);
        let mut args = match path {
            "/api/climate/station-history" => {
                json!({"station":"USW00094728","startDate":"2025-01-01","endDate":"2025-01-02"})
            }
            "/api/crypto/token-price" => json!({"ids":"bitcoin,ethereum"}),
            _ => json!({}),
        };
        args[name] = json!(valid);
        let request = tool
            .route("https://2s.io", args.as_object().unwrap())
            .unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, format!("https://2s.io{path}"));
        assert_eq!(request.query[name], valid);
        assert!(request.body.is_none());
    }
}

#[test]
fn twos_request_snapshot_keeps_incompatible_workflows_excluded() {
    let (document, tools) = bundled("2s");
    let provider = providers::get("2s").unwrap();
    for operation in [
        "POST /api/batch/run",
        "GET /api/countdown/gif",
        "GET /api/gov/osha-violations",
    ] {
        assert!(
            provider
                .config
                .exclude_operations
                .iter()
                .any(|v| v == operation)
        );
        let (method, path) = operation.split_once(' ').unwrap();
        assert!(document["paths"][path][method.to_ascii_lowercase()].is_object());
        assert!(!tools.iter().any(|tool| tool.path == path));
    }
    for namespace in ["store", "lock", "queue", "schedule", "pubsub", "watchers"] {
        let path = format!("/api/{namespace}");
        assert!(provider.config.exclude.contains(&path));
        assert!(
            !tools
                .iter()
                .any(|tool| tool.path.starts_with(&format!("{path}/")))
        );
    }
    assert!(document["components"].get("responses").is_none());
    for item in document["paths"].as_object().unwrap().values() {
        for method in [
            "get", "post", "put", "patch", "delete", "head", "options", "trace",
        ] {
            if let Some(operation) = item.get(method) {
                assert!(operation.get("responses").is_none());
            }
        }
    }
}
