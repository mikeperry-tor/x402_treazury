//! Actual Base consumers retain ownership and financial evidence across Tor retries.
use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

type RecordedRead = (String, Value, tokio::time::Instant);

#[derive(Clone)]
struct Fixture {
    requests: Arc<Mutex<Vec<RecordedRead>>>,
    changed: Arc<tokio::sync::Notify>,
    saturation: Arc<tokio::sync::Barrier>,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            requests: Default::default(),
            changed: Default::default(),
            saturation: Arc::new(tokio::sync::Barrier::new(32)),
        }
    }
}
impl Fixture {
    fn count(&self, path: &str, method: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, q, _)| p == path && q["method"] == method)
            .count()
    }
    fn waited(&self, path: &str, method: &str) {
        let reads = self.requests.lock().unwrap();
        let times = reads
            .iter()
            .filter(|(p, q, _)| p == path && q["method"] == method)
            .map(|(_, _, at)| *at)
            .collect::<Vec<_>>();
        assert_eq!(times.len(), 2);
        assert!(times[1].duration_since(times[0]) >= Duration::from_secs(1));
    }
    async fn seen(&self, path: &str, method: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.count(path, method) > 0 {
                    break;
                }
                changed.await;
            }
        })
        .await
        .expect("Base fixture did not receive request");
    }
}
fn expected() -> TransferExpectation {
    TransferExpectation {
        transaction: B256::repeat_byte(1),
        payer: Address::repeat_byte(2),
        payee: Address::repeat_byte(3),
        amount: U256::from(7000),
        nonce: B256::repeat_byte(4),
    }
}
fn receipt() -> Value {
    let e = expected();
    let log = |topics: Vec<B256>, data: String| {
        json!({
            "address":USDC,"removed":false,"transactionHash":expected().transaction,
            "blockHash":format!("0x{:064x}",80),"blockNumber":"0x50","topics":topics,"data":data
        })
    };
    json!({"transactionHash":expected().transaction,"blockHash":format!("0x{:064x}",80),
    "blockNumber":"0x50","status":"0x1","logs":[
        log(vec![keccak256("AuthorizationUsed(address,bytes32)"),B256::left_padding_from(e.payer.as_slice()),e.nonce],"0x".into()),
        log(vec![keccak256("Transfer(address,address,uint256)"),B256::left_padding_from(e.payer.as_slice()),B256::left_padding_from(e.payee.as_slice())],format!("0x{:064x}",e.amount))
    ]})
}
async fn endpoint(
    State(f): State<Fixture>,
    uri: Uri,
    headers: HeaderMap,
    Json(q): Json<Value>,
) -> Response {
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("x-payment"));
    let path = uri.path();
    let method = q["method"].as_str().unwrap();
    let ordinal = {
        let mut requests = f.requests.lock().unwrap();
        requests.push((path.into(), q.clone(), tokio::time::Instant::now()));
        requests
            .iter()
            .filter(|(p, v, _)| p == path && v["method"] == method)
            .count()
    };
    f.changed.notify_waiters();
    if path.starts_with("/saturated/") {
        // All 32 active slots must reach the rejected endpoint before any
        // response starts its delay, keeping this saturation check deterministic.
        if ordinal == 1 {
            f.saturation.wait().await;
        }
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "1")],
            "unused",
        )
            .into_response();
    }
    if path.starts_with("/receipt-") && method == "eth_getTransactionReceipt" && ordinal == 1 {
        return (StatusCode::FORBIDDEN, "unused").into_response();
    }
    if path == "/view-a" && method == "eth_call" {
        return (StatusCode::SERVICE_UNAVAILABLE, "unused").into_response();
    }
    if ((path == "/shared-view" && method == "eth_chainId")
        || (matches!(path, "/shared-scalar" | "/terminated") && method == "eth_call"))
        && ordinal == 1
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "1")],
            "unused",
        )
            .into_response();
    }
    let result = match method {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = q["params"][0].as_str().unwrap();
            let height = if matches!(tag, "latest" | "finalized") {
                100
            } else {
                u64::from_str_radix(&tag[2..], 16).unwrap()
            };
            let hash = if path == "/view-b" {
                height + 1000
            } else {
                height
            };
            json!({"number":format!("0x{height:x}"),"hash":format!("0x{hash:064x}"),"timestamp":"0x64"})
        }
        "eth_call" => {
            assert!(q["params"][1] == "latest" || q["params"][1]["requireCanonical"] == true);
            json!(format!(
                "0x{:064x}",
                if path == "/view-b" { 22 } else { 11 }
            ))
        }
        "eth_getTransactionReceipt" => {
            let mut receipt = receipt();
            match path {
                "/receipt-null" => Value::Null,
                // Preserve existing pending semantics: no receipt proof is required yet.
                "/receipt-unconfirmed" => {
                    json!({"blockNumber":"0x63", "transactionHash":expected().transaction})
                }
                "/receipt-unconfirmed-wrong" => {
                    json!({"blockNumber":"0x63", "transactionHash":B256::ZERO})
                }
                "/receipt-unconfirmed-missing" => json!({"blockNumber":"0x63"}),
                "/receipt-tx" => {
                    receipt["transactionHash"] = json!(B256::ZERO);
                    receipt
                }
                "/receipt-nonce" => {
                    receipt["logs"][0]["topics"][2] = json!(B256::ZERO);
                    receipt
                }
                _ => receipt,
            }
        }
        method => panic!("unexpected RPC {method}"),
    };
    Json(json!({"jsonrpc":"2.0","id":q["id"],"result":result})).into_response()
}
fn rpc(path: &str) -> BaseRpc {
    BaseRpc::new(&format!("https://api.example.com{path}"), 12).unwrap()
}
fn selected(address: &str) -> Arc<crate::network::HttpTransport> {
    crate::network::global()
        .http_participating(
            &crate::network::IsolationId::evm(address).unwrap(),
            "https://api.example.com",
            None,
            false,
            crate::network::HttpPolicy::COMPATIBLE,
        )
        .unwrap()
}
#[tokio::test]
async fn tor_base_receipts_views_and_retained_owners() {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "rotation::base::tor_ownership_tests::tor_base_ownership_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "private Tor policy; invoked by parent"]
async fn tor_base_ownership_child() {
    use crate::network::{Mode, NetworkContext, NetworkPolicy};
    use crate::rotation::store::{Store, StoreHandle};
    let f = Fixture::default();
    let app = Router::new().fallback(endpoint).with_state(f.clone());
    let (address, server) = crate::test_tls::serve(app).await;
    let _proxy = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(_proxy.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );

    // Rejected operations retain their caller owners during Retry-After, but
    // cannot reserve all active transport slots while performing no I/O.
    let mut delayed = Vec::new();
    for n in 0..32 {
        delayed.push(tokio::spawn(async move {
            let payer = Address::repeat_byte(n + 16).to_string();
            rpc(&format!("/saturated/{n}"))
                .current_balance(&payer)
                .await
        }));
    }
    for n in 0..32 {
        f.seen(&format!("/saturated/{n}"), "eth_call").await;
    }
    let healthy = BaseRpc::new("https://spec.example.com/healthy", 12).unwrap();
    tokio::time::timeout(Duration::from_millis(500), async {
        assert_eq!(
            healthy
                .current_balance(&expected().payer.to_string())
                .await
                .unwrap(),
            U256::from(11)
        );
        assert_eq!(healthy.block("latest").await.unwrap().0.height, 100);
    })
    .await
    .expect("Retry-After sleepers blocked independent Base transport capacity");
    for task in &delayed {
        assert!(!task.is_finished(), "server delay still owns the caller");
        task.abort();
    }
    for task in delayed {
        assert!(task.await.unwrap_err().is_cancelled());
    }
    tokio::time::sleep(Duration::from_millis(1100)).await;
    for n in 0..32 {
        assert_eq!(f.count(&format!("/saturated/{n}"), "eth_call"), 1);
    }
    assert_eq!(
        healthy
            .current_balance(&expected().payer.to_string())
            .await
            .unwrap(),
        U256::from(11),
        "cancellation releases caller and active capacity"
    );

    let e = expected();
    for path in [
        "/receipt-tx",
        "/receipt-nonce",
        "/receipt-null",
        "/receipt-unconfirmed",
        "/receipt-unconfirmed-wrong",
        "/receipt-unconfirmed-missing",
        "/receipt-valid",
    ] {
        let before = selected(&e.payer.to_string());
        let result = rpc(path).verify_transfer(&e).await;
        let after = selected(&e.payer.to_string());
        assert_eq!(f.count(path, "eth_chainId"), 1);
        assert_eq!(f.count(path, "eth_getTransactionReceipt"), 2);
        match path {
            "/receipt-tx" | "/receipt-nonce" => {
                assert!(result.is_err());
                assert!(
                    Arc::ptr_eq(&before, &after),
                    "invalid receipt must not promote its candidate"
                );
                assert_eq!(
                    f.count(path, "eth_getBlockByNumber"),
                    1,
                    "invalid receipt requires no canonical proof RPC"
                );
            }
            "/receipt-unconfirmed-wrong" | "/receipt-unconfirmed-missing" => {
                assert!(result.unwrap().is_none(), "pending semantics unchanged");
                assert!(
                    Arc::ptr_eq(&before, &after),
                    "wrong transaction cannot promote"
                );
                assert_eq!(f.count(path, "eth_getBlockByNumber"), 1);
            }
            "/receipt-null" | "/receipt-unconfirmed" => {
                assert!(result.unwrap().is_none());
                assert!(
                    !Arc::ptr_eq(&before, &after),
                    "pending observation can promote without a confirmed transfer proof"
                );
                assert_eq!(
                    f.count(path, "eth_getBlockByNumber"),
                    1,
                    "pending receipt must not trigger more observations"
                );
            }
            _ => {
                assert_eq!(result.unwrap().unwrap().amount_atomic, "7000");
                assert!(!Arc::ptr_eq(&before, &after));
                // Four acquisition reads plus three distinct pinned-height rechecks.
                assert_eq!(f.count(path, "eth_getBlockByNumber"), 7);
            }
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"Base Tor owner fixture",
    )
    .unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "owned",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(100_000_000)),
        0
    )
    .unwrap();
    let wallet = state.funding_jobs().unwrap().remove(0).wallet_id;
    let query = state
        .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
        .unwrap();
    let payer = query.wallets[0].1.clone();
    let snapshot = state.status().unwrap();
    let generations = query.captures[&wallet].endpoint_generations.clone();
    let (store, worker) = StoreHandle::spawn(state);

    // Endpoint A obtains headers, then exhausts one read's three scopes. B must
    // reacquire its whole view; evidence must contain only B's URL and anchors.
    let fallback = BaseRpc::with_fallbacks(
        &[
            "https://api.example.com/view-a".into(),
            "https://api.example.com/view-b".into(),
        ],
        12,
    )
    .unwrap();
    let endpoint_b = rpc("/view-b").endpoint_id();
    let view = fallback.view(query.clone()).await.unwrap();
    assert_eq!(f.count("/view-a", "eth_chainId"), 1);
    assert_eq!(f.count("/view-a", "eth_call"), 3);
    assert_eq!(f.count("/view-b", "eth_chainId"), 1);
    assert_eq!(view.observations[&wallet].endpoint, endpoint_b);
    assert_eq!(
        view.observations[&wallet].confirmed.anchor.hash,
        format!("0x{:064x}", 1088)
    );
    assert_eq!(
        view.observations[&wallet].latest.anchor.hash,
        format!("0x{:064x}", 1100)
    );
    assert_eq!(view.balances[&wallet], U256::from(22));
    assert!(view.released.is_empty());
    let later_query = {
        let pool = pool.clone();
        let wallet = wallet.clone();
        store
            .call(move |s| s.scoped_chain_query(&pool, vec![wallet], vec![]))
            .await
            .unwrap()
    };
    assert_eq!(
        later_query.captures[&wallet].endpoint_generations, generations,
        "transport recovery cannot change durable endpoint generations"
    );

    let shared = rpc("/shared-view");
    let first = tokio::spawn({
        let shared = shared.clone();
        let query = query.clone();
        let store = store.clone();
        async move { shared.shared_balance_view(query, store).await }
    });
    f.seen("/shared-view", "eth_chainId").await;
    first.abort();
    assert!(matches!(first.await,Err(error) if error.is_cancelled()));
    let joined = tokio::spawn({
        let query = query.clone();
        let store = store.clone();
        async move { shared.shared_balance_view(query, store).await }
    });
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(500),
            rpc("/independent").current_balance(&payer)
        )
        .await
        .unwrap()
        .unwrap(),
        U256::from(11)
    );
    let result = joined.await.unwrap().unwrap();
    assert_eq!(result.balances[&wallet], U256::from(11));
    assert_eq!(
        f.count("/shared-view", "eth_chainId"),
        2,
        "one retained owner performs one selected and one candidate request"
    );
    assert_eq!(f.count("/shared-view", "eth_call"), 2);
    f.waited("/shared-view", "eth_chainId");

    // Advisory scalar observation has its own retained wallet owner and cadence.
    let scalar = rpc("/shared-scalar");
    let first = tokio::spawn({
        let (store, pool, wallet, scalar) =
            (store.clone(), pool.clone(), wallet.clone(), scalar.clone());
        async move { store.observe_balance(pool, wallet, scalar).await }
    });
    f.seen("/shared-scalar", "eth_call").await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let scalar_result = store
        .observe_balance(pool.clone(), wallet.clone(), scalar.clone())
        .await
        .unwrap();
    assert_eq!(scalar_result, U256::from(11));
    assert_eq!(f.count("/shared-scalar", "eth_call"), 2);
    f.waited("/shared-scalar", "eth_call");
    assert_eq!(
        store
            .observe_balance(pool.clone(), wallet.clone(), scalar)
            .await
            .unwrap(),
        U256::from(11)
    );
    assert_eq!(
        f.count("/shared-scalar", "eth_call"),
        2,
        "completed observation cadence cannot be bypassed by a new transport"
    );

    // This scalar future IS the owner; aborting it drops its finite retry wait.
    let terminated = tokio::spawn({
        let payer = payer.clone();
        async move { rpc("/terminated").current_balance(&payer).await }
    });
    f.seen("/terminated", "eth_call").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    terminated.abort();
    assert!(terminated.await.unwrap_err().is_cancelled());
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(f.count("/terminated", "eth_call"), 1);
    let after = store.call(|s| s.status()).await.unwrap();
    assert_eq!(after.funding_jobs.len(), snapshot.funding_jobs.len());
    assert_eq!(
        after.pools[0].addresses.len(),
        snapshot.pools[0].addresses.len()
    );
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(store);
    worker.await.unwrap();
    server.abort();
    let _ = server.await;
}
