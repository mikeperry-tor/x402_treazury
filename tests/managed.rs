//! Offline end-to-end admission against a local Base RPC and x402 seller.
use alloy_primitives::Address;
use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use x402_treazury::mcp_wire::McpResponse;
use x402_treazury::{
    catalog::RoutedRequest,
    payment::{PaidClient, SpendPolicy, USDC},
    rotation::{
        base::{BaseRpc, now},
        manager::ManagedPool,
        store::{Store, StoreHandle},
    },
};
#[path = "support/signatures.rs"]
mod signatures;
struct UnsignedGate {
    barrier: Arc<tokio::sync::Barrier>,
    remaining: usize,
}
#[derive(Clone)]
struct Fake {
    balances: Arc<Mutex<BTreeMap<String, u64>>>,
    challenge: Arc<Mutex<Value>>,
    signed: Arc<Mutex<Vec<Value>>>,
    rpc_calls: Arc<AtomicUsize>,
    rpc_hold: Arc<AtomicBool>,
    nonce_delay: Arc<AtomicBool>,
    balance_delay: Arc<AtomicBool>,
    latest_time: Arc<std::sync::atomic::AtomicU64>,
    rpc_fault: Arc<Mutex<Option<String>>>,
    confirmed_reads: Arc<AtomicUsize>,
    used: Arc<AtomicBool>,
    stale: Arc<AtomicBool>,
    reorg: Arc<AtomicBool>,
    wrong_chain: Arc<AtomicBool>,
    reject: Arc<AtomicBool>,
    free: Arc<AtomicBool>,
    hold: Arc<AtomicBool>,
    arrived: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    unsigned_gate: Arc<Mutex<Option<UnsignedGate>>>,
    db: std::path::PathBuf,
    traces: Arc<Mutex<Vec<(std::net::SocketAddr, String, String)>>>,
}
fn challenge(amount: &str) -> Value {
    json!({"x402Version":2,"resource":{"url":"http://localhost/pay","description":"x".repeat(700),"mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":amount,"payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]})
}
async fn rpc(
    State(f): State<Fake>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    Json(v): Json<Value>,
) -> Json<Value> {
    if f.rpc_hold.swap(false, Ordering::SeqCst) {
        f.arrived.notify_one();
        f.release.notified().await;
    }
    if v["method"] == "eth_call"
        && v["params"][0]["data"]
            .as_str()
            .is_some_and(|s| s.starts_with("0x70a08231"))
        && f.balance_delay.load(Ordering::SeqCst)
    {
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    }
    if v["method"] == "eth_call"
        && v["params"][0]["data"]
            .as_str()
            .is_some_and(|s| !s.starts_with("0x70a08231"))
        && f.nonce_delay.swap(false, Ordering::SeqCst)
    {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    f.rpc_calls.fetch_add(1, Ordering::SeqCst);
    let fault = f.rpc_fault.lock().unwrap().clone().unwrap_or_default();
    let unavailable_nonce = fault == "nonce_unavailable"
        || (fault == "first_nonce_unavailable"
            && f.signed.lock().unwrap().first().is_some_and(|signed| {
                v["params"][0]["data"].as_str().is_some_and(|data| {
                    data.ends_with(
                        signed["payload"]["authorization"]["nonce"]
                            .as_str()
                            .unwrap()
                            .trim_start_matches("0x"),
                    )
                })
            }));
    if unavailable_nonce
        && v["method"] == "eth_call"
        && v["params"][0]["data"]
            .as_str()
            .is_some_and(|s| !s.starts_with("0x70a08231"))
    {
        return Json(
            json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32005,"message":"private upstream outage"}}),
        );
    }
    let mut result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!(if f.wrong_chain.load(Ordering::SeqCst) {
            "0x1"
        } else {
            "0x2105"
        }),
        "eth_getBlockByNumber" => {
            if v["params"][0] == "latest" {
                f.latest_time.store(now().unwrap(), Ordering::SeqCst);
            }
            let timestamp = f.latest_time.load(Ordering::SeqCst);
            let number = if v["params"][0] == "latest" {
                "0x64"
            } else {
                v["params"][0].as_str().unwrap()
            };
            json!({"number":number,"hash":format!("0x{:064x}",if f.reorg.load(Ordering::SeqCst){2}else{1}),"timestamp":format!("0x{:x}",timestamp-if f.stale.load(Ordering::SeqCst){1000}else{0})})
        }
        "eth_call" => {
            assert_eq!(v["params"][1]["requireCanonical"], true);
            let data = v["params"][0]["data"].as_str().unwrap();
            let n = if data.starts_with("0x70a08231") {
                assert_eq!(data.len(), 74);
                let address = format!("0x{}", &data[data.len() - 40..])
                    .parse::<Address>()
                    .unwrap()
                    .to_string();
                f.traces
                    .lock()
                    .unwrap()
                    .push((peer, "balance".into(), address.clone()));
                *f.balances.lock().unwrap().get(&address).unwrap_or(&0)
            } else {
                u64::from(f.used.load(Ordering::SeqCst))
            };
            json!(format!("0x{n:064x}"))
        }
        _ => panic!("unexpected RPC"),
    };
    if v["method"] == "eth_getBlockByNumber" {
        match fault.as_str() {
            "height" if v["params"][0] == "latest" => result["number"] = json!("0x1"),
            "quantity" => result["number"] = json!("0xgg"),
            "oversized_quantity" => result["number"] = json!("0xfffffffffffffffff"),
            "future" => result["timestamp"] = json!(format!("0x{:x}", now().unwrap() + 60)),
            "final_hash"
                if v["params"][0] != "latest"
                    && f.confirmed_reads.fetch_add(1, Ordering::SeqCst) > 0 =>
            {
                result["hash"] = json!(format!("0x{:064x}", 9))
            }
            _ => {}
        }
    }
    if v["method"] == "eth_call" {
        let data = v["params"][0]["data"].as_str().unwrap();
        match fault.as_str() {
            "word_short" => result = json!("0x00"),
            "word_long" => result = json!(format!("0x{}", "0".repeat(65))),
            "word_invalid" => result = json!(format!("0x{}", "g".repeat(64))),
            "nonce" if !data.starts_with("0x70a08231") => result = json!(format!("0x{:064x}", 2)),
            _ => {}
        }
    }
    let mut response = json!({"jsonrpc":"2.0","id":v["id"],"result":result});
    match fault.as_str() {
        "id" => response["id"] = json!(2),
        "version" => response["jsonrpc"] = json!("1.0"),
        "error" => response["error"] = json!({"code":-1,"message":"fixture error"}),
        "null" => response["result"] = Value::Null,
        "missing" => {
            response.as_object_mut().unwrap().remove("result");
        }
        _ => {}
    }
    Json(response)
}
async fn seller(
    State(f): State<Fake>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if f.free.load(Ordering::SeqCst) {
        return "free".into_response();
    }
    if let Some(h) = headers.get("payment-signature") {
        let p: Value = serde_json::from_slice(&STANDARD.decode(h.as_bytes()).unwrap()).unwrap();
        let recovered = signatures::recover_exact(&p);
        f.traces
            .lock()
            .unwrap()
            .push((peer, "signed".into(), recovered.to_string()));
        // Assert the journal is committed before any signed bytes reach the seller.
        let db = rusqlite::Connection::open(&f.db).unwrap();
        let count:i64=db.query_row("SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED' AND nonce=?1",[p["payload"]["authorization"]["nonce"].as_str().unwrap()],|r|r.get(0)).unwrap();
        assert_eq!(count, 1);
        f.signed.lock().unwrap().push(p);
        if f.hold.swap(false, Ordering::SeqCst) {
            f.arrived.notify_one();
            f.release.notified().await;
        }
        if !f.reject.load(Ordering::SeqCst) {
            return "paid".into_response();
        }
    }
    f.traces
        .lock()
        .unwrap()
        .push((peer, "unsigned".into(), String::new()));
    let gate = {
        let mut slot = f.unsigned_gate.lock().unwrap();
        slot.as_mut().and_then(|gate| {
            if gate.remaining == 0 {
                None
            } else {
                gate.remaining -= 1;
                Some(gate.barrier.clone())
            }
        })
    };
    if let Some(gate) = gate {
        tokio::time::timeout(std::time::Duration::from_secs(10), gate.wait())
            .await
            .unwrap();
    }
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&*f.challenge.lock().unwrap()).unwrap()),
        )],
        "insufficient_funds",
    )
        .into_response()
}
struct Harness {
    dir: tempfile::TempDir,
    f: Fake,
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    pool: String,
    client: PaidClient,
    url: String,
    base: String,
    server: tokio::task::JoinHandle<()>,
}
impl Harness {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture snapshot",
        )
        .unwrap();
        let pool = store.ensure_pool("research", "5").unwrap();
        let f = Fake {
            balances: Arc::default(),
            challenge: Arc::new(Mutex::new(challenge("3000000"))),
            signed: Arc::default(),
            rpc_calls: Arc::default(),
            rpc_hold: Arc::default(),
            nonce_delay: Arc::default(),
            balance_delay: Arc::default(),
            latest_time: Arc::default(),
            rpc_fault: Arc::default(),
            confirmed_reads: Arc::default(),
            used: Arc::default(),
            stale: Arc::default(),
            reorg: Arc::default(),
            wrong_chain: Arc::default(),
            reject: Arc::default(),
            free: Arc::default(),
            hold: Arc::default(),
            arrived: Arc::default(),
            release: Arc::default(),
            unsigned_gate: Arc::default(),
            db: dir.path().join("state/state.sqlite"),
            traces: Arc::default(),
        };
        for address in &store.status().unwrap().pools[0].addresses {
            f.balances
                .lock()
                .unwrap()
                .insert(address.address.clone(), 5_000_000);
        }
        let (store, worker) = StoreHandle::spawn(store);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/rpc", post(rpc))
            .route("/pay", get(seller).post(seller))
            .with_state(f.clone());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap()
        });
        let client = make_client(store.clone(), pool.clone(), &base);
        Self {
            dir,
            f,
            store,
            worker,
            pool,
            client,
            url: format!("{base}/pay"),
            base,
            server,
        }
    }
    async fn close(self) {
        self.server.abort();
        drop(self.client);
        drop(self.store);
        self.worker.await.unwrap();
    }
    fn route(&self) -> RoutedRequest {
        RoutedRequest {
            method: "GET".into(),
            url: self.url.clone(),
            query: BTreeMap::new(),
            body: None,
        }
    }
}
fn make_pool(store: StoreHandle, pool: String, base: &str) -> Arc<ManagedPool> {
    let manager = ManagedPool::new(
        store,
        pool,
        BaseRpc::new(&format!("{base}/rpc"), 12, 120).unwrap(),
        "5",
        SpendPolicy::dollars("none").unwrap(),
    )
    .unwrap();
    Arc::new(manager)
}
fn make_client(store: StoreHandle, pool: String, base: &str) -> PaidClient {
    PaidClient::managed(make_pool(store, pool, base))
}
#[tokio::test]
async fn concurrent_calls_reserve_before_send_and_cannot_churn_busy_active() {
    let h = Harness::new().await;
    let other = h.client.clone();
    let (a, b) = tokio::join!(h.client.execute(h.route()), other.execute(h.route()));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let error = a.err().or_else(|| b.err()).unwrap().to_string();
    assert!(error.contains("payment_pending"), "{error}");
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    let status = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.pools[0].generation, 0);
    assert_eq!(status.pools[0].addresses.len(), 2);
    drop(other);
    h.close().await;
}
#[tokio::test]
async fn confirmed_depletion_promotes_once_and_queues_one_replacement() {
    let h = Harness::new().await;
    h.client.execute(h.route()).await.unwrap();
    let active = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(active, 0);
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    db.execute_batch("CREATE TRIGGER fail_replacement BEFORE INSERT ON funding_jobs BEGIN SELECT RAISE(ABORT,'injected replacement failure'); END;").unwrap();
    assert!(h.client.execute(h.route()).await.is_err());
    assert_eq!(
        h.store.call(|s| s.status()).await.unwrap().pools[0].generation,
        0
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    db.execute_batch("DROP TRIGGER fail_replacement;").unwrap();
    h.client.execute(h.route()).await.unwrap();
    let state = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(state.pools[0].generation, 1);
    assert_eq!(state.pools[0].addresses.len(), 3);
    let pool = h.pool.clone();
    let query = h.store.call(move |s| s.chain_query(&pool)).await.unwrap();
    assert_eq!(query.wallets.len(), 2); // Retired, reconciled address needs no more RPC calls.
    assert_eq!(query.pending.len(), 1);
    let signed = h.f.signed.lock().unwrap().clone();
    assert_ne!(
        signed[0]["payload"]["authorization"]["from"],
        signed[1]["payload"]["authorization"]["from"]
    );
    assert_eq!(
        signed[0]["resource"]["description"].as_str().unwrap().len(),
        500
    );
    h.close().await;
}
#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[tokio::test]
async fn managed_extensions_are_stripped_logged_and_do_not_change_authorization() {
    use tracing::instrument::WithSubscriber;
    let logs = LogCapture(Arc::default());
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    async {
        for captured in [false, true] {
            let h = Harness::new().await;
            let mut c = if captured {
                serde_json::from_str(include_str!("fixtures/agentutility_payment_required.json"))
                    .unwrap()
            } else {
                let mut c = challenge("1");
                c["extensions"] =
                    json!({"unknown-extension": {"secret": "private-extension-value"}});
                c
            };
            let expected = c["accepts"][0].clone();
            *h.f.challenge.lock().unwrap() = c.take();
            assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
            {
                let signed = h.f.signed.lock().unwrap();
                assert_eq!(signed.len(), 1);
                assert!(
                    signed[0]["extensions"]
                        .as_object()
                        .is_none_or(|m| m.is_empty())
                );
                assert_eq!(signed[0]["accepted"], expected);
                assert_eq!(
                    signed[0]["payload"]["authorization"]["value"],
                    expected["amount"]
                );
                assert_eq!(
                    signed[0]["payload"]["authorization"]["to"]
                        .as_str()
                        .unwrap()
                        .parse::<Address>()
                        .unwrap(),
                    expected["payTo"]
                        .as_str()
                        .unwrap()
                        .parse::<Address>()
                        .unwrap()
                );
            }
            h.close().await;
        }
    }
    .with_subscriber(subscriber)
    .await;
    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for name in ["unknown-extension", "bazaar", "builder-code"] {
        assert!(logs.contains(name), "{logs}");
    }
    assert!(logs.contains("stripping advertised x402 extension"));
    assert!(!logs.contains("private-extension-value"));
}
// Captured unsigned challenges: top-level extension omission is deliberately
// separate from rejecting unreviewed metadata inside payment offers.
#[tokio::test]
async fn provider_challenges_distinguish_extensions_from_payment_metadata() {
    let fixtures: BTreeMap<String, Value> =
        serde_json::from_str(include_str!("fixtures/provider_payment_challenges.json")).unwrap();
    for (name, challenge) in fixtures {
        let h = Harness::new().await;
        *h.f.challenge.lock().unwrap() = challenge.clone();
        let result = h.client.execute(h.route()).await;
        assert_eq!(result.unwrap(), "paid", "{name}");
        {
            let signed = h.f.signed.lock().unwrap();
            assert_eq!(signed.len(), 1, "{name}");
            assert!(
                signed[0]["extensions"]
                    .as_object()
                    .is_none_or(|m| m.is_empty()),
                "{name}"
            );
            assert_eq!(signed[0]["accepted"]["network"], "eip155:8453");
            assert_eq!(signed[0]["accepted"]["extra"]["name"], "USD Coin");
            assert!(
                challenge["accepts"]
                    .as_array()
                    .unwrap()
                    .contains(&signed[0]["accepted"]),
                "{name}: offer metadata must survive SDK serialization unchanged"
            );
            assert_eq!(
                signed[0]["payload"]["authorization"]["value"],
                signed[0]["accepted"]["amount"]
            );
        }
        h.close().await;
    }
}
#[tokio::test]
async fn output_schema_is_opaque_metadata_and_invalid_shapes_fail_before_admission() {
    let h = Harness::new().await;
    for schema in [json!(null), json!("object"), json!([]), json!(1)] {
        let mut c = challenge("1");
        c["accepts"][0]["outputSchema"] = schema;
        *h.f.challenge.lock().unwrap() = c;
        assert!(h.client.execute(h.route()).await.is_err());
    }
    assert_eq!(h.f.rpc_calls.load(Ordering::SeqCst), 0);
    assert!(h.f.signed.lock().unwrap().is_empty());
    for schema in [
        json!(true),
        json!(false),
        json!({"$ref": "http://127.0.0.1:1/must-not-fetch", "amount": "0",
            "assetTransferMethod": "permit2", "extra": {"name": "fake"}}),
    ] {
        let mut c = challenge("1");
        c["accepts"][0]["outputSchema"] = schema.clone();
        *h.f.challenge.lock().unwrap() = c;
        assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
        let signed = h.f.signed.lock().unwrap();
        let payload = signed.last().unwrap();
        assert_eq!(payload["accepted"]["outputSchema"], schema);
        assert_eq!(payload["payload"]["authorization"]["value"], "1");
        assert_eq!(payload["accepted"]["extra"]["name"], "USD Coin");
    }
    h.close().await;
}
#[tokio::test]
async fn opaque_annotations_and_display_prices_preserve_atomic_payment_terms() {
    let h = Harness::new().await;
    let mut c = challenge("1");
    c["accepts"][0]["vendorAnnotation"] = json!({"nested": [1, "value"]});
    c["accepts"][0]["extra"]["totalUsd"] = json!("0.000001");
    c["accepts"][0]["extra"]["breakdown"] = json!({"search": "display only"});
    c["accepts"][0]["extra"]["merchant"] = json!({"name": "vendor"});
    *h.f.challenge.lock().unwrap() = c.clone();
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    {
        let signed = h.f.signed.lock().unwrap();
        assert_eq!(signed.last().unwrap()["accepted"], c["accepts"][0]);
        assert_eq!(
            signed.last().unwrap()["payload"]["authorization"]["value"],
            "1"
        );
    }
    h.close().await;
}
#[tokio::test]
async fn informational_metadata_cannot_override_payment_safety() {
    let h = Harness::new().await;
    for (key, value) in [
        ("amount", json!("0")),
        ("permit2Authorization", json!({})),
        ("assetTransferMethod", json!("permit2")),
        ("name", json!("GatewayWalletBatched")),
    ] {
        let mut c = challenge("1");
        c["accepts"][0]["extra"]["merchant"] = json!("x402Atlas");
        c["accepts"][0]["extra"][key] = value;
        *h.f.challenge.lock().unwrap() = c;
        assert!(h.client.execute(h.route()).await.is_err(), "{key}");
    }
    let mut c = challenge("5000001");
    c["accepts"][0]["extra"]["totalUsd"] = json!(0);
    c["accepts"][0]["extra"]["breakdown"] = json!({"search": 0});
    *h.f.challenge.lock().unwrap() = c;
    assert!(h.client.execute(h.route()).await.is_err());
    assert_eq!(h.f.rpc_calls.load(Ordering::SeqCst), 0);
    assert!(h.f.signed.lock().unwrap().is_empty());
    h.close().await;
}
#[tokio::test]
async fn unsupported_and_over_target_offers_have_no_admission_side_effects() {
    let h = Harness::new().await;
    for kind in [
        "v1",
        "upto",
        "permit2",
        "flow",
        "malformed_extensions",
        "offer_extensions",
        "extra_extensions",
        "asset",
        "chain",
        "zero",
        "decimal",
        "over_target",
        "domain",
        "envelope",
    ] {
        let mut c = challenge("1");
        c["accepts"][0]["outputSchema"] = json!({"type": "object"});
        match kind {
            "v1" => c["x402Version"] = json!(1),
            "upto" => c["accepts"][0]["scheme"] = json!("upto"),
            "permit2" => c["accepts"][0]["extra"]["assetTransferMethod"] = json!("permit2"),
            "flow" => c["accepts"][0]["extra"]["flow"] = json!("upfront"),
            "malformed_extensions" => c["extensions"] = json!([]),
            "offer_extensions" => c["accepts"][0]["extensions"] = json!({"unknown": {}}),
            "extra_extensions" => c["accepts"][0]["extra"]["extensions"] = json!({"unknown": {}}),
            "asset" => {
                c["accepts"][0]["asset"] = json!("0x0000000000000000000000000000000000000001")
            }
            "chain" => c["accepts"][0]["network"] = json!("eip155:1"),
            "zero" => c["accepts"][0]["amount"] = json!("0"),
            "decimal" => c["accepts"][0]["amount"] = json!("1.0"),
            "over_target" => c["accepts"][0]["amount"] = json!("5000001"),
            "domain" => c["accepts"][0]["extra"]["name"] = json!("fake"),
            "envelope" => c["resource"]["url"] = json!([]),
            _ => unreachable!(),
        }
        *h.f.challenge.lock().unwrap() = c;
        assert!(h.client.execute(h.route()).await.is_err(), "{kind}");
    }
    assert_eq!(h.f.rpc_calls.load(Ordering::SeqCst), 0);
    assert!(h.f.signed.lock().unwrap().is_empty());
    let mut c = challenge("1");
    let supported = c["accepts"][0].clone();
    c["accepts"][0]["scheme"] = json!("upto");
    c["accepts"].as_array_mut().unwrap().push(supported.clone());
    *h.f.challenge.lock().unwrap() = c;
    h.client.execute(h.route()).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap()[0]["accepted"], supported);
    h.close().await;
}
#[tokio::test]
async fn rejected_payment_remains_reserved_across_restart_and_is_not_replayed() {
    let mut h = Harness::new().await;
    h.f.reject.store(true, Ordering::SeqCst);
    h.f.challenge.lock().unwrap()["extensions"] = json!({"unknown": {"secret": "not logged"}});
    let error = h.client.execute(h.route()).await.unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("omitted advertised x402 extensions"),
        "{message}"
    );
    assert!(message.contains("remains reserved"), "{message}");
    assert!(message.contains("Do not automatically retry"), "{message}");
    assert!(message.contains("402"), "{message}");
    assert!(
        h.f.signed.lock().unwrap()[0]["extensions"]
            .as_object()
            .is_none_or(|m| m.is_empty())
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    let id = h.store.call(|s| Ok(s.id().to_owned())).await.unwrap();
    drop(h.client);
    drop(h.store);
    h.worker.await.unwrap();
    let store = Store::open(&h.dir.path().join("state"), &h.dir.path().join("key"), &id).unwrap();
    (h.store, h.worker) = StoreHandle::spawn(store);
    h.client = make_client(h.store.clone(), h.pool.clone(), &h.base);
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("payment_pending")
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    h.close().await;
}
#[tokio::test]
async fn stale_wrong_chain_and_reorg_evidence_never_authorize_or_rotate() {
    let h = Harness::new().await;
    h.f.stale.store(true, Ordering::SeqCst);
    assert!(h.client.execute(h.route()).await.is_err());
    h.f.stale.store(false, Ordering::SeqCst);
    h.f.wrong_chain.store(true, Ordering::SeqCst);
    assert!(h.client.execute(h.route()).await.is_err());
    h.f.wrong_chain.store(false, Ordering::SeqCst);
    h.client.execute(h.route()).await.unwrap();
    h.f.reorg.store(true, Ordering::SeqCst);
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("chain_recovery_required")
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    h.close().await;
}
#[tokio::test]
async fn cancellation_after_submission_retains_liability() {
    let h = Harness::new().await;
    h.f.hold.store(true, Ordering::SeqCst);
    let client = h.client.clone();
    let route = h.route();
    let task = tokio::spawn(async move { client.execute(route).await });
    h.f.arrived.notified().await;
    task.abort();
    let _ = task.await;
    h.f.release.notify_one();
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("payment_pending")
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    h.close().await;
}
#[tokio::test]
async fn journal_failure_sends_no_signature_and_next_admission_recovers() {
    let h = Harness::new().await;
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    db.execute_batch("CREATE TRIGGER fail_journal BEFORE UPDATE OF state ON payment_attempts WHEN NEW.state='POSSIBLY_SUBMITTED' BEGIN SELECT RAISE(ABORT,'injected journal failure'); END;").unwrap();
    assert!(h.client.execute(h.route()).await.is_err());
    assert!(h.f.signed.lock().unwrap().is_empty());
    db.execute_batch("DROP TRIGGER fail_journal;").unwrap();
    h.client.execute(h.route()).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    h.close().await;
}

