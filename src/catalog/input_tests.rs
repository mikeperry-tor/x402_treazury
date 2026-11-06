use super::*;

#[test]
fn cookies_are_explicitly_unsupported_and_locations_do_not_overwrite() {
    let error = input::prepare(&json!({"params":[{"name":"session","in":"cookie"}]}))
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<Unsupported>(),
        Some(&Unsupported::CookieParameter)
    );
    let input = input::prepare(&json!({"params":[
        {"name":"id","in":"query","schema":{"type":"integer"}},
        {"name":"id","in":"path","required":true,"schema":{"type":"string"}},
        {"name":"id_query","in":"query","schema":{"type":"string"}}
    ]}))
    .unwrap();
    assert_eq!(input.routes["id"], "path");
    assert_eq!(input.routes["id_query_2"], "query");
    assert_eq!(input.wire_names["id_query_2"], "id");
    assert_eq!(input.schema["properties"]["id_query_2"]["type"], "integer");
    assert_eq!(input.schema["required"], json!(["id"]));
}

#[test]
fn optional_body_preserves_inner_required_and_separate_presence() {
    let op = json!({"params":[{"name":"body","in":"query"}],"body":{"required":false,
        "schema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}}});
    let input = input::prepare(&op).unwrap();
    let body = input.whole_body.unwrap();
    assert_eq!(body.name, "body_body");
    assert!(!body.required);
    assert!(input.schema.get("required").is_none());
    assert_eq!(
        input.schema["properties"]["body_body"]["required"],
        json!(["id"])
    );
    assert_eq!(
        input.schema["properties"]["body_body"]["additionalProperties"],
        false
    );
}

#[test]
fn required_closed_flat_body_and_suffix_collisions_keep_original_wire_keys() {
    let op = json!({"params":[{"name":"id","in":"path"},{"name":"id_body","in":"query"}],
        "body":{"required":true,"schema":{"type":"object","properties":{"id":{"type":"integer"},"id_body_2":{"type":"string"}},"required":["id"],"additionalProperties":false}}});
    let input = input::prepare(&op).unwrap();
    assert!(input.whole_body.is_none());
    assert!(input.body_closed);
    assert_eq!(input.wire_names["id_body_3"], "id");
    assert_eq!(input.routes["id_body"], "query");
    assert_eq!(input.schema["required"], json!(["id_body_3"]));
}

#[test]
fn property_bearing_allof_and_exclusive_closed_alternatives_remain_whole() {
    let op = json!({"body":{"required":true,"schema":{"allOf":[
        {"type":"object","properties":{"a":{"type":"string"}},"required":["a"]},
        {"type":"object","properties":{"b":{"type":"integer"}}}
    ]}}});
    let input = input::prepare(&op).unwrap();
    assert_eq!(input.whole_body.unwrap().name, "body");
    assert_eq!(input.schema["properties"]["body"], op["body"]["schema"]);
    assert_eq!(input.schema["required"], json!(["body"]));
    let schema = json!({"oneOf":[
        {"type":"object","properties":{"a":{"type":"string"}},"required":["a"],"additionalProperties":false},
        {"type":"object","properties":{"b":{"type":"integer"}},"required":["b"],"additionalProperties":false}
    ],"unevaluatedProperties":false,"readOnly":false,"discriminator":{"propertyName":"kind"},"x-annotation":{"preserved":true}});
    let input = input::prepare(&json!({"body":{"required":true,"schema":schema}})).unwrap();
    assert_eq!(input.whole_body.unwrap().name, "body");
    assert_eq!(input.schema["properties"]["body"], schema);
    assert_eq!(input.schema["required"], json!(["body"]));
}

#[test]
fn presence_alternatives_rename_only_body_arguments() {
    for keyword in ["oneOf", "anyOf", "allOf"] {
        let mut schema =
            json!({"type":"object","properties":{"ids":{},"urls":{}},"additionalProperties":false});
        schema[keyword] = json!([{"required":["ids"]},{"required":["urls"]}]);
        let input = input::prepare(&json!({"params":[{"name":"ids","in":"query"}],
            "body":{"required":true,"schema":schema}}))
        .unwrap();
        assert!(input.whole_body.is_none());
        assert_eq!(
            input.schema[keyword],
            json!([{"required":["ids_body"]},{"required":["urls"]}])
        );
        assert_eq!(input.wire_names["ids_body"], "ids");
    }
}

#[test]
fn scalar_query_unions_preserve_constraints_and_route_values_without_rewriting() {
    for (arm, value) in [
        (json!({"type":"string","enum":["BTC"]}), json!("BTC")),
        (
            json!({"type":"integer","minimum":1,"maximum":365}),
            json!(90),
        ),
        (json!({"type":"number","minimum":0}), json!(1.5)),
        (json!({"type":"boolean"}), json!(true)),
    ] {
        for other in [json!({"type":"null"}), json!({"type":"string"})] {
            for reversed in [false, true] {
                let arms = if reversed {
                    json!([other, arm])
                } else {
                    json!([arm, other])
                };
                let schema = json!({"anyOf":arms,"default":value,"description":"Filter.","x-note":{"opaque":true}});
                let document = json!({"openapi":"3.1.0","paths":{"/lookup":{"get":{"parameters":[
                    {"name":"q","in":"query","schema":schema,"explode":false}
                ]}}}});
                let tool = build_tools(&Config::default(), &document, "api")
                    .unwrap()
                    .remove(0);
                let mut expected = schema;
                expected["description"] =
                    json!("Filter. Use a scalar value or an array of scalars.");
                assert_eq!(tool.input_schema["properties"]["q"], expected);
                let restored: ToolSpec =
                    serde_json::from_value(serde_json::to_value(&tool).unwrap()).unwrap();
                let route = restored
                    .route(
                        "https://example.com",
                        json!({"q":value}).as_object().unwrap(),
                    )
                    .unwrap();
                assert_eq!(route.query["q"], value);
                assert!(route.body.is_none());
                let route = restored
                    .route(
                        "https://example.com",
                        json!({"q":null}).as_object().unwrap(),
                    )
                    .unwrap();
                assert!(route.query.is_empty());
            }
        }
    }
}

#[test]
fn scalar_query_union_near_matches_remain_unsupported() {
    for schema in [
        json!({"anyOf":[{"type":"object"},{"type":"null"}]}),
        json!({"anyOf":[{"type":"array","items":{"type":"string"}},{"type":"null"}]}),
        json!({"anyOf":[{"type":"null"},{"type":"null"}]}),
        json!({"anyOf":[{"type":"integer"},true]}),
        json!({"anyOf":[{"type":"integer"},{}]}),
        json!({"anyOf":[{"type":"integer"},{"type":"string"},{"type":"null"}]}),
        json!({"oneOf":[{"type":"integer"},{"type":"null"}]}),
        json!({"allOf":[{"type":"string"},{"type":"array"}]}),
    ] {
        assert!(
            input::prepare(&json!({"params":[{"name":"q","in":"query","schema":schema}]})).is_err()
        );
    }
    let original = json!({"anyOf":[{"type":"integer"},{"type":"null"}]});
    for patch in [
        json!({"in":"path"}),
        json!({"style":"deepObject"}),
        json!({"allowReserved":true}),
    ] {
        let mut parameter = json!({"name":"q","in":"query","schema":original});
        parameter
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(input::prepare(&json!({"params":[parameter]})).is_err());
    }
    for key in ["allOf", "oneOf", "not", "if", "then", "else", "prefixItems"] {
        let mut schema = original.clone();
        schema[key] = json!({});
        assert!(
            input::prepare(&json!({"params":[{"name":"q","in":"query","schema":schema}]})).is_err()
        );
    }
}

#[test]
fn exact_string_array_query_union_preserves_schema_and_uses_existing_codec_guards() {
    let original = json!({"anyOf":[{"type":"array","items":{"type":"string"}},{"type":"string"}],
        "default":["photos"],"description":"Fields to exclude.","x-note":{"opaque":true}});
    for reversed in [false, true] {
        let mut schema = original.clone();
        if reversed {
            schema["anyOf"].as_array_mut().unwrap().reverse();
        }
        for explode in [None, Some(true)] {
            let mut parameter = json!({"name":"q","in":"query","schema":schema});
            if let Some(explode) = explode {
                parameter["explode"] = json!(explode);
                parameter["style"] = json!("form");
            }
            let result = input::prepare(&json!({"params":[parameter]})).unwrap();
            let mut expected = schema.clone();
            expected["description"] =
                json!("Fields to exclude. Use a scalar value or an array of scalars.");
            assert_eq!(result.schema["properties"]["q"], expected);
            assert!(result.schema["properties"]["q"].get("type").is_none());
            assert_eq!(result.routes["q"], "query");
        }
    }
    for patch in [
        json!({"explode":false}),
        json!({"style":"spaceDelimited"}),
        json!({"allowReserved":true}),
        json!({"in":"path"}),
    ] {
        let mut parameter = json!({"name":"q","in":"query","schema":original});
        parameter
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let error = input::prepare(&json!({"params":[parameter]}))
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<Unsupported>(),
            Some(&Unsupported::ParameterSerialization)
        );
    }
    let rejected = [
        json!({"anyOf":[{"type":"string"},{"type":"object"}]}),
        json!({"anyOf":[{"type":"string"},{"type":"array"}]}),
        json!({"anyOf":[{"type":"string"},{"type":"array","items":true}]}),
        json!({"anyOf":[{"type":"string"},{"type":"array","items":{"type":"object"}}]}),
        json!({"anyOf":[{"type":"string"},{"type":"array","items":{"type":"array","items":{"type":"string"}}}]}),
        json!({"anyOf":[{"type":"string"},{"type":"array","prefixItems":[{"type":"string"}]}]}),
        json!({"oneOf":[{"type":"string"},{"type":"array","items":{"type":"string"}}]}),
        json!({"anyOf":[{"type":"string"},true]}),
    ];
    for schema in rejected {
        let error = input::prepare(&json!({"params":[{"name":"q","in":"query","schema":schema}]}))
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<Unsupported>(),
            Some(&Unsupported::ParameterSerialization)
        );
    }
    for key in ["allOf", "oneOf", "not", "if", "then", "else", "prefixItems"] {
        let mut schema = original.clone();
        schema[key] = if matches!(key, "allOf" | "oneOf" | "prefixItems") {
            json!([{"type":"string"}])
        } else {
            json!({"type":"string"})
        };
        let error = input::prepare(&json!({"params":[{"name":"q","in":"query","schema":schema}]}))
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<Unsupported>(),
            Some(&Unsupported::ParameterSerialization)
        );
    }
}
