use super::*;

#[test]
fn digest_conversion_preserves_payload_and_only_copies_present_credit_keys() {
    let original = json!([
        {"path":"/b","method":"OPTIONS","pricing":"retained","tariff":{"credits":3},
         "credit_pricing":"replaced","opaque":[1,null,{"nested":true}]},
        {"path":"/a","method":"get","tariff":null,"credit_pricing":"replaced"},
        {"path":"/c","method":"get","credit_pricing":"retained"}
    ]);
    let document = json!({"operations":original});
    assert_eq!(
        operations_with_credits(&document, Some("tariff"), None).unwrap(),
        original.as_array().unwrap().clone()
    );
    let mut expected = original.clone();
    expected[0]["credit_pricing"] = json!({"credits":3});
    expected[1]["credit_pricing"] = Value::Null;
    assert_eq!(
        operations_with_credits(&document, Some("tariff"), Some("tariff")).unwrap(),
        expected.as_array().unwrap().clone()
    );
    assert_eq!(document["operations"], original);
}

#[test]
fn openapi_conversion_preserves_complete_parameter_metadata_and_operation_scope() {
    let document = json!({
    "components":{"schemas":{"Count":{"type":"integer","minimum":0,"exclusiveMinimum":true,"description":"Schema prose","example":4}},
        "parameters":{"Count":{"name":"count","in":"query","required":true,"description":null,"schema":{"$ref":"#/components/schemas/Count"}}}},
    "paths":{
        "/a":{"parameters":[{"name":"path_level","in":"query"}],
            "HEAD":false,"head":false,"options":false,"trace":false,
            "get":{"summary":"Summary","description":"Vendor prose.\nStill whole.","tags":null,
                "x-price":{"amount":"2"},"x-credit":{"credits":3},
                "parameters":[
                    {"$ref":"#/components/parameters/Count","description":"Ignored sibling"},
                    {"name":"empty","in":"query","description":"","schema":{"description":"Not used","type":"string"}},
                    {"name":"fallback","schema":{"description":"Fallback"},"required":"true"},
                    {"name":"bare"}, {"name":7}, {"$ref":"#/missing"}
                ],"requestBody":null}},
        "/b":{"post":{}},"/empty":{"parameters":[]}
    }});
    assert_eq!(
        operations_with_credits(&document, Some("x-price"), Some("x-credit")).unwrap(),
        vec![
            json!({"path":"/a","method":"get","summary":"Summary","description":"Vendor prose.\nStill whole.","tags":null,
            "pricing":{"amount":"2"},"credit_pricing":{"credits":3},"body":null,
                "params":[
                    {"name":"path_level","in":"query","required":false,"description":null,"schema":{}},
                {"name":"count","in":"query","required":true,"description":"Schema prose","schema":{"type":"integer","minimum":0,"exclusiveMinimum":0,"description":"Schema prose"}},
                {"name":"empty","in":"query","required":false,"description":"","schema":{"description":"Not used","type":"string"}},
                {"name":"fallback","in":null,"required":false,"description":"Fallback","schema":{"description":"Fallback"}},
                {"name":"bare","in":null,"required":false,"description":null,"schema":{}}
            ]}),
            json!({"path":"/b","method":"post","summary":null,"description":null,"tags":[],"pricing":null,"credit_pricing":null,"params":[],"body":null})
        ]
    );
}

#[test]
fn request_body_conversion_retains_reference_and_json_media_type_semantics() {
    let document = json!({"components":{
        "requestBodies":{"Body":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Body"}}}}},
        "schemas":{"Body":{"type":"object","properties":{"value":{"type":"number","maximum":9,"exclusiveMaximum":true}}}}
    },"paths":{"/body":{
        "delete":{"requestBody":{"$ref":"#/missing","required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Body"}}}}},
        "get":{"requestBody":{"$ref":"#/components/requestBodies/Body","required":false}},
        "patch":{"requestBody":{"required":"true","content":{"text/plain":{"schema":{"type":"string"}}}}},
        "post":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"https://example.invalid/schema"}}}}},
        "put":{"requestBody":{"content":{"application/json":{"schema":{"type":"string"}}}}}
    }}});
    // An unresolved body-level ref leaves its nested schema unresolved. Unlike
    // parameters, the body path does not perform a second schema resolution.
    let bodies = [
        ("delete", json!({"required":true,"schema":{}})),
        (
            "get",
            json!({"required":true,"schema":{"type":"object","properties":{"value":{"type":"number","maximum":9,"exclusiveMaximum":9}}}}),
        ),
        ("patch", json!({"required":false,"schema":{}})),
        ("post", json!({"required":false,"schema":{}})),
        ("put", json!({"required":false,"schema":{"type":"string"}})),
    ];
    let expected: Vec<_> = bodies.into_iter().map(|(method,body)| json!({"path":"/body","method":method,"summary":null,"description":null,"tags":[],"pricing":null,"credit_pricing":null,"params":[],"body":body})).collect();
    assert_eq!(
        operations_with_credits(&document, None, None).unwrap(),
        expected
    );
}

#[test]
fn document_shape_errors_and_empty_inputs_keep_original_precedence() {
    for (document, error) in [
        (json!({}), "expected OpenAPI paths or operations digest"),
        (
            json!({"operations":null}),
            "expected OpenAPI paths or operations digest",
        ),
        (
            json!({"paths":null,"operations":[]}),
            "paths must be an object",
        ),
        (
            json!({"paths":[],"operations":7}),
            "paths must be an object",
        ),
        (
            json!({"paths":{"/a":{"get":{}},"/b":null},"operations":[]}),
            "invalid path item",
        ),
    ] {
        assert_eq!(
            operations_with_credits(&document, None, None)
                .unwrap_err()
                .to_string(),
            error
        );
    }
    for document in [
        json!({"operations":[]}),
        json!({"paths":{}}),
        json!({"paths":{"/a":{"options":null}}}),
    ] {
        assert_eq!(
            operations_with_credits(&document, None, None).unwrap(),
            Vec::<Value>::new()
        );
    }
}
