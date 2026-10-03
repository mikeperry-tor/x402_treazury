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
use x402_treazure::{
    catalog::RoutedRequest,
    payment::{PaidClient, Payer, SpendPolicy, USDC},
};

#[path = "support/signatures.rs"]
mod signatures;

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
            Router::new()
                .route("/pay", get(handle).post(handle))
                .with_state(gate),
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
    // Independent typed data: do not reuse the SDK's Solidity types or hash helpers.
    use alloy_primitives::{Address, Signature, U256};
    use alloy_sol_types::{SolStruct, eip712_domain, sol};
    sol! {
        struct TokenPermissions { address token; uint256 amount; }
        struct Witness { address to; address facilitator; uint256 validAfter; }
        struct PermitWitnessTransferFrom { TokenPermissions permitted; address spender; uint256 nonce; uint256 deadline; Witness witness; }
    }
    let uint = |v: &Value| {
        U256::from_str_radix(
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
                .as_str(),
            10,
        )
        .unwrap()
    };
    let auth = PermitWitnessTransferFrom {
        permitted: TokenPermissions {
            token: USDC.parse().unwrap(),
            amount: U256::from(250000),
        },
        spender: "0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002"
            .parse()
            .unwrap(),
        nonce: uint(&a["nonce"]),
        deadline: uint(&a["deadline"]),
        witness: Witness {
            to: "0x0000000000000000000000000000000000000003"
                .parse()
                .unwrap(),
            facilitator: "0x0000000000000000000000000000000000000004"
                .parse()
                .unwrap(),
            validAfter: uint(&a["witness"]["validAfter"]),
        },
    };
    assert_eq!(
        a["spender"].as_str().unwrap().parse::<Address>().unwrap(),
        auth.spender
    );
    assert_eq!(
        a["witness"]["to"]
            .as_str()
            .unwrap()
            .parse::<Address>()
            .unwrap(),
        auth.witness.to
    );
    let now = x402_treazure::rotation::base::now().unwrap();
    assert!(auth.witness.validAfter <= U256::from(now));
    assert!(auth.deadline >= U256::from(now) && auth.deadline <= U256::from(now + 60));
    let domain = eip712_domain! {name:"Permit2", chain_id:8453, verifying_contract:"0x000000000022D473030F116dDEE9F6B43aC78BA3".parse::<Address>().unwrap(),};
    let signature: Signature = signed[0]["payload"]["signature"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let recovered = signature
        .recover_address_from_prehash(&auth.eip712_signing_hash(&domain))
        .unwrap();
    assert_eq!(recovered.to_string(), payer(KEY1, "1").address);
    assert_eq!(a["from"], recovered.to_string());
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
    let signed = gate.signed.lock().unwrap();
    assert_eq!(signed[0]["x402Version"], 1);
    assert_eq!(
        signatures::recover_exact(&signed[0]).to_string(),
        payer(KEY1, "1").address
    );
    assert_eq!(signed[0]["payload"]["authorization"]["value"], "14000");
    assert_eq!(
        signed[0]["payload"]["authorization"]["to"],
        "0x0000000000000000000000000000000000000003"
    );
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

#[tokio::test]
async fn mixed_offers_caps_and_invalid_requirements_never_bypass_policy() {
    for reversed in [false, true] {
        let mut c = challenge("exact", "1000000");
        let good = c["accepts"][0].clone();
        let mut wrong_asset = good.clone();
        wrong_asset["asset"] = json!("0x0000000000000000000000000000000000000005");
        let mut wrong_chain = good.clone();
        wrong_chain["network"] = json!("eip155:1");
        let mut offers = vec![wrong_asset, wrong_chain, good.clone()];
        if reversed {
            offers.reverse();
        }
        c["accepts"] = json!(offers);
        let gate = gate(c);
        let (url, task) = start(gate.clone()).await;
        client("1").execute(route(url)).await.unwrap();
        assert_eq!(gate.count.load(Ordering::SeqCst), 2);
        assert_eq!(gate.signed.lock().unwrap()[0]["accepted"], good);
        task.abort();
    }
    for (field, value) in [
        ("amount", json!("1000001")),
        ("amount", json!("-1")),
        ("amount", json!("1.5")),
        ("amount", json!("garbage")),
        ("amount", json!("9".repeat(80))),
        ("payTo", json!("invalid")),
        ("asset", json!("invalid")),
        ("scheme", json!("unknown")),
    ] {
        let mut c = challenge("exact", "1");
        c["accepts"][0][field] = value;
        let gate = gate(c);
        let (url, task) = start(gate.clone()).await;
        assert!(client("1").execute(route(url)).await.is_err(), "{field}");
        assert_eq!(gate.count.load(Ordering::SeqCst), 1);
        assert!(gate.signed.lock().unwrap().is_empty());
        task.abort();
    }
    for alias in ["none", "off", "", "NONE", "OFF"] {
        assert!(SpendPolicy::dollars(alias).unwrap().max_atomic.is_none());
        for allowed in [true, false] {
            let mut c = challenge("exact", "1000001");
            if !allowed {
                c["accepts"][0]["asset"] = json!("0x0000000000000000000000000000000000000005");
            }
            let gate = gate(c);
            let (url, task) = start(gate.clone()).await;
            assert_eq!(client(alias).execute(route(url)).await.is_ok(), allowed);
            assert_eq!(gate.signed.lock().unwrap().len(), usize::from(allowed));
            task.abort();
        }
    }
}

#[tokio::test]
async fn malformed_headers_and_versions_send_no_paid_retry() {
    let mut version = challenge("exact", "1");
    version["x402Version"] = json!(3);
    let mut empty = challenge("exact", "1");
    empty["accepts"] = json!([]);
    for header in [
        None,
        Some("%%%".into()),
        Some(STANDARD.encode(b"not json")),
        Some(STANDARD.encode(b"null")),
        Some(STANDARD.encode(version.to_string())),
        Some(STANDARD.encode(empty.to_string())),
    ] {
        let hits = Arc::new(AtomicUsize::new(0));
        let calls = hits.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/pay", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/pay",
                    get(move |headers: HeaderMap| {
                        let (header, calls) = (header.clone(), calls.clone());
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            assert!(
                                !headers.contains_key("payment-signature")
                                    && !headers.contains_key("x-payment")
                            );
                            let mut response = StatusCode::PAYMENT_REQUIRED.into_response();
                            if let Some(value) = header {
                                response
                                    .headers_mut()
                                    .insert("payment-required", value.parse().unwrap());
                            }
                            response
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        assert!(client("1").execute(route(url)).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn unicode_challenge_description_boundary_preserves_other_fields() {
    for length in [499, 500, 501] {
        let mut c = challenge("exact", "1");
        c["resource"]["description"] = json!("雪".repeat(length));
        let gate = gate(c);
        let (url, task) = start(gate.clone()).await;
        client("1").execute(route(url)).await.unwrap();
        let signed = gate.signed.lock().unwrap();
        let mut expected = gate.challenge["resource"].clone();
        expected["description"] = json!("雪".repeat(length.min(500)));
        assert_eq!(signed[0]["resource"], expected);
        assert_eq!(signed[0]["accepted"], gate.challenge["accepts"][0]);
        assert_eq!(
            signatures::recover_exact(&signed[0]).to_string(),
            payer(KEY1, "1").address
        );
        task.abort();
    }
}

#[tokio::test]
async fn signed_post_failures_preserve_request_bytes_and_never_replay() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for outcome in [
        "400 Bad Request",
        "500 Internal Server Error",
        "402 Payment Required",
        "disconnect",
        "timeout",
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/pay?fixed=yes", listener.local_addr().unwrap());
        let captured = Arc::new(Mutex::new(vec![]));
        let logs = captured.clone();
        let server = tokio::spawn(async move {
            for index in 0..2 {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(3), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut bytes = vec![];
                let (header_end, content_length) = loop {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_owned)
                            })
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < header_end + content_length {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let header = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                assert!(header.starts_with("POST /pay?fixed=yes&"));
                assert!(
                    header
                        .to_lowercase()
                        .contains("content-type: application/json")
                );
                let signature = header
                    .lines()
                    .find_map(|line| line.strip_prefix("payment-signature: "));
                assert_eq!(signature.is_some(), index == 1);
                if let Some(signature) = signature {
                    let payload: Value =
                        serde_json::from_slice(&STANDARD.decode(signature).unwrap()).unwrap();
                    assert_eq!(
                        signatures::recover_exact(&payload).to_string(),
                        payer(KEY1, "1").address
                    );
                }
                logs.lock().unwrap().push((
                    header.lines().next().unwrap().to_owned(),
                    bytes[header_end..].to_vec(),
                ));
                if index == 0 {
                    let encoded = STANDARD.encode(challenge("exact", "1").to_string());
                    socket.write_all(format!("HTTP/1.1 402 Payment Required\r\nPayment-Required: {encoded}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                } else if outcome == "timeout" {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                } else if outcome != "disconnect" {
                    socket.write_all(format!("HTTP/1.1 {outcome}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                }
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(150), listener.accept())
                    .await
                    .is_err(),
                "unexpected replay after signed {outcome}"
            );
        });
        let body = json!({"text":"雪","nested":{"enabled":true},"count":3});
        let request = RoutedRequest {
            method: "POST".into(),
            url,
            query: BTreeMap::from([
                ("q".into(), json!(["a b", "雪"])),
                ("flag".into(), json!(true)),
            ]),
            body: Some(body.clone()),
        };
        assert!(
            client("1")
                .with_timeout(Duration::from_millis(150))
                .execute(request)
                .await
                .is_err()
        );
        server.await.unwrap();
        let logs = captured.lock().unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0], logs[1]);
        assert_eq!(serde_json::from_slice::<Value>(&logs[1].1).unwrap(), body);
        assert!(logs[0].0.contains("q=a+b&q=%E9%9B%AA"));
    }
}

#[tokio::test]
async fn agentutility_captured_v2_challenge_signs_base_with_static_payer() {
    let challenge: Value =
        serde_json::from_str(include_str!("fixtures/agentutility_payment_required.json")).unwrap();
    assert!(challenge["extensions"].get("bazaar").is_some());
    assert!(challenge["extensions"].get("builder-code").is_some());
    let fixture = gate(challenge);
    let (url, task) = start(fixture.clone()).await;
    let client = PaidClient::new(payer(KEY1, "1"));
    assert_eq!(
        client
            .execute(RoutedRequest {
                method: "POST".into(),
                url,
                query: Default::default(),
                body: None
            })
            .await
            .unwrap(),
        "paid"
    );
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
    let signed = fixture.signed.lock().unwrap();
    assert_eq!(signed[0]["accepted"]["network"], "eip155:8453");
    assert_eq!(signed[0]["accepted"]["amount"], "6000");
    drop(signed);
    task.abort();
}
