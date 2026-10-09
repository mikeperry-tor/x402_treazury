//! Multithreaded, forced-overlap admission and wire-level MCP qualification.
use super::*;
use std::time::Duration;
use tokio::sync::Barrier;

fn gate(h: &Harness, count: usize) -> Arc<Barrier> {
    let gate = Arc::new(Barrier::new(count + 1));
    *h.f.unsigned_gate.lock().unwrap() = Some(UnsignedGate {
        barrier: gate.clone(),
        remaining: count,
    });
    gate
}
pub(super) async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(15), future)
        .await
        .expect("concurrency test deadline")
}
pub(super) async fn other_pool(h: &Harness) -> PaidClient {
    let pool = h.store.call(|s| s.ensure_pool("other", "5")).await.unwrap();
    let status = h.store.call(|s| s.status()).await.unwrap();
    for a in &status
        .pools
        .iter()
        .find(|p| p.id == pool)
        .unwrap()
        .addresses
    {
        h.f.balances
            .lock()
            .unwrap()
            .insert(a.address.clone(), 5_000_000);
    }
    make_client(h.store.clone(), pool, &h.base)
}
pub(super) fn assert_liabilities(h: &Harness, count: i64, total: i64) {
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let observed: (i64, i64) = db.query_row("SELECT COUNT(*),COALESCE(SUM(CAST(amount AS INTEGER)),0) FROM payment_attempts WHERE state!='RESOLVED'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(observed, (count, total));
    let nonces: i64 = db
        .query_row(
            "SELECT COUNT(DISTINCT nonce) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(nonces, count);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_burst_across_listeners_shares_reservations_and_isolates_pools() {
    let h = Harness::new().await;
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    let other = other_pool(&h).await;
    let tools = x402_treazury::catalog::build_tools(
        &Default::default(),
        &json!({"paths":{"/pay":{"get":{}}}}),
        "test",
    )
    .unwrap();
    let mut endpoints = vec![];
    let mut servers = vec![];
    let stop = tokio_util::sync::CancellationToken::new();
    for (i, client) in [h.client.clone(), h.client.clone(), other.clone()]
        .into_iter()
        .enumerate()
    {
        let server =
            x402_treazury::server::Server::new(tools.clone(), client, h.base.clone(), None, None);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        endpoints.push(format!("http://{}/mcp", listener.local_addr().unwrap()));
        let app = x402_treazury::server::http_app(server, format!("token-{i}"));
        let stopped = stop.clone();
        servers.push(tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
                .unwrap()
        }));
    }
    let barrier = gate(&h, 16);
    let mut calls = tokio::task::JoinSet::new();
    for i in 0..16 {
        let endpoint = if i < 8 { i % 2 } else { 2 };
        let url = endpoints[endpoint].clone();
        let name = tools[0].name.clone();
        calls.spawn(async move {
            let v: Value = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(12)).build().unwrap()
                .post(url).bearer_auth(format!("token-{endpoint}"))
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":i,"method":"tools/call","params":{"name":name,"arguments":{}}}))
                .send().await.unwrap().mcp_json().await.unwrap();
            assert_eq!(v["id"], i);
            (usize::from(endpoint == 2), v["result"].clone())
        });
    }
    bounded(barrier.wait()).await;
    let mut successes = [0, 0];
    bounded(async {
        while let Some(result) = calls.join_next().await {
            let (pool, result) = result.unwrap();
            if result["isError"] == true {
                assert!(result.to_string().contains("payment_pending"), "{result}");
            } else {
                assert_eq!(result["content"][0]["text"], "paid");
                successes[pool] += 1;
            }
        }
    })
    .await;
    assert_eq!(successes, [5, 5]);
    assert_liabilities(&h, 10, 10_000_000);
    let signed = h.f.signed.lock().unwrap().clone();
    let mut counts = BTreeMap::new();
    for p in signed {
        *counts.entry(signatures::recover_exact(&p)).or_insert(0) += 1;
    }
    assert_eq!(counts.values().copied().collect::<Vec<_>>(), vec![5, 5]);
    let status = h.store.call(|s| s.status()).await.unwrap();
    assert!(
        status
            .pools
            .iter()
            .all(|p| p.generation == 0 && p.addresses.len() == 2)
    );
    for balance in h.f.balances.lock().unwrap().values_mut() {
        *balance = 20_000_000;
    }
    h.f.hold.store(true, Ordering::SeqCst);
    let url = endpoints[0].clone();
    let name = tools[0].name.clone();
    let draining = tokio::spawn(async move {
        reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap()
            .post(url).bearer_auth("token-0").header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":name,"arguments":{}}}))
            .send().await.unwrap().mcp_json::<Value>().await.unwrap()
    });
    bounded(h.f.arrived.notified()).await;
    stop.cancel();
    assert!(!draining.is_finished());
    assert!(!servers[0].is_finished());
    h.f.release.notify_one();
    let response = bounded(draining).await.unwrap();
    assert_eq!(response["id"], 99);
    assert_eq!(response["result"]["content"][0]["text"], "paid");
    assert_liabilities(&h, 11, 11_000_000);
    for server in servers {
        bounded(server).await.unwrap();
    }
    drop(other);
    h.close().await;
}

