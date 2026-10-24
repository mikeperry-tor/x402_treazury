//! Declared header contracts through public catalog generation and routing.
use serde_json::{Value, json};
use x402_treazury::catalog::{Config, ToolSpec, build_tools};

fn tool(parameters: Value) -> ToolSpec {
    build_tools(
        &Config::default(),
        &json!({"paths":{"/purchase":{"post":{
            "parameters":parameters
        }}}}),
        "fixture",
    )
    .unwrap()
    .remove(0)
}

#[test]
fn declared_headers_preserve_case_insensitive_overrides_and_simple_serialization() {
    let document = json!({"paths":{"/purchase":{
        "parameters":[{"name":"X-Quote-Id","in":"header","required":false,"schema":{"type":"integer"}}],
        "post":{"parameters":[
            {"name":"x-quote-id","in":"header","required":true,"schema":{"type":"string"}},
            {"name":"X-Flags","in":"header","schema":{"type":"array","items":{"type":"string"}}},
            {"name":"X-Options","in":"header","style":"simple","explode":true,"schema":{"type":"object"}},
            {"name":"X-Pairs","in":"header","schema":{"type":"object"}}
        ]}
    }}});
    let tool = build_tools(&Config::default(), &document, "fixture")
        .unwrap()
        .remove(0);
    assert_eq!(tool.header_params.len(), 4);
    assert_eq!(
        tool.input_schema["properties"]["x-quote-id"]["type"],
        "string"
    );
    assert_eq!(tool.input_schema["required"], json!(["x-quote-id"]));
    let args = json!({"x-quote-id":"q_original","X-Flags":["a","b"],"X-Options":{"a":1,"b":true},"X-Pairs":{"a":1,"b":true}});
    let route = tool
        .route("https://example.com", args.as_object().unwrap())
        .unwrap();
    assert_eq!(
        serde_json::to_value(route.headers).unwrap(),
        json!({
            "x-quote-id":"q_original","x-flags":"a,b","x-options":"a=1,b=true","x-pairs":"a,1,b,true"
        })
    );
    assert!(route.query.is_empty());
    assert!(route.body.is_none());
}

#[test]
fn header_aliases_survive_tool_serialization_without_overwriting_query_or_body() {
    let document = json!({"paths":{"/purchase":{"post":{
        "parameters":[{"name":"shared","in":"query","schema":{"type":"string"}},
            {"name":"shared","in":"header","required":true,"schema":{"type":"string"}},
            {"name":"body-key","in":"header","schema":{"type":"string"}}],
        "requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"body-key":{"type":"string"}}}}}}
    }}}});
    let tool = build_tools(&Config::default(), &document, "fixture")
        .unwrap()
        .remove(0);
    let restored: ToolSpec = serde_json::from_value(serde_json::to_value(&tool).unwrap()).unwrap();
    for tool in [tool, restored] {
        let args = json!({"shared":"query","shared_header":"header","body-key":"body","body-key_header":"another header"});
        let route = tool
            .route("https://example.com", args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.query["shared"], "query");
        assert_eq!(route.headers["shared"], "header");
        assert_eq!(route.headers["body-key"], "another header");
        assert_eq!(route.body, Some(json!({"body-key":"body"})));
    }
}

#[test]
fn declared_idempotency_is_owned_per_invocation_and_never_derived_from_arguments() {
    let tool = tool(json!([
        {"name":"IDEMPOTENCY-KEY","in":"header","required":true,"schema":{"type":"string","format":"uuid","maxLength":36}},
        {"name":"X-Quote-Id","in":"header","required":true,"schema":{"type":"string"}}
    ]));
    assert!(
        tool.input_schema["properties"]
            .get("IDEMPOTENCY-KEY")
            .is_none()
    );
    assert_eq!(tool.input_schema["required"], json!(["X-Quote-Id"]));
    let args = json!({"X-Quote-Id":"q_original"});
    let first = tool
        .route("https://example.com", args.as_object().unwrap())
        .unwrap();
    let second = tool
        .route("https://example.com", args.as_object().unwrap())
        .unwrap();
    assert!(uuid::Uuid::parse_str(&first.headers["idempotency-key"]).is_ok());
    assert_ne!(
        first.headers["idempotency-key"],
        second.headers["idempotency-key"]
    );
    assert_eq!(first.headers["x-quote-id"], "q_original");
    assert!(
        tool.route(
            "https://example.com",
            json!({"X-Quote-Id":"q","IDEMPOTENCY-KEY":"caller-key"})
                .as_object()
                .unwrap()
        )
        .is_err()
    );
}

