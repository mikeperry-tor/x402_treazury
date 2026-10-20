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