async fn rotation_burst() -> Vec<(std::net::SocketAddr, String, String)> {
    let mut traces = vec![];
    for method in ["GET", "POST"] {
        let h = Harness::new().await;
        *h.f.challenge.lock().unwrap() = challenge("1000000");
        h.client.execute(h.route()).await.unwrap();
        let initial = h.store.call(|s| s.status()).await.unwrap();
        let active = initial.pools[0]
            .addresses
            .iter()
            .find(|a| a.role == "ACTIVE")
            .unwrap()
            .address
            .clone();
        let ready = initial.pools[0]
            .addresses
            .iter()
            .find(|a| a.role == "READY")
            .unwrap()
            .address
            .clone();
        h.f.used.store(true, Ordering::SeqCst);
        h.f.balances.lock().unwrap().insert(active, 0);
        let pool = h.pool.clone();
        let query = h.store.call(move |s| s.chain_query(&pool)).await.unwrap();
        let view = BaseRpc::new(&format!("{}/rpc", h.base), 12, 120)
            .unwrap()
            .view(query)
            .await
            .unwrap();
        let pool = h.pool.clone();
        h.store
            .call(move |s| s.reconcile_pool(&pool, view))
            .await
            .unwrap();
        h.f.used.store(false, Ordering::SeqCst);
        // Every caller snapshots the old payer before any may trigger promotion.
        let barrier = gate(&h, 8);
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let client = h.client.clone();
            let mut route = h.route();
            route.method = method.into();
            calls.spawn(async move { client.execute(route).await });
        }
        bounded(barrier.wait()).await;
        let mut successes = 0;
        bounded(async {
            while let Some(r) = calls.join_next().await {
                match r.unwrap() {
                    Ok(_) => successes += 1,
                    Err(e) => assert!(
                        e.to_string().contains(if method == "POST" {
                            "payer_changed_before_payment"
                        } else {
                            "payment_pending"
                        }),
                        "{e}"
                    ),
                }
            }
        })
        .await;
        assert_eq!(successes, if method == "GET" { 5 } else { 0 });
        let state = h.store.call(|s| s.status()).await.unwrap();
        assert_eq!(state.pools[0].generation, 1);
        assert_eq!(state.pools[0].addresses.len(), 3);
        assert_eq!(state.funding_jobs.len(), initial.funding_jobs.len() + 1);
        assert_liabilities(&h, successes as i64, successes as i64 * 1_000_000);
        let signed = h.f.signed.lock().unwrap().clone();
        assert_eq!(signed.len(), 1 + successes);
        assert!(
            signed[1..]
                .iter()
                .all(|p| signatures::recover_exact(p).to_string() == ready)
        );
        traces.extend(h.f.traces.lock().unwrap().clone());
        h.close().await;
    }
    traces
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_depletion_promotes_once_and_never_replays_posts() {
    rotation_burst().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_and_rejected_waiters_preserve_liability_and_other_pool_progress() {
    let h = Harness::new().await;
    let other = other_pool(&h).await;
    h.f.hold.store(true, Ordering::SeqCst);
    let client = h.client.clone();
    let route = h.route();
    let submitted = tokio::spawn(async move { client.execute(route).await });
    bounded(h.f.arrived.notified()).await;
    let barrier = gate(&h, 2);
    let client = h.client.clone();
    let route = h.route();
    let cancelled = tokio::spawn(async move { client.execute(route).await });
    let client = h.client.clone();
    let route = h.route();
    let timed = tokio::spawn(async move { client.execute(route).await });
    bounded(barrier.wait()).await;
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    assert_eq!(bounded(other.execute(h.route())).await.unwrap(), "paid");
    assert!(
        bounded(timed)
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("payment_pending")
    );
    assert!(!submitted.is_finished());
    assert_liabilities(&h, 2, 6_000_000);
    submitted.abort();
    assert!(submitted.await.unwrap_err().is_cancelled());
    h.f.release.notify_one();
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("payment_pending")
    );
    assert_liabilities(&h, 2, 6_000_000);
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    drop(other);
    h.close().await;
}

#[tokio::test]
async fn tor_concurrent_rotation_keeps_each_signer_and_rpc_identity() {
    let result = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "concurrency::tor_burst_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "private Tor policy; invoked by parent"]
async fn tor_burst_child() {
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
    let traces = rotation_burst().await;
    let records = proxy.records.lock().unwrap();
    let mut checked = 0;
    for (peer, phase, address) in traces {
        if phase == "unsigned" {
            continue;
        }
        let record = records
            .iter()
            .find(|r| r.upstream_peer == Some(peer))
            .unwrap();
        assert_eq!(
            (record.user.clone(), record.password.clone()),
            network::global().credentials(&IsolationId::evm(&address).unwrap()),
            "{phase}"
        );
        checked += 1;
    }
    assert!(
        checked > 20,
        "must observe signed calls and RPC across the burst"
    );
}
