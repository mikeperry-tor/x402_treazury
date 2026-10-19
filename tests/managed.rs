//! Offline end-to-end admission against a local Base RPC and x402 seller.
#[path = "support/capacity.rs"]
mod capacity_fixture;
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
struct ChainSnapshot {
    balances: BTreeMap<String, u64>,
    used: bool,
    timestamp: u64,
}
#[derive(Default)]
struct ChainHistory {
    latest: u64,
    blocks: BTreeMap<u64, ChainSnapshot>,
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
    balance_unavailable: Arc<Mutex<std::collections::BTreeSet<String>>>,
    balance_delays: Arc<Mutex<BTreeMap<String, u64>>>,
    balance_gates: Arc<Mutex<BTreeMap<String, Arc<tokio::sync::Semaphore>>>>,
    chain: Arc<Mutex<ChainHistory>>,
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
    body_only: Arc<AtomicBool>,
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
    if v["method"] == "eth_call"
        && let Some(data) = v["params"][0]["data"]
            .as_str()
            .filter(|data| data.starts_with("0x70a08231"))
    {
        let wallet = format!("0x{}", &data[data.len() - 40..])
            .parse::<Address>()
            .unwrap()
            .to_string();
        let unavailable = f.balance_unavailable.lock().unwrap().contains(&wallet);
        let gate = f.balance_gates.lock().unwrap().get(&wallet).cloned();
        if let Some(gate) = gate {
            let _ = gate.acquire().await;
        }
        let delay = f.balance_delays.lock().unwrap().get(&wallet).copied();
        if let Some(delay) = delay {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
        if unavailable {
            return Json(
                json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32005,"message":"fixture wallet unavailable"}}),
            );
        }
    }
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
            let mut chain = f.chain.lock().unwrap();
            let height = if v["params"][0] == "latest" {
                // Every acquisition has a new confirmed head. Historical hashes
                // retain their exact balances, even after settlement changes the
                // fixture's current wallet balances.
                let next = if chain.latest == 0 {
                    100
                } else {
                    chain.latest + 24
                };
                let snapshot = ChainSnapshot {
                    balances: f.balances.lock().unwrap().clone(),
                    used: f.used.load(Ordering::SeqCst),
                    timestamp: now().unwrap(),
                };
                for height in chain.latest + 1..=next {
                    chain.blocks.insert(height, snapshot.clone());
                }
                chain.latest = next;
                next
            } else {
                u64::from_str_radix(
                    v["params"][0].as_str().unwrap().trim_start_matches("0x"),
                    16,
                )
                .unwrap()
            };
            let timestamp = chain
                .blocks
                .get(&height)
                .map_or_else(|| now().unwrap(), |block| block.timestamp);
            let fork = u64::from(f.reorg.load(Ordering::SeqCst));
            json!({"number":format!("0x{height:x}"),"hash":format!("0x{:064x}",height*2+fork),"timestamp":format!("0x{:x}",timestamp-if f.stale.load(Ordering::SeqCst){1000}else{0})})
        }
        "eth_call" => {
            let data = v["params"][0]["data"].as_str().unwrap();
            let snapshot = if v["params"][1] == "latest" {
                assert!(data.starts_with("0x70a08231"));
                ChainSnapshot {
                    balances: f.balances.lock().unwrap().clone(),
                    used: f.used.load(Ordering::SeqCst),
                    timestamp: now().unwrap(),
                }
            } else {
                assert_eq!(v["params"][1]["requireCanonical"], true);
                let hash = u64::from_str_radix(
                    v["params"][1]["blockHash"]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("0x"),
                    16,
                )
                .unwrap();
                f.chain
                    .lock()
                    .unwrap()
                    .blocks
                    .get(&(hash / 2))
                    .cloned()
                    .expect("known fixture block")
            };
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
                *snapshot.balances.get(&address).unwrap_or(&0)
            } else {
                u64::from(snapshot.used)
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
    if let Some(h) = headers
        .get("payment-signature")
        .or_else(|| headers.get("x-payment"))
    {
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
    if f.body_only.load(Ordering::SeqCst) {
        return (
            StatusCode::PAYMENT_REQUIRED,
            axum::Json(f.challenge.lock().unwrap().clone()),
        )
            .into_response();
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
        let pool = crate::capacity_fixture::declare_and_allocate!(
            store,
            "research",
            crate::capacity_fixture::two_reserves("5"),
            Some(alloy_primitives::U256::from(5_000_000u64)),
            0
        )
        .unwrap();
        let f = Fake {
            balances: Arc::default(),
            challenge: Arc::new(Mutex::new(challenge("3000000"))),
            signed: Arc::default(),
            rpc_calls: Arc::default(),
            rpc_hold: Arc::default(),
            nonce_delay: Arc::default(),
            balance_delay: Arc::default(),
            balance_unavailable: Arc::default(),
            balance_delays: Arc::default(),
            balance_gates: Arc::default(),
            chain: Arc::default(),
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
            body_only: Arc::default(),
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
        publish_initial_credit(&store, &pool, &base).await;
        f.rpc_calls.store(0, Ordering::SeqCst);
        f.confirmed_reads.store(0, Ordering::SeqCst);
        f.traces.lock().unwrap().clear();
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
// Establish prior initial funding through its owner, not ordinary hint refresh.
async fn publish_initial_credit(store: &StoreHandle, pool: &str, base: &str) {
    let status = store.call(|s| s.status()).await.unwrap();
    let wallets: Vec<_> = status
        .pools
        .iter()
        .find(|p| p.id == pool)
        .unwrap()
        .addresses
        .iter()
        .filter(|w| w.role == "ALLOCATED")
        .map(|w| w.id.clone())
        .collect();
    for wallet in wallets {
        let pool = pool.to_owned();
        let query = store.call(move |s| s.chain_query(&pool)).await.unwrap();
        let view = BaseRpc::new(&format!("{base}/rpc"), 12)
            .unwrap()
            .view(query)
            .await
            .unwrap();
        store
            .call(move |s| s.record_verified_credit(&wallet, view).map(|_| ()))
            .await
            .unwrap();
    }
}
async fn restart(h: Harness) -> Harness {
    let Harness {
        dir,
        f,
        store,
        worker,
        pool,
        client,
        url,
        base,
        server,
    } = h;
    let id = store.call(|s| Ok(s.id().to_owned())).await.unwrap();
    client.drain().await;
    drop(client);
    drop(store);
    worker.await.unwrap();
    let state = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let client = make_client(store.clone(), pool.clone(), &base);
    Harness {
        dir,
        f,
        store,
        worker,
        pool,
        client,
        url,
        base,
        server,
    }
}
async fn exhaust_ready(h: &Harness) {
    let payer = make_pool(h.store.clone(), h.pool.clone(), &h.base)
        .candidate()
        .await
        .unwrap();
    let status = h.store.call(|s| s.status()).await.unwrap();
    for wallet in status
        .pools
        .iter()
        .find(|p| p.id == h.pool)
        .unwrap()
        .addresses
        .iter()
        .filter(|w| w.role == "READY" && w.id != payer.wallet)
    {
        h.f.balances
            .lock()
            .unwrap()
            .insert(wallet.address.clone(), 0);
    }
    make_pool(h.store.clone(), h.pool.clone(), &h.base)
        .reconcile()
        .await
        .unwrap();
}
// Independently completed balance reads need not be simultaneously compatible
// with a history proof committed by another read in this same bounded pass.
fn assert_balance_observation_outcome(result: anyhow::Result<()>) {
    if let Err(error) = result {
        use x402_treazury::rotation::base::evidence::EvidenceFailure;
        assert!(
            matches!(
                error.downcast_ref::<EvidenceFailure>(),
                Some(EvidenceFailure::BeforeDebit | EvidenceFailure::CompatibilityRequired)
            ),
            "{error:#}"
        );
    }
}
async fn reconcile_complete_history(h: &Harness) {
    let pool = h.pool.clone();
    let query = h.store.call(move |s| s.chain_query(&pool)).await.unwrap();
    let view = BaseRpc::new(&format!("{}/rpc", h.base), 12)
        .unwrap()
        .view(query)
        .await
        .unwrap();
    let pool = h.pool.clone();
    h.store
        .call(move |s| s.reconcile_pool(&pool, view))
        .await
        .unwrap();
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let remaining: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining, 0,
        "explicit canonical history must resolve the original attempts"
    );
}
fn make_pool(store: StoreHandle, pool: String, base: &str) -> Arc<ManagedPool> {
    let manager = ManagedPool::new(
        store,
        pool,
        BaseRpc::new(&format!("{base}/rpc"), 12).unwrap(),
        SpendPolicy::dollars("none").unwrap(),
    )
    .unwrap();
    Arc::new(manager)
}
fn make_client(store: StoreHandle, pool: String, base: &str) -> PaidClient {
    PaidClient::managed(make_pool(store, pool, base))
}
#[tokio::test]
async fn concurrent_calls_journal_unique_attempts_without_a_local_balance_ceiling() {
    let h = Harness::new().await;
    let before = h.store.call(|s| s.status()).await.unwrap();
    let (a, b) = tokio::join!(h.client.execute(h.route()), h.client.execute(h.route()));
    assert_eq!(a.unwrap(), "paid");
    assert_eq!(b.unwrap(), "paid");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    concurrency::assert_liabilities(&h, 3, 9_000_000);
    let signed = h.f.signed.lock().unwrap().clone();
    assert_eq!(signed.len(), 3);
    let nonces: std::collections::BTreeSet<_> = signed
        .iter()
        .map(|p| p["payload"]["authorization"]["nonce"].as_str().unwrap())
        .collect();
    assert_eq!(nonces.len(), 3);
    let after = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        after.pools[0].addresses.len(),
        before.pools[0].addresses.len()
    );
    assert_eq!(after.funding_jobs.len(), before.funding_jobs.len());
    assert_eq!(h.f.rpc_calls.load(Ordering::SeqCst), 0);
    h.close().await;
}
#[tokio::test]
async fn confirmed_depletion_consumes_ready_independently_of_failed_refill() {
    let h = Harness::new().await;
    h.client.execute(h.route()).await.unwrap();
    let active = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(active, 0);
    reconcile_complete_history(&h).await;
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    db.execute_batch("CREATE TRIGGER fail_replacement BEFORE INSERT ON funding_jobs BEGIN SELECT RAISE(ABORT,'injected replacement failure'); END;").unwrap();
    h.client.execute(h.route()).await.unwrap();
    let state = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(state.pools[0].generation, 0);
    assert_eq!(state.pools[0].addresses.len(), 2);
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    let pool = h.pool.clone();
    assert!(
        h.store
            .call(move |s| s.allocate_capacity_kind(
                &pool,
                x402_treazury::rotation::store::AllocationKind::Small,
                true
            ))
            .await
            .is_err()
    );
    db.execute_batch("DROP TRIGGER fail_replacement;").unwrap();
    let pool = h.pool.clone();
    assert!(
        h.store
            .call(move |s| s.allocate_capacity_kind(
                &pool,
                x402_treazury::rotation::store::AllocationKind::Small,
                true
            ))
            .await
            .unwrap()
            .is_some()
    );
    let state = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(state.pools[0].addresses.len(), 3);
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
        .with_max_level(tracing::Level::DEBUG)
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
            c["resource"]["description"] = json!("short public description");
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
    let omissions: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("omitting advertised x402 extension"))
        .collect();
    assert!(!omissions.is_empty(), "{logs}");
    assert!(
        omissions.iter().all(|line| line.contains("INFO")
            && line.contains("call_id=")
            && line.contains("source=\"standalone\"")
            && line.contains("method=\"GET\"")),
        "{logs}"
    );
    assert!(logs.contains("Payment call completed"), "{logs}");
    assert!(!logs.contains("WARN"), "{logs}");
    assert!(!logs.contains("private-extension-value"));
}
#[tokio::test]
async fn managed_failure_diagnostics_survive_cancellation_and_hide_upstream_prose() {
    use tracing::instrument::WithSubscriber;
    for cancel in [false, true] {
        let logs = LogCapture(Arc::default());
        let sink = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .with_max_level(if cancel {
                tracing::Level::WARN
            } else {
                tracing::Level::INFO
            })
            .finish();
        async {
            let h = Harness::new().await;
            {
                let mut c = h.f.challenge.lock().unwrap();
                c["resource"]["description"] = json!("short");
                c["error"] = json!("private-upstream-prose");
                c["extensions"] =
                    json!({"payment-identifier":{"secret":"private-extension-value"}});
            }
            h.f.reject.store(true, Ordering::SeqCst);
            h.f.hold.store(cancel, Ordering::SeqCst);
            let task = tokio::spawn({
                let client = h.client.clone();
                let mut route = h.route();
                route
                    .query
                    .insert("credential".into(), json!("private-query-value"));
                async move {
                    if cancel {
                        client.execute(route).await.map(|_| ())
                    } else {
                        let tools = x402_treazury::catalog::build_tools(
                            &x402_treazury::catalog::Config::default(),
                            &json!({"paths":{"/pay":{"get":{"operationId":"diagnostic_call"}}}}),
                            "fixture",
                        )
                        .unwrap();
                        let mut bound = x402_treazury::catalog_state::bind(vec![(
                            tools[0].clone(),
                            client,
                            route.url.trim_end_matches("/pay").to_owned(),
                        )])
                        .pop()
                        .unwrap();
                        bound.source = Some(("registered-fixture".into(), 1));
                        bound.invoke_output(&Default::default()).await.map(|_| ())
                    }
                }
                .with_current_subscriber()
            });
            if cancel {
                concurrency::bounded(h.f.arrived.notified()).await;
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                h.f.release.notify_one();
            } else {
                assert!(task.await.unwrap().is_err());
            }
            h.client.drain().await;
            assert_eq!(h.f.signed.lock().unwrap().len(), 1);
            h.close().await;
        }
        .with_subscriber(subscriber)
        .await;
        let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
        assert_eq!(warnings.len(), 1, "{logs}");
        let warning = warnings[0];
        assert_eq!(warning.matches("stage=").count(), 1, "{logs}");
        assert_eq!(warning.matches("last_http_status=").count(), 1, "{logs}");
        for expected in [
            "Payment call failed",
            "call_id=",
            "last_http_status=402",
            "stage=\"response_body\"",
            "authorization_may_charge=true",
            "failure_kind=\"http_status\"",
        ] {
            assert!(warning.contains(expected), "{logs}");
        }
        if cancel {
            assert!(warning.contains("source=\"standalone\""), "{logs}");
        } else {
            assert!(warning.contains("source=\"registered-fixture\""), "{logs}");
            assert!(warning.contains("tool="), "{logs}");
        }
        for secret in [
            "private-upstream-prose",
            "private-extension-value",
            "private-query-value",
            "127.0.0.1",
            "payment-signature",
        ] {
            assert!(!logs.contains(secret), "{logs}");
        }
    }
}

#[tokio::test]
async fn direct_managed_failure_diagnostics_distinguish_refusal_and_rejection() {
    use tracing::instrument::WithSubscriber;
    let logs = LogCapture(Arc::default());
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .with_max_level(tracing::Level::INFO)
        .finish();
    async {
        let h = Harness::new().await;
        let manager = make_pool(h.store.clone(), h.pool.clone(), &h.base);
        let http = reqwest::Client::new();
        let mut c = challenge("1");
        c["resource"]["description"] = json!("short");
        c["extensions"] = json!({"agentkit":{"secret":"private-extension-value"}});
        let response = |c: &Value| {
            reqwest::Response::from(
                axum::http::Response::builder()
                    .status(402)
                    .header("payment-required", STANDARD.encode(c.to_string()))
                    .body("")
                    .unwrap(),
            )
        };
        let mut malformed = c.clone();
        malformed["accepts"] = json!([]);
        assert!(
            manager
                .pay(
                    &http,
                    manager.candidate().await.unwrap(),
                    reqwest::Request::new(reqwest::Method::GET, h.url.parse().unwrap()),
                    response(&malformed)
                )
                .await
                .is_err()
        );
        assert!(h.f.signed.lock().unwrap().is_empty());
        h.f.reject.store(true, Ordering::SeqCst);
        let (paid, _) = manager
            .pay(
                &http,
                manager.candidate().await.unwrap(),
                reqwest::Request::new(reqwest::Method::GET, h.url.parse().unwrap()),
                response(&c),
            )
            .await
            .unwrap();
        assert_eq!(paid.status(), StatusCode::PAYMENT_REQUIRED);
        assert_eq!(h.f.signed.lock().unwrap().len(), 1);
        drop(manager);
        h.close().await;
    }
    .with_subscriber(subscriber)
    .await;
    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
    assert_eq!(warnings.len(), 2, "{logs}");
    assert!(
        warnings[0].contains("stage=\"challenge_preparation\""),
        "{logs}"
    );
    assert!(
        warnings[0].contains("authorization_may_charge=false"),
        "{logs}"
    );
    assert!(warnings[1].contains("last_http_status=402"), "{logs}");
    assert!(
        warnings[1].contains("authorization_may_charge=true"),
        "{logs}"
    );
    assert!(
        warnings.iter().all(|line| line.contains("call_id=")),
        "{logs}"
    );
    assert!(!logs.contains("private-extension-value"), "{logs}");
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
async fn direct_managed_calls_bound_body_challenges_and_drop_unused_header_bodies() {
    let h = Harness::new().await;
    let manager = ManagedPool::new(
        h.store.clone(),
        h.pool.clone(),
        BaseRpc::new(&format!("{}/rpc", h.base), 12).unwrap(),
        SpendPolicy::dollars("5").unwrap(),
    )
    .unwrap();
    let http = reqwest::Client::new();
    let retry = || reqwest::Request::new(reqwest::Method::GET, h.url.parse().unwrap());
    let oversized = reqwest::Response::from(
        axum::http::Response::builder()
            .status(402)
            .header("content-length", x402_treazury::limits::RESPONSE_BYTES + 1)
            .body("")
            .unwrap(),
    );
    assert!(
        manager
            .pay(
                &http,
                manager.candidate().await.unwrap(),
                retry(),
                oversized
            )
            .await
            .is_err()
    );
    assert!(h.f.signed.lock().unwrap().is_empty());
    let c = challenge("1");
    for header in [true, false] {
        let response = if header {
            // Neither a claimed oversized body nor an unreadable body
            // can obstruct a complete direct header challenge.
            struct UnreadableBody;
            impl http_body::Body for UnreadableBody {
                type Data = axum::body::Bytes;
                type Error = std::io::Error;
                fn poll_frame(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>>
                {
                    panic!("unused direct challenge body must never be polled")
                }
            }
            let body = reqwest::Body::wrap(UnreadableBody);
            reqwest::Response::from(
                axum::http::Response::builder()
                    .status(402)
                    .header("content-length", x402_treazury::limits::RESPONSE_BYTES + 1)
                    .header("payment-required", STANDARD.encode(c.to_string()))
                    .body(body)
                    .unwrap(),
            )
        } else {
            let mut v1 = c.clone();
            v1["x402Version"] = json!(1);
            let raw = &mut v1["accepts"][0];
            raw["network"] = json!("base");
            raw["maxAmountRequired"] = raw.as_object_mut().unwrap().remove("amount").unwrap();
            raw["resource"] = json!("http://localhost/pay");
            raw["description"] = json!("legacy direct fixture");
            reqwest::Response::from(
                axum::http::Response::builder()
                    .status(402)
                    .body(v1.to_string())
                    .unwrap(),
            )
        };
        let (paid, _) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            manager.pay(&http, manager.candidate().await.unwrap(), retry(), response),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(paid.status(), reqwest::StatusCode::OK);
    }
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    drop(manager);
    h.close().await;
}

#[tokio::test]
async fn unsupported_and_over_target_offers_have_no_admission_side_effects() {
    let h = Harness::new().await;
    for kind in [
        "v3",
        "v1_with_v2_offer",
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
            "v3" => c["x402Version"] = json!(3),
            "v1_with_v2_offer" => c["x402Version"] = json!(1),
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
    *h.f.challenge.lock().unwrap() = json!({
        "x402Version": 1,
        "accepts": [{
            "scheme": "exact",
            "network": "base",
            "asset": USDC,
            "maxAmountRequired": "1",
            "resource": "http://localhost/pay",
            "description": "v1 exact payment",
            "mimeType": "application/json",
            "payTo": "0x0000000000000000000000000000000000000003",
            "maxTimeoutSeconds": 60,
            "extra": {"name": "USD Coin", "version": "2"}
        }]
    });
    h.client.execute(h.route()).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap()[1]["x402Version"], 1);
    assert_eq!(h.f.signed.lock().unwrap()[1]["network"], "base");
    h.f.body_only.store(true, Ordering::SeqCst);
    h.client.execute(h.route()).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap().len(), 3);
    assert_eq!(h.f.signed.lock().unwrap()[2]["x402Version"], 1);
    h.close().await;
}
#[tokio::test]
async fn rejected_payment_stays_journaled_across_restart_without_automatic_replay() {
    let h = Harness::new().await;
    h.f.reject.store(true, Ordering::SeqCst);
    h.f.challenge.lock().unwrap()["extensions"] = json!({"unknown": {"secret": "not logged"}});
    let error = h.client.execute(h.route()).await.unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("omitted advertised x402 extensions"),
        "{message}"
    );
    assert!(message.contains("may still charge"), "{message}");
    assert!(message.contains("do not automatically repeat"), "{message}");
    assert!(message.contains("402"), "{message}");
    let original = h.f.signed.lock().unwrap().clone();
    assert_eq!(original.len(), 1);
    assert!(
        original[0]["extensions"]
            .as_object()
            .is_none_or(|m| m.is_empty())
    );
    let h = restart(h).await;
    assert_eq!(
        *h.f.signed.lock().unwrap(),
        original,
        "reopen never sends a request"
    );
    concurrency::assert_liabilities(&h, 1, 3_000_000);
    h.f.reject.store(false, Ordering::SeqCst);
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    concurrency::assert_liabilities(&h, 2, 6_000_000);
    assert_eq!(h.f.signed.lock().unwrap()[0], original[0]);
    h.close().await;
}
#[tokio::test]
async fn foreground_signing_does_not_fetch_stale_wrong_chain_or_reorg_evidence() {
    let h = restart(Harness::new().await).await;
    for flag in [&h.f.stale, &h.f.wrong_chain, &h.f.reorg] {
        flag.store(true, Ordering::SeqCst);
        assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
        flag.store(false, Ordering::SeqCst);
    }
    assert_eq!(h.f.rpc_calls.load(Ordering::SeqCst), 0);
    concurrency::assert_liabilities(&h, 3, 9_000_000);
    assert_eq!(
        h.store
            .call(|s| Ok(s.status()?.funding_jobs.len()))
            .await
            .unwrap(),
        2
    );
    h.close().await;
}
#[tokio::test]
async fn cancellation_after_submission_retains_liability() {
    let h = Harness::new().await;
    h.f.hold.store(true, Ordering::SeqCst);
    let client = h.client.clone();
    let route = h.route();
    let task = tokio::spawn(async move { client.execute(route).await });
    concurrency::bounded(h.f.arrived.notified()).await;
    let original = h.f.signed.lock().unwrap()[0].clone();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    h.f.release.notify_one();
    h.client.drain().await;
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    assert_eq!(h.f.signed.lock().unwrap()[0], original);
    concurrency::assert_liabilities(&h, 2, 6_000_000);
    h.close().await;
}
#[tokio::test]
async fn journal_failure_sends_no_signature_and_next_admission_recovers() {
    let h = Harness::new().await;
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    db.execute_batch("CREATE TRIGGER fail_journal BEFORE INSERT ON payment_attempts WHEN NEW.state='POSSIBLY_SUBMITTED' BEGIN SELECT RAISE(ABORT,'injected journal failure'); END;").unwrap();
    let before = h.store.call(|s| s.status()).await.unwrap();
    let error = h.client.execute(h.route()).await.unwrap_err();
    assert!(format!("{error:#}").contains("injected journal failure"));
    assert!(h.f.signed.lock().unwrap().is_empty());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "an untyped journal failure rolls back the entire attempt and cannot trigger private replacement"
    );
    let after = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        serde_json::to_value(&before.pools[0].addresses).unwrap(),
        serde_json::to_value(&after.pools[0].addresses).unwrap(),
        "journal failure must not retain exposure or mutate wallet identity"
    );
    assert_eq!(
        h.f.traces
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, kind, _)| kind == "unsigned")
            .count(),
        1,
        "failed call must not fetch another challenge"
    );
    db.execute_batch("DROP TRIGGER fail_journal;").unwrap();
    h.client.execute(h.route()).await.unwrap();
    let signed = h.f.signed.lock().unwrap().clone();
    assert_eq!(signed.len(), 1);
    let authorization = &signed[0]["payload"]["authorization"];
    let (payer, nonce, state): (String, String, String) = db
        .query_row("SELECT payer,nonce,state FROM payment_attempts", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(
        payer.to_lowercase(),
        authorization["from"].as_str().unwrap().to_lowercase()
    );
    assert_eq!(nonce, authorization["nonce"].as_str().unwrap());
    assert_eq!(state, "POSSIBLY_SUBMITTED");
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
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
max_refund_shielding_fee_zec="0.001"
[funding]
auto_fund=false
base_rpc_url_env="BASE"
base_rpc_fallback_url_envs=[]
[wallets.research]
mode="zcash_rotation"
max_api_payment_usdc="5"
reserves={{small={{funding_amount_usdc="5"}}}}
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[wallets.unused]
mode="zcash_rotation"
max_api_payment_usdc="5"
reserves={{small={{funding_amount_usdc="5"}}}}
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
    assert_eq!(state.pools[0].addresses.len(), 0);
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
    assert_eq!(t.status().await.unwrap().pools[0].addresses.len(), 0);
    t.close().await.unwrap();
    // A paused observer also resolves an omitted treasury identity and releases
    // all ownership on shutdown without allocating or contacting NEAR.
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
    // Omitted ID resolves from durable state.
    std::fs::write(
        &path,
        config.replace(&format!("id=\"{treasury_id}\"\n"), ""),
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
    let before_static_attempts = serde_json::to_value(
        x402_treazury::rotation::store::status(&dir.path().join("state")).unwrap(),
    )
    .unwrap();
    let mut static_config: toml::Value = toml::from_str(&config).unwrap();
    for name in ["research", "unused"] {
        static_config["wallets"][name] = toml::Value::try_from(
            json!({"mode":"static","private_key_env":"KEY","max_api_payment_usdc":"5"}),
        )
        .unwrap();
        assert_eq!(
            static_config["wallets"][name]["mode"].as_str(),
            Some("static")
        );
        std::fs::write(&path, toml::to_string(&static_config).unwrap()).unwrap();
        let error = match Deployment::load(&path).await.unwrap().bind(&env).await {
            Ok(_) => panic!("accepted managed-to-static identity change"),
            Err(e) => e.to_string(),
        };
        assert!(error.contains("cannot become static"), "{error}");
        assert_eq!(
            serde_json::to_value(
                x402_treazury::rotation::store::status(&dir.path().join("state")).unwrap()
            )
            .unwrap(),
            before_static_attempts
        );
    }
    h.close().await;
}

#[tokio::test]
async fn free_calls_do_not_touch_admission_and_funded_active_serves_independently() {
    let h = Harness::new().await;
    let before = h.f.rpc_calls.load(Ordering::SeqCst);
    h.f.free.store(true, Ordering::SeqCst);
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "free");
    assert_eq!(h.f.rpc_calls.load(Ordering::SeqCst), before);
    assert!(h.f.signed.lock().unwrap().is_empty());
    h.f.free.store(false, Ordering::SeqCst);
    let active = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    for (address, balance) in h.f.balances.lock().unwrap().iter_mut() {
        if address != &active {
            *balance = 0;
        }
    }
    make_pool(h.store.clone(), h.pool.clone(), &h.base)
        .reconcile()
        .await
        .unwrap();
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
    assert_eq!(
        h.store
            .call(|s| s.status())
            .await
            .unwrap()
            .funding_jobs
            .len(),
        2
    );
    h.close().await;
}
#[tokio::test]
async fn unused_expired_authorization_requires_explicit_canonical_resolution() {
    let h = Harness::new().await;
    h.f.challenge.lock().unwrap()["accepts"][0]["maxTimeoutSeconds"] = json!(5);
    h.f.reject.store(true, Ordering::SeqCst);
    assert!(h.client.execute(h.route()).await.is_err());
    let expiry = h.f.signed.lock().unwrap()[0]["payload"]["authorization"]["validBefore"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    while now().unwrap() <= expiry {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    h.f.stale.store(true, Ordering::SeqCst);
    make_pool(h.store.clone(), h.pool.clone(), &h.base)
        .reconcile()
        .await
        .unwrap();
    concurrency::assert_liabilities(&h, 1, 3_000_000);
    h.f.stale.store(false, Ordering::SeqCst);
    reconcile_complete_history(&h).await;
    concurrency::assert_liabilities(&h, 0, 0);
    h.f.reject.store(false, Ordering::SeqCst);
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    h.close().await;
}
#[tokio::test]
async fn pending_exposure_does_not_block_independent_permitted_calls() {
    let h = Harness::new().await;
    exhaust_ready(&h).await;
    let other_id = h
        .store
        .call(|s| {
            crate::capacity_fixture::declare_and_allocate!(
                s,
                "other",
                crate::capacity_fixture::two_reserves("5"),
                Some(alloy_primitives::U256::from(5_000_000u64)),
                0
            )
        })
        .await
        .unwrap();
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
    publish_initial_credit(&h.store, &other_id, &h.base).await;
    make_pool(h.store.clone(), other_id.clone(), &h.base)
        .reconcile()
        .await
        .unwrap();
    let other = make_client(h.store.clone(), other_id, &h.base);
    h.f.hold.store(true, Ordering::SeqCst);
    let client = h.client.clone();
    let route = h.route();
    let task = tokio::spawn(async move { client.execute(route).await });
    h.f.arrived.notified().await;
    other.execute(h.route()).await.unwrap();
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 3);
    concurrency::assert_liabilities(&h, 3, 9_000_000);
    task.abort();
    let _ = task.await;
    h.f.release.notify_one();
    drop(other);
    h.close().await;
}
#[tokio::test]
async fn configured_payment_cap_allows_attempts_above_estimated_funded_capacity() {
    let h = Harness::new().await;
    h.client.execute(h.route()).await.unwrap();
    let active = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    h.f.balances.lock().unwrap().insert(active, 0);
    h.f.used.store(true, Ordering::SeqCst);
    reconcile_complete_history(&h).await;
    h.store
        .call(|s| {
            crate::capacity_fixture::declare_and_allocate!(
                s,
                "research",
                crate::capacity_fixture::two_reserves("10"),
                Some(alloy_primitives::U256::from(10_000_000u64)),
                0
            )
        })
        .await
        .unwrap();
    *h.f.challenge.lock().unwrap() = challenge("6000000");
    let client = PaidClient::managed(Arc::new(
        ManagedPool::new(
            h.store.clone(),
            h.pool.clone(),
            BaseRpc::new(&format!("{}/rpc", h.base), 12).unwrap(),
            SpendPolicy::dollars("none").unwrap(),
        )
        .unwrap(),
    ));
    assert_eq!(client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(
        h.store.call(|s| s.status()).await.unwrap().pools[0].generation,
        0
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    drop(client);
    h.close().await;
}

#[tokio::test]
async fn ordinary_balance_refresh_preserves_unresolved_authorizations() {
    let h = Harness::new().await;
    h.client.execute(h.route()).await.unwrap();
    let payer = signatures::recover_exact(&h.f.signed.lock().unwrap()[0]).to_string();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(payer.clone(), 0);
    let manager = make_pool(h.store.clone(), h.pool.clone(), &h.base);
    manager.reconcile().await.unwrap();
    concurrency::assert_liabilities(&h, 1, 3_000_000);
    let state = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        state.pools[0]
            .addresses
            .iter()
            .find(|w| w.address == payer)
            .unwrap()
            .confirmed_balance,
        "0"
    );
    h.f.reorg.store(true, Ordering::SeqCst);
    manager.reconcile().await.unwrap();
    concurrency::assert_liabilities(&h, 1, 3_000_000);
    assert_eq!(h.f.signed.lock().unwrap().len(), 1);
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
    confirmed_depletion_consumes_ready_independently_of_failed_refill();
    concurrent_calls_journal_unique_attempts_without_a_local_balance_ceiling();
    unsupported_and_over_target_offers_have_no_admission_side_effects();
    managed_extensions_are_stripped_logged_and_do_not_change_authorization();
    rejected_payment_stays_journaled_across_restart_without_automatic_replay();
    free_calls_do_not_touch_admission_and_funded_active_serves_independently();
    post_handoff_gets_one_new_challenge_and_signs_once();
}

#[tokio::test]
async fn post_handoff_gets_one_new_challenge_and_signs_once() {
    let h = Harness::new().await;
    h.client.execute(h.route()).await.unwrap();
    let active = h.store.call(|s| s.status()).await.unwrap().pools[0].addresses[0]
        .address
        .clone();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(active, 0);
    reconcile_complete_history(&h).await;
    let before =
        h.f.traces
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, kind, _)| kind == "unsigned")
            .count();
    let mut route = h.route();
    route.method = "POST".into();
    h.client.execute(route).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    assert_eq!(
        h.store.call(|s| s.status()).await.unwrap().pools[0].generation,
        0
    );
    let after =
        h.f.traces
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, kind, _)| kind == "unsigned")
            .count();
    assert_eq!(after - before, 2);
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
        reconcile_complete_history(&h).await;
        let mut route = h.route();
        route.method = method.into();
        let result = h.client.execute(route).await;
        result.unwrap();
        let status = h.store.call(|s| s.status()).await.unwrap();
        assert_eq!(status.pools[0].generation, 0);
        assert_eq!(status.pools[0].addresses.len(), 2);
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
        "word_short",
        "word_long",
        "word_invalid",
        "final_hash",
    ] {
        let h = Harness::new().await;
        h.client.execute(h.route()).await.unwrap();
        *h.f.rpc_fault.lock().unwrap() = Some(fault.into());
        h.f.used.store(true, Ordering::SeqCst);
        let before = serde_json::to_value(h.store.call(|s| s.status()).await.unwrap()).unwrap();
        let pool = h.pool.clone();
        let query = h.store.call(move |s| s.chain_query(&pool)).await.unwrap();
        assert!(
            BaseRpc::new(&format!("{}/rpc", h.base), 12)
                .unwrap()
                .view(query)
                .await
                .is_err(),
            "{fault}"
        );
        assert_eq!(
            before,
            serde_json::to_value(h.store.call(|s| s.status()).await.unwrap()).unwrap(),
            "{fault}"
        );
        assert_eq!(
            h.client.execute(h.route()).await.unwrap(),
            "paid",
            "{fault}"
        );
        concurrency::assert_liabilities(&h, 2, 6_000_000);
        h.close().await;
    }
}

