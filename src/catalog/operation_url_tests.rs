use super::*;

fn configured() -> Config {
    Config {
        operation_url_extension: Some("x-direct-url".into()),
        ..Default::default()
    }
}

#[test]
fn operation_urls_preserve_identity_and_route_each_service_with_original_arguments() {
    let document = json!({"paths":{
        "/service/items/{id}":{"get":{
            "tags":["items"], "x-direct-url":"https://items.example.com/items/{id}?fixed=yes",
            "parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}},
                {"name":"q","in":"query","schema":{"type":"string"}},
                {"name":"X-Region","in":"header","schema":{"type":"string"}}]}},
        "/rates/rates":{"get":{"x-direct-url":"HTTPS://rates.example.com/rates"}},
        "/fallback":{"get":{}}
    }});
    let mut cfg = configured();
    cfg.response_mappings = serde_json::from_value(json!({"GET /service/items/{id}":{
        "images":[{"pointer":"/image","encoding":"data_uri"}]
    }}))
    .unwrap();
    let tools = build_tools(&cfg, &document, "api").unwrap();
    let item = tools
        .iter()
        .find(|t| t.name == "api_service_items_id")
        .unwrap();
    assert_eq!(item.path, "/service/items/{id}");
    assert!(item.response_mapping.is_some());
    assert!(matches_operation_tags(
        &operations(&document, None).unwrap(),
        item,
        &["items".into()],
        &[]
    ));
    let args = json!({"id":"a/b?c#d", "q":"original", "X-Region":"north"});
    let routed = item
        .route("https://gateway.example.com/api", args.as_object().unwrap())
        .unwrap();
    assert_eq!(
        routed.url,
        "https://items.example.com/items/a%2Fb%3Fc%23d?fixed=yes"
    );
    assert_eq!(routed.query["q"], "original");
    assert_eq!(routed.headers["x-region"], "north");
    assert!(routed.body.is_none());
    assert!(
        item.route("https://gateway.example.com/api", &Map::new())
            .is_err()
    );
    for id in [".", ".."] {
        assert!(
            item.route(
                "https://gateway.example.com/api",
                json!({"id":id}).as_object().unwrap()
            )
            .is_err()
        );
    }
    for (name, expected) in [
        ("api_rates_rates", "HTTPS://rates.example.com/rates"),
        ("api_fallback", "https://gateway.example.com/api/fallback"),
    ] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        assert_eq!(
            tool.route("https://gateway.example.com/api", &Map::new())
                .unwrap()
                .url,
            expected
        );
    }
    // Legacy serialized tools keep base/path routing; new tools retain the template.
    let saved = serde_json::to_value(item).unwrap();
    let restored: ToolSpec = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(restored.request_url, item.request_url);
    let mut legacy = saved;
    legacy.as_object_mut().unwrap().remove("request_url");
    assert!(
        serde_json::from_value::<ToolSpec>(legacy)
            .unwrap()
            .request_url
            .is_none()
    );
}

#[test]
fn operation_urls_require_opt_in_and_do_not_validate_discarded_get_twins() {
    let document = json!({"paths":{"/buy":{
        "get":{"x-direct-url":false},
        "post":{"x-direct-url":"https://buy.example.com/purchase", "requestBody":{"content":{
            "application/json":{"schema":{"type":"object","properties":{"amount":{"type":"integer"}}}}
        }}}
    }}});
    for cfg in [Config::default(), configured()] {
        let tools = build_tools(&cfg, &document, "api").unwrap();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        let args = json!({"amount":3});
        let route = tool
            .route("https://gateway.example.com", args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.method, "POST");
        assert_eq!(route.body, Some(args));
        assert_eq!(
            route.url,
            if cfg.operation_url_extension.is_some() {
                "https://buy.example.com/purchase"
            } else {
                "https://gateway.example.com/buy"
            }
        );
    }
    let digest = json!({"operations":[{"path":"/read","method":"get",
        "request_url":"https://forged.example.com/ignored",
        "x-direct-url":"https://read.example.com/direct"}]});
    assert!(
        build_tools(&Config::default(), &digest, "api").unwrap()[0]
            .request_url
            .is_none()
    );
    assert_eq!(
        build_tools(&configured(), &digest, "api").unwrap()[0]
            .request_url
            .as_deref(),
        Some("https://read.example.com/direct")
    );
}

#[test]
fn operation_urls_reject_malformed_private_and_parameterized_authorities() {
    for target in [
        Value::Null,
        json!(17),
        json!(""),
        json!("/relative"),
        json!("http://api.example.com/read"),
        json!("https://user:secret@api.example.com/read"),
        json!("https://api.example.com/read#fragment"),
        json!("https://127.0.0.1/read"),
        json!("https://localhost/read"),
        json!("https://{host}.example.com/read"),
    ] {
        let document = json!({"paths":{"/read":{"get":{"x-direct-url":target}}}});
        assert!(
            build_tools(&configured(), &document, "api").is_err(),
            "{target}"
        );
        assert!(build_tools(&Config::default(), &document, "api").is_ok());
    }
}
