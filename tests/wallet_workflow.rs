//! The ordinary deployment-driven wallet workflow never fetches API catalogs or funds pools.
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
    let text_status = wallet(&config, &["status"]);
    assert!(text_status.status.success());
    let text_status = String::from_utf8(text_status.stdout).unwrap();
    assert!(text_status.contains("Zcash wallet status"));
    assert!(text_status.contains("Shielded balances: unknown"));
    assert!(text_status.contains("Managed wallet pools: 0"));
    let before = success(wallet(&config, &["status", "--json"]));
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
    assert!(text_status.contains("Primary shielded ZEC receive address:"));
    assert!(
        text_status.contains(
            addresses["receive_addresses"][0]["encoded_address"]
                .as_str()
                .unwrap()
        )
    );
    assert_eq!(before, success(wallet(&config, &["status", "--json"])));
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
fn status_displays_receive_address_while_owned_and_keeps_metadata_without_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("deployment.toml");
    let text = "version=1\nservers={}\n[treasury]\nstate_dir='wallet'\n";
    std::fs::write(&config, text).unwrap();
    let mnemonic = dir.path().join("mnemonic");
    let mut seed_options = std::fs::OpenOptions::new();
    seed_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        seed_options.mode(0o600);
    }
    use std::io::Write;
    seed_options.open(&mnemonic).unwrap().write_all(
        b"abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    ).unwrap();
    let initialized = success(wallet(
        &config,
        &[
            "init",
            "--birthday",
            "2000000",
            "--json",
            "--mnemonic-file",
            mnemonic.to_str().unwrap(),
        ],
    ));
    let address = initialized["receive_addresses"][0]["encoded_address"]
        .as_str()
        .unwrap();
    let state = dir.path().join("wallet");
    let key = state.join("wallet.key");
    let mut owner = x402_treazury::rotation::store::Store::open(
        &state,
        &key,
        initialized["state"]["treasury_id"].as_str().unwrap(),
    )
    .unwrap();
    let before = serde_json::to_value(owner.status().unwrap()).unwrap();
    let snapshot = owner.snapshot().unwrap();
    let status = wallet(&config, &["status"]);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status = String::from_utf8(status.stdout).unwrap();
    assert!(status.contains(address), "{status}");
    assert!(!status.contains("unavailable"), "{status}");
    let addresses = success(wallet(&config, &["addresses"]));
    assert_eq!(
        addresses["receive_addresses"],
        initialized["receive_addresses"]
    );
    assert_eq!(before, success(wallet(&config, &["status", "--json"])));
    assert_eq!(
        serde_json::to_value(owner.status().unwrap()).unwrap(),
        before
    );
    assert_eq!(owner.snapshot().unwrap(), snapshot);
    assert!(!wallet(&config, &["address"]).status.success());

    // A later owner commit is visible without releasing ownership or deriving addresses.
    let revision = owner.save_snapshot(snapshot.0, &snapshot.1).unwrap();
    let shown = success(wallet(&config, &["addresses"]));
    assert_eq!(shown["state"]["snapshot_revision"], revision);
    assert_eq!(shown["receive_addresses"], initialized["receive_addresses"]);

    let wrong_key = dir.path().join("wrong.key");
    std::fs::copy(&key, &wrong_key).unwrap();
    std::fs::write(&wrong_key, [0u8; 32]).unwrap();
    for key_name in ["missing.key", "wrong.key"] {
        std::fs::write(
            &config,
            text.replace("[treasury]", &format!("[treasury]\nkey_file='{key_name}'")),
        )
        .unwrap();
        let status = wallet(&config, &["status"]);
        assert!(status.status.success());
        let status = String::from_utf8(status.stdout).unwrap();
        assert!(status.contains("Zcash wallet status"));
        assert!(status.contains("Primary shielded ZEC receive address: unavailable"));
        assert!(!status.contains(address));
        assert_eq!(
            success(wallet(&config, &["status", "--json"])),
            serde_json::to_value(owner.status().unwrap()).unwrap()
        );
    }
    assert_eq!(owner.snapshot().unwrap().0, revision);
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
    let before = success(wallet(&config, &["status", "--json"]));
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
    assert_eq!(success(wallet(&config, &["status", "--json"])), before);
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

#[test]
fn init_json_preserves_state_and_address_schema() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("deployment.toml");
    std::fs::write(
        &config,
        "version=1\nservers={}\n[treasury]\nstate_dir='wallet'\n",
    )
    .unwrap();
    let initial = success(wallet(&config, &["init", "--birthday", "1", "--json"]));
    assert_eq!(initial["state"]["birthday"], 1);
    let addresses = success(wallet(&config, &["addresses"]));
    assert_eq!(initial["receive_addresses"], addresses["receive_addresses"]);
    assert_eq!(
        initial["state"]["treasury_id"],
        success(wallet(&config, &["status", "--json"]))["treasury_id"]
    );
}
