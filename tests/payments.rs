use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;
use x402_mcp_prototype::{
    catalog::RoutedRequest,
    payment::{PaidClient, Payer, SpendPolicy, USDC},
};

const KEY1: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const KEY2: &str = "0000000000000000000000000000000000000000000000000000000000000002";
#[derive(Clone)]
struct Gate {
    challenge: Value,
    count: Arc<AtomicUsize>,
    signed: Arc<Mutex<Vec<Value>>>,
    reject: bool,
    arrived: Option<Arc<Notify>>,
    release: Option<Arc<Notify>>,
}
async fn handle(State(gate): State<Gate>, headers: HeaderMap) -> Response {
    gate.count.fetch_add(1, Ordering::SeqCst);
    if let Some(signature) = headers
        .get("payment-signature")
        .or_else(|| headers.get("x-payment"))
    {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
        gate.signed.lock().unwrap().push(payload);
        if !gate.reject {
            return (StatusCode::OK, "paid").into_response();
        }
    } else if let Some(arrived) = &gate.arrived {
        arrived.notify_one();
        gate.release.as_ref().unwrap().notified().await;
    }
    if gate.challenge["x402Version"] == 1 {
        return (StatusCode::PAYMENT_REQUIRED, axum::Json(gate.challenge)).into_response();
    }
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&gate.challenge).unwrap()),
        )],
        "unpaid",
    )
        .into_response()
}
fn challenge(scheme: &str, amount: &str) -> Value {
    json!({"x402Version":2,"error":"insufficient_funds","resource":{"url":"http://localhost/pay","description":"x".repeat(700),"mimeType":"application/json"},
        "accepts":[{"scheme":scheme,"network":"eip155:8453","asset":USDC,"amount":amount,
        "payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,
        "extra":{"name":"USD Coin","version":"2","facilitatorAddress":"0x0000000000000000000000000000000000000004"}}]})
}
fn gate(challenge: Value) -> Gate {
    Gate {
        challenge,
        count: Arc::default(),
        signed: Arc::default(),
        reject: false,
        arrived: None,
        release: None,
    }
}
async fn start(gate: Gate) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/pay", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/pay", get(handle)).with_state(gate),
        )
        .await
        .unwrap();
    });
    (url, task)
}
fn payer(key: &str, cap: &str) -> Payer {
    Payer::new(key, SpendPolicy::dollars(cap).unwrap()).unwrap()
}
fn client(cap: &str) -> PaidClient {
    PaidClient::new(payer(KEY1, cap))
}
fn route(url: String) -> RoutedRequest {
    RoutedRequest {
        method: "GET".into(),
        url,
        query: BTreeMap::new(),
        body: None,
    }
}

#[tokio::test]
async fn exact_signature_recovers_payer_and_preserves_requirements() {
    use alloy_primitives::{Address, B256, Signature, U256};
    use alloy_sol_types::{SolStruct, eip712_domain, sol};
    sol! { struct TransferWithAuthorization { address from; address to; uint256 value; uint256 validAfter; uint256 validBefore; bytes32 nonce; } }
    let gate = gate(challenge("exact", "14000"));
    let (url, task) = start(gate.clone()).await;
    assert_eq!(client("1").execute(route(url)).await.unwrap(), "paid");
    assert_eq!(gate.count.load(Ordering::SeqCst), 2);
    let signed = gate.signed.lock().unwrap();
    let p = &signed[0];
    let a = &p["payload"]["authorization"];
    assert_eq!(p["accepted"], gate.challenge["accepts"][0]);
    assert_eq!(p["resource"]["description"].as_str().unwrap().len(), 500);
    let uint = |field: &str| U256::from_str_radix(a[field].as_str().unwrap(), 10).unwrap();
    let auth = TransferWithAuthorization {
        from: a["from"].as_str().unwrap().parse::<Address>().unwrap(),
        to: a["to"].as_str().unwrap().parse().unwrap(),
        value: uint("value"),
        validAfter: uint("validAfter"),
        validBefore: uint("validBefore"),
        nonce: a["nonce"].as_str().unwrap().parse::<B256>().unwrap(),
    };
    let domain = eip712_domain! { name:"USD Coin", version:"2", chain_id:8453, verifying_contract:USDC.parse::<Address>().unwrap(), };
    let signature: Signature = p["payload"]["signature"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        signature
            .recover_address_from_prehash(&auth.eip712_signing_hash(&domain))
            .unwrap(),
        auth.from
    );
    assert_eq!(auth.from.to_string(), payer(KEY1, "1").address);
    task.abort();
}

