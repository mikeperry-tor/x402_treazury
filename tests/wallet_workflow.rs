//! The ordinary deployment-driven wallet workflow never fetches API catalogs or funds pools.
#![cfg(feature = "zcash")]
use std::{
    path::Path,
    process::{Command, Output},
};
fn wallet(config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_treazury"))
        .arg("wallet")
        .args(args)
        .arg("--config")
        .arg(config)
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .output()
        .unwrap()
}
fn success(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn one_config_initializes_inspects_and_backs_up_without_ids_keys_endpoints_or_catalogs() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("deployment.toml");
    let text = r#"version=1
servers={}
[treasury]
state_dir="nested/wallet"
daily_input_zec="0.01"
shield_max_fee_zec="0.0003"
[funding]
confidentiality="public"
[sources.never_fetch]
spec="https://unreachable.invalid/openapi.json"
"#;
    std::fs::write(&config, text).unwrap();
    let initialized = success(wallet(&config, &["init", "--birthday", "1"]));
    assert!(dir.path().join("nested/wallet/wallet.key").is_file());
    let before = success(wallet(&config, &["status"]));
    assert!(before["pools"].as_array().unwrap().is_empty());
    let addresses = success(wallet(&config, &["addresses"]));
    assert_eq!(
        initialized["receive_addresses"],
        addresses["receive_addresses"]
    );
    assert_eq!(before, success(wallet(&config, &["status"])));
    assert!(
        !wallet(&config, &["init", "--birthday", "1"])
            .status
            .success()
    );
    let backup = dir.path().join("backup");
    success(wallet(
        &config,
        &["backup", "--destination", backup.to_str().unwrap()],
    ));
    assert!(backup.join("backup.json").is_file());
    let wrong = text.replace(
        "[treasury]",
        "[treasury]\nid='11111111-1111-4111-8111-111111111111'",
    );
    std::fs::write(&config, wrong).unwrap();
    let rejected = wallet(&config, &["addresses"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("state identity mismatch"));
}
#[test]
fn conflicting_wallet_inputs_and_missing_explicit_endpoint_fail_before_creation() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("deployment.toml");
    std::fs::write(&config, "version=1\nservers={}\n[treasury]\nstate_dir='wallet'\ndaily_input_zec='0.01'\nshield_max_fee_zec='0.001'\nindexer_url_env='TREAZURY_TEST_ABSENT_INDEXER'\n").unwrap();
    let failed = wallet(&config, &["init"]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("missing or empty"));
    assert!(!dir.path().join("wallet").exists());
    assert!(
        !wallet(
            &config,
            &["init", "--birthday", "1", "--state-dir", "elsewhere"]
        )
        .status
        .success()
    );
    assert!(
        !wallet(&config, &["init", "--mnemonic-file", "missing"])
            .status
            .success()
    );
    assert!(!dir.path().join("wallet").exists());
}
