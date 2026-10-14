//! The ordinary deployment-driven wallet workflow never fetches API catalogs or funds pools.
#![cfg(feature = "zcash")]
use std::{
    path::Path,
    process::{Command, Output},
};
fn wallet(config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
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
max_refund_shielding_fee_zec="0.0003"
[funding]
confidentiality="public"
[sources.never_fetch]
spec="https://unreachable.invalid/openapi.json"
"#;
    std::fs::write(&config, text).unwrap();
    let output = wallet(&config, &["init", "--birthday", "1"]);
    let logs = String::from_utf8_lossy(&output.stderr);
    for stage in [
        "using explicit birthday",
        "creating offline wallet",
        "receive address derived",
        "writing encrypted state and key",
        "initialization completed",
    ] {
        assert!(logs.contains(stage), "missing {stage}: {logs}");
    }
    assert!(!logs.contains("connecting to birthday indexer"));
    assert!(output.status.success());
    let initialized = String::from_utf8(output.stdout).unwrap();
    assert!(dir.path().join("nested/wallet/wallet.key").is_file());
    let before = success(wallet(&config, &["status"]));
    assert!(before["pools"].as_array().unwrap().is_empty());
    let addresses = success(wallet(&config, &["addresses"]));
    assert_eq!(
        initialized,
        format!(
            "Zcash wallet initialized.\n\nPrimary ZEC address:\n{}\n\nSend ZEC to this address to fund your wallet.\n",
            addresses["receive_addresses"][0]["encoded_address"]
                .as_str()
                .unwrap()
        )
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
    std::fs::write(&config, "version=1\nservers={}\n[treasury]\nstate_dir='wallet'\nmax_refund_shielding_fee_zec='0.001'\nindexer_url_env='TREAZURY_TEST_ABSENT_INDEXER'\n").unwrap();
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

#[test]
fn wallet_clap_conflicts_and_legacy_config_alias_preserve_resolution_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_x402_treazury");
    let run = |args: &[&str]| {
        Command::new(binary)
            .args(args)
            .current_dir(dir.path())
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
            .output()
            .unwrap()
    };
    for option in [
        "--network-config",
        "--state-dir",
        "--key-file",
        "--treasury-id",
    ] {
        let output = run(&[
            "wallet",
            "status",
            "--config",
            "missing.toml",
            option,
            "missing",
        ]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("cannot be used with"), "{error}");
        assert!(!error.contains("panicked"));
    }
    for args in [
        vec![
            "wallet",
            "init",
            "--state-dir",
            "state",
            "--birthday",
            "1",
            "--indexer-url-env",
            "MISSING",
        ],
        vec![
            "wallet",
            "init",
            "--state-dir",
            "state",
            "--mnemonic-file",
            "missing",
        ],
    ] {
        let output = run(&args);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("cannot be used with") || error.contains("--birthday"),
            "{error}"
        );
    }
    let config = dir.path().join("deployment.toml");
    std::fs::write(&config,"version=1\nservers={}\n[treasury]\nstate_dir='state'\nmax_refund_shielding_fee_zec='0.001'\nid='11111111-1111-4111-8111-111111111111'\nindexer_url_env='MISSING'\n").unwrap();
    for args in [
        vec!["wallet", "--config", "deployment.toml", "init"],
        vec!["wallet", "init", "--meta-config", "deployment.toml"],
    ] {
        let output = run(&args);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("init creates a new treasury identity")
        );
    }
    assert!(!dir.path().join("state").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wallet_rejections_and_saved_inspection_never_request_catalogs() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let app = axum::Router::new().route(
        "/catalog",
        axum::routing::get(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            async { axum::Json(serde_json::json!({"paths":{}})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/catalog", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("deployment.toml");
    std::fs::write(&config,format!("version=1\nservers={{}}\n[treasury]\nstate_dir='state'\nmax_refund_shielding_fee_zec='0.001'\n[sources.unused]\nspec='{endpoint}'\n")).unwrap();
    let initialized = wallet(&config, &["init", "--birthday", "1"]);
    assert!(initialized.status.success());
    let initialized = String::from_utf8(initialized.stdout).unwrap();
    let before = success(wallet(&config, &["status"]));
    let addresses = success(wallet(&config, &["addresses"]));
    assert!(
        initialized.contains(
            addresses["receive_addresses"][0]["encoded_address"]
                .as_str()
                .unwrap()
        )
    );
    assert!(
        !wallet(&config, &["init", "--birthday", "1"])
            .status
            .success()
    );
    assert_eq!(success(wallet(&config, &["status"])), before);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(before["pools"].as_array().unwrap().is_empty());
    server.abort();
}

#[test]
fn birthday_connection_failure_is_actionable_and_creates_no_state() {
    let dir = tempfile::tempdir().unwrap();
    // A bound, non-listening socket fails connection setup without a port reuse race.
    // Depending on the OS, this produces refusal or a connect timeout.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = socket.local_addr().unwrap().port();
    let output = Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .args(["wallet", "init", "--state-dir"])
        .arg(dir.path().join("wallet"))
        .env_clear()
        .env(
            "ZCASH_INDEXER_URL",
            format!("http://127.0.0.1:{port}/private-token"),
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(logs.contains("connecting to birthday indexer"), "{logs}");
    assert!(
        logs.contains("reason=connection_refused") || logs.contains("reason=connection_timeout"),
        "{logs}"
    );
    assert!(logs.contains("before wallet creation"), "{logs}");
    assert!(logs.contains("--indexer-url-env"), "{logs}");
    assert!(!logs.contains("private-token"), "{logs}");
    assert!(!logs.contains("initialization completed"), "{logs}");
    assert!(!dir.path().join("wallet").exists());
    assert!(output.stdout.is_empty());
}
