use std::num::NonZeroU32;
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
    let bytes = client
        .wallet()
        .write()
        .await
        .save()
        .unwrap()
        .expect("new address marks wallet dirty");
    let mut restored = LightClient::from_bytes(bytes, cfg).await.unwrap();
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
