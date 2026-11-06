//! Public fixture key addresses for payer-binding assertions.
use alloy_primitives::Address;

pub fn address_for_key(key: &str) -> Address {
    let bytes = alloy_primitives::hex::decode_to_array(key).unwrap();
    let key = secp256k1::SecretKey::from_secret_bytes(bytes).unwrap();
    let public = key.public_key().serialize_uncompressed();
    let hash = alloy_primitives::keccak256(&public[1..]);
    Address::from_slice(&hash[12..])
}
