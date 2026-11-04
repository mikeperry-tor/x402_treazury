use super::*;

fn partial() -> Value {
    json!({"openapi":"3.1.0","paths":{
        "/same":{"get":{},"post":{"parameters":[{"name":"session","in":"cookie","schema":{"type":"string"}}]}},
        "/healthy":{"get":{}}
    }})
}
#[test]
fn rich_partial_and_strict_vec_preserve_healthy_get_before_post_preference() {
    let document = partial();
    let built = build_catalog(&Config::default(), &document, "api").unwrap();
    assert_eq!(built.tools.len(), 2);
    assert_eq!(built.issues.len(), 1);
    assert_eq!(built.issues[0].reason, Unsupported::CookieParameter);
    assert_eq!(built.issues[0].method, "POST");
    assert!(
        build_tools(&Config::default(), &document, "api")
            .unwrap_err()
            .is::<IncompleteCatalog>()
    );
    let mut compatible = document.clone();
    compatible["paths"]["/same"]
        .as_object_mut()
        .unwrap()
        .remove("post");
    assert_eq!(
        built
            .tools
            .iter()
            .map(|tool| &tool.name)
            .collect::<Vec<_>>(),
        build_tools(&Config::default(), &compatible, "api")
            .unwrap()
            .iter()
            .map(|tool| &tool.name)
            .collect::<Vec<_>>()
    );
    let excluded = Config {
        exclude_operations: vec!["POST /same".into()],
        ..Default::default()
    };
    assert_eq!(build_tools(&excluded, &document, "api").unwrap().len(), 2);
    for cfg in [
        Config {
            include_tools: vec!["api_missing".into()],
            ..Default::default()
        },
        Config {
            exclude_tools: vec!["api_missing".into()],
            ..Default::default()
        },
    ] {
        let error = build_catalog(&cfg, &document, "api")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no compatible tools"));
        assert!(!error.contains("unknown provider"));
    }
}
#[test]
fn whole_body_absent_empty_null_and_required_empty_have_distinct_requests() {
    let document = json!({"paths":{
        "/optional":{"post":{"parameters":[{"name":"body","in":"query"}],"requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}}}}}},
        "/required":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","properties":{},"additionalProperties":false}}}}}}
    }});
    let tools = build_tools(&Config::default(), &document, "api").unwrap();
    let optional = tools.iter().find(|tool| tool.path == "/optional").unwrap();
    assert_eq!(optional.whole_body.as_ref().unwrap().name, "body_body");
    assert!(optional.input_schema.get("required").is_none());
    for (args, expected) in [
        (json!({}), None),
        (json!({"body_body":{}}), Some(json!({}))),
        (json!({"body_body":null}), Some(Value::Null)),
    ] {
        assert_eq!(
            optional
                .route("https://example.com", args.as_object().unwrap())
                .unwrap()
                .body,
            expected
        );
    }
    assert!(
        optional
            .route(
                "https://example.com",
                json!({"unknown":7}).as_object().unwrap()
            )
            .is_err()
    );
    let required = tools.iter().find(|tool| tool.path == "/required").unwrap();
    assert!(required.whole_body.is_none());
    assert_eq!(
        required
            .route("https://example.com", &Default::default())
            .unwrap()
            .body,
        Some(json!({}))
    );
}
#[test]
fn alternate_json_media_survives_preferred_unsupported_and_keeps_its_schema() {
    let document = json!({"openapi":"3.1.0","paths":{"/patch":{"patch":{"requestBody":{"required":true,"content":{
        "application/json; charset=utf-16":{"schema":{"type":"object"}},
        "application/json-patch+json; profile=\"https://example.com/a;b\"":{"schema":{"type":"array","items":{"type":"object"}}}
    }}}}}});
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    assert_eq!(tool.input_schema["properties"]["body"]["type"], "array");
    assert_eq!(
        tool.json_media_type.as_deref(),
        Some("application/json-patch+json; profile=\"https://example.com/a;b\"")
    );
    assert!(
        tool.route("https://example.com", &Default::default())
            .is_err()
    );
    let restored: ToolSpec = serde_json::from_value(serde_json::to_value(&tool).unwrap()).unwrap();
    let request = restored
        .route(
            "https://example.com",
            json!({"body":[{"op":"remove","path":"/old"}]})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(request.body, Some(json!([{"op":"remove","path":"/old"}])));
    assert_eq!(request.json_media_type, tool.json_media_type);
}
#[test]
fn used_schema_ref_siblings_are_visible_without_rejecting_unused_definitions() {
    let document = json!({"openapi":"3.1.0","components":{"schemas":{
        "Value":{"type":"string"},"Unused":{"$ref":"#/components/schemas/Value","unevaluatedProperties":false}
    }},"paths":{
        "/healthy":{"get":{}},
        "/body":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Value","maxLength":4}}}}}},
        "/parameter":{"get":{"parameters":[{"name":"q","in":"query","schema":{"$ref":"#/components/schemas/Value","description":"sibling"}}]}}
    }});
    let built = build_catalog(&Config::default(), &document, "api").unwrap();
    assert_eq!(built.tools.len(), 1);
    assert_eq!(built.tools[0].path, "/healthy");
    assert_eq!(built.issues.len(), 2);
    assert!(
        built
            .issues
            .iter()
            .all(|issue| issue.reason == Unsupported::SchemaReferenceSiblings)
    );
    let cfg = Config {
        include: vec!["/healthy".into()],
        ..Default::default()
    };
    assert_eq!(build_tools(&cfg, &document, "api").unwrap().len(), 1);
}
#[test]
fn aliases_persist_and_parameter_metadata_targets_existing_body_properties() {
    let document = json!({"paths":{"/flights/{ident}":{"post":{
        "parameters":[{"name":"ident","in":"path","required":true,"schema":{"type":"string"}}],
        "requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","properties":{"ident":{"type":"string"},"intent":{"type":"string"}}}}}}
    }}}});
    let mut cfg = Config::default();
    let name = build_tools(&cfg, &document, "api").unwrap()[0].name.clone();
    cfg.overrides[name.as_str()] = json!({"params":{"ident":"URL identifier","ident_body":"JSON identifier","not_a_field":"ignored"}});
    let tool = build_tools(&cfg, &document, "api").unwrap().remove(0);
    assert_eq!(
        tool.input_schema["properties"]["ident"]["description"],
        "URL identifier"
    );
    assert_eq!(
        tool.input_schema["properties"]["body"]["properties"]["ident"]["description"],
        "JSON identifier"
    );
    assert!(tool.input_schema["properties"].get("ident_body").is_none());
    assert!(tool.input_schema["properties"].get("not_a_field").is_none());
    let document = json!({"paths":{"/{id}":{"get":{"parameters":[
        {"name":"id","in":"path","required":true,"schema":{"type":"string"}},
        {"name":"id","in":"query","schema":{"type":"integer"}},
        {"name":"id_query","in":"query","schema":{"type":"string"}}
    ]}}}});
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    let tool: ToolSpec = serde_json::from_value(serde_json::to_value(tool).unwrap()).unwrap();
    let request = tool
        .route(
            "https://example.com",
            json!({"id":"a/b","id_query_2":3,"id_query":"kept"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(request.url, "https://example.com/a%2Fb");
    assert_eq!(request.query["id"], 3);
    assert_eq!(request.query["id_query"], "kept");
}

#[test]
fn schema_projection_resolves_only_used_schema_positions_and_keeps_annotation_bytes() {
    let annotation = json!({"$ref":"#/does-not-exist","minimum":3,"exclusiveMinimum":true});
    let document = json!({"openapi":"3.1.0","components":{"schemas":{
        "Value":{"type":"string"},"Unused":{"$ref":"https://example.invalid/unused","maxLength":4}
    }},"paths":{
        "/body":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{
            "type":"object","properties":{"value":{"$ref":"#/components/schemas/Value"}},
            "example":annotation,"default":annotation,"x-opaque":{"$ref":7},"readOnly":false
        }}}}}},
        "/parameter":{"get":{"parameters":[{"name":"q","in":"query","schema":{"type":"string","example":annotation}}]}}
    }});
    let tools = build_tools(&Config::default(), &document, "api").unwrap();
    let body = tools.iter().find(|tool| tool.path == "/body").unwrap();
    let schema = &body.input_schema["properties"]["body"];
    assert_eq!(schema["properties"]["value"], json!({"type":"string"}));
    assert_eq!(schema["example"], annotation);
    assert_eq!(schema["default"], annotation);
    assert_eq!(schema["x-opaque"], json!({"$ref":7}));
    let parameter = tools.iter().find(|tool| tool.path == "/parameter").unwrap();
    assert_eq!(
        parameter.input_schema["properties"]["q"]["example"],
        annotation
    );
    let loaded = parse_document(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(
        loaded, document,
        "cache admission must retain original opaque data"
    );
}
#[test]
fn known_unsupported_schema_and_cookie_do_not_expand_unused_bomb_targets() {
    let mut definitions = Map::new();
    definitions.insert("0".into(), json!({"type":"string"}));
    for index in 1..20 {
        let reference = format!("#/defs/{}", index - 1);
        definitions.insert(
            index.to_string(),
            json!({"allOf":[{"$ref":reference},{"$ref":reference}]}),
        );
    }
    let document = json!({"openapi":"3.1.0","defs":definitions,"paths":{
        "/healthy":{"get":{}},
        "/body":{"post":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"#/defs/19","maxLength":3}}}}}},
        "/cookie":{"post":{"parameters":[{"in":"cookie","name":"session","schema":{"$ref":"#/defs/19"}}]}}
    }});
    let built = build_catalog(&Config::default(), &document, "api").unwrap();
    assert_eq!(built.tools.len(), 1);
    assert_eq!(built.issues.len(), 2);
    assert!(
        built
            .issues
            .iter()
            .any(|issue| issue.reason == Unsupported::SchemaReferenceSiblings)
    );
    assert!(
        built
            .issues
            .iter()
            .any(|issue| issue.reason == Unsupported::CookieParameter)
    );
}
#[test]
fn exact_media_precedence_cannot_be_evaded_but_other_json_offer_remains_usable() {
    let mut document = json!({"openapi":"3.1.0","components":{"schemas":{"Value":{"type":"string"}}},"paths":{
        "/healthy":{"get":{}},"/body":{"post":{"requestBody":{"required":true,"content":{
            "application/json":{"schema":{"$ref":"#/components/schemas/Value","maxLength":3}},
            "application/*":{"schema":{"type":"object","properties":{}}}
        }}}}
    }});
    let built = build_catalog(&Config::default(), &document, "api").unwrap();
    assert_eq!(built.tools.len(), 1);
    assert_eq!(built.issues[0].reason, Unsupported::SchemaReferenceSiblings);
    document["paths"]["/body"]["post"]["requestBody"]["content"]["application/vendor+json; charset=\"UTF-8\""] =
        json!({"schema":{"type":"array","items":{"type":"string"}}});
    let tools = build_tools(&Config::default(), &document, "api").unwrap();
    let body = tools.iter().find(|tool| tool.path == "/body").unwrap();
    assert_eq!(
        body.json_media_type.as_deref(),
        Some("application/vendor+json; charset=\"UTF-8\"")
    );
    assert_eq!(body.input_schema["properties"]["body"]["type"], "array");
}
#[test]
fn digest_schemas_use_actual_projection_and_preserve_opaque_payload() {
    let annotation = json!({"$ref":"#/missing","minimum":2,"exclusiveMinimum":true});
    let document = json!({"operations":[{"method":"POST","path":"/body","opaque":{"kept":true},
        "body":{"required":true,"schema":{"type":"object","properties":{"value":{"type":"number","minimum":2,"exclusiveMinimum":true,"example":annotation}}}}}]});
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    assert_eq!(
        tool.input_schema["properties"]["value"]["exclusiveMinimum"],
        2
    );
    assert_eq!(
        tool.input_schema["properties"]["value"]["example"],
        annotation
    );
    assert_eq!(
        operations(&document, None).unwrap()[0]["opaque"],
        json!({"kept":true})
    );
}

