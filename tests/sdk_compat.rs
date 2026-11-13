use pepper_sync::wallet::traits::SyncWallet;
use std::num::NonZeroU32;
#[path = "support/sdk_merge.rs"]
mod sdk_merge;
// Compile the maintained private primitive itself: dependency unit tests are
// excluded from ordinary workspace selection, and no public SDK shim is needed.
#[path = "../vendor/pepper-sync/src/scan/task_publication.rs"]
mod task_publication;
use x402_treazury::payment::{Payer, SpendPolicy};
use zingolib::{
    config::{ClientConfig, WalletConfig},
    lightclient::LightClient,
    wallet::{WalletSettings, keys::unified::ReceiverSelection},
};

// Public BIP39 test mnemonic; only used in isolated temporary, offline wallets.
const MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
#[tokio::test]
async fn offline_wallet_restore_and_payment_signer_coexist() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ClientConfig::builder()
        .set_wallet_dir(dir.path().to_owned())
        .set_wallet_config(WalletConfig::MnemonicPhrase {
            mnemonic_phrase: MNEMONIC.into(),
            no_of_accounts: NonZeroU32::new(1).unwrap(),
            birthday: 2_000_000,
            wallet_settings: WalletSettings::default(),
        })
        .build()
        .unwrap();
    let mut client = LightClient::new(cfg.clone(), false).await.unwrap();
    assert!(client.indexer_uri().is_none());
    assert_eq!(client.mnemonic_phrase().as_deref(), Some(MNEMONIC));
    client
        .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
        .await
        .unwrap();
    let addresses = client.unified_addresses_json().await.to_string();
    assert_ne!(addresses, "[]");
    let target = pepper_sync::wallet::ScanTarget::new(
        zcash_protocol::consensus::BlockHeight::from_u32(2_000_010),
        Some(zcash_protocol::TxId::from_bytes([7; 32])),
    );
    let (bytes, expected_scan_state) = {
        let wallet = client.wallet();
        let mut wallet = wallet.write().await;
        let state = wallet.get_sync_state_mut().unwrap();
        let mut before = Vec::new();
        state.write(&mut before).unwrap();
        pepper_sync::add_scan_targets(state, &[target]);
        let mut after = Vec::new();
        state.write(&mut after).unwrap();
        assert_ne!(before, after, "the current scan target must be persisted");
        wallet.mark_dirty();
        (wallet.save().unwrap().unwrap(), after)
    };
    assert_eq!(u64::from_le_bytes(bytes[..8].try_into().unwrap()), 45);
    let mut restored = LightClient::from_bytes(bytes, cfg).await.unwrap();
    assert!(restored.indexer_uri().is_none());
    {
        let wallet = restored.wallet();
        let mut wallet = wallet.write().await;
        let state = wallet.get_sync_state_mut().unwrap();
        let mut saved = Vec::new();
        state.write(&mut saved).unwrap();
        assert_eq!(saved, expected_scan_state);
        pepper_sync::add_scan_targets(state, &[target]);
        let mut repeated = Vec::new();
        state.write(&mut repeated).unwrap();
        assert_eq!(repeated, expected_scan_state);
    }
    assert_eq!(restored.mnemonic_phrase().as_deref(), Some(MNEMONIC));
    assert_eq!(
        restored.unified_addresses_json().await.to_string(),
        addresses
    );
    let next = client
        .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
        .await
        .unwrap();
    let restored_next = restored
        .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
        .await
        .unwrap();
    assert_eq!(format!("{next:?}"), format!("{restored_next:?}"));
    let payer = Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap();
    assert_eq!(
        payer.address.to_lowercase(),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
}

#[test]
fn mainnet_nu7_schedule_branch_and_expiry_match_sdk_cohort() {
    use zcash_protocol::consensus::{
        BlockHeight, BranchId, MAIN_NETWORK, NetworkUpgrade, Parameters,
    };
    use zingolib::{config::ChainType, wallet::expiry};

    let activation = BlockHeight::from_u32(3_543_405);
    let chain = ChainType::Mainnet;
    assert_eq!(
        MAIN_NETWORK.activation_height(NetworkUpgrade::Nu7),
        Some(activation)
    );
    assert_eq!(
        chain.activation_height(NetworkUpgrade::Nu7),
        Some(activation)
    );
    for (height, branch, delta) in [
        (activation - 1, BranchId::Nu6_3, 40),
        (activation, BranchId::Nu7, 120),
        (activation + 1, BranchId::Nu7, 120),
    ] {
        assert_eq!(BranchId::for_height(&MAIN_NETWORK, height), branch);
        assert_eq!(BranchId::for_height(&chain, height), branch);
        assert_eq!(expiry::tx_expiry_delta(&chain, height), delta);
        assert_eq!(expiry::tx_expiry_height(&chain, height), height + delta);
    }
    assert_eq!(u32::from(BranchId::Nu7), 0x7719_0ad9);
}
