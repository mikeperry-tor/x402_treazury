use super::*;
use crate::payment::{Payer, SpendPolicy, USDC, V1ExactClient};
use alloy_signer_local::PrivateKeySigner;
use serde_json::Value;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use x402_chain_eip155::{V2Eip155ExactClient, V2Eip155UptoClient};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn offer(version: u8, scheme: &str, timeout: u64) -> Value {
    let mut offer = serde_json::json!({"scheme":scheme,"network":"eip155:8453", "asset":USDC,
        "amount":"1","payTo":format!("0x{:040x}", 1),"maxTimeoutSeconds":timeout,
        "extra":{"name":"USD Coin","version":"2", "facilitatorAddress":format!("0x{:040x}", 2)},
        "unknown":{"nested":["original",42]}});
    if version == 1 {
        offer["network"] = "base".into();
        offer["maxAmountRequired"] = offer["amount"].take();
        offer["resource"] = "https://example.com/pay".into();
        offer["description"] = "test".into();
        offer["mimeType"] = "application/json".into();
    }
    offer
}
fn challenge(version: u8, offers: Vec<Value>) -> Value {
    serde_json::json!({"x402Version":version,"resource":{"url":"https://example.com/pay"},"accepts":offers})
}
fn response(version: u8, challenge: &Value) -> reqwest::Response {
    let mut builder = axum::http::Response::builder().status(402);
    if version == 2 {
        builder = builder.header("payment-required", STANDARD.encode(challenge.to_string()));
    }
    reqwest::Response::from(builder.body(challenge.to_string()).unwrap())
}
fn assert_expiry(payload: &Value, scheme: &str, start: u64, timeout: u64) {
    let expiry = if scheme == "upto" {
        &payload["payload"]["permit2Authorization"]["deadline"]
    } else {
        &payload["payload"]["authorization"]["validBefore"]
    };
    let expiry: u64 = expiry.as_str().unwrap().parse().unwrap();
    let timeout = timeout.min(MAX_AUTHORITY_SECONDS);
    assert!(expiry >= start + timeout);
    assert!(expiry <= now() + timeout);
}

#[tokio::test]
async fn static_payer_bounds_all_registered_schemes_and_preserves_original_offer() {
    let key = format!("0x{}", "5b".repeat(32));
    let payer = Payer::new(&key, SpendPolicy { max_atomic: None }).unwrap();
    for (version, scheme) in [(1, "exact"), (2, "exact"), (2, "upto")] {
        for timeout in [30, 3600, u64::MAX] {
            let original = offer(version, scheme, timeout);
            let challenge = challenge(version, vec![original.clone()]);
            let start = now();
            let headers = payer
                .client
                .make_payment_headers(response(version, &challenge))
                .await
                .unwrap();
            let header = if version == 1 {
                "x-payment"
            } else {
                "payment-signature"
            };
            let payload: Value =
                serde_json::from_slice(&STANDARD.decode(headers[header].as_bytes()).unwrap())
                    .unwrap();
            assert_expiry(&payload, scheme, start, timeout);
            if scheme == "exact" {
                assert_eq!(
                    crate::test_signatures::recover_exact(&payload).to_string(),
                    payer.address
                );
            }
            assert_eq!(payload["x402Version"], version);
            if version == 2 {
                assert_eq!(payload["accepted"], original);
            }
        }
        assert!(
            payer
                .client
                .make_payment_headers(response(
                    version,
                    &challenge(version, vec![offer(version, scheme, 0)])
                ))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn static_offer_keeps_opaque_numbers_exactly() {
    let payer = Payer::new(&format!("{:064x}", 91), SpendPolicy { max_atomic: None }).unwrap();
    let original = offer(2, "exact", 3600).to_string().replace(
        "[\"original\",42]",
        "[\"original\",18446744073709551617,1e400]",
    );
    assert!(original.contains("1e400"));
    let challenge = format!(
        r#"{{"x402Version":2,"resource":{{"url":"https://example.com/pay"}},"accepts":[{original}]}}"#
    );
    let response = reqwest::Response::from(
        axum::http::Response::builder()
            .status(402)
            .header("payment-required", STANDARD.encode(challenge))
            .body(Vec::<u8>::new())
            .unwrap(),
    );
    let headers = payer.client.make_payment_headers(response).await.unwrap();
    let payload: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(
        &STANDARD
            .decode(headers["payment-signature"].as_bytes())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["accepted"].get(), original);
    let signed: Value = serde_json::from_str(payload["payload"].get()).unwrap();
    let before: u64 = signed["authorization"]["validBefore"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(before <= now() + MAX_AUTHORITY_SECONDS);
}

#[tokio::test]
async fn duplicate_offers_retain_their_own_original_metadata() {
    let signer = Arc::new(PrivateKeySigner::from_slice(&[91; 32]).unwrap());
    for version in [1, 2] {
        for scheme in if version == 1 {
            vec!["exact"]
        } else {
            vec!["exact", "upto"]
        } {
            let first = offer(version, scheme, 3600);
            let mut second = first.clone();
            second["unknown"]["nested"] = serde_json::json!(["different"]);
            second["maxTimeoutSeconds"] = 7200.into();
            let envelope = challenge(version, vec![first.clone(), second.clone()]);
            let required = if version == 1 {
                PaymentRequired::V1(serde_json::from_value(envelope).unwrap())
            } else {
                PaymentRequired::V2(serde_json::from_value(envelope).unwrap())
            };
            let candidates = if version == 1 {
                BoundedClient(V1ExactClient::new(signer.clone())).accept(&required)
            } else if scheme == "upto" {
                BoundedClient(V2Eip155UptoClient::new(signer.clone())).accept(&required)
            } else {
                BoundedClient(V2Eip155ExactClient::new(signer.clone())).accept(&required)
            };
            assert_eq!(candidates.len(), 2);
            for (candidate, original) in candidates.iter().zip([first, second]) {
                let start = now();
                let payload: Value = serde_json::from_slice(
                    &STANDARD.decode(candidate.sign().await.unwrap()).unwrap(),
                )
                .unwrap();
                assert_expiry(&payload, scheme, start, 300);
                if version == 2 {
                    assert_eq!(payload["accepted"], original);
                }
            }
        }
    }
}