#[test]
fn all_unsupported_rich_result_is_visible_and_help_cannot_disguise_it() {
    let document =
        json!({"paths":{"/bad":{"post":{"parameters":[{"in":"cookie","name":"token"}]}}}});
    let cfg = Config {
        help_url: Some("https://example.com/help".into()),
        ..Default::default()
    };
    let built = build_catalog(&cfg, &document, "api").unwrap();
    assert!(built.tools.is_empty());
    assert_eq!(built.issues.len(), 1);
    assert!(
        build_tools(&cfg, &document, "api")
            .unwrap_err()
            .is::<IncompleteCatalog>()
    );
    let help_only = Config {
        include_tools: vec!["api_help".into()],
        ..cfg
    };
    assert!(build_catalog(&help_only, &document, "api").is_err());
    let healthy = json!({"paths":{"/good":{"get":{}}}});
    let tools = build_tools(&help_only, &healthy, "api").unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "api_help");
}

#[test]
fn parameter_schema_constraints_remain_authored_and_composed_wire_types_are_explicit() {
    let document = json!({"paths":{
        "/boolean":{"get":{"parameters":[{"in":"query","name":"never","schema":false},{"in":"query","name":"any","schema":true}]}},
        "/untype":{"get":{"parameters":[{"in":"query","name":"choice","schema":{"enum":["x","y"],"x-annotation":{"$ref":"unresolved annotation"}}}]}},
        "/composed":{"get":{"parameters":[{"in":"query","name":"choice","schema":{"oneOf":[{"type":"string"},{"type":"number"}]}}]}},
        "/typed":{"get":{"parameters":[{"in":"query","name":"choice","schema":{"type":"string","anyOf":[{"const":"x"},{"const":"y"}]}}]}},
        "/path/{id}":{"get":{"parameters":[{"in":"path","name":"id","schema":true}]}},
        "/legacy":{"get":{"parameters":[{"in":"query","name":"choice"}]}}
    }});
    let built = build_catalog(&Config::default(), &document, "api").unwrap();
    assert_eq!(built.tools.len(), 5);
    assert_eq!(built.issues.len(), 1);
    assert_eq!(built.issues[0].path, "/composed");
    assert_eq!(built.issues[0].reason, Unsupported::ParameterSerialization);
    let schema = |path: &str| {
        &built
            .tools
            .iter()
            .find(|tool| tool.path == path)
            .unwrap()
            .input_schema["properties"]
    };
    assert_eq!(schema("/boolean")["never"], false);
    assert_eq!(schema("/boolean")["any"], true);
    assert!(schema("/untype")["choice"].get("type").is_none());
    assert_eq!(schema("/untype")["choice"]["enum"], json!(["x", "y"]));
    assert!(
        schema("/untype")["choice"]["description"]
            .as_str()
            .unwrap()
            .contains("scalar")
    );
    assert_eq!(
        schema("/typed")["choice"]["anyOf"],
        document["paths"]["/typed"]["get"]["parameters"][0]["schema"]["anyOf"]
    );
    assert_eq!(schema("/legacy")["choice"]["type"], "string");
    let path = built
        .tools
        .iter()
        .find(|tool| tool.path == "/path/{id}")
        .unwrap();
    assert!(
        path.route(
            "https://example.com",
            json!({"id":{"bad":"object"}}).as_object().unwrap()
        )
        .is_err()
    );
    assert!(
        path.route(
            "https://example.com",
            json!({"id":["bad"]}).as_object().unwrap()
        )
        .is_err()
    );
}

