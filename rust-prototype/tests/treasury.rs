#![cfg(feature = "zcash")]
use x402_mcp_prototype::treasury::Treasury;
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
    wallet
        .ensure_pool("research".into(), "5.00".into())
        .await
        .unwrap();
    let pool = wallet.status().await.unwrap().pools[0].id.clone();
    wallet.close().await.unwrap();
    let mut wallet = Treasury::open(state.clone(), key.clone(), id)
        .await
        .unwrap();
    assert_eq!(wallet.addresses().await.unwrap(), before);
    assert_eq!(
        wallet
            .ensure_pool("research".into(), "5".into())
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
        std::process::Command::new(env!("CARGO_BIN_EXE_x402-mcp-prototype"))
            .env_clear()
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
    let initial: serde_json::Value = serde_json::from_str(&text).unwrap();
    let id = initial["state"]["treasury_id"].as_str().unwrap();
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
        "--deposit-size",
        "5",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = run(vec![
        "wallet",
        "status",
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
}
