//! Public naming and portable config paths must survive repository layout changes.
use serde_json::Value;
use std::{path::Path, process::Command};
#[test]
fn executable_name_version_and_example_inspection_are_portable() {
    let dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_x402_treazury");
    let run = |args: &[&str]| {
        Command::new(binary)
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let version = run(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        concat!("x402_treazury ", env!("CARGO_PKG_VERSION"))
    );
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("Usage: x402_treazury")
    );
    // No implicit serving, even when inherited environment could configure a provider.
    let no_command = run(&[]);
    let help_text = format!(
        "{}{}",
        String::from_utf8_lossy(&no_command.stdout),
        String::from_utf8_lossy(&no_command.stderr)
    );
    assert!(help_text.contains("Usage: x402_treazury <COMMAND>"));
    assert!(!help_text.contains("EVM_PRIVATE_KEY required"));
    for command in [
        "serve",
        "catalog",
        "config",
        "wallet",
        "sources",
        "build-info",
    ] {
        assert!(help_text.contains(command));
    }
    for args in [
        vec!["serve", "--help"],
        vec!["catalog", "tools", "--help"],
        vec!["catalog", "warm", "--help"],
        vec!["catalog", "route", "--help"],
        vec!["config", "show", "--help"],
        vec!["wallet", "init", "--help"],
        vec!["wallet", "recover", "--help"],
        vec!["sources", "inspect", "--help"],
    ] {
        let result = run(&args);
        assert!(result.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&result.stdout).contains(&format!(
            "Usage: x402_treazury {}",
            args[..args.len() - 1].join(" ")
        )));
    }
    for args in [
        vec!["--provider", "missing.toml"],
        vec!["serve", "--list-tools"],
        vec!["catalog", "tools", "--no-auth"],
        vec!["catalog", "tags", "--discover-pricing"],
        vec![
            "config",
            "show",
            "--config",
            "missing.toml",
            "--env-file",
            "missing.env",
        ],
        vec!["catalog", "route", "api_read", "--config", "missing.toml"],
        vec!["build-info", "unexpected"],
    ] {
        let result = run(&args);
        assert!(!result.status.success(), "{args:?}");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(!error.contains("panicked"), "{error}");
        assert!(error.contains("error:"), "{error}");
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for example in [
        "managed.toml",
        "comprehensive.toml",
        "public-swap-demo.toml",
        "public-payment-demo.toml",
    ] {
        let directory = if example.starts_with("public-") {
            "tests/live/deployments"
        } else {
            "examples/deployments"
        };
        let path = root.join(directory).join(example);
        let shown = run(&["config", "show", "--config", path.to_str().unwrap()]);
        let legacy = run(&["config", "show", "--meta-config", path.to_str().unwrap()]);
        assert!(legacy.status.success());
        assert_eq!(shown.stdout, legacy.stdout);
        assert!(
            shown.status.success(),
            "{example}: {}",
            String::from_utf8_lossy(&shown.stderr)
        );
        let value: Value = serde_json::from_slice(&shown.stdout).unwrap();
        for source in value["sources"].as_object().unwrap().values() {
            let spec = source["settings"]["spec"].as_str().unwrap();
            if !spec.starts_with("http") {
                assert!(Path::new(spec).is_absolute());
                assert!(Path::new(spec).is_file(), "{example}: {spec}");
            }
        }
    }
}

#[test]
fn retired_response_character_flag_is_rejected_by_name() {
    let result = Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .args(["serve", "--max-response-chars", "1"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--max-response-chars"));
}

#[test]
fn deployment_config_environment_applies_to_each_command_and_cli_wins() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str], config: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
        command.env_clear().current_dir(dir.path()).args(args);
        if let Some(config) = config {
            command.env("X402_TREAZURY_CONFIG", config);
        }
        command.output().unwrap()
    };
    for args in [
        vec!["serve"],
        vec!["catalog", "tools"],
        vec!["catalog", "tags"],
        vec!["catalog", "warm"],
        vec!["config", "show"],
        vec!["config", "check"],
        vec!["wallet", "status"],
        vec!["sources", "inspect"],
        vec![
            "sources", "refresh", "--server", "research", "--source", "api",
        ],
        vec!["sources", "remove", "--server", "research", "--all"],
    ] {
        let inherited = run(&args, Some("missing.toml"));
        let mut explicit_args = args.clone();
        explicit_args.extend(["--config", "missing.toml"]);
        let explicit = run(&explicit_args, None);
        let overridden = run(&explicit_args, Some("wrong.toml"));
        assert!(!inherited.status.success(), "{args:?}");
        assert_eq!(inherited.status.code(), explicit.status.code(), "{args:?}");
        assert_eq!(inherited.stderr, explicit.stderr, "{args:?}");
        assert_eq!(overridden.stderr, explicit.stderr, "{args:?}");
        assert!(!String::from_utf8_lossy(&inherited.stderr).contains("required arguments"));
    }
    for args in [
        vec!["config", "show", "--provider", "provider.toml"],
        vec![
            "catalog",
            "route",
            "api_read",
            "--provider",
            "provider.toml",
        ],
        vec!["wallet", "status", "--state-dir", "state"],
    ] {
        let result = run(&args, Some("missing.toml"));
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("cannot be used"));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("examples/deployments/managed.toml");
    let shown = run(&["config", "show"], Some(path.to_str().unwrap()));
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let explicit = run(
        &["config", "show", "--config", path.to_str().unwrap()],
        Some("wrong.toml"),
    );
    assert!(explicit.status.success());
    assert_eq!(shown.stdout, explicit.stdout);
}
