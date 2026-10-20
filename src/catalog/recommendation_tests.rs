use super::*;

fn document() -> Value {
    json!({"paths":{
        "/read":{"get":{"summary":"GET guidance"},"post":{"summary":"POST guidance"}},
        "/other":{"get":{"summary":"Other guidance"}},
        "/collision-a":{"get":{}},"/collision_a":{"get":{}}
    }})
}

#[test]
fn recommendations_are_advisory_and_selection_preserves_named_contracts() {
    let mut cfg = Config {
        recommended_tools: vec!["api_read".into(), "api_help".into()],
        help_url: Some("https://docs.example/guide".into()),
        ..Default::default()
    };
    let full = build_tools(&cfg, &document(), "api").unwrap();
    assert_eq!(full.len(), 5);
    assert_eq!(
        full.iter().find(|t| t.name == "api_read").unwrap().method,
        "POST"
    );
    cfg.use_recommended_tools = true;
    // Ordinary selectors validate against the same full inventory, even where
    // advice removes their matches. Applying filters in sequence would fail.
    cfg.include_tools = vec!["api_read".into(), "api_other".into(), "api_help".into()];
    cfg.exclude_tools = vec!["api_other".into()];
    let selected = build_tools(&cfg, &document(), "api").unwrap();
    assert_eq!(selected.len(), 2);
    let priced = build_tools_with_prices(
        &cfg,
        &document(),
        "api",
        &BTreeMap::from([(("POST".into(), "/read".into()), "$0.01".into())]),
    )
    .unwrap();
    assert_eq!(
        priced.iter().map(|t| &t.name).collect::<Vec<_>>(),
        selected.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
    for tool in selected {
        let original = full.iter().find(|t| t.name == tool.name).unwrap();
        assert_eq!(
            serde_json::to_value(&tool).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        assert!(cfg.recommendation_matcher().unwrap()(&tool.name));
        assert!(
            serde_json::to_value(tool)
                .unwrap()
                .get("recommended")
                .is_none()
        );
    }
    cfg.recommended_tools = vec!["api_collision_a*".into()];
    cfg.include_tools.clear();
    cfg.exclude_tools.clear();
    let selected = build_tools(&cfg, &document(), "api").unwrap();
    assert_eq!(selected.len(), 2);
    assert!(
        selected
            .iter()
            .all(|t| full.iter().any(|old| old.name == t.name))
    );
}

#[test]
fn recommendations_fail_closed_only_when_selected_and_never_rewrite_prefixes() {
    let mut cfg = Config {
        recommended_tools: vec!["api_absent".into()],
        ..Default::default()
    };
    assert_eq!(build_tools(&cfg, &document(), "alias").unwrap().len(), 4);
    cfg.use_recommended_tools = true;
    let error = build_tools(&cfg, &document(), "alias").unwrap_err();
    assert!(error.is::<ToolConfigurationError>());
    assert!(
        error
            .to_string()
            .contains("unknown provider tool selector api_absent")
    );
    for advice in [vec![], vec!["api_*".into()]] {
        cfg.recommended_tools = advice;
        assert!(
            build_tools(&cfg, &document(), "alias")
                .unwrap_err()
                .to_string()
                .contains("no tools matched recommended_tools")
        );
    }
    cfg.recommended_tools = vec!["alias_read".into(), "alias_stale*".into()];
    assert_eq!(build_tools(&cfg, &document(), "alias").unwrap().len(), 1);
    cfg.exclude_tools = vec!["alias_read".into()];
    assert!(
        build_tools(&cfg, &document(), "alias")
            .unwrap_err()
            .to_string()
            .contains("no tools matched provider tool-name filters")
    );
    cfg.exclude_tools.clear();
    cfg.recommended_tools = vec!["".into()];
    cfg.use_recommended_tools = false;
    assert!(build_tools(&cfg, &document(), "alias").is_err());
    assert!(cfg.recommendation_matcher().is_err());
}

#[test]
fn recommendation_markers_are_operator_metadata_and_help_is_explicit() {
    let mut cfg = Config {
        recommended_tools: vec!["api_read".into()],
        help_url: Some("https://docs.example/guide".into()),
        use_recommended_tools: true,
        ..Default::default()
    };
    assert_eq!(build_tools(&cfg, &document(), "api").unwrap().len(), 1);
    cfg.use_recommended_tools = false;
    let tools = build_tools(&cfg, &document(), "api").unwrap();
    let inventory: Vec<_> = tools
        .iter()
        .map(|tool| ToolInspection {
            tool,
            recommended: cfg.recommendation_matcher().unwrap()(&tool.name),
        })
        .collect();
    let shown = serde_json::to_value(inventory).unwrap();
    assert_eq!(
        shown
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["recommended"] == true)
            .count(),
        1
    );
    assert_eq!(
        shown
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "api_help")
            .unwrap()["recommended"],
        false
    );
    // Later name selection cannot resurrect the GET twin.
    cfg.recommended_tools = vec!["api_read_get".into()];
    cfg.use_recommended_tools = true;
    assert!(build_tools(&cfg, &document(), "api").is_err());
    cfg.prefer_post = false;
    assert_eq!(
        build_tools(&cfg, &document(), "api").unwrap()[0].method,
        "GET"
    );
}