#[cfg(feature = "zcash")]
#[tokio::test]
async fn managed_deployment_shares_pools_keeps_static_profiles_and_releases_ownership() {
    use x402_treazury::{deployment::Deployment, treasury::Treasury};
    let h = Harness::new().await;
    *h.f.challenge.lock().unwrap() = challenge("1");
    let dir = tempfile::tempdir().unwrap();
    let t=Treasury::create(dir.path().join("state"),dir.path().join("key"),2_000_000,Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
    let id = t.status().await.unwrap().treasury_id;
    t.close().await.unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        json!({"servers":[{"url":h.base}],"paths":{"/pay":{"get":{}}}}).to_string(),
    )
    .unwrap();
    let config = format!(
        r#"version=1
[treasury]
id="{id}"
state_dir="state"
key_file="key"
indexer_url_env="INDEXER"
submission_url_env="SUBMIT"
daily_treasury_spend_limit_zec="0.1"
max_refund_shielding_fee_zec="0.001"
[funding]
auto_fund=false
base_rpc_url_env="BASE"
base_rpc_fallback_url_envs=[]
[wallets.research]
mode="zcash_rotation"
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[wallets.unused]
mode="zcash_rotation"
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[wallets.static_wallet]
mode="static"
private_key_env="KEY"
[sources.api]
spec="spec.json"
probe_pricing=false
[sources.managed]
spec="spec.json"
probe_pricing=false
wallet="research"
[servers.one]
listen="127.0.0.1:0"
bearer_token_env="TOKEN"
wallet="static_wallet"
sources=["managed"]
[servers.two]
listen="127.0.0.1:0"
bearer_token_env="TOKEN"
sources=["managed"]
[servers.three]
listen="127.0.0.1:0"
bearer_token_env="TOKEN"
wallet="static_wallet"
sources=["api"]
"#
    );
    let path = dir.path().join("servers.toml");
    std::fs::write(&path, &config).unwrap();
    let env = BTreeMap::from([
        ("TOKEN".into(), "secret".into()),
        ("KEY".into(), "1".repeat(64)),
        ("INDEXER".into(), "https://example.invalid".into()),
        ("SUBMIT".into(), "https://example.invalid".into()),
        ("BASE".into(), format!("{}/rpc", h.base)),
    ]);
    let mut missing_auth = env.clone();
    missing_auth.remove("TOKEN");
    let error = match Deployment::load(&path)
        .await
        .unwrap()
        .bind(&missing_auth)
        .await
    {
        Ok(_) => panic!("missing authentication accepted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("TOKEN"));
    assert!(
        x402_treazury::rotation::store::status(&dir.path().join("state"))
            .unwrap()
            .pools
            .is_empty(),
        "authentication must be checked before allocating managed pools"
    );

    // Fail after wallets and an earlier listener exist; retry must reuse identities.
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_address = first.local_addr().unwrap();
    drop(first);
    let mut conflicting: toml::Value = toml::from_str(&config).unwrap();
    conflicting["servers"]["one"]["listen"] = toml::Value::String(first_address.to_string());
    conflicting["servers"]["three"]["listen"] =
        toml::Value::String(occupied.local_addr().unwrap().to_string());
    std::fs::write(&path, toml::to_string(&conflicting).unwrap()).unwrap();
    let error = match Deployment::load(&path).await.unwrap().bind(&env).await {
        Ok(_) => panic!("occupied listener accepted"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("server three: cannot bind"),
        "{error}"
    );
    drop(tokio::net::TcpListener::bind(first_address).await.unwrap());
    // Store shutdown is asynchronous once its last handle is dropped.
    let reopened = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(owner) =
                Treasury::open(dir.path().join("state"), dir.path().join("key"), id.clone()).await
            {
                break owner;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("failed startup retained treasury ownership");
    let allocated = reopened.status().await.unwrap().pools;
    assert_eq!(allocated.len(), 2);
    reopened.close().await.unwrap();
    std::fs::write(&path, &config).unwrap();
    let running = Deployment::load(&path)
        .await
        .unwrap()
        .bind(&env)
        .await
        .unwrap();
    let state = x402_treazury::rotation::store::status(&dir.path().join("state")).unwrap();
    for (before, after) in allocated.iter().zip(&state.pools) {
        assert_eq!(
            serde_json::to_value(&before.addresses).unwrap(),
            serde_json::to_value(&after.addresses).unwrap(),
            "startup retry must retain allocated wallet identities"
        );
    }
    assert_eq!(state.pools.len(), 2);
    assert_eq!(state.pools[0].addresses.len(), 2);
    assert!(
        Treasury::open(dir.path().join("state"), dir.path().join("key"), id.clone())
            .await
            .is_err()
    );
    let addresses = running.addresses();
    let stop = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    for (name, address) in addresses {
        if name == "three" {
            continue;
        }
        let response:Value=reqwest::Client::new().post(format!("http://{address}/mcp")).bearer_auth("secret").header("accept","application/json, text/event-stream").json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"managed_pay","arguments":{}}})).send().await.unwrap().mcp_json().await.unwrap();
        assert_eq!(response["result"]["isError"], true, "{response}");
        assert!(
            response.to_string().contains("wallet_not_ready"),
            "{response}"
        );
    }
    stop.cancel();
    task.await.unwrap().unwrap();
    let t = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
        .await
        .unwrap();
    assert_eq!(t.status().await.unwrap().pools[0].addresses.len(), 2);
    t.close().await.unwrap();
    // Exercise the opt-in owner's shutdown with all quote work deferred. This
    // must not contact NEAR, and must release every worker's store handle.
    let mut state = x402_treazury::rotation::store::Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &state.treasury_id,
    )
    .unwrap();
    for job in state.funding_jobs().unwrap() {
        state
            .defer_funding(&job.id, i64::MAX as u64, None, false)
            .unwrap();
    }
    let treasury_id = state.id().to_owned();
    drop(state);
    // Omitted ID resolves from durable state; omitted auto_fund now starts workers.
    std::fs::write(
        &path,
        config
            .replace("auto_fund=false\n", "")
            .replace(&format!("id=\"{treasury_id}\"\n"), ""),
    )
    .unwrap();
    let running = Deployment::load(&path)
        .await
        .unwrap()
        .bind(&env)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    stop.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let static_config = config.replace(
        "mode=\"zcash_rotation\"\nmax_funding_spend_zec=\"0.02\"\nmax_conversion_overhead_percent=5",
        "mode=\"static\"\nprivate_key_env=\"KEY\"",
    );
    std::fs::write(&path, static_config).unwrap();
    let error = match Deployment::load(&path).await.unwrap().bind(&env).await {
        Ok(_) => panic!("accepted managed-to-static identity change"),
        Err(e) => e.to_string(),
    };
    assert!(error.contains("cannot become static"), "{error}");
    h.close().await;
}

