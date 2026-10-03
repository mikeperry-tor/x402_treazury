//! Independent EIP-3009 typed-data reconstruction for captured fixture payments.
use alloy_primitives::{Address, B256, Signature, U256};
use alloy_sol_types::{SolStruct, eip712_domain, sol};
use serde_json::Value;
pub fn recover_exact(payload: &Value) -> Address {
    sol! { struct TransferWithAuthorization { address from; address to; uint256 value; uint256 validAfter; uint256 validBefore; bytes32 nonce; } }
    let a = &payload["payload"]["authorization"];
    let uint = |key: &str| U256::from_str_radix(a[key].as_str().unwrap(), 10).unwrap();
    let auth = TransferWithAuthorization {
        from: a["from"].as_str().unwrap().parse().unwrap(),
        to: a["to"].as_str().unwrap().parse().unwrap(),
        value: uint("value"),
        validAfter: uint("validAfter"),
        validBefore: uint("validBefore"),
        nonce: a["nonce"].as_str().unwrap().parse::<B256>().unwrap(),
    };
    let domain = eip712_domain! {name:"USD Coin",version:"2",chain_id:8453,verifying_contract:"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".parse::<Address>().unwrap(),};
    let signature: Signature = payload["payload"]["signature"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let recovered = signature
        .recover_address_from_prehash(&auth.eip712_signing_hash(&domain))
        .unwrap();
    assert_eq!(recovered, auth.from);
    recovered
}
