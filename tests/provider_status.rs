use std::process::{Command, Output};

fn run(path: &std::path::Path, meta: bool, command: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .args(command)
        .arg(if meta { "--config" } else { "--provider" })
        .arg(path)
        .env_clear()
        .output()
        .unwrap()
}

fn fixture(dir: &std::path::Path) {
    std::fs::write(
        dir.join("spec.json"),
        r#"{"openapi":"3.0.0","servers":[{"url":"https://example.invalid"}],"paths":{"/data":{"get":{"tags":["data"],"responses":{"200":{"description":"ok"}}}}}}"#,
    ).unwrap();
}

#[test]
fn standalone_warns_before_catalog_failure_and_inspection_is_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.toml");
    std::fs::write(
        &path,
        r#"
spec = "missing.json"
name = "Observed provider"
reliability_tags = ["intermittent_response_body", "intermittent_response_body"]
reliability_note = "Dated evidence: paid response lost"
"#,
    )
    .unwrap();
    let result = run(&path, false, &["serve"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let err = String::from_utf8(result.stderr).unwrap();
    assert_eq!(
        err.matches("Provider reliability observation:").count(),
        1,
        "{err}"
    );
    assert!(err.contains("may still be charged"), "{err}");
    assert!(err.contains("Dated evidence: paid response lost"), "{err}");

    let result = run(&path, false, &["config", "show"]);
    assert!(result.status.success(), "{:?}", result);
    assert!(!String::from_utf8_lossy(&result.stderr).contains("Provider reliability"));
    assert!(String::from_utf8_lossy(&result.stdout).contains("intermittent_response_body"));
}

#[test]
fn slow_pricing_warning_is_visible_before_catalog_io_and_inspection_is_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("slow-pricing.toml");
    std::fs::write(&path, "spec = 'missing.json'\nreliability_tags = ['slow_pricing', 'slow_pricing']\nreliability_note = 'Pricing discovery was slow'\n").unwrap();
    let result = run(&path, false, &["serve"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let log = String::from_utf8(result.stderr).unwrap();
    assert_eq!(log.matches("Provider reliability observation:").count(), 1);
    assert!(
        log.contains("Unsigned pricing discovery has shown long response waits"),
        "{log}"
    );
    assert!(log.contains("Pricing discovery was slow"), "{log}");
    let shown = run(&path, false, &["config", "show"]);
    assert!(shown.status.success());
    assert!(String::from_utf8_lossy(&shown.stdout).contains("slow_pricing"));
    assert!(!String::from_utf8_lossy(&shown.stderr).contains("Provider reliability"));
}

#[test]
fn metadata_preserves_tools_and_only_bound_sources_warn_once() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        r#"
version = 1
[sources.observed]
spec = "spec.json"
reliability_tags = ["upstream_timeout", "upstream_timeout"]
reliability_note = "test observation"
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
"#,
    )
    .unwrap();
    for command in [
        ["config", "check"],
        ["catalog", "tools"],
        ["catalog", "tags"],
        ["config", "show"],
    ] {
        let result = run(&path, true, &command);
        assert!(result.status.success(), "{command:?}: {:?}", result);
        assert!(!String::from_utf8_lossy(&result.stderr).contains("Provider reliability"));
    }
    let result = run(&path, true, &["serve"]);
    assert!(!result.status.success()); // No credentials: never binds a listener.
    let err = String::from_utf8(result.stderr).unwrap();
    assert_eq!(
        err.matches("Provider reliability observation:").count(),
        1,
        "{err}"
    );
    assert!(err.contains("upstream_timeout"), "{err}");
    assert!(!err.contains("catalog_http_403"), "{err}");

    let standalone = dir.path().join("standalone.toml");
    std::fs::write(&standalone, "spec = 'spec.json'\ntags = ['data']\n").unwrap();
    let baseline = run(&standalone, false, &["catalog", "tools"]);
    assert!(baseline.status.success());
    std::fs::write(
        &standalone,
        "spec = 'spec.json'\ntags = ['data']\nreliability_tags = ['upstream_rate_limited']\n",
    )
    .unwrap();
    let annotated = run(&standalone, false, &["catalog", "tools"]);
    assert!(annotated.status.success());
    assert_eq!(baseline.stdout, annotated.stdout);
    assert!(!String::from_utf8_lossy(&annotated.stderr).contains("Provider reliability"));
}

#[tokio::test]
async fn annotations_inherit_replace_clear_and_reject_unknown_tags() {
    use x402_treazury::config;
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