#[path = "support/concurrency.rs"]
mod concurrency;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_signed_response_does_not_block_shared_pool_or_reconciliation() {
    let h = Harness::new().await;
    exhaust_ready(&h).await;
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
    *h.f.challenge.lock().unwrap() = challenge("4000000");
    assert_eq!(
        concurrency::bounded(client.execute(h.route()))
            .await
            .unwrap(),
        "paid"
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 3);
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    h.f.release.notify_one();
    concurrency::bounded(pool.reconcile()).await.unwrap();
    concurrency::assert_liabilities(&h, 3, 6_000_000);
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
    reconcile_complete_history(&h).await;
    assert_eq!(
        concurrency::bounded(h.client.execute(h.route()))
            .await
            .unwrap(),
        "paid"
    );
    assert!(!first.is_finished());
    let status = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.pools[0].generation, 0);
    assert_eq!(status.pools[0].addresses.len(), 2);
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
        0
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
    let admission = tokio::spawn(async move { waiting.execute(route).await });
    // The background RPC owns no payment gate. This same-group payment
    // completes while that exact response remains deliberately held.
    assert_eq!(
        concurrency::bounded(admission).await.unwrap().unwrap(),
        "paid"
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    h.f.release.notify_one();
    concurrency::bounded(reconciliation).await.unwrap().unwrap();
    concurrency::assert_liabilities(&h, 2, 6_000_000);
    drop(other);
    drop(client);
    drop(pool);
    h.close().await;
}