#[test]
fn a_differently_cased_query_argument_cannot_replace_the_managed_header() {
    let tool = tool(json!([
        {"name":"Idempotency-Key","in":"header","schema":{"type":"string"}},
        {"name":"idempotency-key","in":"query","schema":{"type":"string"}}
    ]));
    let route = tool
        .route(
            "https://example.com",
            json!({"idempotency-key":"query-value"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(route.query["idempotency-key"], "query-value");
    assert!(uuid::Uuid::parse_str(&route.headers["idempotency-key"]).is_ok());
}

#[test]
fn optional_declared_idempotency_is_also_automatic_and_missing_headers_fail_before_io() {
    let tool = tool(json!([
        {"name":"Idempotency-Key","in":"header","schema":{"type":"string"}},
        {"name":"X-Quote-Id","in":"header","required":true,"schema":{"type":"string"}},
        {"name":"X-Optional","in":"header","schema":{"type":"string"}}
    ]));
    assert!(
        tool.route("https://example.com", json!({}).as_object().unwrap())
            .is_err()
    );
    let route = tool
        .route(
            "https://example.com",
            json!({"X-Quote-Id":"q","X-Optional":null})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(route.headers.len(), 2);
    for args in [
        json!({"X-Quote-Id":"q\r\ninjected: yes"}),
        json!({"X-Quote-Id":{"nested":[]}}),
    ] {
        let error = tool
            .route("https://example.com", args.as_object().unwrap())
            .unwrap_err();
        assert!(!format!("{error:#}").contains("injected"));
    }
}

#[test]
fn unsupported_header_contracts_and_alias_collisions_are_visible_errors() {
    for parameter in [
        json!({"name":"X-Test","in":"header","style":"form"}),
        json!({"name":"X-Test","in":"header","content":{"application/json":{"schema":{"type":"object"}}}}),
        json!({"name":"bad header","in":"header"}),
        json!({"name":"Idempotency-Key","in":"header","schema":{"type":"integer"}}),
        json!({"name":"Idempotency-Key","in":"header","schema":{"type":"string","pattern":"^purchase-"}}),
        json!({"name":"Idempotency-Key","in":"header","schema":{"type":"string","maxLength":8}}),
        json!({"name":"Idempotency-Key","in":"header","schema":{"type":"string","not":{"type":"string"}}}),
    ] {
        assert!(
            build_tools(
                &Config::default(),
                &json!({"paths":{"/purchase":{"post":{"parameters":[parameter]}}}}),
                "fixture"
            )
            .is_err()
        );
    }
    assert!(build_tools(&Config::default(), &json!({"paths":{"/purchase":{"post":{"parameters":[
        {"name":"x","in":"query"},{"name":"x_header","in":"query"},{"name":"x","in":"header"}
    ]}}}}), "fixture").is_err());
}

#[test]
fn automatic_key_constraints_survive_schema_and_parameter_references() {
    let mut document = json!({"components":{
        "schemas":{"Key":{"type":"string","format":"uuid"},"Alias":{"$ref":"#/components/schemas/Key"}},
        "parameters":{"Key":{"name":"Idempotency-Key","in":"header","required":true,"schema":{"$ref":"#/components/schemas/Alias"}}}
    },"paths":{"/purchase":{"post":{"parameters":[{"$ref":"#/components/parameters/Key"}]}}}});
    let generated = build_tools(&Config::default(), &document, "fixture")
        .unwrap()
        .remove(0)
        .route("https://example.com", json!({}).as_object().unwrap())
        .unwrap();
    assert!(uuid::Uuid::parse_str(&generated.headers["idempotency-key"]).is_ok());
    for invalid in [
        json!({"$ref":"#/components/schemas/Key","maxLength":8}),
        json!({"$ref":"#/components/schemas/Key","not":{"type":"string"}}),
        json!({"$ref":"#/components/schemas/Missing"}),
        json!({"$ref":"https://example.com/key-schema"}),
        json!({"$ref":"#/components/schemas/Alias"}),
    ] {
        document["components"]["schemas"]["Alias"] = invalid;
        assert!(build_tools(&Config::default(), &document, "fixture").is_err());
    }
    document["paths"]["/purchase"]["post"]["parameters"] = json!([
        {"name":"Idempotency-Key","in":"header","schema":{"$ref":"#/components/schemas/Key","maxLength":8}}
    ]);
    assert!(build_tools(&Config::default(), &document, "fixture").is_err());
    let digest = json!({"operations":[{"path":"/purchase","method":"post","params":[
        {"name":"Idempotency-Key","in":"header","schema":{"type":"string","not":{"type":"string"}},"idempotency_schema":{"type":"string"}}
    ]}]});
    assert!(build_tools(&Config::default(), &digest, "fixture").is_err());
}

#[test]
fn openapi_ignored_headers_stay_ignored_and_transport_headers_are_rejected() {
    let ignored = tool(json!([
        {"name":"aCcEpT","in":"header","required":true},
        {"name":"Content-Type","in":"header","required":true},
        {"name":"Authorization","in":"header","required":true}
    ]));
    assert!(ignored.header_params.is_empty());
    assert_eq!(ignored.input_schema["properties"], json!({}));
    for name in [
        "Host",
        "Content-Length",
        "Transfer-Encoding",
        "Connection",
        "Proxy-Authorization",
        "Cookie",
    ] {
        let error = build_tools(
            &Config::default(),
            &json!({"paths":{"/purchase":{"post":{"parameters":[{"name":name,"in":"header"}]}}}}),
            "fixture",
        )
        .unwrap_err();
        assert!(error.to_string().contains("reserved"));
    }
}

#[test]
fn documented_payment_headers_are_ignored_but_cannot_be_injected_into_routes() {
    for name in [
        "PAYMENT-SIGNATURE",
        "x-PaYmEnT",
        "Payment-Required",
        "PAYMENT-RESPONSE",
        "X-Payment-Response",
    ] {
        for required in [false, true] {
            let document = json!({"components":{"parameters":{"Payment":{
                "name":name,"in":"header","required":required,"schema":{"type":"string"}
            }}},"paths":{"/purchase":{
                "parameters":[{"$ref":"#/components/parameters/Payment"}],
                "post":{"parameters":[{"name":"X-Quote-Id","in":"header","schema":{"type":"string"}}]}
            }}});
            let generated = build_tools(&Config::default(), &document, "fixture")
                .unwrap()
                .remove(0);
            assert_eq!(generated.header_params.len(), 1);
            assert!(generated.input_schema["properties"].get(name).is_none());
            assert!(generated.input_schema["required"].is_null());
            let args = json!({(name):"caller-forged-payment", "X-Quote-Id":"quote"});
            let route = generated
                .route("https://example.com", args.as_object().unwrap())
                .unwrap();
            assert_eq!(route.headers.len(), 1);
            assert_eq!(route.headers["x-quote-id"], "quote");

            // Persisted or Rust-constructed routing metadata must not bypass runtime ownership.
            let mut forged = generated;
            forged.header_params.insert(
                name.into(),
                x402_treazury::catalog::HeaderParameter {
                    name: name.into(),
                    required,
                    explode: false,
                },
            );
            assert!(
                forged
                    .route("https://example.com", args.as_object().unwrap())
                    .is_err()
            );
        }
    }
}
