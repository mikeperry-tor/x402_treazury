use super::*;

#[test]
fn operation_filters_precede_collision_counts_and_preserve_payloads() {
    let kept = json!({"path":"/v1/item", "method":"get", "tags":["Keep", 7],
        "description":"Vendor prose remains whole.\nSecond paragraph.",
        "params":[{"name":"query","in":"query","schema":{"type":"string"}}],
        "body":{"schema":{"type":"object"}}, "credit_pricing":{"credits":3},
        "opaque":{"nested":[1,2,3]}});
    let root = json!({"operations":[
        {"path":"/v1/item","method":"post","tags":["Keep"]},
        kept,
        {"path":"/v1/hidden","method":"get","tags":["Keep","Blocked"]},
        {"path":"/v1/other","method":"get","tags":["keep"]},
        {"path":"/v1/itemized","method":"get","tags":["Keep"]},
        {"path":"/v10/item","method":"get","tags":["Keep"]}
    ]});
    let cfg = Config {
        include_operations: vec![
            "GET /v1/item".into(),
            "GET /v1/hidden".into(),
            "GET /v1/other".into(),
            "GET /v1/itemized".into(),
            "GET /v10/item".into(),
        ],
        include: vec!["/v1".into(), "/v1/item".into()],
        exclude: vec!["/v1/itemized".into()],
        tags: vec!["Keep".into()],
        exclude_tags: vec!["Blocked".into()],
        ..Default::default()
    };
    assert_eq!(
        selected_operations(&cfg, &root, "api").unwrap(),
        vec![(kept, "api_root".into())]
    );
}

#[test]
fn stable_inventory_order_resolves_base_method_and_duplicate_collisions() {
    let a = json!({"path":"/a", "method":"get", "description":"first duplicate"});
    let b = json!({"path":"/a", "method":"get", "description":"second duplicate"});
    let post = json!({"path":"/a", "method":"post"});
    let suffix = json!({"path":"/a_get", "method":"get"});
    let root_op = json!({"path":"/", "method":"get"});
    let root = json!({"operations":[suffix, post, a, root_op, b]});
    assert_eq!(
        selected_operations(&Config::default(), &root, "api").unwrap(),
        vec![
            (root_op, "api_root".into()),
            (a, "api_a_get".into()),
            (b, "api_a_get_2".into()),
            (post, "api_a_post".into()),
            (suffix, "api_a_get_3".into()),
        ]
    );
}

#[test]
fn empty_source_filtered_inventory_and_invalid_operations_keep_error_categories() {
    let empty =
        selected_operations(&Config::default(), &json!({"operations":[]}), "api").unwrap_err();
    assert_eq!(empty.to_string(), "no operations matched");
    assert!(empty.downcast_ref::<ToolConfigurationError>().is_none());
    let cfg = Config {
        include_operations: vec!["GET /absent".into()],
        ..Default::default()
    };
    let filtered = selected_operations(
        &cfg,
        &json!({"operations":[{"path":"/a","method":"get"}]}),
        "api",
    )
    .unwrap_err();
    assert_eq!(
        filtered.to_string(),
        "invalid provider tool configuration: no operations matched"
    );
    assert!(filtered.downcast_ref::<ToolConfigurationError>().is_some());
    // Invalid entries must fail even when the allowlist would discard them.
    for invalid in [json!({"path":7,"method":"get"}), json!({"path":"/a"})] {
        let error = selected_operations(&cfg, &json!({"operations":[invalid]}), "api").unwrap_err();
        assert_eq!(error.to_string(), "invalid operation");
        assert!(error.downcast_ref::<ToolConfigurationError>().is_none());
    }
}
