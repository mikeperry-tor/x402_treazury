use super::*;

fn partial() -> Value {
    json!({"openapi":"3.1.0","paths":{
        "/same":{"get":{},"post":{"parameters":[{"name":"session","in":"cookie","schema":{"type":"string"}}]}},
        "/healthy":{"get":{}}
    }})
}

#[test]
fn parameter_merge_keeps_first_position_and_last_resolved_definition() {
    let document = json!({"components":{"parameters":{"Shared":{"name":"id","in":"query","schema":{"type":"string"}}}},
    "paths":{"/{id}":{
        "parameters":[{"$ref":"#/components/parameters/Shared"},
            {"name":"id","in":"path","required":true,"schema":{"type":"string"}},
            {"name":"X-Trace","in":"header","schema":{"type":"string"}},
            {"name":"ID","in":"query","schema":{"type":"string"}}],
        "get":{"parameters":[
            {"name":"id","in":"query","schema":{"type":"number"}},
            {"name":"x-trace","in":"header","schema":{"type":"string","minLength":2}},
            {"name":"id","in":"query","schema":{"type":"integer"}},
            {"name":"added","in":"query","schema":{"type":"boolean"}}]}
    }}});
    let operations = operations(&document, None).unwrap();
    let parameters = operations[0]["params"].as_array().unwrap();
    assert_eq!(parameters.len(), 5);
    assert_eq!(
        parameters
            .iter()
            .map(|p| (p["in"].as_str().unwrap(), p["name"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        vec![
            ("query", "id"),
            ("path", "id"),
            ("header", "x-trace"),
            ("query", "ID"),
            ("query", "added")
        ]
    );
    assert_eq!(parameters[0]["schema"]["type"], "integer");
    assert_eq!(parameters[2]["schema"]["minLength"], 2);
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    let request = tool
        .route(
            "https://example.com",
            json!({"id":"path", "id_query":7,"ID":"case","x-trace":"trace","added":true})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(request.url, "https://example.com/path");
    assert_eq!(
        request.query,
        BTreeMap::from([
            ("id".into(), json!(7)),
            ("ID".into(), json!("case")),
            ("added".into(), json!(true))
        ])
    );
    assert_eq!(request.headers["x-trace"], "trace");
}

#[test]
fn explicit_unencodable_scalar_shapes_omit_only_the_affected_operations() {
    let mut paths = Map::from_iter([("/healthy".into(), json!({"get":{}}))]);
    for (index, schema) in [
        json!({"type":"array","items":{"type":"object"},"minItems":1}),
        json!({"type":"array","items":{"type":"array","items":{"type":"string"}}}),
        json!({"type":["array"],"items":{"type":["object","array","null"]}}),
        json!({"type":"array","items":{"type":"null"}}),
        json!({"type":["array","null"],"items":{"type":"object"},"minItems":1}),
        json!({"type":"null"}),
        json!({"type":["null"]}),
    ]
    .into_iter()
    .enumerate()
    {
        for location in ["query", "header", "path"] {
            let path = if location == "path" {
                format!("/bad-{index}/{{q}}")
            } else {
                format!("/bad-{index}-{location}")
            };
            paths.insert(path, json!({"get":{"parameters":[{"name":"q","in":location,"required":true,"schema":schema}]}}));
        }
    }
    let document = json!({"paths":paths});
    let built = build_catalog(&Config::default(), &document, "api").unwrap();
    assert_eq!(built.tools.len(), 1);
    assert_eq!(built.tools[0].path, "/healthy");
    assert_eq!(built.issues.len(), 21);
    for issue in built.issues {
        let expected = if issue.path.ends_with("header") {
            Unsupported::HeaderContract
        } else {
            Unsupported::ParameterSerialization
        };
        assert_eq!(issue.reason, expected, "{}", issue.path);
    }
    assert!(
        build_tools(&Config::default(), &document, "api")
            .unwrap_err()
            .is::<IncompleteCatalog>()
    );
    let selected = Config {
        include: vec!["/healthy".into()],
        ..Default::default()
    };
    assert_eq!(build_tools(&selected, &document, "api").unwrap().len(), 1);
    // OpenAPI 3.0 nullable projection reaches the same explicit type domain.
    let nullable = json!({"openapi":"3.0.3","paths":{"/nullable":{"get":{"parameters":[
        {"in":"query","name":"q","schema":{"type":"array","nullable":true,"items":{"type":"object"}}}
    ]}},"/header":{"get":{"parameters":[
        {"in":"header","name":"X-Value","schema":{"type":"array","nullable":true,"items":{"type":"object"}}}
    ]}}}});
    let built = build_catalog(&Config::default(), &nullable, "api").unwrap();
    assert!(built.tools.is_empty());
    assert_eq!(built.issues.len(), 2);
    assert!(
        built
            .issues
            .iter()
            .any(|issue| issue.reason == Unsupported::HeaderContract)
    );
    assert!(
        built
            .issues
            .iter()
            .any(|issue| issue.reason == Unsupported::ParameterSerialization)
    );
}

#[test]
fn scalar_shape_checks_preserve_encodable_alternatives_and_protocol_ownership() {
    for (schema, value) in [
        (
            json!({"type":"array","items":{"type":["object","string"]}}),
            json!(["kept"]),
        ),
        (
            json!({"type":["array","string"],"items":{"type":"object"}}),
            json!("kept"),
        ),
        (
            json!({"type":"array","prefixItems":[{"type":"string"}],"items":{"type":"object"}}),
            json!(["kept"]),
        ),
        (json!({"type":["null","string"]}), json!("kept")),
        (json!({"type":"array","items":true}), json!(["kept"])),
    ] {
        let document = json!({"paths":{"/values":{"get":{"parameters":[
            {"name":"q","in":"query","schema":schema},
            {"name":"X-Value","in":"header","schema":schema}
        ]}}}});
        let tool = build_tools(&Config::default(), &document, "api")
            .unwrap()
            .remove(0);
        assert_eq!(tool.input_schema["properties"]["q"], schema);
        assert_eq!(tool.input_schema["properties"]["X-Value"], schema);
        let routed = tool
            .route(
                "https://example.com",
                json!({"q":value,"X-Value":value}).as_object().unwrap(),
            )
            .unwrap();
        assert_eq!(routed.query["q"], value);
        assert_eq!(routed.headers["x-value"], "kept");
    }
    let impossible = json!({"type":"array","items":{"type":"object"},"minItems":1});
    let document = json!({"paths":{
        "/protocol":{"get":{"parameters":[
            {"name":"Authorization","in":"header","required":true,"schema":impossible},
            {"name":"PAYMENT-SIGNATURE","in":"header","required":true,"schema":impossible},
            {"name":"X-Object","in":"header","schema":{"type":"object","additionalProperties":{"type":"string"}}}
        ]}},
        "/managed":{"get":{"parameters":[{"name":"Idempotency-Key","in":"header","schema":impossible}]}}
    }});
    let built = build_catalog(&Config::default(), &document, "api").unwrap();
    assert_eq!(built.issues.len(), 1);
    assert_eq!(built.issues[0].reason, Unsupported::IdempotencyKeySchema);
    assert_eq!(built.tools.len(), 1);
    assert_eq!(built.tools[0].path, "/protocol");
    let routed = built.tools[0]
        .route(
            "https://example.com",
            json!({"X-Object":{"key":"value"}}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(
        routed.headers,
        BTreeMap::from([("x-object".into(), "key,value".into())])
    );
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
            recommended_tools: vec!["api_missing".into()],
            use_recommended_tools: true,
            ..Default::default()
        },
    ] {
        let error = build_catalog(&cfg, &document, "api").unwrap_err();
        let configuration = error
            .downcast_ref::<ToolConfigurationError>()
            .expect("partial selector refusal remains a configuration error");
        let incomplete = configuration
            .error
            .downcast_ref::<IncompleteCatalog>()
            .expect("the exact omitted operation remains available");
        assert_eq!(
            incomplete.0,
            vec![OperationIssue {
                method: "POST".into(),
                path: "/same".into(),
                reason: Unsupported::CookieParameter,
            }]
        );
        let detail = format!("{error:#}");
        assert!(detail.contains("provider tool selector api_missing matches no compatible tools"));
        assert!(detail.contains("POST /same: Cookie parameters are not supported"));
        assert!(!detail.contains("unknown provider"));
        // Startup's public safe projection selects authored configuration guidance,
        // while CLI/MCP operation results retain the detailed cause chain.
        let failure = diagnostics::Failure::from_error(&error, "generation");
        assert!(failure.reason.contains("no compatible tools"));
        assert!(failure.reason.contains("inspect omitted operations"));
        assert!(!failure.reason.contains("api_missing"));
        assert!(!failure.reason.contains("/same"));

        let complete_error = build_catalog(&cfg, &compatible, "api").unwrap_err();
        let complete = complete_error
            .downcast_ref::<ToolConfigurationError>()
            .expect("unknown selectors remain configuration errors");
        assert!(complete.error.downcast_ref::<IncompleteCatalog>().is_none());
        assert!(
            format!("{complete_error:#}").contains("unknown provider tool selector api_missing")
        );
    }
}
#[test]
fn unmatched_exclusions_preserve_compatibility_failures_without_selector_errors() {
    let cfg = Config {
        exclude_tools: vec!["api_missing".into()],
        ..Default::default()
    };
    let document = partial();
    let built = build_catalog(&cfg, &document, "api").unwrap();
    assert_eq!(built.tools.len(), 2);
    assert_eq!(built.issues.len(), 1);
    assert!(built.complete().unwrap_err().is::<IncompleteCatalog>());
    let mut unsupported = document;
    unsupported["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/healthy");
    unsupported["paths"]["/same"]
        .as_object_mut()
        .unwrap()
        .remove("get");
    let built = build_catalog(&cfg, &unsupported, "api").unwrap();
    assert!(built.tools.is_empty());
    assert_eq!(built.issues.len(), 1);
    assert!(built.complete().unwrap_err().is::<IncompleteCatalog>());
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
        "/parameter":{"get":{"parameters":[{"name":"q","in":"query","schema":{"$ref":"#/components/schemas/Value","description":"sibling","maxLength":4}}]}}
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
fn used_dynamic_and_recursive_references_are_operation_local_omissions() {
    for keyword in ["$dynamicRef", "$recursiveRef"] {
        let reference = json!({(keyword):"#/components/schemas/Value"});
        let document = json!({"openapi":"3.1.0","components":{"schemas":{
            "Value":{"type":"string"},"Indirect":reference,"Unused":reference
        }},"paths":{
            "/healthy":{"get":{}},
            "/body":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":reference}}}}},
            "/property":{"post":{"requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"value":reference}}}}}}},
            "/parameter":{"get":{"parameters":[{"in":"query","name":"q","schema":{"type":"string",(keyword):"#/components/schemas/Value"}}]}},
            "/header":{"get":{"parameters":[{"in":"header","name":"X-Value","schema":{"type":"string",(keyword):"#/components/schemas/Value"}}]}},
            "/indirect":{"post":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Indirect"}}}}}}
        }});
        let built = build_catalog(&Config::default(), &document, "api").unwrap();
        assert_eq!(built.tools.len(), 1, "{keyword}");
        assert_eq!(built.tools[0].path, "/healthy");
        assert_eq!(built.issues.len(), 5, "{keyword}");
        assert!(
            built
                .issues
                .iter()
                .all(|issue| issue.reason == Unsupported::InputReference)
        );
        assert!(
            build_tools(&Config::default(), &document, "api")
                .unwrap_err()
                .is::<IncompleteCatalog>()
        );
        let selected = Config {
            include: vec!["/healthy".into()],
            ..Default::default()
        };
        assert_eq!(build_tools(&selected, &document, "api").unwrap().len(), 1);
    }
}

#[test]
fn dynamic_reference_data_stays_opaque_and_openapi30_ignored_siblings_stay_unused() {
    for keyword in ["$dynamicRef", "$recursiveRef"] {
        let data = json!({(keyword):"#/missing"});
        let schema = json!({"type":"object","properties":{(keyword):{"type":"string"}},
            "default":data,"examples":[data],"x-opaque":data,"$defs":{"Unused":data}});
        let document = json!({"openapi":"3.1.0","components":{"schemas":{"Unused":data}},
            "paths":{"/body":{"post":{"requestBody":{"content":{"application/json":{"schema":schema}}}}}}});
        let tool = build_tools(&Config::default(), &document, "api")
            .unwrap()
            .remove(0);
        assert_eq!(tool.input_schema["properties"]["body"], schema);
        let mut root = json!({"openapi":"3.0.3","Value":{"type":"string"}});
        let sibling = json!({"$ref":"#/Value",(keyword):"#/missing"});
        assert_eq!(
            contracts::schema(&root, &sibling, &mut expansion::Budget::default()).unwrap(),
            root["Value"]
        );
        root["openapi"] = json!("3.1.0");
        assert_eq!(
            contracts::schema(&root, &sibling, &mut expansion::Budget::default())
                .unwrap_err()
                .downcast_ref::<Unsupported>(),
            Some(&Unsupported::SchemaReferenceSiblings)
        );
    }
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

#[test]
fn media_buckets_preserve_mixed_case_parameter_ties_and_duplicate_specificity() {
    let first = "Application/Example+Json; profile=a; level=b";
    let last = "application/example+json; level=b; profile=a";
    let mut document = json!({"paths":{"/body":{"post":{"requestBody":{"content":{
        (first):{"schema":{"type":"number"}},
        (last):{"schema":{"type":"string"}},
        "application/*":{"schema":{"$ref":"#/missing"}},
        "*/*":{"schema":{"$ref":"#/missing"}}
    }}}}}});
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    assert_eq!(tool.json_media_type.as_deref(), Some(first));
    assert_eq!(tool.input_schema["properties"]["body"]["type"], "string");
    document["paths"]["/body"]["post"]["requestBody"]["content"]["application/example+json; profile=a; profile=a; profile=a"] =
        json!({"schema":{"type":"array"}});
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    assert_eq!(tool.json_media_type.as_deref(), Some(first));
    assert_eq!(tool.input_schema["properties"]["body"]["type"], "array");
    // Keep parser-accepted any-type ranges with a concrete subtype too.
    document["paths"]["/body"]["post"]["requestBody"]["content"] = json!({
        "*/*":{"schema":{"type":"number"}},
        "*/json":{"schema":{"type":"array"}}
    });
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    assert_eq!(tool.json_media_type.as_deref(), Some("application/json"));
    assert_eq!(tool.input_schema["properties"]["body"]["type"], "array");
}

#[test]
fn unrelated_json_essences_do_not_hide_a_later_usable_representation() {
    let mut content = serde_json::Map::new();
    for index in 0..32 {
        content.insert(
            format!("application/a{index:02}+json"),
            json!({"schema":{"$ref":"#/missing"}}),
        );
    }
    let selected = "text/z-example+json";
    content.insert(
        selected.into(),
        json!({"schema":{"type":"array","items":{"type":"string"}}}),
    );
    content.insert("text/*".into(), json!({"schema":{"type":"number"}}));
    let document =
        json!({"paths":{"/body":{"post":{"requestBody":{"required":true,"content":content}}}}});
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    assert_eq!(tool.json_media_type.as_deref(), Some(selected));
    let request = tool
        .route(
            "https://example.com",
            json!({"body":["kept"]}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(request.body, Some(json!(["kept"])));
    assert_eq!(request.json_media_type.as_deref(), Some(selected));
}

#[test]
fn parameterized_wildcard_fallback_preserves_the_selected_contract() {
    let document = json!({"openapi":"3.1.0","paths":{"/body":{"post":{"requestBody":{"required":true,"content":{
        "application/*; profile=a":{"schema":{"$ref":"#/missing"}},
        "application/*; profile=b":{"schema":{"type":"array","items":{"type":"string"}}}
    }}}}}});
    let tool = build_tools(&Config::default(), &document, "api")
        .unwrap()
        .remove(0);
    assert_eq!(
        tool.json_media_type.as_deref(),
        Some("application/json; profile=b")
    );
    assert_eq!(tool.input_schema["properties"]["body"]["type"], "array");
    let routed = tool
        .route(
            "https://example.com",
            json!({"body":["kept"]}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(routed.body, Some(json!(["kept"])));
    assert_eq!(routed.json_media_type, tool.json_media_type);
}

#[test]
fn media_candidate_amplification_is_rejected_before_catalog_admission() {
    let content: Map<_, _> = (0..1024)
        .map(|i| {
            (
                format!("application/*; profile=p{i:04}"),
                json!({"schema":{"$ref":"#/components/schemas/Value","minLength":1}}),
            )
        })
        .collect();
    let document = json!({"openapi":"3.1.0","components":{"schemas":{"Value":{"type":"string"}}},
        "paths":{"/body":{"post":{"requestBody":{"content":content}}}}});
    // The physical input fits. Repeated matching, rather than input size,
    // exhausts the same conversion budget at both public admission boundaries.
    expansion::shape(&document).unwrap();
    assert!(
        build_catalog(&Config::default(), &document, "api")
            .unwrap_err()
            .is::<expansion::ComplexityLimit>()
    );
    assert!(
        parse_document(&serde_json::to_vec(&document).unwrap())
            .unwrap_err()
            .is::<expansion::ComplexityLimit>()
    );
}

#[test]
fn header_omissions_retain_fixed_specific_causes() {
    for (parameters, reason) in [
        (
            json!([{"in":"header","name":"Cookie"}]),
            Unsupported::HeaderReserved,
        ),
        (
            json!([{"in":"header","name":"Idempotency-Key","schema":{"type":"integer"}}]),
            Unsupported::IdempotencyKeySchema,
        ),
        (
            json!([{"in":"header","name":"bad\nname"}]),
            Unsupported::HeaderName,
        ),
        (
            json!([{"in":"header","name":"X-Business","style":"form"}]),
            Unsupported::HeaderSerialization,
        ),
        (
            json!([{"in":"header","name":"X-Business"},{"in":"header","name":"x-business"}]),
            Unsupported::HeaderDuplicate,
        ),
    ] {
        // Digest input retains duplicates; OpenAPI's parameter override merge
        // intentionally replaces matching names before this header check.
        let document = json!({"operations":[
            {"path":"/private_path","method":"GET","description":"PRIVATE_PROSE","params":parameters},
            {"path":"/healthy","method":"GET"}
        ]});
        let built = build_catalog(&Config::default(), &document, "api").unwrap();
        assert_eq!(built.tools.len(), 1);
        assert_eq!(built.tools[0].path, "/healthy");
        assert_eq!(
            built.issues,
            vec![OperationIssue {
                method: "GET".into(),
                path: "/private_path".into(),
                reason
            }]
        );
        let summary = omission_instructions(&built.issues).unwrap();
        assert!(summary.contains(reason.explanation()));
        assert!(!summary.contains("private_path") && !summary.contains("PRIVATE_PROSE"));
        assert!(
            build_tools(&Config::default(), &document, "api")
                .unwrap_err()
                .is::<IncompleteCatalog>()
        );
    }
}

#[test]
fn preferred_compatible_post_suppresses_only_its_exact_get_omission() {
    let document = json!({"paths":{
        "/same":{"get":{"parameters":[{"name":"nested","in":"query","schema":{"type":"object","properties":{"value":{"type":"string"}}}}]},
            "post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}}}}}},
        "/other":{"get":{}}
    }});
    let cfg = Config::default();
    let built = build_catalog(&cfg, &document, "api").unwrap();
    assert!(built.issues.is_empty());
    let tools = build_tools(&cfg, &document, "api").unwrap();
    assert_eq!(tools.len(), 2);
    let post = tools.iter().find(|tool| tool.path == "/same").unwrap();
    assert_eq!(
        (post.name.as_str(), post.method.as_str()),
        ("api_same", "POST")
    );
    let body = json!({"q":"original body"});
    assert_eq!(
        post.route("https://example.com", body.as_object().unwrap())
            .unwrap()
            .body,
        Some(body)
    );
    assert!(
        tools
            .iter()
            .any(|tool| tool.path == "/other" && tool.method == "GET")
    );

    let unpreferred = Config {
        prefer_post: false,
        ..Default::default()
    };
    let reported = build_catalog(&unpreferred, &document, "api").unwrap();
    assert_eq!(reported.tools.len(), 2);
    assert_eq!(
        reported.issues,
        vec![OperationIssue {
            method: "GET".into(),
            path: "/same".into(),
            reason: Unsupported::ParameterSerialization,
        }]
    );
    assert_eq!(
        build_tools(&unpreferred, &document, "api")
            .unwrap_err()
            .downcast_ref::<IncompleteCatalog>()
            .unwrap()
            .0,
        reported.issues
    );

    // Exact wire identity: a preferred POST cannot hide a different child path.
    let mut distinct = document.clone();
    distinct["paths"]["/same/child"] = json!({"get":document["paths"]["/same"]["get"]});
    let reported = build_catalog(&cfg, &distinct, "api").unwrap();
    assert_eq!(
        reported.issues,
        vec![OperationIssue {
            method: "GET".into(),
            path: "/same/child".into(),
            reason: Unsupported::ParameterSerialization,
        }]
    );
    assert!(
        build_tools(&cfg, &distinct, "api")
            .unwrap_err()
            .is::<IncompleteCatalog>()
    );
    let excluded = Config {
        exclude_operations: vec!["GET /same/child".into()],
        ..Default::default()
    };
    assert_eq!(build_tools(&excluded, &distinct, "api").unwrap().len(), 2);
}

fn unrestricted_definition() -> Value {
    json!({"description":"Any JSON value.","oneOf":[
        {"type":"null"},{"type":"boolean"},{"type":"number"},{"type":"string"},
        {"type":"array","items":{"$ref":"#/components/schemas/Value"}},
        {"type":"object","additionalProperties":{"$ref":"#/components/schemas/Value"},"propertyNames":{"type":"string"}}
    ]})
}
#[test]
fn exact_unrestricted_recursive_schema_preserves_every_json_wire_domain() {
    let document = json!({"openapi":"3.1.0","components":{"schemas":{"Value":unrestricted_definition()}},
        "paths":{"/values":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Value"}}}}}}}});
    let tools = build_tools(&Config::default(), &document, "api").unwrap();
    let tool = &tools[0];
    let body = tool.whole_body.as_ref().unwrap();
    assert!(body.required);
    assert_eq!(
        tool.input_schema["properties"][&body.name],
        json!({"description":"Any JSON value."})
    );
    let restored: ToolSpec = serde_json::from_value(serde_json::to_value(tool).unwrap()).unwrap();
    for value in [
        json!(null),
        json!(false),
        json!(3.5),
        json!("text"),
        json!([1,{"a":[null,true]}]),
        json!({"nested":[{},[]]}),
    ] {
        let arguments = Map::from_iter([(body.name.clone(), value.clone())]);
        for tool in [tool, &restored] {
            let request = tool.route("https://example.com", &arguments).unwrap();
            assert_eq!(request.body, Some(value.clone()));
            assert_eq!(request.url, "https://example.com/values");
            assert!(request.query.is_empty() && request.headers.is_empty());
        }
    }
    // Whole-document dynamic admission deliberately remains stricter than used-schema projection.
    assert!(expansion::import(&document).is_err());
}
#[test]
fn constrained_recursive_near_matches_and_resource_bases_stay_visible() {
    for pointer in [
        "/maxLength",
        "/oneOf/0/type",
        "/oneOf/4/items/$ref",
        "/oneOf/5/propertyNames/pattern",
        "/$id",
    ] {
        let mut definition = unrestricted_definition();
        match pointer {
            "/maxLength" => definition["maxLength"] = json!(5),
            "/oneOf/0/type" => definition["oneOf"][0]["type"] = json!("integer"),
            "/oneOf/4/items/$ref" => definition["oneOf"][4]["items"]["$ref"] = json!("#/missing"),
            "/oneOf/5/propertyNames/pattern" => {
                definition["oneOf"][5]["propertyNames"]["pattern"] = json!("^x")
            }
            _ => definition["$id"] = json!("https://example.com/resource"),
        }
        let root = json!({"components":{"schemas":{"Value":definition}}});
        let error = contracts::schema(
            &root,
            &json!({"$ref":"#/components/schemas/Value"}),
            &mut expansion::Budget::default(),
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<Unsupported>(),
            Some(&Unsupported::InputReference),
            "{pointer}: {error:#}"
        );
    }
}
#[test]
fn description_only_schema_reference_preserves_constraints_and_annotations() {
    let mut root =
        json!({"openapi":"3.1.0","Value":{"type":"string","minLength":2,"description":"target"}});
    let input = json!({"$ref":"#/Value","description":"use here"});
    assert_eq!(
        contracts::schema(&root, &input, &mut expansion::Budget::default()).unwrap(),
        json!({"allOf":[{"type":"string","minLength":2,"description":"target"},{"description":"use here"}]})
    );
    root["openapi"] = json!("3.0.3");
    assert_eq!(
        contracts::schema(&root, &input, &mut expansion::Budget::default()).unwrap(),
        root["Value"]
    );
    root["openapi"] = json!("3.1.0");
    let error = contracts::schema(
        &root,
        &json!({"$ref":"#/Value","description":"use here","maxLength":3}),
        &mut expansion::Budget::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<Unsupported>(),
        Some(&Unsupported::SchemaReferenceSiblings)
    );
}

#[test]
fn new_schema_normalizations_require_known_semantics_without_gating_ordinary_refs() {
    let ordinary = json!({"$ref":"#/Plain"});
    let annotated = json!({"$ref":"#/Plain","description":"use here"});
    let recursive = json!({"$ref":"#/components/schemas/Value"});
    let mut root = json!({"openapi":"3.1.0","Plain":{"type":"string"},"components":{"schemas":{"Value":unrestricted_definition()}}});
    for dialect in [json!("https://example.com/custom"), json!(null)] {
        root["jsonSchemaDialect"] = dialect;
        assert_eq!(
            contracts::schema(&root, &ordinary, &mut expansion::Budget::default()).unwrap(),
            root["Plain"]
        );
        for (schema, reason) in [
            (&recursive, Unsupported::InputReference),
            (&annotated, Unsupported::SchemaReferenceSiblings),
        ] {
            let error =
                contracts::schema(&root, schema, &mut expansion::Budget::default()).unwrap_err();
            assert_eq!(error.downcast_ref::<Unsupported>(), Some(&reason));
        }
    }
    for dialect in [
        "https://spec.openapis.org/oas/3.1/dialect/base",
        "https://json-schema.org/draft/2020-12/schema",
    ] {
        root["jsonSchemaDialect"] = json!(dialect);
        assert_eq!(
            contracts::schema(&root, &recursive, &mut expansion::Budget::default()).unwrap(),
            json!({"description":"Any JSON value."})
        );
        assert!(contracts::schema(&root, &annotated, &mut expansion::Budget::default()).is_ok());
    }
    root["jsonSchemaDialect"] = json!("https://example.com/custom");
    root["openapi"] = json!("3.0.3");
    assert_eq!(
        contracts::schema(&root, &annotated, &mut expansion::Budget::default()).unwrap(),
        root["Plain"],
        "old3.0 replacement is unchanged"
    );
}
#[test]
fn used_schema_and_target_ancestor_dialects_bound_only_the_new_transformations() {
    let mut root = json!({"openapi":"3.1.0","Plain":{"type":"string"},"components":{"schemas":{"Value":unrestricted_definition()}}});
    for declaration in [
        json!({"$schema":"https://example.com/custom"}),
        json!({"$vocabulary":{"https://example.com/assertions":true}}),
    ] {
        for (reference, reason) in [
            (
                json!({"$ref":"#/components/schemas/Value"}),
                Unsupported::InputReference,
            ),
            (
                json!({"$ref":"#/Plain","description":"use here"}),
                Unsupported::SchemaReferenceSiblings,
            ),
        ] {
            let mut schema = declaration.clone();
            schema["properties"] = json!({"v":reference});
            let error =
                contracts::schema(&root, &schema, &mut expansion::Budget::default()).unwrap_err();
            assert_eq!(error.downcast_ref::<Unsupported>(), Some(&reason));
        }
    }
    let pointer = "#/Container/Value";
    let mut value = unrestricted_definition();
    value["oneOf"][4]["items"]["$ref"] = json!(pointer);
    value["oneOf"][5]["additionalProperties"]["$ref"] = json!(pointer);
    root["Container"] = json!({"$schema":"https://json-schema.org/draft/2020-12/schema","Value":value,"Plain":{"type":"string"}});
    let reference = json!({"$ref":pointer});
    assert_eq!(
        contracts::schema(&root, &reference, &mut expansion::Budget::default()).unwrap(),
        json!({"description":"Any JSON value."})
    );
    root["Container"]["$schema"] = json!("https://example.com/custom");
    let error =
        contracts::schema(&root, &reference, &mut expansion::Budget::default()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<Unsupported>(),
        Some(&Unsupported::InputReference)
    );
    let annotated = json!({"$ref":"#/Container/Plain","description":"use here"});
    let error =
        contracts::schema(&root, &annotated, &mut expansion::Budget::default()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<Unsupported>(),
        Some(&Unsupported::SchemaReferenceSiblings)
    );
    assert_eq!(
        contracts::schema(
            &root,
            &json!({"$ref":"#/Container/Plain"}),
            &mut expansion::Budget::default()
        )
        .unwrap(),
        json!({"type":"string"})
    );
}
#[test]
fn opaque_dialects_and_schema_entries_named_like_declarations_do_not_change_context() {
    let root = json!({"openapi":"3.1.0","$schema":"https://example.com/catalog-metadata",
        "components":{"schemas":{"Value":unrestricted_definition(),"$schema":"https://example.com/unused-malformed-schema","$vocabulary":{}}}});
    let schema = json!({"type":"object","properties":{
        "$schema":{"type":"string"},"$vocabulary":{},"v":{"$ref":"#/components/schemas/Value"}},
        "example":{"$schema":"https://example.com/custom","$vocabulary":{"https://example.com/custom":true}},
        "default":{"$schema":"https://example.com/custom"}});
    let mut expected = schema.clone();
    expected["properties"]["v"] = json!({"description":"Any JSON value."});
    assert_eq!(
        contracts::schema(&root, &schema, &mut expansion::Budget::default()).unwrap(),
        expected
    );
    let error = contracts::schema(
        &root,
        &json!({"$ref":"#/components/schemas/$schema"}),
        &mut expansion::Budget::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<Unsupported>(),
        Some(&Unsupported::InputSchema),
        "explicitly selecting the malformed entry still fails"
    );
}

#[test]
fn reference_basic_annotations_preserve_the_full_bag_and_opaque_values() {
    let target =
        json!({"type":"string","minLength":2,"description":"target","default":"target default"});
    let mut root = json!({"openapi":"3.1.0","Value":target});
    let annotations = json!({"title":"Value","description":"use here",
        "default":{"$ref":"#/missing","$schema":"https://example.com/opaque"},
        "examples":[{"$ref":"#/also-missing","$schema":"https://example.com/data"}],
        "deprecated":false,"readOnly":true,"writeOnly":false});
    let mut input = annotations.clone();
    input["$ref"] = json!("#/Value");
    assert_eq!(
        contracts::schema(&root, &input, &mut expansion::Budget::default()).unwrap(),
        json!({"allOf":[target,annotations]})
    );
    for (key, value) in [
        ("title", json!(false)),
        ("description", json!(1)),
        ("examples", json!({})),
        ("deprecated", json!(null)),
        ("readOnly", json!("true")),
        ("writeOnly", json!(1)),
        ("maxLength", json!(3)),
        ("x-unknown", json!("annotation")),
    ] {
        let mut invalid = input.clone();
        invalid[key] = value;
        let error =
            contracts::schema(&root, &invalid, &mut expansion::Budget::default()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<Unsupported>(),
            Some(&Unsupported::SchemaReferenceSiblings),
            "{key}"
        );
    }
    for declaration in [
        json!({"$schema":"https://example.com/custom"}),
        json!({"$vocabulary":{"https://example.com/custom":true}}),
    ] {
        let mut used = declaration;
        used["properties"] = json!({"value":input});
        let error = contracts::schema(&root, &used, &mut expansion::Budget::default()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<Unsupported>(),
            Some(&Unsupported::SchemaReferenceSiblings)
        );
    }
    root["jsonSchemaDialect"] = json!("https://example.com/custom");
    let error = contracts::schema(&root, &input, &mut expansion::Budget::default()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<Unsupported>(),
        Some(&Unsupported::SchemaReferenceSiblings)
    );
    root.as_object_mut().unwrap().remove("jsonSchemaDialect");
    root["Container"] = json!({"$schema":"https://example.com/custom","Value":target});
    input["$ref"] = json!("#/Container/Value");
    let error = contracts::schema(&root, &input, &mut expansion::Budget::default()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<Unsupported>(),
        Some(&Unsupported::SchemaReferenceSiblings)
    );
    root["Container"] = json!({"$id":"https://example.com/base","Value":{"$ref":"#/Value"}});
    let error = contracts::schema(&root, &input, &mut expansion::Budget::default()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<Unsupported>(),
        Some(&Unsupported::InputReference)
    );
    root["openapi"] = json!("3.0.3");
    input["$ref"] = json!("#/Value");
    assert_eq!(
        contracts::schema(&root, &input, &mut expansion::Budget::default()).unwrap(),
        target
    );
}

#[test]
fn reference_defaults_are_metadata_and_do_not_insert_omitted_request_fields() {
    let document = json!({"openapi":"3.1.0","components":{"schemas":{"Value":{"type":"string","minLength":2}}},
    "paths":{"/value":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{
        "type":"object","properties":{"optional":{"$ref":"#/components/schemas/Value","default":"fallback","examples":["example"]}}
    }}}}}}}});
    let tools = build_tools(&Config::default(), &document, "api").unwrap();
    assert_eq!(tools.len(), 1);
    let tool = &tools[0];
    assert_eq!(
        tool.input_schema["properties"]["optional"],
        json!({"allOf":[{"type":"string","minLength":2},{"default":"fallback","examples":["example"]}]})
    );
    assert_eq!(
        tool.route("https://example.com", &Default::default())
            .unwrap()
            .body,
        Some(json!({}))
    );
}
