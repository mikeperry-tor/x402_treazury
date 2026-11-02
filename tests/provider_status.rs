use serde_json::{Value, json};
use std::process::Command;
use x402_treazury::{catalog, config, deployment::Deployment};
#[path = "support/log_capture.rs"]
mod log_capture;

fn fixture(dir: &std::path::Path) -> Value {
    let document = json!({"openapi":"3.0.0","servers":[{"url":"https://example.invalid"}],
        "paths":{"/data":{"get":{"tags":["data"],"responses":{"200":{"description":"ok"}}}}}});
    std::fs::write(dir.join("spec.json"), document.to_string()).unwrap();
    document
}

#[tokio::test]
async fn advisory_observations_preserve_tools_and_cli_inspection_is_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let document = fixture(dir.path());
    let path = dir.path().join("source.toml");
    let baseline = "spec='spec.json'\nprefix='fixture'\ntags=['data']\n";
    let plain = config::resolve(toml::from_str(baseline).unwrap(), &path)
        .await
        .unwrap()
        .settings;
    let annotated = format!(
        "{baseline}reliability_tags=['slow_pricing','intermittent_response_body']\nreliability_note='fixture observation'\n"
    );
    let observed = config::resolve(toml::from_str(&annotated).unwrap(), &path)
        .await
        .unwrap()
        .settings;
    assert_eq!(
        serde_json::to_value(&observed.reliability_tags).unwrap(),
        json!(["slow_pricing", "intermittent_response_body"])
    );
    assert_eq!(observed.reliability_note, "fixture observation");
    let tools = catalog::build_tools(&plain, &document, "fixture").unwrap();
    let annotated_tools = catalog::build_tools(&observed, &document, "fixture").unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(
        serde_json::to_value(&tools).unwrap(),
        serde_json::to_value(&annotated_tools).unwrap()
    );
    std::fs::write(&path, annotated).unwrap();
    let shown = Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .args(["catalog", "tools", "--provider"])
        .arg(&path)
        .env_clear()
        .envs(std::env::var_os("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let inventory: Value = serde_json::from_slice(&shown.stdout).unwrap();
    let recommended = observed.recommendation_matcher().unwrap();
    let expected: Vec<_> = tools
        .iter()
        .map(|tool| catalog::ToolInspection {
            tool,
            recommended: recommended(&tool.name),
        })
        .collect();
    assert_eq!(inventory, serde_json::to_value(expected).unwrap());
    assert!(!String::from_utf8_lossy(&shown.stderr).contains("Provider reliability"));
}

#[tokio::test]
async fn bound_sources_warn_once_before_catalog_failure() {
    if log_capture::run_in_child("bound_sources_warn_once_before_catalog_failure") {
        return;
    }
    use tracing::instrument::WithSubscriber;
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let path = dir.path().join("deployment.toml");
    let configuration = r#"
version = 1
[sources.observed]
spec = "spec.json"
reliability_tags = ["intermittent_response_body", "intermittent_response_body"]
reliability_note = "fixture reliability evidence"
[sources.unused]
spec = "spec.json"
reliability_tags = ["catalog_http_403"]
[wallets.default]
mode = "static"
private_key_env = "MISSING_TEST_KEY"
[servers.one]
listen = "127.0.0.1:18081"
bearer_token_env = "MISSING_TEST_TOKEN"
wallet = "default"
sources = ["observed"]
[servers.two]
listen = "127.0.0.1:18082"
bearer_token_env = "MISSING_TEST_TOKEN"
wallet = "default"
sources = ["observed"]
"#;
    std::fs::write(&path, configuration).unwrap();
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_env_filter("off,x402_treazury::provider_status=warn")
        .with_writer(std::sync::Mutex::new(log.reopen().unwrap()))
        .finish();
    async {
        let inspected = Deployment::load_for_inspection(&path, false).await.unwrap();
        let inventory = inspected.inventory();
        assert_eq!(inventory.len(), 2);
        assert!(
            inventory
                .iter()
                .all(|server| server.tools.len() == 1 && server.tools[0].source == "observed")
        );
        assert!(std::fs::read_to_string(log.path()).unwrap().is_empty());
        // Serving warnings precede catalog failure and follow actual endpoint bindings.
        std::fs::write(
            &path,
            configuration.replacen("spec.json", "missing.json", 1),
        )
        .unwrap();
        assert!(Deployment::load_for_unsigned_serving(&path).await.is_err());
    }
    .with_subscriber(subscriber)
    .await;
    let warnings = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(
        warnings
            .lines()
            .filter(|line| line.contains("WARN"))
            .count(),
        1,
        "{warnings}"
    );
    assert!(
        warnings.contains("intermittent_response_body"),
        "{warnings}"
    );
    assert!(!warnings.contains("catalog_http_403"), "{warnings}");
    assert!(
        warnings.contains("fixture reliability evidence"),
        "{warnings}"
    );
    assert!(warnings.contains("may still be charged"), "{warnings}");
    assert!(
        warnings.contains("Do not automatically replay paid calls"),
        "{warnings}"
    );
}

#[tokio::test]
async fn annotations_inherit_replace_clear_and_reject_unknown_tags() {
    let dir = tempfile::tempdir().unwrap();
    let provider = dir.path().join("provider.toml");
    std::fs::write(
        &provider,
        "spec = 'spec.json'\nreliability_tags = ['catalog_http_403']\nreliability_note = 'old'\n",
    )
    .unwrap();
    let path = dir.path().join("source.toml");
    let resolve = |input: &str| config::resolve(toml::from_str(input).unwrap(), &path);
    let inherited = resolve("extends = 'provider.toml'").await.unwrap();
    assert_eq!(inherited.settings.reliability_tags.len(), 1);
    assert_eq!(inherited.settings.reliability_note, "old");
    assert!(inherited.origins["reliability_tags"].ends_with("provider.toml"));
    let cleared =
        resolve("extends = 'provider.toml'\nreliability_tags = []\nreliability_note = ''")
            .await
            .unwrap();
    assert!(cleared.settings.reliability_tags.is_empty());
    assert!(cleared.settings.reliability_note.is_empty());
    let replaced = resolve("extends = 'provider.toml'\nreliability_tags = ['upstream_timeout']\nreliability_note = 'new'").await.unwrap();
    assert_eq!(replaced.settings.reliability_note, "new");
    assert_eq!(
        serde_json::to_value(replaced.settings.reliability_tags).unwrap(),
        serde_json::json!(["upstream_timeout"])
    );
    assert!(
        resolve("spec = 'spec.json'\nreliability_tags = ['unreliable_tor']")
            .await
            .is_err()
    );
    std::fs::write(&provider, "spec = 'spec.json'\nreliability_tags = ['typo']").unwrap();
    assert!(
        resolve("extends = 'provider.toml'\nreliability_tags = []")
            .await
            .is_err()
    );
}
