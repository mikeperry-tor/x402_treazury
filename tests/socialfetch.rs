use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
use x402_treazury::{
    catalog::{Config, build_tools, tag_counts},
    deployment::Deployment,
};
fn fixture() -> (Config, Value) {
    (
        toml::from_str(include_str!("../providers/socialfetch.toml")).unwrap(),
        serde_json::from_str(include_str!("../tests/fixtures/socialfetch_openapi.json")).unwrap(),
    )
}
#[test]
fn platform_tags_names_pricing_and_routing_preserve_versions() {
    let (mut cfg, mut root) = fixture();
    // Synthetic public metadata witness, not a captured vendor operation.
    root["paths"]["/v1/webhook-public-info"] =
        json!({"get":{"summary":"Public webhook metadata","tags":["System"],"security":[]}});
    let all = build_tools(&cfg, &root, "socialfetch").unwrap();
    assert!(all.iter().any(|t| t.path == "/v1/monitors/sources"));
    assert!(!all.iter().any(|t| t.path == "/v1/monitors"
        || t.path == "/v1/monitors/{id}"
        || t.path.starts_with("/v1/monitors/{id}/")));
    assert!(all.iter().all(|t| !t.name.starts_with("socialfetch_v1_")));
    assert!(
        all.iter()
            .any(|t| t.name.starts_with("socialfetch_v2_linkedin_"))
    );
    assert!(all.iter().any(|t| t.path == "/v1/web/ask"));
    assert!(all.iter().any(|t| t.path == "/v1/webhook-public-info"));
    for path in ["/v1/webhook-endpoints", "/v1/webhook-deliveries"] {
        assert!(
            !all.iter()
                .any(|t| t.path == path || t.path.starts_with(&format!("{path}/")))
        );
    }
    let profile = all
        .iter()
        .find(|t| t.name == "socialfetch_twitter_profiles_handle")
        .unwrap();
    assert!(
        profile
            .description
            .contains("1 credit per successful request")
    );
    assert_eq!(
        profile
            .route(
                cfg.base_url.as_ref().unwrap(),
                json!({"handle":"alice"}).as_object().unwrap()
            )
            .unwrap()
            .url,
        "https://api.socialfetch.dev/v1/twitter/profiles/alice"
    );
    cfg.tags = vec!["LinkedIn".into()];
    let linkedin = build_tools(&cfg, &root, "socialfetch").unwrap();
    assert!(linkedin.iter().any(|t| t.path.starts_with("/v1/linkedin/")));
    assert!(linkedin.iter().any(|t| t.path.starts_with("/v2/linkedin/")));
    cfg.include = vec!["/v1".into()];
    assert!(
        build_tools(&cfg, &root, "socialfetch")
            .unwrap()
            .iter()
            .all(|t| t.help_url.is_some() || t.path.starts_with("/v1/"))
    );
    cfg.tags = vec!["Twitter".into(), "YouTube".into()];
    cfg.exclude_tags.push("YouTube".into());
    assert!(
        build_tools(&cfg, &root, "socialfetch")
            .unwrap()
            .iter()
            .all(|t| t.help_url.is_some() || t.name.starts_with("socialfetch_twitter_"))
    );
    cfg.tags = vec!["twitter".into()];
    assert!(build_tools(&cfg, &root, "socialfetch").is_err());
    let counts = tag_counts(&root).unwrap();
    assert!(counts.contains_key("Yelp"));
}
#[tokio::test]
async fn meta_sources_override_tags_and_cli_inventories_need_no_credentials() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = tempfile::tempdir().unwrap();
    let (mut cfg, root) = fixture();
    cfg.spec = repo
        .join("tests/fixtures/socialfetch_openapi.json")
        .display()
        .to_string();
    std::fs::write(
        dir.path().join("socialfetch.toml"),
        toml::to_string(&cfg).unwrap(),
    )
    .unwrap();
    let path = dir.path().join("servers.toml");
    std::fs::write(
        &path,
        r#"
version = 1
[sources.socialfetch]
extends = "socialfetch.toml"
tags = ["Twitter", "YouTube", "Auth"]
exclude_tags = ["Auth", "YouTube"]
[wallets.default]
mode = "static"
private_key_env = "UNSET_KEY"
[servers.social]
listen = "127.0.0.1:0"
bearer_token_env = "UNSET_TOKEN"
wallet = "default"
sources = ["socialfetch"]
include_tools = ["socialfetch_twitter_*"]
"#,
    )
    .unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    assert!(
        deployment.inventory()[0]
            .tools
            .iter()
            .all(|t| t.tool.name.starts_with("socialfetch_twitter_"))
    );
    assert_eq!(
        deployment.tag_inventory().unwrap()["socialfetch"],
        tag_counts(&root).unwrap()
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .args(["catalog", "tags", "--config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let counts: BTreeMap<String, BTreeMap<String, usize>> =
        serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        counts["socialfetch"]["Twitter"],
        tag_counts(&root).unwrap()["Twitter"]
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .args([
            "catalog",
            "tools",
            "--provider",
            dir.path().join("socialfetch.toml").to_str().unwrap(),
            "--tags",
            "Twitter,YouTube",
            "--exclude-tags",
            "YouTube",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let tools: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(tools.iter().all(|t| {
        t["name"] == "socialfetch_help"
            || t["name"]
                .as_str()
                .unwrap()
                .starts_with("socialfetch_twitter_")
    }));
    assert!(tools.iter().any(|t| t["name"] == "socialfetch_help"));
}
