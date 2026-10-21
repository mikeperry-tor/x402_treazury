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
    let before_removed_command = std::fs::read(state.join("state.sqlite")).unwrap();
    let output = run(vec![
        "wallet",
        "pool",
        "--state-dir",
        state.to_str().unwrap(),
        "--key-file",
        key.to_str().unwrap(),
        "--treasury-id",
        id,
        "--name",
        "research",
        "--funding-amount-usdc",
        "5",
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("wallet pool was removed"), "{error}");
    assert!(error.contains("wallet bootstrap --config FILE"), "{error}");
    assert!(!error.contains(SEED));
    assert_eq!(
        std::fs::read(state.join("state.sqlite")).unwrap(),
        before_removed_command
    );
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

#[tokio::test]
async fn v6_sdk_snapshot_retains_addresses_and_scan_targets_through_v7_save() {
    use pepper_sync::wallet::traits::SyncWallet;
    use zingolib::{
        config::{ClientConfig, WalletConfig},
        lightclient::LightClient,
        wallet::keys::unified::ReceiverSelection,
    };
    let dir = tempfile::tempdir().unwrap();
    let config = ClientConfig::builder()
        .set_wallet_dir(dir.path().to_owned())
        .set_wallet_config(WalletConfig::Read)
        .build()
        .unwrap();
    // Public BIP39 test mnemonic only; generated with the pinned v6 SDK before
    // importing v7. Includes two extra shielded addresses and a pending scan target.
    let old = include_bytes!("fixtures/state/sdk_v6_wallet.bin");
    assert_eq!(u64::from_le_bytes(old[..8].try_into().unwrap()), 42);
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/state/sdk_v6_addresses.json")).unwrap();
    let client = LightClient::from_reader(old.as_slice(), config.clone())
        .await
        .unwrap();
    assert!(client.indexer_uri().is_none());
    assert_eq!(client.mnemonic_phrase().as_deref(), Some(SEED));
    let addresses = |value: String| {
        serde_json::from_str::<serde_json::Value>(&value)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                serde_json::json!({
                    "account": a["account"], "address_index": a["address_index"],
                    "encoded_address": a["encoded_address"],
                })
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        serde_json::json!(addresses(client.unified_addresses_json().await.to_string())),
        expected["before"]
    );
    let (bytes, expected_scan_state) = {
        let wallet = client.wallet();
        let mut wallet = wallet.write().await;
        let state = wallet.get_sync_state_mut().unwrap();
        let mut before = Vec::new();
        state.write(&mut before).unwrap();
        // Reinserting the legacy target must be idempotent: reading the old
        // nested ScanTarget format cannot silently lose it.
        pepper_sync::add_scan_targets(
            state,
            &[pepper_sync::wallet::ScanTarget::new(
                zcash_protocol::consensus::BlockHeight::from_u32(2_000_010),
                Some(zcash_protocol::TxId::from_bytes([7; 32])),
            )],
        );
        let mut after = Vec::new();
        state.write(&mut after).unwrap();
        assert_eq!(before, after);
        wallet.mark_dirty();
        (wallet.save().unwrap().unwrap(), after)
    };
    assert_eq!(u64::from_le_bytes(bytes[..8].try_into().unwrap()), 45);
    let mut reopened = LightClient::from_reader(bytes.as_slice(), config)
        .await
        .unwrap();
    assert!(reopened.indexer_uri().is_none());
    {
        let wallet = reopened.wallet();
        let mut wallet = wallet.write().await;
        let mut scan_state = Vec::new();
        wallet
            .get_sync_state_mut()
            .unwrap()
            .write(&mut scan_state)
            .unwrap();
        assert_eq!(scan_state, expected_scan_state);
    }
    assert_eq!(
        serde_json::json!(addresses(
            reopened.unified_addresses_json().await.to_string()
        )),
        expected["before"]
    );
    reopened
        .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
        .await
        .unwrap();
    assert_eq!(
        serde_json::json!(addresses(
            reopened.unified_addresses_json().await.to_string()
        )),
        expected["after_next"]
    );
}
