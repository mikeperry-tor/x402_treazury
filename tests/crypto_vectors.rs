//! Expected outputs come from Python's independent crypto implementation,
//! rather than signing and verifying a hash produced by the same Rust code.
use alloy_primitives::{Address, B256, Keccak256, Signature, U256, keccak256};
use alloy_sol_types::{SolStruct, eip712_domain, sol};
use serde_json::Value;

#[test]
fn keccak_matches_independent_vectors_across_block_boundaries() {
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/crypto_vectors.json")).unwrap();
    for vector in vectors["keccak"].as_array().unwrap() {
        let bytes: Vec<u8> = (0..vector["length"].as_u64().unwrap())
            .map(|i| (i % 256) as u8)
            .collect();
        let expected: B256 = vector["hash"].as_str().unwrap().parse().unwrap();
        assert_eq!(keccak256(&bytes), expected);
        let mut state = Keccak256::new();
        for chunk in bytes.chunks(17) {
            state.update(chunk);
        }
        assert_eq!(state.finalize(), expected);
    }
}

#[test]
fn eip3009_hash_and_signature_match_python() {
    sol! { struct TransferWithAuthorization { address from; address to; uint256 value; uint256 validAfter; uint256 validBefore; bytes32 nonce; } }
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/crypto_vectors.json")).unwrap();
    let vector = &vectors["eip3009"];
    let msg = &vector["typed_data"]["message"];
    let auth = TransferWithAuthorization {
        from: msg["from"].as_str().unwrap().parse().unwrap(),
        to: msg["to"].as_str().unwrap().parse().unwrap(),
        value: U256::from(msg["value"].as_u64().unwrap()),
        validAfter: U256::from(msg["validAfter"].as_u64().unwrap()),
        validBefore: U256::from(msg["validBefore"].as_u64().unwrap()),
        nonce: msg["nonce"].as_str().unwrap().parse().unwrap(),
    };
    let domain = eip712_domain! { name: "USD Coin", version: "2", chain_id: 8453,
    verifying_contract: x402_treazure::payment::USDC.parse::<Address>().unwrap(), };
    let hash = auth.eip712_signing_hash(&domain);
    assert_eq!(
        hash,
        vector["hash"].as_str().unwrap().parse::<B256>().unwrap()
    );
    let signature: Signature = vector["signature"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        signature.recover_address_from_prehash(&hash).unwrap(),
        auth.from
    );
}
