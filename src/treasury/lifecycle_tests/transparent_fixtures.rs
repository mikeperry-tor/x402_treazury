//! Owned transparent coins discovered incidentally through a shielded receipt.
use super::*;

pub(super) async fn transparent_fixture(
    net: &MockNet,
    owner: &Treasury,
    outputs: Vec<(&str, u64, Option<&str>)>,
) -> Vec<u8> {
    let addresses = owner.addresses().await.unwrap();
    let marker = addresses
        .as_array()
        .unwrap()
        .iter()
        .find(|address| address["receiver_capabilities"]["orchard_protocol"] == true)
        .unwrap()["encoded_address"]
        .as_str()
        .unwrap();
    // Trial decryption makes the full transaction relevant without transparent
    // address discovery. The zero-value receipt preserves existing balances;
    // its real transparent outputs retain their ordinary SDK ownership/history.
    let mut receivers = outputs;
    receivers.push((marker, 0, None));
    net.funding_transaction(receivers).await
}