#[test]
fn closed_parent_allof_keeps_original_property_scope_and_contradictions() {
    for child_required in [false, true] {
        let mut schema = json!({"type":"object","properties":{"a":{"type":"string"}},
            "additionalProperties":false,"allOf":[{"properties":{"b":{"type":"string"}}}]});
        if child_required {
            schema["allOf"][0]["required"] = json!(["b"]);
        }
        let document = json!({"operations":[{"method":"POST","path":"/closed",
            "body":{"required":true,"schema":schema}}]});
        let tools = build_tools(&Config::default(), &document, "api").unwrap();
        let tool = &tools[0];
        let body = tool
            .whole_body
            .as_ref()
            .expect("closed parent must not inherit child-only properties");
        assert_eq!(body.name, "body");
        assert!(body.required);
        assert_eq!(tool.input_schema["properties"]["body"], schema);
        assert!(
            tool.input_schema["properties"]["body"]["properties"]
                .get("b")
                .is_none()
        );
        assert!(tool.input_schema["properties"].get("b").is_none());
        assert!(
            tool.route("https://example.com", &Default::default())
                .is_err()
        );
        if child_required {
            // This is intentionally unsatisfiable: the parent forbids b while
            // its child requires b. Preserve both facts instead of repairing it.
            assert_eq!(
                tool.input_schema["properties"]["body"]["allOf"][0]["required"],
                json!(["b"])
            );
            assert_eq!(
                tool.input_schema["properties"]["body"]["additionalProperties"],
                false
            );
        } else {
            let route = tool
                .route(
                    "https://example.com",
                    json!({"body":{"a":"original"}}).as_object().unwrap(),
                )
                .unwrap();
            assert_eq!(route.url, "https://example.com/closed");
            assert_eq!(route.body, Some(json!({"a":"original"})));
            assert!(route.query.is_empty() && route.headers.is_empty());
        }
    }
    // Pure required-only alternatives keep the established ordinary flat
    // argument contract, without merging any child property declarations.
    let schema = json!({"type":"object","properties":{"a":{"type":"string"}},
        "additionalProperties":false,"allOf":[{"required":["a"]}]});
    let tools = build_tools(&Config::default(), &json!({"operations":[{"method":"POST","path":"/same", "body":{"required":true,"schema":schema}}]}), "api").unwrap();
    assert!(tools[0].whole_body.is_none());
    assert_eq!(tools[0].input_schema["allOf"], json!([{"required":["a"]}]));
    assert_eq!(
        tools[0]
            .route(
                "https://example.com",
                json!({"a":"value"}).as_object().unwrap()
            )
            .unwrap()
            .body,
        Some(json!({"a":"value"}))
    );
}

