use serde_json::json;
use x402_treazury::{
    catalog::{Config, build_tools},
    config,
    deployment::Deployment,
};

fn document() -> serde_json::Value {
    json!({"openapi":"3.0.0", "paths": {
        "/company": {
            "get": {"tags":["Company"],"responses":{}},
            "post": {"tags":["Company"],"responses":{}}
        },
        "/person": {"get":{"tags":["Person"],"responses":{}}}
    }})
}

#[test]
fn names_filter_after_collision_naming_and_include_help_explicitly() {
    let mut cfg = Config {
        help_url: Some("https://example.com/llms.txt".into()),
        include_tools: vec!["api_company_?et".into(), "api_help".into()],
        ..Default::default()
    };
    let tools = build_tools(&cfg, &document(), "api").unwrap();
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["api_company_get", "api_help"]
    );
    assert_eq!(tools[0].method, "GET");
    assert_eq!(tools[0].path, "/company");
    cfg.exclude_tools = vec!["*_help".into()];
    assert_eq!(build_tools(&cfg, &document(), "api").unwrap().len(), 1);
    cfg.include_tools = vec!["api_help".into()];
    cfg.exclude_tools.clear();
    assert!(
        build_tools(&cfg, &document(), "api").unwrap()[0]
            .help_url
            .is_some()
    );
}

#[test]
fn name_filters_intersect_existing_selection_and_fail_on_unknown_exact_names() {
    let mut cfg = Config {
        tags: vec!["Company".into()],
        include_tools: vec!["api_*".into()],
        exclude_tools: vec!["*_post".into()],
        ..Default::default()
    };
    assert_eq!(
        build_tools(&cfg, &document(), "api").unwrap()[0].name,
        "api_company_get"
    );
    cfg.include_tools = vec!["api_person".into()];
    assert!(
        build_tools(&cfg, &document(), "api")
            .unwrap_err()
            .to_string()
            .contains("unknown provider tool selector api_person")
    );
    cfg.include_tools = vec!["missing_*".into()];
    assert!(
        build_tools(&cfg, &document(), "api")
            .unwrap_err()
            .to_string()
            .contains("no tools matched")
    );
    cfg.include_tools = vec!["api_company_get".into()];
    cfg.exclude_tools = vec!["api_company_get".into()];
    assert!(build_tools(&cfg, &document(), "api").is_err());
    cfg.exclude_tools.clear();
    cfg.include_operations = vec!["POST /company".into()];
    assert!(build_tools(&cfg, &document(), "api").is_err());
}

#[tokio::test]
async fn composition_replaces_and_clears_name_filters_and_listener_narrows_them() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("spec.json"), document().to_string()).unwrap();
    std::fs::write(
        dir.path().join("provider.toml"),
        r#"
spec = "spec.json"
base_url = "https://example.com"
prefix = "api"
include_tools = ["api_company_*"]
exclude_tools = ["api_company_post"]
"#,
    )
    .unwrap();
    let path = dir.path().join("deployment.toml");
    let cleared = config::resolve(
        toml::from_str("extends = 'provider.toml'\ninclude_tools = []\nexclude_tools = []")
            .unwrap(),
        &path,
    )
    .await
    .unwrap();
    assert_eq!(
        build_tools(&cleared.settings, &document(), "api")
            .unwrap()
            .len(),
        3
    );
    let resolved = config::resolve(
        toml::from_str(
            "extends = 'provider.toml'\ninclude_tools = ['api_person']\nexclude_tools = []",
        )
        .unwrap(),
        &path,
    )
    .await
    .unwrap();
    assert_eq!(resolved.settings.include_tools, ["api_person"]);
    assert!(resolved.settings.exclude_tools.is_empty());
    assert_eq!(
        build_tools(&resolved.settings, &document(), "api").unwrap()[0].name,
        "api_person"
    );
    std::fs::write(
        &path,
        r#"
version = 1
[sources.research]
extends = "provider.toml"
exclude_tools = []
[wallets.default]
mode = "static"
private_key_env = "UNFUNDED_TEST_KEY"
[servers.research]
listen = "127.0.0.1:7339"
bearer_token_env = "UNSET_TEST_TOKEN"
wallet = "default"
sources = ["research"]
include_tools = ["api_company_post"]
"#,
    )
    .unwrap();
    let inventories = Deployment::load(&path).await.unwrap().inventory();
    assert_eq!(inventories[0].tools.len(), 1);
    assert_eq!(inventories[0].tools[0].tool.name, "api_company_post");
    let empty_selector = Config {
        spec: "spec.json".into(),
        include_tools: vec![String::new()],
        ..Default::default()
    };
    assert!(
        config::validate(&empty_selector)
            .unwrap_err()
            .to_string()
            .contains("empty tool selector")
    );
}

#[test]
fn unmatched_wildcards_warn_while_other_matches_remain_usable() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let writer = log.reopen().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.try_clone().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let cfg = Config {
        include_tools: vec!["api_person".into(), "missing_*".into()],
        exclude_tools: vec!["absent_*".into()],
        ..Default::default()
    };
    assert_eq!(
        build_tools(&cfg, &document(), "api").unwrap()[0].name,
        "api_person"
    );
    let evidence = std::fs::read_to_string(log.path()).unwrap();
    assert!(evidence.contains("Provider tool pattern matches nothing"));
    assert!(evidence.contains("missing_*"));
    assert!(evidence.contains("absent_*"));
}