#[tokio::test]
async fn upto_authorizes_maximum_and_facilitator() {
    let gate = gate(challenge("upto", "250000"));
    let (url, task) = start(gate.clone()).await;
    assert_eq!(client("1").execute(route(url)).await.unwrap(), "paid");
    let signed = gate.signed.lock().unwrap();
    let a = &signed[0]["payload"]["permit2Authorization"];
    assert_eq!(a["permitted"]["amount"], "250000");
    assert_eq!(
        a["witness"]["facilitator"],
        "0x0000000000000000000000000000000000000004"
    );
    assert_eq!(
        a["permitted"]["token"].as_str().unwrap().to_lowercase(),
        USDC.to_lowercase()
    );
    assert!(signed[0]["payload"]["signature"].as_str().unwrap().len() > 100);
    task.abort();
}

#[tokio::test]
async fn cap_asset_and_network_reject_before_paid_retry() {
    for kind in ["cap", "asset", "network"] {
        let mut c = challenge("exact", "1000001");
        if kind == "asset" {
            c["accepts"][0]["asset"] = json!("0x0000000000000000000000000000000000000005");
        }
        if kind == "network" {
            c["accepts"][0]["network"] = json!("eip155:1");
        }
        let gate = gate(c);
        let (url, task) = start(gate.clone()).await;
        assert!(
            client(if kind == "cap" { "1" } else { "none" })
                .execute(route(url))
                .await
                .is_err()
        );
        assert_eq!(gate.count.load(Ordering::SeqCst), 1);
        assert!(gate.signed.lock().unwrap().is_empty());
        task.abort();
    }
}

#[tokio::test]
async fn final_rejection_is_reported_without_third_attempt() {
    let mut gate = gate(challenge("exact", "1"));
    gate.reject = true;
    let (url, task) = start(gate.clone()).await;
    let error = client("1")
        .execute(route(url))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("insufficient_funds"), "{error}");
    assert_eq!(gate.count.load(Ordering::SeqCst), 2);
    task.abort();
}

#[tokio::test]
async fn wallet_switch_keeps_inflight_signer() {
    let arrived = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut gate = gate(challenge("exact", "1"));
    gate.arrived = Some(arrived.clone());
    gate.release = Some(release.clone());
    let (url, task) = start(gate.clone()).await;
    let client = client("1");
    let pending = tokio::spawn({
        let client = client.clone();
        let url = url.clone();
        async move { client.execute(route(url)).await }
    });
    arrived.notified().await;
    client.replace_payer(payer(KEY2, "1"));
    release.notify_one();
    pending.await.unwrap().unwrap();
    release.notify_one();
    client.execute(route(url)).await.unwrap();
    let signed = gate.signed.lock().unwrap();
    assert_eq!(
        signed[0]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_lowercase(),
        payer(KEY1, "1").address.to_lowercase()
    );
    assert_eq!(
        signed[1]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_lowercase(),
        payer(KEY2, "1").address.to_lowercase()
    );
    task.abort();
}

#[test]
fn caps_use_exact_decimal_arithmetic() {
    assert_eq!(
        SpendPolicy::dollars("0.000001")
            .unwrap()
            .max_atomic
            .unwrap(),
        alloy_primitives::U256::from(1)
    );
    for bad in ["NaN", "-1", "1e3", "0.0000001"] {
        assert!(SpendPolicy::dollars(bad).is_err());
    }
}

#[tokio::test]
async fn legacy_v1_uses_x_payment_header() {
    let c = json!({"x402Version":1,"accepts":[{"scheme":"exact","network":"base","asset":USDC,
        "maxAmountRequired":"14000","resource":"http://localhost/pay","description":"legacy",
        "mimeType":"application/json","payTo":"0x0000000000000000000000000000000000000003",
        "maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
    let gate = gate(c);
    let (url, task) = start(gate.clone()).await;
    assert_eq!(client("1").execute(route(url)).await.unwrap(), "paid");
    assert_eq!(gate.count.load(Ordering::SeqCst), 2);
    assert_eq!(gate.signed.lock().unwrap()[0]["x402Version"], 1);
    task.abort();
}

#[tokio::test]
async fn source_transports_share_payer_replacement_but_profiles_are_isolated() {
    let gate = gate(challenge("exact", "1"));
    let (url, task) = start(gate.clone()).await;
    let wallet = client("1");
    let source = wallet.clone();
    let independent = client("1");
    wallet.replace_payer(payer(KEY2, "1"));
    source.execute(route(url.clone())).await.unwrap();
    independent.execute(route(url)).await.unwrap();
    let signed = gate.signed.lock().unwrap();
    assert_eq!(
        signed[0]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_lowercase(),
        payer(KEY2, "1").address.to_lowercase()
    );
    assert_eq!(
        signed[1]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_lowercase(),
        payer(KEY1, "1").address.to_lowercase()
    );
    task.abort();
}
