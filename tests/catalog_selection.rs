use serde_json::json;
use x402_treazury::{
    catalog::{Config, build_tools},
    config,
};

#[test]
fn post_preference_uses_exact_paths_before_collision_naming_and_name_filters() {
    let doc = json!({"paths":{
        "/pair":{"get":{},"post":{"requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"q":{"type":"string"}}}}}}}},
        "/get":{"get":{}}, "/post":{"post":{}},
        "/a-b":{"get":{}}, "/a_b":{"post":{}},
        "/many":{"get":{},"post":{},"delete":{}}
    }});
    let mut cfg = Config::default();
    let tools = build_tools(&cfg, &doc, "api").unwrap();
    assert!(cfg.prefer_post);
    let pair = tools.iter().find(|t| t.name == "api_pair").unwrap();
    assert_eq!(
        (pair.method.as_str(), pair.path.as_str()),
        ("POST", "/pair")
    );
    let args = json!({"q":"private query"});
    let route = pair
        .route("https://example.com", args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.body, Some(args));
    assert!(route.query.is_empty());
    for name in [
        "api_get",
        "api_post",
        "api_a_b_get",
        "api_a_b_post",
        "api_many_delete",
        "api_many_post",
    ] {
        assert!(tools.iter().any(|t| t.name == name), "{name}");
    }
    assert!(!tools.iter().any(|t| t.name == "api_many_get"));
    cfg.exclude_tools = vec!["api_pair".into()];
    assert!(
        !build_tools(&cfg, &doc, "api")
            .unwrap()
            .iter()
            .any(|t| t.path == "/pair")
    );
    cfg.include_tools = vec!["api_pair_get".into()];
    assert!(build_tools(&cfg, &doc, "api").is_err());
    cfg.exclude_tools.clear();
    cfg.prefer_post = false;
    let selected = build_tools(&cfg, &doc, "api").unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].method, "GET");
}

#[test]
fn compatibility_can_remove_post_and_digest_methods_are_case_insensitive() {
    let doc = json!({"operations":[
        {"path":"/pair","method":"GET","tags":["read"]},
        {"path":"/pair","method":"post","tags":["blocked"]}
    ]});
    let mut cfg = Config::default();
    assert_eq!(build_tools(&cfg, &doc, "api").unwrap()[0].method, "POST");
    cfg.exclude_tags = vec!["blocked".into()];
    let tools = build_tools(&cfg, &doc, "api").unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(
        (tools[0].name.as_str(), tools[0].method.as_str()),
        ("api_pair", "GET")
    );
    cfg.exclude_tags.clear();
    cfg.include_operations = vec!["GET /pair".into()];
    assert_eq!(build_tools(&cfg, &doc, "api").unwrap()[0].method, "GET");
}