#[tokio::test]
async fn qualification_guard_allows_existing_ready_consumption_but_denies_refill() {
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
        .find(|w| w.role == "ACTIVE")
        .unwrap();
    h.f.balances
        .lock()
        .unwrap()
        .insert(active.address.clone(), 2_000_000);
    h.f.used.store(true, Ordering::SeqCst);
    reconcile_complete_history(&h).await;
    let (a, b) = tokio::join!(h.client.execute(h.route()), h.client.execute(h.route()));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 2);
    assert_eq!(h.f.signed.lock().unwrap().len(), 3);
    let after = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(after.funding_jobs.len(), before.funding_jobs.len());
    assert_eq!(after.pools[0].generation, 0);
    assert_eq!(
        after.pools[0]
            .addresses
            .iter()
            .find(|w| w.id == active.id)
            .unwrap()
            .role,
        "RETAINED"
    );
    let pool = h.pool.clone();
    let error = h
        .store
        .call(move |s| {
            s.allocate_capacity_kind(
                &pool,
                x402_treazury::rotation::store::AllocationKind::Small,
                true,
            )
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("qualification_funding_denied"));
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 4);
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
            crate::capacity_fixture::treasury_pair(
                &owner,
                "research",
                "5",
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
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
max_refund_shielding_fee_zec="0.001"
[funding]
auto_fund=true
base_rpc_url_env="BASE"
base_rpc_fallback_url_envs=[]
[wallets.research]
mode="zcash_rotation"
max_api_payment_usdc="5"
reserves={{small={{funding_amount_usdc="5"}}}}
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
        drop(result.unwrap());
        // All fixtures are local files and unfunded wallets.
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
        let after = reopened.status().unwrap();
        assert_eq!(after.funding_jobs.len(), baseline.funding_jobs.len());
        assert_eq!(
            after.pools.iter().map(|p| p.addresses.len()).sum::<usize>(),
            if existing { 2 } else { 0 }
        );
        if existing {
            assert_eq!(
                serde_json::to_value(&after.pools[0].addresses).unwrap(),
                serde_json::to_value(&baseline.pools[0].addresses).unwrap()
            );
        }
    }
}

