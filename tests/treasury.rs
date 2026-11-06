#[path = "support/capacity.rs"]
mod capacity_fixture;
use x402_treazury::treasury::Treasury;
use zeroize::Zeroizing;
const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
#[tokio::test]
async fn encrypted_zingolib_treasury_restores_addresses_and_pool_keys_offline() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut wallet = Treasury::create(
        state.clone(),
        key.clone(),
        2_000_000,
        Some(Zeroizing::new(SEED.into())),
    )
    .await
    .unwrap();
    let id = wallet.status().await.unwrap().treasury_id;
    let before = wallet.derive_address().await.unwrap();
    assert_eq!(before[0]["receiver_capabilities"]["orchard_protocol"], true);
    assert_eq!(
        before[0]["orchard_protocol_pools"],
        serde_json::json!(["orchard", "ironwood"])
    );
    assert!(before[0].get("has_orchard").is_none());
    crate::capacity_fixture::treasury_pair(
        &wallet,
        "research",
        "5.00",
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0,
    )
    .await
    .unwrap();
    let pool = wallet.status().await.unwrap().pools[0].id.clone();
    wallet.close().await.unwrap();
    let mut wallet = Treasury::open(state.clone(), key.clone(), id)
        .await
        .unwrap();
    assert_eq!(wallet.addresses().await.unwrap(), before);
    assert_eq!(
        crate::capacity_fixture::treasury_pair(
            &wallet,
            "research",
            "5",
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .await
        .unwrap(),
        pool
    );
    assert_ne!(wallet.derive_address().await.unwrap(), before);
    wallet.close().await.unwrap();
    for entry in std::fs::read_dir(&state).unwrap() {
        let entry = entry.unwrap();
        let bytes = std::fs::read(entry.path()).unwrap();
        assert!(!bytes.windows(SEED.len()).any(|w| w == SEED.as_bytes()));
        assert_ne!(entry.file_name(), "zingo-wallet.dat");
    }
}

