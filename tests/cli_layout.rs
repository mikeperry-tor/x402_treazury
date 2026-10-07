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
        "privacy.toml",
        "servers.toml",
        "socialfetch.toml",
        "servers-auto-wallets.toml",
        "servers-managed.toml",
        "public-swap-demo.toml",
        "public-payment-demo.toml",
    ] {
        let path = root.join("examples/deployments").join(example);
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