#[test]
fn used_schema_resource_bases_are_explicitly_unsupported_without_changing_harmless_ids() {
    let opaque = json!({"$id":"https://example.com/data","$ref":"this is literal data","v":1});
    let mut document = json!({"openapi":"3.1.0","$id":"catalog metadata is not a schema resource",
    "components":{"schemas":{
        "X":{"type":"number"},
        "Container":{"$id":"https://example.com/embedded","$defs":{"X":{"type":"object","properties":{"v":{"$ref":"#/components/schemas/X"}}}}},
        "NamedEntries":{"$defs":{"$id":{"type":"string"}},"properties":{"$id":{"type":"integer"}}},
        "IdentifiedPlain":{"$id":"https://example.com/no-ref","type":"string"},
        "ContainerPlain":{"$id":"https://example.com/plain-base","$defs":{"X":{"type":"string","maxLength":7}}},
        "Unused":{"$id":"https://example.com/unused","properties":{"v":{"$ref":"unresolved unused reference"}}}
    }},"paths":{
        "/active":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{
            "$id":"https://example.com/body","type":"object","components":{"schemas":{"X":{"type":"string"}}},
            "properties":{"v":{"$ref":"#/components/schemas/X"}}
        }}}}}},
        "/below":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Container/$defs/X"}}}}}},
        "/ordinary":{"get":{"parameters":[{"in":"query","name":"v","schema":{"$ref":"#/components/schemas/X"}}]}},
        "/below-plain":{"get":{"parameters":[{"in":"query","name":"v","schema":{"$ref":"#/components/schemas/ContainerPlain/$defs/X"}}]}},
        "/named":{"get":{"parameters":[
            {"in":"query","name":"definition","schema":{"$ref":"#/components/schemas/NamedEntries/$defs/$id"}},
            {"in":"query","name":"property","schema":{"$ref":"#/components/schemas/NamedEntries/properties/$id"}}
        ]}},
        "/identified-target":{"get":{"parameters":[{"in":"query","name":"v","schema":{"$ref":"#/components/schemas/IdentifiedPlain"}}]}},
        "/harmless":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{
            "$id":"https://example.com/harmless","type":"object","properties":{"v":{"type":"string"}}
        }}}}}},
        "/opaque":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","properties":{"$id":{"type":"string"},"v":{"$ref":"#/components/schemas/X"}}}}}}}}
    }});
    let schema = &mut document["paths"]["/opaque"]["post"]["requestBody"]["content"]["application/json"]
        ["schema"];
    schema["example"] = opaque.clone();
    schema["default"] = opaque.clone();
    let built = build_catalog(&Config::default(), &document, "api").unwrap();
    assert_eq!(built.issues.len(), 2);
    assert!(
        built
            .issues
            .iter()
            .all(|issue| issue.reason == Unsupported::InputReference)
    );
    assert_eq!(
        built
            .issues
            .iter()
            .map(|issue| issue.path.as_str())
            .collect::<Vec<_>>(),
        vec!["/active", "/below"]
    );
    assert_eq!(built.tools.len(), 6);
    let tool = |path: &str| built.tools.iter().find(|tool| tool.path == path).unwrap();
    assert_eq!(
        tool("/ordinary").input_schema["properties"]["v"]["type"],
        "number"
    );
    assert_eq!(
        tool("/below-plain").input_schema["properties"]["v"],
        document["components"]["schemas"]["ContainerPlain"]["$defs"]["X"]
    );
    assert_eq!(
        tool("/below-plain")
            .route(
                "https://example.com",
                json!({"v":"plain"}).as_object().unwrap()
            )
            .unwrap()
            .query["v"],
        "plain"
    );
    assert_eq!(
        tool("/named").input_schema["properties"]["definition"]["type"],
        "string"
    );
    assert_eq!(
        tool("/named").input_schema["properties"]["property"]["type"],
        "integer"
    );
    assert_eq!(
        tool("/identified-target").input_schema["properties"]["v"]["$id"],
        "https://example.com/no-ref"
    );
    let harmless = tool("/harmless");
    let name = &harmless.whole_body.as_ref().unwrap().name;
    assert_eq!(
        harmless.input_schema["properties"][name],
        document["paths"]["/harmless"]["post"]["requestBody"]["content"]["application/json"]["schema"]
    );
    let route = harmless
        .route(
            "https://example.com",
            json!({"body":{"v":"unchanged"}}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(route.body, Some(json!({"v":"unchanged"})));
    let opaque_schema = &tool("/opaque").input_schema["properties"]["body"];
    assert_eq!(opaque_schema["example"], opaque);
    assert_eq!(opaque_schema["default"], opaque);
    assert_eq!(opaque_schema["properties"]["$id"]["type"], "string");
    assert_eq!(opaque_schema["properties"]["v"]["type"], "number");
}