#[tokio::test]
async fn free_calls_do_not_touch_admission_and_bootstrap_requires_both_candidates() {
    let h = Harness::new().await;
    h.f.balances.lock().unwrap().clear();
    h.f.free.store(true, Ordering::SeqCst);
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "free");
    assert_eq!(h.f.rpc_calls.load(Ordering::SeqCst), 0);
    h.f.free.store(false, Ordering::SeqCst);
    let address = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    h.f.balances.lock().unwrap().insert(address, 5_000_000);
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("wallet_not_ready")
    );
    assert!(!h.store.call(|s| s.status()).await.unwrap().pools[0].bootstrapped);
    assert!(h.f.signed.lock().unwrap().is_empty());
    h.close().await;
}
#[tokio::test]
async fn unused_expired_authorization_requires_fresh_chain_evidence_to_release() {
    let h = Harness::new().await;
    // Leave more time than the client's three-second request timeout for signing.
    // A one-second lifetime can expire at a wall-clock boundary before submission.
    h.f.challenge.lock().unwrap()["accepts"][0]["maxTimeoutSeconds"] = json!(5);
    h.f.reject.store(true, Ordering::SeqCst);
    assert!(h.client.execute(h.route()).await.is_err());
    let expiry = {
        let signed = h.f.signed.lock().unwrap();
        assert_eq!(signed.len(), 1, "test must reach signed submission");
        signed[0]["payload"]["authorization"]["validBefore"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    while now().unwrap() <= expiry {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    h.f.stale.store(true, Ordering::SeqCst);
    assert!(h.client.execute(h.route()).await.is_err());
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    h.f.stale.store(false, Ordering::SeqCst);
    h.f.reject.store(false, Ordering::SeqCst);
    h.client.execute(h.route()).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    assert_eq!(
        h.store.call(|s| s.status()).await.unwrap().pools[0].generation,
        0
    );
    h.close().await;
}
#[tokio::test]
async fn pending_exposure_rejects_promptly_while_other_pools_continue() {
    let h = Harness::new().await;
    let other_id = h.store.call(|s| s.ensure_pool("other", "5")).await.unwrap();
    let status = h.store.call(|s| s.status()).await.unwrap();
    for pool in status.pools {
        if pool.name == "other" {
            for address in pool.addresses {
                h.f.balances
                    .lock()
                    .unwrap()
                    .insert(address.address, 5_000_000);
            }
        }
    }
    let other = make_client(h.store.clone(), other_id, &h.base);
    h.f.hold.store(true, Ordering::SeqCst);
    let client = h.client.clone();
    let route = h.route();
    let task = tokio::spawn(async move { client.execute(route).await });
    h.f.arrived.notified().await;
    other.execute(h.route()).await.unwrap();
    let error = h.client.execute(h.route()).await.unwrap_err().to_string();
    assert!(error.contains("payment_pending"), "{error}");
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    task.abort();
    let _ = task.await;
    h.f.release.notify_one();
    drop(other);
    h.close().await;
}
#[tokio::test]
async fn larger_target_does_not_churn_through_smaller_standby() {
    let h = Harness::new().await;
    h.client.execute(h.route()).await.unwrap();
    let active = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    h.f.balances.lock().unwrap().insert(active, 0);
    h.f.used.store(true, Ordering::SeqCst);
    h.store
        .call(|s| s.ensure_pool("research", "10"))
        .await
        .unwrap();
    *h.f.challenge.lock().unwrap() = challenge("6000000");
    let client = PaidClient::managed(Arc::new(
        ManagedPool::new(
            h.store.clone(),
            h.pool.clone(),
            BaseRpc::new(&format!("{}/rpc", h.base), 12, 120).unwrap(),
            "10",
            SpendPolicy::dollars("none").unwrap(),
        )
        .unwrap(),
    ));
    assert!(
        client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("funding_unavailable")
    );
    assert_eq!(
        h.store.call(|s| s.status()).await.unwrap().pools[0].generation,
        0
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    drop(client);
    h.close().await;
}

#[tokio::test]
async fn background_reconciliation_releases_confirmed_payment_without_rotating_or_signing() {
    let h = Harness::new().await;
    h.client.execute(h.route()).await.unwrap();
    let address = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(address, 0);
    let manager = ManagedPool::new(
        h.store.clone(),
        h.pool.clone(),
        BaseRpc::new(&format!("{}/rpc", h.base), 12, 120).unwrap(),
        "5",
        SpendPolicy::dollars("none").unwrap(),
    )
    .unwrap();
    manager.reconcile().await.unwrap();
    let state = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(state.pools[0].generation, 0);
    assert_eq!(state.pools[0].addresses.len(), 2);
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state!='RESOLVED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unresolved, 0);
    h.f.reorg.store(true, Ordering::SeqCst);
    assert!(manager.reconcile().await.is_err());
    drop(db);
    drop(manager);
    h.close().await;
}

#[cfg(feature = "zcash")]
#[path = "support/lifecycle.rs"]
mod lifecycle;

#[path = "support/socks.rs"]
mod socks;
#[tokio::test]
async fn managed_payments_use_isolated_tor_connections() {
    let proxy = socks::Socks::start(
        std::collections::BTreeMap::from([("loopback".into(), "127.0.0.1:1".parse().unwrap())]),
        socks::Fault::None,
    )
    .await;
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "tor_managed_child", "--nocapture"])
        .env("TOR_TEST_PROXY", proxy.address.to_string())
        .env("RUST_BACKTRACE", "0")
        // Policy must win even against ambient proxy bypass instructions.
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "*")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let records = proxy.records.lock().unwrap();
    assert!(records.len() > 4);
    assert!(records.iter().all(|r| r.user == "<torS0X>0"));
    assert!(
        records
            .iter()
            .map(|r| r.password.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 3
    );
}
#[test]
#[ignore = "subprocess helper runs with its own immutable Tor policy"]
fn tor_managed_child() {
    use x402_treazury::network::{Mode, NetworkPolicy, install};
    install(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(std::env::var("TOR_TEST_PROXY").unwrap().parse().unwrap()),
        ..Default::default()
    })
    .unwrap();
    confirmed_depletion_promotes_once_and_queues_one_replacement();
    concurrent_calls_reserve_before_send_and_cannot_churn_busy_active();
    unsupported_and_over_target_offers_have_no_admission_side_effects();
    managed_extensions_are_stripped_logged_and_do_not_change_authorization();
    rejected_payment_remains_reserved_across_restart_and_is_not_replayed();
    free_calls_do_not_touch_admission_and_bootstrap_requires_both_candidates();
    post_rotation_returns_before_signing_and_requires_caller_retry();
}

#[tokio::test]
async fn post_rotation_returns_before_signing_and_requires_caller_retry() {
    let h = Harness::new().await;
    h.client.execute(h.route()).await.unwrap();
    let active = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(active, 0);
    let mut route = h.route();
    route.method = "POST".into();
    let error = h.client.execute(route).await.unwrap_err();
    assert!(error.to_string().contains("payer_changed_before_payment"));
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    assert_eq!(
        h.store.call(|s| s.status()).await.unwrap().pools[0].generation,
        1
    );
    let mut route = h.route();
    route.method = "POST".into();
    h.client.execute(route).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    h.close().await;
}

#[tokio::test]
async fn promotion_keeps_rpc_and_payment_transport_bound_to_address() {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "promotion_identity_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test]
#[ignore = "private process Tor policy; exercised by parent"]
async fn promotion_identity_child() {
    use x402_treazury::network::{self, IsolationId, Mode, NetworkPolicy};
    let proxy = socks::Socks::start(
        BTreeMap::from([("loopback".into(), "127.0.0.1:1".parse().unwrap())]),
        socks::Fault::None,
    )
    .await;
    network::install(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(proxy.address),
        ..Default::default()
    })
    .unwrap();
    for method in ["GET", "POST"] {
        let h = Harness::new().await;
        let initial = h.store.call(|s| s.status()).await.unwrap();
        let active = initial.pools[0].addresses[0].address.clone();
        let standby = initial.pools[0].addresses[1].address.clone();
        h.client.execute(h.route()).await.unwrap();
        h.f.used.store(true, Ordering::SeqCst);
        h.f.balances.lock().unwrap().insert(active.clone(), 0);
        let mut route = h.route();
        route.method = method.into();
        let result = h.client.execute(route).await;
        if method == "POST" {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("payer_changed_before_payment")
            );
            assert_eq!(h.f.signed.lock().unwrap().len(), 1);
            let mut retry = h.route();
            retry.method = method.into();
            h.client.execute(retry).await.unwrap();
        } else {
            result.unwrap();
        }
        let status = h.store.call(|s| s.status()).await.unwrap();
        assert_eq!(status.pools[0].generation, 1);
        assert_eq!(status.pools[0].addresses.len(), 3);
        assert_eq!(h.f.signed.lock().unwrap().len(), 2);
        let traces = h.f.traces.lock().unwrap().clone();
        let records = proxy.records.lock().unwrap().clone();
        let payment = traces
            .iter()
            .filter(|(_, phase, _)| phase != "balance")
            .collect::<Vec<_>>();
        assert_eq!(
            payment.len(),
            5,
            "unsigned active, signed active, unsigned depleted, unsigned standby, signed standby"
        );
        for ((peer, phase, signer), expected) in payment
            .into_iter()
            .zip([&active, &active, &active, &standby, &standby])
        {
            let connection = records
                .iter()
                .find(|r| r.upstream_peer == Some(*peer))
                .unwrap();
            assert_eq!(
                (connection.user.clone(), connection.password.clone()),
                network::global().credentials(&IsolationId::evm(expected).unwrap()),
                "{phase}"
            );
            if phase == "signed" {
                assert_eq!(signer, expected);
            }
        }
        let balances = traces
            .iter()
            .filter(|(_, phase, _)| phase == "balance")
            .collect::<Vec<_>>();
        assert!(!balances.is_empty());
        for (peer, _, address) in balances {
            let connection = records
                .iter()
                .find(|r| r.upstream_peer == Some(*peer))
                .unwrap();
            assert_eq!(
                (connection.user.clone(), connection.password.clone()),
                network::global().credentials(&IsolationId::evm(address).unwrap())
            );
        }
        h.close().await;
    }
}

