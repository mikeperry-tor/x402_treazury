//! Offline end-to-end admission against a local Base RPC and x402 seller.
use alloy_primitives::Address;
use axum::{
    Json, Router,
    extract::State,
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
use x402_mcp_prototype::{
    catalog::RoutedRequest,
    payment::{PaidClient, SpendPolicy, USDC},
    rotation::{
        base::{BaseRpc, now},
        manager::ManagedPool,
        store::{Store, StoreHandle},
    },
};
#[derive(Clone)]
struct Fake {
    balances: Arc<Mutex<BTreeMap<String, u64>>>,
    challenge: Arc<Mutex<Value>>,
    signed: Arc<Mutex<Vec<Value>>>,
    rpc_calls: Arc<AtomicUsize>,
    used: Arc<AtomicBool>,
    stale: Arc<AtomicBool>,
    reorg: Arc<AtomicBool>,
    wrong_chain: Arc<AtomicBool>,
    reject: Arc<AtomicBool>,
    free: Arc<AtomicBool>,
    hold: Arc<AtomicBool>,
    arrived: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    db: std::path::PathBuf,
}
fn challenge(amount: &str) -> Value {
    json!({"x402Version":2,"resource":{"url":"http://localhost/pay","description":"x".repeat(700),"mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":amount,"payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]})
}
async fn rpc(State(f): State<Fake>, Json(v): Json<Value>) -> Json<Value> {
    f.rpc_calls.fetch_add(1, Ordering::SeqCst);
    let result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!(if f.wrong_chain.load(Ordering::SeqCst) {
            "0x1"
        } else {
            "0x2105"
        }),
        "eth_getBlockByNumber" => {
            let number = if v["params"][0] == "latest" {
                "0x64"
            } else {
                v["params"][0].as_str().unwrap()
            };
            json!({"number":number,"hash":format!("0x{:064x}",if f.reorg.load(Ordering::SeqCst){2}else{1}),"timestamp":format!("0x{:x}",now().unwrap()-if f.stale.load(Ordering::SeqCst){1000}else{0})})
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
                *f.balances.lock().unwrap().get(&address).unwrap_or(&0)
            } else {
                u64::from(f.used.load(Ordering::SeqCst))
            };
            json!(format!("0x{n:064x}"))
        }
        _ => panic!("unexpected RPC"),
    };
    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
}
async fn seller(State(f): State<Fake>, headers: HeaderMap) -> Response {
    if f.free.load(Ordering::SeqCst) {
        return "free".into_response();
    }
    if let Some(h) = headers.get("payment-signature") {
        let p: Value = serde_json::from_slice(&STANDARD.decode(h.as_bytes()).unwrap()).unwrap();
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
            used: Arc::default(),
            stale: Arc::default(),
            reorg: Arc::default(),
            wrong_chain: Arc::default(),
            reject: Arc::default(),
            free: Arc::default(),
            hold: Arc::default(),
            arrived: Arc::default(),
            release: Arc::default(),
            db: dir.path().join("state/state.sqlite"),
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
            .route("/pay", get(seller))
            .with_state(f.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
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
fn make_client(store: StoreHandle, pool: String, base: &str) -> PaidClient {
    let manager = ManagedPool::new(
        store,
        pool,
        BaseRpc::new(&format!("{base}/rpc"), 12, 120).unwrap(),
        "5",
        SpendPolicy::dollars("none").unwrap(),
        2,
    )
    .unwrap();
    PaidClient::managed(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap(),
        Arc::new(manager),
    )
}
#[tokio::test]
async fn concurrent_calls_reserve_before_send_and_cannot_churn_busy_active() {
    let h = Harness::new().await;
    let other = h.client.with_http(h.client.http.clone());
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
#[tokio::test]
async fn unsupported_and_over_target_offers_have_no_admission_side_effects() {
    let h = Harness::new().await;
    for kind in [
        "v1",
        "upto",
        "permit2",
        "flow",
        "extensions",
        "asset",
        "chain",
        "zero",
        "decimal",
        "over_target",
        "domain",
        "envelope",
    ] {
        let mut c = challenge("1");
        match kind {
            "v1" => c["x402Version"] = json!(1),
            "upto" => c["accepts"][0]["scheme"] = json!("upto"),
            "permit2" => c["accepts"][0]["extra"]["assetTransferMethod"] = json!("permit2"),
            "flow" => c["accepts"][0]["extra"]["flow"] = json!("upfront"),
            "extensions" => c["extensions"] = json!({"unknown":{}}),
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
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("402")
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
    use x402_mcp_prototype::{deployment::Deployment, treasury::Treasury};
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
daily_input_zec="0.1"
shield_max_fee_zec="0.001"
[funding]
base_rpc_url_env="BASE"
[wallets.research]
mode="zcash_rotation"
max_input_zec="0.02"
max_fee_bps=500
[wallets.unused]
mode="zcash_rotation"
max_input_zec="0.02"
max_fee_bps=500
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
    let running = Deployment::load(&path)
        .await
        .unwrap()
        .bind(&env)
        .await
        .unwrap();
    let state = x402_mcp_prototype::rotation::store::status(&dir.path().join("state")).unwrap();
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
        let response:Value=reqwest::Client::new().post(format!("http://{address}/mcp")).bearer_auth("secret").header("accept","application/json, text/event-stream").json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"managed_pay","arguments":{}}})).send().await.unwrap().json().await.unwrap();
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
    let static_config = config.replace(
        "mode=\"zcash_rotation\"\nmax_input_zec=\"0.02\"\nmax_fee_bps=500",
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
async fn waiting_for_gate_has_a_deadline_and_other_pools_continue() {
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
    assert!(error.contains("deadline"), "{error}");
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
    let client = PaidClient::managed(
        h.client.http.clone(),
        Arc::new(
            ManagedPool::new(
                h.store.clone(),
                h.pool.clone(),
                BaseRpc::new(&format!("{}/rpc", h.base), 12, 120).unwrap(),
                "10",
                SpendPolicy::dollars("none").unwrap(),
                2,
            )
            .unwrap(),
        ),
    );
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