#[test]
fn wallet_cli_initializes_restores_and_refuses_overwrite_without_secrets_in_output() {
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("mnemonic");
    std::fs::write(&seed, SEED).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&seed, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let run = |args: Vec<&str>| {
        std::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
            .env("RUST_BACKTRACE", "1")
            .env("RUST_LIB_BACKTRACE", "1")
            .env("ZCASH_INDEXER_URL", "deliberately-invalid-offline-endpoint")
            .args(args)
            .output()
            .unwrap()
    };
    let output = run(vec![
        "wallet",
        "init",
        "--state-dir",
        state.to_str().unwrap(),
        "--key-file",
        key.to_str().unwrap(),
        "--birthday",
        "2000000",
        "--mnemonic-file",
        seed.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains(SEED));
    assert!(text.contains("Send ZEC to this address to fund your wallet."));
    let inspected = run(vec![
        "wallet",
        "addresses",
        "--state-dir",
        state.to_str().unwrap(),
        "--key-file",
        key.to_str().unwrap(),
    ]);
    assert!(inspected.status.success());
    let initial: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert!(
        text.contains(
            initial["receive_addresses"][0]["encoded_address"]
                .as_str()
                .unwrap()
        )
    );
    let id = initial["state"]["treasury_id"].as_str().unwrap();
    // Explicit temporary-store setup retains the address/key persistence coverage
    // without reintroducing a CLI allocation path or fabricating funded readiness.
    {
        let mut store = x402_treazury::rotation::store::Store::open(&state, &key, id).unwrap();
        crate::capacity_fixture::declare_and_allocate!(
            store,
            "research",
            crate::capacity_fixture::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
    }
    let output = run(vec![
        "wallet",
        "status",
        "--json",
        "--state-dir",
        state.to_str().unwrap(),
    ]);
    assert!(output.status.success());
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["pools"][0]["addresses"].as_array().unwrap().len(), 2);
    let output = run(vec![
        "wallet",
        "init",
        "--state-dir",
        state.to_str().unwrap(),
        "--key-file",
        key.to_str().unwrap(),
        "--birthday",
        "2000000",
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("already exists"));
    assert!(error.contains("wallet addresses"));
    assert!(!error.contains("Stack backtrace") && !error.contains("stack backtrace"));
    // Exercise the public notes dispatch with the existing unfunded fixture.
    // These references are deliberately absent from run's cleared environment;
    // offline inspection must not resolve either endpoint or construct an owner.
    let config = dir.path().join("notes.toml");
    std::fs::write(&config, format!(
        "version=1\nservers={{}}\n[treasury]\nid={id:?}\nstate_dir={:?}\nkey_file={:?}\nindexer_url_env='X402_NOTES_UNSET_INDEXER'\nsubmission_url_env='X402_NOTES_UNSET_SUBMISSION'\n",
        state.to_str().unwrap(), key.to_str().unwrap(),
    )).unwrap();
    let before_key = std::fs::read(&key).unwrap();
    let key_hex = alloy_primitives::hex::encode(&before_key);
    let snapshot = || {
        let db = rusqlite::Connection::open_with_flags(
            state.join("state.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.query_row(
            "SELECT revision,bytes FROM snapshots ORDER BY revision DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .unwrap()
    };
    let before_snapshot = snapshot();
    let before_database = std::fs::read(state.join("state.sqlite")).unwrap();
    let before_wal = std::fs::read(state.join("state.sqlite-wal")).ok();
    for args in [
        vec![
            "wallet",
            "notes",
            "--state-dir",
            state.to_str().unwrap(),
            "--key-file",
            key.to_str().unwrap(),
        ],
        vec!["wallet", "notes", "--config", config.to_str().unwrap()],
    ] {
        let output = run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let notes: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(notes["format_version"], 1);
        assert_eq!(notes["state_schema_version"], 23);
        assert_eq!(notes["treasury_id"], id);
        assert_eq!(notes["snapshot_revision"], before_snapshot.0);
        assert_eq!(notes["snapshot_revision"], status["snapshot_revision"]);
        assert_eq!(notes["notes"], serde_json::json!([]));
        assert!(notes["sdk_scanned_height"].is_null());
        assert!(notes["selection_target_height"].is_null());
        assert!(notes["selection_anchor_height"].is_null());
        assert_eq!(
            notes["spendability_unavailable_reason"],
            "saved_target_or_anchor_unavailable"
        );
        assert_eq!(
            notes["spendability_limitations"],
            serde_json::json!([
                "live_process_locks_unavailable",
                "migration_reservations_may_be_selected",
            ])
        );
        for bytes in [&output.stdout, &output.stderr] {
            let rendered = String::from_utf8_lossy(bytes);
            assert!(!rendered.contains(SEED));
            assert!(!rendered.contains(&key_hex));
            assert!(!rendered.contains("Zcash wallet initialized"));
        }
        assert_eq!(snapshot(), before_snapshot);
        assert_eq!(std::fs::read(&key).unwrap(), before_key);
        assert_eq!(
            std::fs::read(state.join("state.sqlite")).unwrap(),
            before_database
        );
        assert_eq!(
            std::fs::read(state.join("state.sqlite-wal")).ok(),
            before_wal
        );
        let after = run(vec![
            "wallet",
            "status",
            "--json",
            "--state-dir",
            state.to_str().unwrap(),
        ]);
        assert!(after.status.success());
        let after: serde_json::Value = serde_json::from_slice(&after.stdout).unwrap();
        assert_eq!(after, status, "inspection changed saved financial records");
    }
    let before_status = std::fs::read(state.join("state.sqlite")).unwrap();
    for _ in 0..2 {
        let output = run(vec![
            "wallet",
            "addresses",
            "--state-dir",
            state.to_str().unwrap(),
            "--key-file",
            key.to_str().unwrap(),
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let shown: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(shown["receive_addresses"], initial["receive_addresses"]);
        assert_eq!(shown["state"]["treasury_id"], id);
        assert_eq!(
            std::fs::read(state.join("state.sqlite")).unwrap(),
            before_status
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains(SEED));
    }
    let output = run(vec![
        "wallet",
        "address",
        "--state-dir",
        state.to_str().unwrap(),
        "--key-file",
        key.to_str().unwrap(),
        "--treasury-id",
        "0",
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("UUID, not an account number"));
    assert!(error.contains("wallet status"));
    assert!(!error.contains("Stack backtrace") && !error.contains("stack backtrace"));
}