#[tokio::test]
async fn malformed_base_evidence_never_applies_a_partial_view() {
    for fault in [
        "id",
        "version",
        "error",
        "null",
        "missing",
        "height",
        "quantity",
        "oversized_quantity",
        "future",
        "word_short",
        "word_long",
        "word_invalid",
        "nonce",
        "final_hash",
    ] {
        let h = Harness::new().await;
        h.client.execute(h.route()).await.unwrap();
        let before = serde_json::to_value(h.store.call(|s| s.status()).await.unwrap()).unwrap();
        let count = h.f.signed.lock().unwrap().len();
        *h.f.rpc_fault.lock().unwrap() = Some(fault.into());
        h.f.used.store(true, Ordering::SeqCst); // Earlier nonce evidence alone must not release.
        assert!(h.client.execute(h.route()).await.is_err(), "{fault}");
        assert_eq!(h.f.signed.lock().unwrap().len(), count, "{fault}");
        assert_eq!(
            before,
            serde_json::to_value(h.store.call(|s| s.status()).await.unwrap()).unwrap(),
            "{fault}"
        );
        h.close().await;
    }
}

#[path = "support/concurrency.rs"]
mod concurrency;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_signed_response_does_not_block_shared_pool_or_reconciliation() {
    let h = Harness::new().await;
    let pool = make_pool(h.store.clone(), h.pool.clone(), &h.base);
    let client = PaidClient::managed(pool.clone());
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    h.f.hold.store(true, Ordering::SeqCst);
    let first_client = client.clone();
    let route = h.route();
    let first = tokio::spawn(async move { first_client.execute(route).await });
    concurrency::bounded(h.f.arrived.notified()).await;
    assert_eq!(
        concurrency::bounded(client.execute(h.route()))
            .await
            .unwrap(),
        "paid"
    );
    concurrency::bounded(pool.reconcile()).await.unwrap();
    assert!(!first.is_finished());
    concurrency::assert_liabilities(&h, 2, 2_000_000);
    // The first request is still unresolved. Neither a second success nor
    // background reconciliation makes its reserved balance spendable.
    *h.f.challenge.lock().unwrap() = challenge("4000000");
    assert!(
        concurrency::bounded(client.execute(h.route()))
            .await
            .unwrap_err()
            .to_string()
            .contains("payment_pending")
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    h.f.release.notify_one();
    concurrency::bounded(pool.reconcile()).await.unwrap();
    concurrency::assert_liabilities(&h, 2, 2_000_000);
    assert_eq!(
        h.store.call(|s| s.status()).await.unwrap().pools[0].generation,
        0
    );
    drop(client);
    drop(pool);
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confirmed_payment_can_rotate_while_original_seller_response_is_pending() {
    let h = Harness::new().await;
    h.f.hold.store(true, Ordering::SeqCst);
    let client = h.client.clone();
    let route = h.route();
    let first = tokio::spawn(async move { client.execute(route).await });
    concurrency::bounded(h.f.arrived.notified()).await;
    let payer = h.f.signed.lock().unwrap()[0]["payload"]["authorization"]["from"]
        .as_str()
        .unwrap()
        .parse::<Address>()
        .unwrap()
        .to_string();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(payer, 0);
    assert_eq!(
        concurrency::bounded(h.client.execute(h.route()))
            .await
            .unwrap(),
        "paid"
    );
    assert!(!first.is_finished());
    let status = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.pools[0].generation, 1);
    assert_eq!(status.pools[0].addresses.len(), 3);
    {
        let signed = h.f.signed.lock().unwrap();
        assert_eq!(signed.len(), 2);
        assert_ne!(
            signatures::recover_exact(&signed[0]),
            signatures::recover_exact(&signed[1])
        );
    }
    h.f.release.notify_one();
    assert_eq!(concurrency::bounded(first).await.unwrap().unwrap(), "paid");
    concurrency::assert_liabilities(&h, 1, 3_000_000);
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    assert_eq!(
        h.store.call(|s| s.status()).await.unwrap().pools[0].generation,
        1
    );
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_chain_reconciliation_allows_waiting_admission_without_blocking_other_pools() {
    let h = Harness::new().await;
    let pool = make_pool(h.store.clone(), h.pool.clone(), &h.base);
    let client = PaidClient::managed(pool.clone());
    let other = concurrency::other_pool(&h).await;
    h.f.rpc_hold.store(true, Ordering::SeqCst);
    let reconciling = pool.clone();
    let reconciliation = tokio::spawn(async move { reconciling.reconcile().await });
    concurrency::bounded(h.f.arrived.notified()).await;
    assert_eq!(
        concurrency::bounded(other.execute(h.route()))
            .await
            .unwrap(),
        "paid"
    );
    let waiting = client.clone();
    let route = h.route();
    let mut admission = tokio::spawn(async move { waiting.execute(route).await });
    // Waiting for the pool lock must survive the former 30-second default.
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(31), &mut admission)
            .await
            .is_err()
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    h.f.release.notify_one();
    concurrency::bounded(reconciliation).await.unwrap().unwrap();
    assert_eq!(
        concurrency::bounded(admission).await.unwrap().unwrap(),
        "paid"
    );
    concurrency::assert_liabilities(&h, 2, 6_000_000);
    drop(other);
    drop(client);
    drop(pool);
    h.close().await;
}

#[tokio::test]
async fn qualification_guard_allows_active_calls_but_blocks_replacement_before_signing() {
    let h = Harness::new().await;
    h.store
        .call(|s| {
            s.deny_new_funding();
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    let before = h.store.call(|s| s.status()).await.unwrap();
    let active = before.pools[0]
        .addresses
        .iter()
        .find(|a| a.role == "ACTIVE")
        .unwrap();
    h.f.balances
        .lock()
        .unwrap()
        .insert(active.address.clone(), 2_000_000);
    h.f.used.store(true, Ordering::SeqCst);
    // Parallel refusals must not rotate, allocate or sign; confirmed reconciliation survives.
    let (a, b) = tokio::join!(h.client.execute(h.route()), h.client.execute(h.route()));
    for result in [a, b] {
        let error = result.unwrap_err().to_string();
        assert!(error.contains("qualification_funding_denied"), "{error}");
        assert!(error.contains("limit is zero"), "{error}");
    }
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    let after = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(after.pools[0].generation, 0);
    assert_eq!(after.pools[0].addresses.len(), 2);
    assert_eq!(
        after.pools[0]
            .addresses
            .iter()
            .find(|a| a.role == "ACTIVE")
            .unwrap()
            .id,
        active.id
    );
    assert_eq!(after.funding_jobs.len(), before.funding_jobs.len());
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let rows: Vec<String> = db
        .prepare("SELECT state FROM payment_attempts")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(rows, ["RESOLVED"]);
    // A cheaper call still uses the original active wallet after the refusal.
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    h.close().await;
}

#[cfg(feature = "zcash")]
#[tokio::test]
async fn qualification_deployment_installs_guard_before_bootstrap_even_with_auto_fund() {
    use x402_treazury::rotation::restriction::FundingRestriction;
    use x402_treazury::{deployment::Deployment, treasury::Treasury};
    for existing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let owner = Treasury::create(state.clone(), key.clone(), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        if existing {
            owner
                .ensure_pool("research".into(), "5".into())
                .await
                .unwrap();
        }
        let baseline = owner.status().await.unwrap();
        let id = baseline.treasury_id.clone();
        owner.close().await.unwrap();
        std::fs::write(
            dir.path().join("spec.json"),
            r#"{"servers":[{"url":"http://127.0.0.1:1"}],"paths":{"/pay":{"get":{}}}}"#,
        )
        .unwrap();
        let config = format!(
            r#"version=1
[treasury]
id="{id}"
state_dir="state"
key_file="key"
indexer_url_env="INDEXER"
submission_url_env="SUBMIT"
daily_treasury_spend_limit_zec="0.1"
max_refund_shielding_fee_zec="0.001"
[funding]
auto_fund=true
base_rpc_url_env="BASE"
base_rpc_fallback_url_envs=[]
[wallets.research]
mode="zcash_rotation"
funding_amount_usdc="5"
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[sources.api]
spec="spec.json"
probe_pricing=false
[servers.one]
listen="127.0.0.1:0"
bearer_token_env="TOKEN"
wallet="research"
sources=["api"]
"#
        );
        let path = dir.path().join("servers.toml");
        std::fs::write(&path, config).unwrap();
        let env = BTreeMap::from([
            ("TOKEN".into(), "fixture-token".into()),
            ("INDEXER".into(), "http://127.0.0.1:1".into()),
            ("SUBMIT".into(), "http://127.0.0.1:1".into()),
            ("BASE".into(), "http://127.0.0.1:1".into()),
        ]);
        let result = Deployment::load(&path)
            .await
            .unwrap()
            .bind_restricted(&env, FundingRestriction::DenyNewFunding)
            .await;
        match result {
            Ok(running) => {
                assert!(existing);
                drop(running);
            }
            Err(e) => {
                assert!(!existing);
                assert!(
                    e.to_string().contains("qualification_funding_denied"),
                    "{e:#}"
                );
            }
        }
        // Binding starts no workers; all fixtures are local files/unfunded wallets.
        // Wait for asynchronous owner teardown, then verify no new allocation survived.
        let reopened = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(store) = Store::open(&state, &key, &id) {
                    break store;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(reopened.status().unwrap().pools).unwrap(),
            serde_json::to_value(baseline.pools).unwrap()
        );
    }
}

#[tokio::test]
async fn explicit_target_ceiling_opt_out_uses_api_cap_and_confirmed_capacity() {
    let mut h = Harness::new().await;
    for balance in h.f.balances.lock().unwrap().values_mut() {
        *balance = 6_000_000;
    }
    let manager = ManagedPool::new(
        h.store.clone(),
        h.pool.clone(),
        BaseRpc::new(&format!("{}/rpc", h.base), 12, 120).unwrap(),
        "5",
        SpendPolicy::dollars("6").unwrap(),
    )
    .unwrap()
    .with_funding_target_limit(false);
    h.client = PaidClient::managed(Arc::new(manager));
    *h.f.challenge.lock().unwrap() = challenge("5500000");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    *h.f.challenge.lock().unwrap() = challenge("6000001");
    assert!(h.client.execute(h.route()).await.is_err());
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    h.close().await;
}

#[tokio::test]
async fn slow_historical_authorization_sweep_acquires_fresh_admission_balances() {
    let mut h = Harness::new().await;
    *h.f.challenge.lock().unwrap() = challenge("1");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    h.f.nonce_delay.store(true, Ordering::SeqCst);
    let manager = ManagedPool::new(
        h.store.clone(),
        h.pool.clone(),
        BaseRpc::new(&format!("{}/rpc", h.base), 12, 1).unwrap(),
        "5",
        SpendPolicy::dollars("5").unwrap(),
    )
    .unwrap();
    h.client = PaidClient::managed(Arc::new(manager));
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    h.close().await;
}

#[tokio::test]
async fn nonce_outages_retain_liabilities_but_allow_remaining_capacity_across_restart() {
    let mut h = Harness::new().await;
    let fallback = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fallback_url = format!("http://{}", fallback.local_addr().unwrap());
    let fallback_calls = Arc::new(AtomicUsize::new(0));
    let calls = fallback_calls.clone();
    let app = Router::new().fallback(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::SERVICE_UNAVAILABLE }
    });
    let fallback_server = tokio::spawn(async move { axum::serve(fallback, app).await.unwrap() });
    h.client = PaidClient::managed(Arc::new(
        ManagedPool::new(
            h.store.clone(),
            h.pool.clone(),
            BaseRpc::with_fallbacks(&[format!("{}/rpc", h.base), fallback_url], 12, 120).unwrap(),
            "5",
            SpendPolicy::dollars("none").unwrap(),
        )
        .unwrap(),
    ));
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    h.client.execute(h.route()).await.unwrap();
    *h.f.rpc_fault.lock().unwrap() = Some("nonce_unavailable".into());
    // A failed observation cannot consume the remaining four USDC.
    h.client.execute(h.route()).await.unwrap();
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let pending = || {
        db.query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(pending(), 2);
    // There is still ample capacity, but malformed evidence must not be
    // downgraded to an availability failure and silently retained.
    *h.f.rpc_fault.lock().unwrap() = Some("nonce".into());
    assert!(h.client.execute(h.route()).await.is_err());
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    *h.f.rpc_fault.lock().unwrap() = Some("nonce_unavailable".into());
    *h.f.challenge.lock().unwrap() = challenge("4000000");
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("payment_pending")
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    // Reconciliation remains an explicitly failed operation while observations
    // are unavailable; it cannot report completed recovery or release anything.
    assert!(
        make_pool(h.store.clone(), h.pool.clone(), &h.base)
            .reconcile()
            .await
            .is_err()
    );
    assert_eq!(pending(), 2);
    // Retain one failed nonce while committing another's actual canonical proof.
    *h.f.rpc_fault.lock().unwrap() = Some("first_nonce_unavailable".into());
    h.f.used.store(true, Ordering::SeqCst);
    h.client.execute(h.route()).await.unwrap();
    assert_eq!(pending(), 2);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM payment_resolutions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 3);
    assert_eq!(
        h.store
            .call(|s| Ok(s.status()?.pools[0].generation))
            .await
            .unwrap(),
        0
    );
    drop(db);
    let id = h.store.call(|s| Ok(s.id().to_owned())).await.unwrap();
    drop(h.client);
    drop(h.store);
    h.worker.await.unwrap();
    let store = Store::open(&h.dir.path().join("state"), &h.dir.path().join("key"), &id).unwrap();
    (h.store, h.worker) = StoreHandle::spawn(store);
    h.client = make_client(h.store.clone(), h.pool.clone(), &h.base);
    *h.f.rpc_fault.lock().unwrap() = Some("nonce_unavailable".into());
    *h.f.challenge.lock().unwrap() = challenge("1");
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("payment_pending")
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 3);
    *h.f.rpc_fault.lock().unwrap() = None;
    h.client.execute(h.route()).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap().len(), 4);
    assert_eq!(
        fallback_calls.load(Ordering::SeqCst),
        0,
        "an optional nonce outage must not require another provider to authorize remaining capacity"
    );
    fallback_server.abort();
    h.close().await;
}

