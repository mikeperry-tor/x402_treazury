use serde_json::{Value, json};
use std::path::Path;
use x402_treazure::{config, deployment::Deployment};

#[tokio::test]
async fn composition_replaces_fields_and_resolves_paths_at_their_declaration() {
    let dir = tempfile::tempdir().unwrap();
    let provider = dir.path().join("provider.toml");
    std::fs::write(
        &provider,
        r#"
spec = "catalog.json"
prefix = "stable"
tags = ["A", "B"]
timeout = 90
max_response_bytes = 100
max_help_bytes = 50
max_spec_bytes = 200
[overrides.stable_old]
description = "Old"
"#,
    )
    .unwrap();
    let deployment = dir.path().join("deploy/server.toml");
    std::fs::create_dir(deployment.parent().unwrap()).unwrap();
    let source = toml::from_str(
        r#"extends = "../provider.toml"
tags = []
timeout = 42
max_help_bytes = 80
[overrides.stable_new]
description = "New"
"#,
    )
    .unwrap();
    let resolved = config::resolve(source, &deployment).await.unwrap();
    assert!(resolved.settings.tags.is_empty());
    assert_eq!(resolved.settings.timeout, 42.0);
    assert_eq!(resolved.settings.max_response_bytes, 100);
    assert_eq!(resolved.settings.max_help_bytes, 80);
    assert_eq!(resolved.settings.max_spec_bytes, 200);
    assert!(resolved.origins["max_response_bytes"].ends_with("provider.toml"));
    assert_eq!(
        resolved.origins["max_help_bytes"],
        deployment.display().to_string()
    );
    assert_eq!(resolved.settings.prefix.as_deref(), Some("stable"));
    assert!(Path::new(&resolved.settings.spec).ends_with("catalog.json"));
    assert_eq!(
        Path::new(&resolved.settings.spec)
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap(),
        dir.path().canonicalize().unwrap()
    );
    assert_eq!(
        resolved.settings.overrides,
        json!({"stable_new":{"description":"New"}})
    );
    assert_eq!(
        resolved.origins["timeout"],
        deployment.display().to_string()
    );
    assert!(resolved.origins["spec"].ends_with("provider.toml"));
    let resolved = config::resolve(
        toml::from_str("extends = \"../provider.toml\"\nspec = \"local.json\"").unwrap(),
        &deployment,
    )
    .await
    .unwrap();
    assert_eq!(
        resolved.settings.spec,
        deployment
            .parent()
            .unwrap()
            .join("local.json")
            .display()
            .to_string()
    );
}

#[tokio::test]
async fn composition_rejects_json_unknown_fields_nested_extends_and_invalid_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.toml");
    for invalid in [
        r#"{"spec":"api.json"}"#,
        "spec = 'api.json'\nunknown = true",
        "spec = 'api.json'\ntimeout = 0",
        "extends = ['a.toml']",
        "spec = 'api.json'\nmax_response_bytes = 0",
        "spec = 'api.json'\nmax_help_bytes = 0",
        "spec = 'api.json'\nmax_spec_bytes = 0",
        "spec = 'api.json'\nmax_response_bytes = -1",
        "config = 'old.json'",
        "spec = 'api.json'\nprobe_methods = ['POST']",
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(config::load(&path).await.is_err(), "{invalid}");
    }
    std::fs::write(
        dir.path().join("provider.toml"),
        "extends = 'source.toml'\nspec = 'api.json'",
    )
    .unwrap();
    std::fs::write(&path, "extends = 'provider.toml'").unwrap();
    assert!(format!("{:#}", config::load(&path).await.err().unwrap()).contains("nested extends"));
}

#[tokio::test]
async fn bundled_provider_settings_match_reviewed_snapshots() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected: std::collections::BTreeMap<String, Value> =
        serde_json::from_str(include_str!("fixtures/catalogs/settings.json")).unwrap();
    assert_eq!(expected.len(), 19);
    for (provider, expected) in expected {
        let resolved = config::load(&repo.join(&provider)).await.unwrap();
        let mut actual = serde_json::to_value(&resolved.settings).unwrap();
        let spec = Path::new(&resolved.settings.spec);
        if spec.is_absolute() {
            assert!(spec.exists(), "{provider}");
            actual["spec"] = json!(spec.strip_prefix(repo).unwrap().to_str().unwrap());
        }
        assert_eq!(actual, expected, "{provider}");
    }
}

fn deployment_text() -> &'static str {
    r#"
version = 1
[sources.shared]
spec = "api.json"
prefix = "api"
base_url = "http://127.0.0.1:1"
probe_pricing = false
[wallets.default]
mode = "static"
private_key_env = "PRIVATE_ENV_REFERENCE"
[servers.a]
listen = "127.0.0.1:0"
bearer_token_env = "AUTH_ENV_REFERENCE"
wallet = "default"
sources = ["shared"]
tags = ["A", "B"]
exclude_tags = ["B"]
[servers.b]
listen = "127.0.0.1:0"
bearer_token_env = "AUTH_ENV_REFERENCE"
wallet = "default"
sources = ["shared"]
tags = ["B"]
"#
}

#[tokio::test]
async fn listeners_select_tags_from_one_shared_inline_source() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("api.json"),
        json!({"paths":{
            "/a":{"get":{"tags":["A"]}},"/b":{"get":{"tags":["B"]}},"/untagged":{"get":{}}
        }})
        .to_string(),
    )
    .unwrap();
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, deployment_text()).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    assert_eq!(inventory[0].tools.len(), 1);
    assert_eq!(inventory[0].tools[0].tool.name, "api_a");
    assert_eq!(inventory[1].tools.len(), 1);
    assert_eq!(inventory[1].tools[0].tool.name, "api_b");
    let running = deployment
        .bind(&std::collections::BTreeMap::from([
            ("PRIVATE_ENV_REFERENCE".into(), format!("{:064x}", 1)),
            ("AUTH_ENV_REFERENCE".into(), "test-token".into()),
        ]))
        .await
        .unwrap();
    assert_eq!(running.addresses().len(), 2);
}

#[test]
fn show_config_is_offline_reports_origins_and_never_resolves_secret_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    // api.json intentionally does not exist: inspecting composition never loads specs.
    std::fs::write(&path, deployment_text()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treazure"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .env("PRIVATE_ENV_REFERENCE", "do-not-expose-this")
        .args(["--meta-config", path.to_str().unwrap(), "--show-config"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("do-not-expose-this"));
    let shown: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(shown["sources"]["shared"]["settings"]["prefix"], "api");
    assert_eq!(shown["sources"]["shared"]["origins"]["timeout"], "default");
    assert!(
        shown["sources"]["shared"]["origins"]["spec"]
            .as_str()
            .unwrap()
            .ends_with("deployment.toml")
    );
}