#[tokio::test]
async fn preference_inherits_and_sources_override_either_value() {
    let dir = tempfile::tempdir().unwrap();
    let provider = dir.path().join("provider.toml");
    let source = dir.path().join("deployment.toml");
    for value in [true, false] {
        std::fs::write(&provider, format!("spec='spec.json'\nprefer_post={value}")).unwrap();
        let inherited =
            config::resolve(toml::from_str("extends='provider.toml'").unwrap(), &source)
                .await
                .unwrap();
        assert_eq!(inherited.settings.prefer_post, value);
        assert!(inherited.origins["prefer_post"].ends_with("provider.toml"));
        let replaced = config::resolve(
            toml::from_str(&format!("extends='provider.toml'\nprefer_post={}", !value)).unwrap(),
            &source,
        )
        .await
        .unwrap();
        assert_eq!(replaced.settings.prefer_post, !value);
        assert_eq!(
            replaced.origins["prefer_post"],
            source.display().to_string()
        );
    }
    assert!(
        config::resolve(
            toml::from_str("spec='spec.json'\nprefer_post='false'").unwrap(),
            &source
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn captured_method_pairs_keep_post_body_contracts_and_canonical_names() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (provider, paths) in [
        (
            "genuinegood",
            vec![
                "/v1/grants/search",
                "/v1/grants/detail",
                "/v1/grants/fit",
                "/v1/grants/brief",
                "/v1/grants/preflight",
            ],
        ),
        (
            "otto",
            vec![
                "/generate-meme",
                "/llm-research",
                "/tx-explainer",
                "/meta-intelligence",
            ],
        ),
    ] {
        let mut cfg = config::load(&root.join(format!("providers/{provider}.toml")))
            .await
            .unwrap()
            .settings;
        let doc = serde_json::from_slice(
            &std::fs::read(root.join(format!("tests/fixtures/{provider}_openapi.json"))).unwrap(),
        )
        .unwrap();
        let canonical = build_tools(&cfg, &doc, provider).unwrap();
        if provider == "genuinegood" {
            assert!(!canonical.iter().any(|t| t.path == "/v1/grants/pass"));
        }
        cfg.prefer_post = false;
        let twins = build_tools(&cfg, &doc, provider).unwrap();
        for path in paths {
            let tool = canonical.iter().find(|t| t.path == path).unwrap();
            assert_eq!(canonical.iter().filter(|t| t.path == path).count(), 1);
            assert_eq!(tool.method, "POST");
            let post = twins
                .iter()
                .find(|t| t.path == path && t.method == "POST")
                .unwrap();
            assert_eq!(post.name, format!("{}_post", tool.name));
            assert_eq!(tool.input_schema, post.input_schema);
            let args = match path {
                "/v1/grants/search" => json!({"keyword":"education","rows":1,"agencies":["ED"]}),
                "/meta-intelligence" => {
                    json!({"ask":"research sources","required_capabilities":["source_identity"]})
                }
                _ => continue,
            };
            let base = cfg
                .base_url
                .as_deref()
                .unwrap_or_else(|| doc["servers"][0]["url"].as_str().unwrap());
            let route = tool.route(base, args.as_object().unwrap()).unwrap();
            assert_eq!(route.body, Some(args));
            assert!(route.query.is_empty());
        }
    }
}

#[test]
fn exact_operation_exclusions_preserve_methods_children_and_independent_sources() {
    let doc = json!({"paths": {
        "/pair":{"get":{},"post":{}},
        "/api/feedback":{"post":{"tags":["account"]}},
        "/api/feedback/summary":{"get":{"tags":["public"]}},
        "/v1/monitors":{"get":{},"post":{}},
        "/v1/monitors/sources":{"get":{"tags":["public"]}},
        "/v1/monitors/{id}":{"get":{}}
    }});
    let mut cfg = Config {
        exclude_operations: vec![
            "POST /pair".into(),
            "POST /api/feedback".into(),
            "GET /v1/monitors".into(),
            "POST /v1/monitors".into(),
        ],
        exclude: vec!["/v1/monitors/{id}".into()],
        ..Default::default()
    };
    let tools = build_tools(&cfg, &doc, "first").unwrap();
    assert_eq!(tools.len(), 3);
    let pair = tools.iter().find(|t| t.path == "/pair").unwrap();
    assert_eq!(
        (pair.name.as_str(), pair.method.as_str()),
        ("first_pair", "GET")
    );
    cfg.include_operations = vec!["POST /pair".into(), "GET /api/feedback/summary".into()];
    let included = build_tools(&cfg, &doc, "first").unwrap();
    assert_eq!(included.len(), 1);
    assert_eq!(included[0].path, "/api/feedback/summary");
    cfg.include_operations.clear();
    cfg.tags = vec!["public".into()];
    for prefix in ["first", "independent"] {
        let tools = build_tools(&cfg, &doc, prefix).unwrap();
        assert_eq!(tools.len(), 2);
        for path in ["/api/feedback/summary", "/v1/monitors/sources"] {
            let tool = tools.iter().find(|t| t.path == path).unwrap();
            assert!(tool.name.starts_with(prefix));
            assert_eq!(
                tool.route("https://example.com", &serde_json::Map::new())
                    .unwrap()
                    .url,
                format!("https://example.com{path}")
            );
        }
    }
}

#[tokio::test]
async fn exact_operation_exclusions_validate_replace_and_clear_offline() {
    let dir = tempfile::tempdir().unwrap();
    let provider = dir.path().join("provider.toml");
    let source = dir.path().join("source.toml");
    std::fs::write(
        &provider,
        "spec='missing.json'\nexclude_operations=['POST /parent']",
    )
    .unwrap();
    for (extra, expected, origin) in [
        ("", vec!["POST /parent"], &provider),
        (
            "exclude_operations=['GET /child']",
            vec!["GET /child"],
            &source,
        ),
        ("exclude_operations=[]", vec![], &source),
    ] {
        let resolved = config::resolve(
            toml::from_str(&format!("extends='provider.toml'\n{extra}")).unwrap(),
            &source,
        )
        .await
        .unwrap();
        assert_eq!(resolved.settings.exclude_operations, expected);
        assert_eq!(
            resolved.origins["exclude_operations"],
            origin.display().to_string()
        );
    }
    for field in ["include_operations", "exclude_operations"] {
        for invalid in ["get /parent", "HEAD /parent", "POST relative", "POST"] {
            let error = config::resolve(
                toml::from_str(&format!("spec='missing.json'\n{field}=['{invalid}']")).unwrap(),
                &source,
            )
            .await
            .err()
            .unwrap();
            assert!(format!("{error:#}").contains(field));
        }
    }
}