#[tokio::test]
async fn slow_balance_collection_retains_resolution_without_signing_or_looping() {
    let mut h = Harness::new().await;
    *h.f.challenge.lock().unwrap() = challenge("1");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balance_delay.store(true, Ordering::SeqCst);
    let manager = Arc::new(
        ManagedPool::new(
            h.store.clone(),
            h.pool.clone(),
            BaseRpc::new(&format!("{}/rpc", h.base), 12, 1).unwrap(),
            "5",
            SpendPolicy::dollars("5").unwrap(),
        )
        .unwrap(),
    );
    h.client = PaidClient::managed(manager.clone());
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        h.client.execute(h.route()),
    )
    .await
    .expect("completed balance work must return, not restart")
    .unwrap_err();
    assert!(
        error.to_string().contains("fresh Base balances required"),
        "{error:#}"
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let resolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='RESOLVED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        resolved, 1,
        "canonical resolution persists despite stale admission balances"
    );
    let attempts: i64 = db
        .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(attempts, 1, "stale balances grant no new reservation");
    // Background reconciliation also completes, retaining the original block
    // evidence. Once fresh reads are possible the pool gate is immediately usable.
    tokio::time::timeout(std::time::Duration::from_secs(8), manager.reconcile())
        .await
        .unwrap()
        .unwrap();
    h.f.balance_delay.store(false, Ordering::SeqCst);
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    drop(db);
    drop(manager);
    h.close().await;
}
