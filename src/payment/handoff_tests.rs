//! Managed payer handoff through actual challenge and signing middleware.
//! Local middleware follows fixtures/x402/handoff-contract.json; no live settlement claim.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    config::ReservesConfig,
    manager::ManagedPool,
    store::{AllocationKind, Store, StoreHandle},
};
use crate::test_signatures as signatures;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{any, post},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy)]
enum Second {
    Price(&'static str),
    Free,
    Malformed,
    Unsupported,
    Redirect,
}
#[derive(Clone, Copy)]
enum Signed {
    Paid,
    SettlementFailure,
    InvalidImage,
    Redirect,
}
#[derive(Clone, Debug)]
struct Seen {
    peer: SocketAddr,
    method: Method,
    uri: String,
    content_type: Option<String>,
    body: Vec<u8>,
    signed: bool,
}
#[derive(Clone)]
struct Fixture {
    balances: Arc<BTreeMap<String, u64>>,
    requests: Arc<Mutex<Vec<Seen>>>,
    handlers: Arc<AtomicUsize>,
    redirects: Arc<AtomicUsize>,
    unsigned: Arc<AtomicUsize>,
    timestamp: u64,
    first: Option<Value>,
    second: Second,
    signed: Signed,
    expected_signer: String,
    db: std::path::PathBuf,
}
fn challenge(amount: &str) -> Value {
    json!({"x402Version":2,"resource":{"url":"http://localhost/protected"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":amount,"payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]})
}
fn required(challenge: Value) -> Response {
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
        "unpaid",
    )
        .into_response()
}
async fn rpc(State(f): State<Fixture>, Json(v): Json<Value>) -> Json<Value> {
    let result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = v["params"][0].as_str().unwrap();
            let height = if tag == "latest" {
                100
            } else {
                u64::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap()
            };
            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",f.timestamp)})
        }
        "eth_call" => {
            assert!(v["params"][1] == "latest" || v["params"][1]["requireCanonical"] == true);
            let data = v["params"][0]["data"].as_str().unwrap();
            let amount = if data.starts_with("0x70a08231") {
                *f.balances
                    .get(&format!("0x{}", &data[data.len() - 40..]).to_lowercase())
                    .unwrap()
            } else {
                0
            };
            json!(format!("0x{amount:064x}"))
        }
        other => panic!("unexpected RPC {other}"),
    };
    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
}
async fn protected(
    State(f): State<Fixture>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let signed = headers.get("payment-signature");
    f.requests.lock().unwrap().push(Seen {
        peer,
        method,
        uri: uri.to_string(),
        content_type: headers
            .get("content-type")
            .map(|h| h.to_str().unwrap().into()),
        body: body.to_vec(),
        signed: signed.is_some(),
    });
    if let Some(signed) = signed {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(signed.as_bytes()).unwrap()).unwrap();
        let recovered = signatures::recover_exact(&payload);
        assert_eq!(
            recovered.to_string().to_lowercase(),
            f.expected_signer.to_lowercase()
        );
        let auth = &payload["payload"]["authorization"];
        assert_eq!(
            auth["from"].as_str().unwrap().to_lowercase(),
            f.expected_signer.to_lowercase()
        );
        let db = rusqlite::Connection::open(&f.db).unwrap();
        let count:i64=db.query_row("SELECT COUNT(*) FROM payment_attempts WHERE nonce=?1 AND state='POSSIBLY_SUBMITTED'",[auth["nonce"].as_str().unwrap()],|r|r.get(0)).unwrap();
        assert_eq!(count, 1, "signed bytes precede journal acknowledgement");
        // The pinned middleware can run this handler before settlement fails.
        f.handlers.fetch_add(1, Ordering::SeqCst);
        return match f.signed {
            Signed::Paid => "paid".into_response(),
            Signed::SettlementFailure => required(challenge("3000000")),
            Signed::InvalidImage => ([("content-type", "image/png")], "not a PNG").into_response(),
            Signed::Redirect => (
                StatusCode::TEMPORARY_REDIRECT,
                [("location", "/redirect-target")],
            )
                .into_response(),
        };
    }
    let ordinal = f.unsigned.fetch_add(1, Ordering::SeqCst);
    if ordinal == 0 {
        return required(f.first.clone().unwrap_or_else(|| challenge("3000000")));
    }
    assert_eq!(ordinal, 1, "a third unsigned request is forbidden");
    match f.second {
        Second::Price(amount) => required(challenge(amount)),
        Second::Free => {
            f.handlers.fetch_add(1, Ordering::SeqCst);
            "free".into_response()
        }
        Second::Malformed => required(json!({"x402Version":2,"accepts":"invalid"})),
        Second::Unsupported => {
            let mut c = challenge("3000000");
            c["accepts"][0]["network"] = json!("eip155:1");
            required(c)
        }
        Second::Redirect => (
            StatusCode::TEMPORARY_REDIRECT,
            [("location", "/redirect-target")],
        )
            .into_response(),
    }
}
async fn run(method: &str, second: Second, signed: Signed) -> (bool, Vec<Seen>, usize, usize) {
    run_case(method, None, second, signed).await
}
async fn run_case(
    method: &str,
    first: Option<Value>,
    second: Second,
    signed: Signed,
) -> (bool, Vec<Seen>, usize, usize) {
    run_case_private(method, first, second, signed, false).await
}
async fn run_case_private(
    method: &str,
    first: Option<Value>,
    second: Second,
    signed: Signed,
    private: bool,
) -> (bool, Vec<Seen>, usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let reserves: ReservesConfig = serde_json::from_value(
        json!({"small":{"funding_amount_usdc":"1"},"large":{"funding_amount_usdc":"5"}}),
    )
    .unwrap();
    let pool = store
        .ensure_capacity_group("handoff", &reserves, None, 0)
        .unwrap();
    for kind in [AllocationKind::Small, AllocationKind::Large] {
        store
            .allocate_capacity_kind(&pool, kind, true)
            .unwrap()
            .unwrap();
    }
    let jobs = store.funding_jobs().unwrap();
    for job in &jobs {
        store
            .record_credit(&job.wallet_id, &job.target, "fixture", 100)
            .unwrap();
    }
    let expected_signer = jobs
        .iter()
        .find(|j| j.target == "5000000")
        .unwrap()
        .recipient
        .clone();
    let balances = jobs
        .iter()
        .map(|j| (j.recipient.to_lowercase(), j.target.parse().unwrap()))
        .collect();
    let f = Fixture {
        balances: Arc::new(balances),
        requests: Arc::default(),
        handlers: Arc::default(),
        redirects: Arc::default(),
        unsigned: Arc::default(),
        timestamp: crate::rotation::base::now().unwrap(),
        first,
        second,
        signed,
        expected_signer,
        db: dir.path().join("state/state.sqlite"),
    };
    let (store, owner) = StoreHandle::spawn(store);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let redirects = f.redirects.clone();
    let app = Router::new()
        .route("/rpc", post(rpc))
        .route("/protected", any(protected))
        .route(
            "/redirect-target",
            any(move || {
                let count = redirects.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    "forbidden"
                }
            }),
        )
        .with_state(f.clone());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    // Adapt constructor only if final manager configuration API changes.
    let manager = Arc::new(
        ManagedPool::new(
            store.clone(),
            pool,
            BaseRpc::new(&format!("{origin}/rpc"), 12).unwrap(),
            SpendPolicy::dollars("none").unwrap(),
        )
        .unwrap(),
    );
    manager.reconcile().await.unwrap();
    store
        .call(|s| {
            s.deny_new_funding();
            Ok(())
        })
        .await
        .unwrap();
    let client = PaidClient::managed(manager.clone());
    let request = RoutedRequest {
        headers: Default::default(),
        method: method.into(),
        url: format!("{origin}/protected"),
        query: BTreeMap::from([("business".into(), json!("unchanged & exact"))]),
        body: Some(json!({"business":"unchanged","nested":[1,true,"bytes"]})),
    };
    let result = if private {
        client
            .execute_manual_raw(
                request,
                crate::discovery::raw::ManualRawHeaders::parse(Some(
                    &json!({"X-Label":"private-value"}),
                ))
                .unwrap(),
            )
            .await
            .map(|_| String::new())
    } else {
        client.execute(request).await
    };
    client.drain().await;
    let requests = f.requests.lock().unwrap().clone();
    let db = rusqlite::Connection::open(&f.db).unwrap();
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, requests.iter().filter(|r| r.signed).count() as i64);
    assert_eq!(
        store.call(|s| Ok(s.funding_jobs()?.len())).await.unwrap(),
        2,
        "handoff cannot allocate"
    );
    assert_eq!(f.redirects.load(Ordering::SeqCst), 0);
    drop(db);
    drop(client);
    drop(manager);
    drop(store);
    owner.await.unwrap();
    server.abort();
    (
        result.is_ok(),
        requests,
        f.handlers.load(Ordering::SeqCst),
        rows as usize,
    )
}
fn assert_bytes(requests: &[Seen], method: &str) {
    assert!(requests.len() >= 2);
    for r in requests {
        assert_eq!(r.method.as_str(), method);
        assert_eq!(r.uri, requests[0].uri);
        assert_eq!(r.body, requests[0].body);
        assert_eq!(r.content_type, requests[0].content_type);
    }
    assert!(!requests[0].signed && !requests[1].signed);
    assert_ne!(
        requests[0].peer, requests[1].peer,
        "handoff must rebuild wallet-isolated transport"
    );
    if requests.len() == 3 {
        assert!(requests[2].signed);
        // The signed response may open another socket on the same client.
        // Cryptographic signer binding is checked in the handler; this fixture
        // does not treat TCP reuse as proof of Tor or cover ownership.
    }
}
#[tokio::test]
async fn all_methods_handoff_repeats_business_bytes_and_runs_handler_once() {
    let pin: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/x402/handoff-contract.json"
    ))
    .unwrap();
    assert_eq!(pin["commit"], "f8f83309706b14cc696962f0a002eeeb66c3dc26");
    for method in [
        "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE",
    ] {
        let (ok, requests, handlers, signed) =
            run(method, Second::Price("3000000"), Signed::Paid).await;
        assert!(ok, "{method}");
        assert_eq!((requests.len(), handlers, signed), (3, 1, 1));
        assert_bytes(&requests, method);
    }
}
#[tokio::test]
async fn second_challenge_uses_advisory_estimate_and_actual_protocol_permission() {
    for (second, success, handlers, signed) in [
        (Second::Price("1000000"), true, 1, 1),
        (Second::Price("6000000"), true, 1, 1),
        (Second::Free, true, 1, 0),
        (Second::Malformed, false, 0, 0),
        (Second::Unsupported, false, 0, 0),
        (Second::Redirect, false, 0, 0),
    ] {
        let (ok, requests, count, submissions) = run("POST", second, Signed::Paid).await;
        assert_eq!(ok, success);
        assert_eq!((count, submissions), (handlers, signed));
        assert_eq!(requests.len(), 2 + signed);
        assert_bytes(&requests, "POST");
    }
}
#[tokio::test]
async fn signed_settlement_output_and_redirect_failures_never_replay_handler() {
    for signed in [
        Signed::SettlementFailure,
        Signed::InvalidImage,
        Signed::Redirect,
    ] {
        let (ok, requests, handlers, submissions) =
            run("POST", Second::Price("3000000"), signed).await;
        assert!(!ok);
        assert_eq!((requests.len(), handlers, submissions), (3, 1, 1));
        assert_bytes(&requests, "POST");
    }
}

#[tokio::test]
async fn invalid_first_challenge_never_hands_off_or_executes_handler() {
    let mut unsupported = challenge("3000000");
    unsupported["accepts"][0]["asset"] = json!("0x0000000000000000000000000000000000000009");
    let mut malformed_alternative = challenge("3000000");
    malformed_alternative["accepts"] = json!([{"scheme":"exact","amount":"bad"}]);
    for first in [
        json!({"x402Version":2,"accepts":"invalid"}),
        unsupported,
        malformed_alternative,
    ] {
        let (ok, requests, handlers, signed) =
            run_case("POST", Some(first), Second::Price("3000000"), Signed::Paid).await;
        assert!(!ok);
        assert_eq!((requests.len(), handlers, signed), (1, 0, 0));
        assert!(!requests[0].signed);
    }
}

#[tokio::test]
async fn manual_headers_block_price_handoff_before_second_wallet_io() {
    for method in ["GET", "POST"] {
        let (success, seen, handlers, signatures) =
            run_case_private(method, None, Second::Free, Signed::Paid, true).await;
        assert!(!success);
        assert_eq!(seen.len(), 1);
        assert_eq!(handlers, 0);
        assert_eq!(signatures, 0);
    }
}