#[tokio::test]
async fn explicit_api_cap_independent_of_funding_target_uses_confirmed_capacity() {
    let mut h = Harness::new().await;
    for balance in h.f.balances.lock().unwrap().values_mut() {
        *balance = 6_000_000;
    }
    let group = h.pool.clone();
    h.store
        .call(move |s| {
            s.configure_capacity_group(
                &group,
                Some(alloy_primitives::U256::from(6_000_000u64)),
                0,
                &crate::capacity_fixture::two_reserves("5"),
            )
        })
        .await
        .unwrap();
    let manager = ManagedPool::new(
        h.store.clone(),
        h.pool.clone(),
        BaseRpc::new(&format!("{}/rpc", h.base), 12).unwrap(),
        SpendPolicy::dollars("6").unwrap(),
    )
    .unwrap();
    manager.reconcile().await.unwrap();
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
async fn foreground_admission_does_not_start_historical_authorization_sweep() {
    let mut h = Harness::new().await;
    *h.f.challenge.lock().unwrap() = challenge("1");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    h.f.nonce_delay.store(true, Ordering::SeqCst);
    let manager = ManagedPool::new(
        h.store.clone(),
        h.pool.clone(),
        BaseRpc::new(&format!("{}/rpc", h.base), 12).unwrap(),
        SpendPolicy::dollars("5").unwrap(),
    )
    .unwrap();
    h.client = PaidClient::managed(Arc::new(manager));
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    assert!(
        h.f.nonce_delay.load(Ordering::SeqCst),
        "foreground never requested historical nonce state"
    );
    h.close().await;
}

#[tokio::test]
async fn nonce_outages_retain_journals_without_vetoing_signing_across_restart() {
    let h = Harness::new().await;
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    h.client.execute(h.route()).await.unwrap();
    *h.f.rpc_fault.lock().unwrap() = Some("nonce_unavailable".into());
    h.client.execute(h.route()).await.unwrap();
    make_pool(h.store.clone(), h.pool.clone(), &h.base)
        .reconcile()
        .await
        .unwrap();
    concurrency::assert_liabilities(&h, 2, 2_000_000);
    let pool = h.pool.clone();
    let query = h.store.call(move |s| s.chain_query(&pool)).await.unwrap();
    assert!(
        BaseRpc::new(&format!("{}/rpc", h.base), 12)
            .unwrap()
            .view(query)
            .await
            .is_err()
    );
    *h.f.challenge.lock().unwrap() = challenge("4000000");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    let original = h.f.signed.lock().unwrap().clone();
    let h = restart(h).await;
    assert_eq!(*h.f.signed.lock().unwrap(), original);
    *h.f.challenge.lock().unwrap() = challenge("1");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    concurrency::assert_liabilities(&h, 4, 6_000_001);
    h.close().await;
}

#[tokio::test]
async fn held_balance_refresh_does_not_gate_signing_or_resolve_history() {
    let h = Harness::new().await;
    *h.f.challenge.lock().unwrap() = challenge("1");
    h.client.execute(h.route()).await.unwrap();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.rpc_hold.store(true, Ordering::SeqCst);
    let manager = make_pool(h.store.clone(), h.pool.clone(), &h.base);
    let background = tokio::spawn(async move { manager.reconcile().await });
    concurrency::bounded(h.f.arrived.notified()).await;
    assert_eq!(
        concurrency::bounded(h.client.execute(h.route()))
            .await
            .unwrap(),
        "paid"
    );
    assert!(!background.is_finished());
    concurrency::assert_liabilities(&h, 2, 2);
    h.f.release.notify_one();
    concurrency::bounded(background).await.unwrap().unwrap();
    concurrency::assert_liabilities(&h, 2, 2);
    h.close().await;
}

#[path = "support/consumption.rs"]
mod consumption;

#[path = "support/draining.rs"]
mod draining;

#[tokio::test]
async fn replacement_credit_survives_restart_without_releasing_old_authorizations() {
    let h = Harness::new().await;
    let (a, b) = tokio::join!(h.client.execute(h.route()), h.client.execute(h.route()));
    a.unwrap();
    b.unwrap();
    let original = h.f.signed.lock().unwrap().clone();
    assert_eq!(original.len(), 2);
    let mut allocations = Vec::new();
    for _ in 0..4 {
        let store = h.store.clone();
        let pool = h.pool.clone();
        allocations.push(tokio::spawn(async move {
            store
                .call(move |s| {
                    s.allocate_capacity_kind(
                        &pool,
                        x402_treazury::rotation::store::AllocationKind::Small,
                        true,
                    )
                })
                .await
                .unwrap()
        }));
    }
    let mut allocated = Vec::new();
    for task in allocations {
        if let Some(job) = task.await.unwrap() {
            allocated.push(job);
        }
    }
    assert_eq!(
        allocated.len(),
        1,
        "concurrent refill must allocate one reserve generation"
    );
    let job = allocated.pop().unwrap();
    let replacement = h
        .store
        .call(move |s| Ok(s.funding_job(&job)?.unwrap()))
        .await
        .unwrap();
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    assert_ne!(
        signatures::recover_exact(h.f.signed.lock().unwrap().last().unwrap()),
        replacement.recipient.parse::<Address>().unwrap()
    );
    h.f.balances
        .lock()
        .unwrap()
        .insert(replacement.recipient.clone(), 5_000_000);
    publish_initial_credit(&h.store, &h.pool, &h.base).await;
    let jobs = h
        .store
        .call(|s| Ok(s.status()?.funding_jobs.len()))
        .await
        .unwrap();
    let h = restart(h).await;
    assert_eq!(
        h.store
            .call(|s| Ok(s.status()?.funding_jobs.len()))
            .await
            .unwrap(),
        jobs
    );
    h.client.execute(h.route()).await.unwrap();
    let signed = h.f.signed.lock().unwrap().clone();
    assert_eq!(signed.len(), 4);
    assert_eq!(
        &signed[..2],
        original.as_slice(),
        "old authorizations must not be replayed or replaced"
    );
    assert_eq!(
        signed[3]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .parse::<Address>()
            .unwrap(),
        replacement.recipient.parse::<Address>().unwrap()
    );
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        unresolved, 4,
        "replacement credit must retain all unresolved payment liabilities"
    );
    let state = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(state.pools[0].addresses.len(), 3);
    assert_eq!(
        state.pools[0]
            .addresses
            .iter()
            .find(|w| w.id == replacement.wallet_id)
            .unwrap()
            .role,
        "ACTIVE"
    );
    drop(db);
    h.close().await;
}
