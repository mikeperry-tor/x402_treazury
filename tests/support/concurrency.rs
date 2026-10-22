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
    let pool = h
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
    publish_initial_credit(&h.store, &pool, &h.base).await;
    make_pool(h.store.clone(), pool.clone(), &h.base)
        .reconcile()
        .await
        .unwrap();
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
async fn mcp_burst_across_listeners_retains_every_journal_and_isolates_pools() {
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
            x402_treazury::server::Server::new(tools.clone(), client, h.base.clone(), None);
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
    let barrier = gate(&h, 24);
    let mut calls = tokio::task::JoinSet::new();
    for i in 0..24 {
        let endpoint = if i < 12 { i % 2 } else { 2 };
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
                assert!(
                    result.to_string().contains("funding_unavailable"),
                    "{result}"
                );
            } else {
                assert_eq!(result["content"][0]["text"], "paid");
                successes[pool] += 1;
            }
        }
    })
    .await;
    assert_eq!(successes, [12, 12]);
    assert_liabilities(&h, 24, 24_000_000);
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let per_pool: Vec<(String, i64)> = db.prepare("SELECT g.name,COUNT(*) FROM payment_attempts p JOIN pools g ON g.id=p.pool_id GROUP BY g.id ORDER BY g.name").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(per_pool, [("other".into(), 12), ("research".into(), 12)]);
    drop(db);
    let signed = h.f.signed.lock().unwrap().clone();
    let mut counts = BTreeMap::new();
    for p in signed {
        *counts.entry(signatures::recover_exact(&p)).or_insert(0) += 1;
    }
    assert_eq!(counts.values().sum::<usize>(), 24);
    assert!(counts.len() <= 4);
    let identities = h
        .store
        .call(|s| {
            Ok(s.status()?
                .pools
                .into_iter()
                .flat_map(|p| p.addresses)
                .map(|w| w.address.parse::<Address>().unwrap())
                .collect::<std::collections::BTreeSet<_>>())
        })
        .await
        .unwrap();
    assert!(counts.keys().all(|payer| identities.contains(payer)));
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
    assert_balance_observation_outcome(
        make_pool(h.store.clone(), h.pool.clone(), &h.base)
            .reconcile()
            .await,
    );
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
    assert_liabilities(&h, 25, 25_000_000);
    for server in servers {
        bounded(server).await.unwrap();
    }
    drop(other);
    h.close().await;
}

async fn rotation_burst() -> Vec<(String, std::net::SocketAddr, String, String)> {
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
        h.f.used.store(true, Ordering::SeqCst);
        h.f.balances.lock().unwrap().insert(active, 0);
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
                    Err(e) => assert!(e.to_string().contains("funding_unavailable"), "{e}"),
                }
            }
        })
        .await;
        assert_eq!(successes, 8);
        let state = h.store.call(|s| s.status()).await.unwrap();
        assert_eq!(state.pools[0].generation, 0);
        assert_eq!(state.pools[0].addresses.len(), 2);
        assert_eq!(state.funding_jobs.len(), initial.funding_jobs.len());
        assert_liabilities(&h, successes as i64, successes as i64 * 1_000_000);
        let signed = h.f.signed.lock().unwrap().clone();
        assert_eq!(signed.len(), 1 + successes);
        let identities: std::collections::BTreeSet<_> = initial.pools[0]
            .addresses
            .iter()
            .map(|w| w.address.parse::<Address>().unwrap())
            .collect();
        assert!(
            signed[1..]
                .iter()
                .all(|p| identities.contains(&signatures::recover_exact(p)))
        );
        let url = h.url.clone();
        traces.extend(
            h.f.traces
                .lock()
                .unwrap()
                .iter()
                .cloned()
                .map(|(peer, phase, address)| (url.clone(), peer, phase, address)),
        );
        h.close().await;
    }
    traces
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_depletion_consumes_once_with_one_handoff_for_get_and_post() {
    rotation_burst().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_waiters_preserve_journals_and_other_pool_progress() {
    let h = Harness::new().await;
    let other = other_pool(&h).await;
    h.client.execute(h.route()).await.unwrap();
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
    assert_eq!(bounded(timed).await.unwrap().unwrap(), "paid");
    assert!(!submitted.is_finished());
    submitted.abort();
    assert!(submitted.await.unwrap_err().is_cancelled());
    h.f.release.notify_one();
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    h.client.drain().await;
    assert!(h.client.execute(h.route()).await.is_err());
    assert_liabilities(&h, 6, 18_000_000);
    assert_eq!(h.f.signed.lock().unwrap().len(), 6);
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
    let mut signed = 0;
    let mut rpc = 0;
    for (url, peer, phase, address) in traces {
        if phase == "unsigned" {
            continue;
        }
        let record = records
            .iter()
            .find(|r| r.upstream_peer == Some(peer))
            .unwrap();
        let id = IsolationId::evm(&address).unwrap();
        if phase == "signed" {
            let transport = network::global()
                .http_participating(&id, &url, None, false, Default::default())
                .unwrap();
            assert_eq!(
                (record.user.clone(), record.password.clone()),
                network::global().credentials(&transport.scope),
                "{phase}"
            );
            assert_ne!(record.password, network::global().credentials(&id).1);
            signed += 1;
        } else {
            assert_eq!(
                (record.user.clone(), record.password.clone()),
                network::global().credentials(
                    &network::global()
                        .http_participating(
                            &id,
                            &url,
                            None,
                            false,
                            network::HttpPolicy {
                                allow_http1: true,
                                allow_tls12: true
                            },
                        )
                        .unwrap()
                        .scope
                ),
                "{phase}"
            );
            rpc += 1;
        }
    }
    assert_eq!(
        signed, 18,
        "all nine signed calls in both bursts retain signer isolation"
    );
    assert!(
        rpc > 0,
        "actual initial-credit and balance RPC retains payer isolation"
    );
}

#[tokio::test]
async fn fresh_small_only_group_promotes_once_under_concurrent_first_payments() {
    let mut h = Harness::new().await;
    let pool = h
        .store
        .call(|s| {
            let pool = s.ensure_capacity_group(
                "fresh_single",
                &crate::capacity_fixture::small_reserve("5"),
                Some(alloy_primitives::U256::from(5_000_000)),
                0,
            )?;
            s.allocate_capacity_kind(
                &pool,
                x402_treazury::rotation::store::AllocationKind::Small,
                true,
            )?;
            Ok(pool)
        })
        .await
        .unwrap();
    h.pool = pool.clone();
    let selected = pool.clone();
    let wallet = h
        .store
        .call(move |s| {
            Ok(s.status()?
                .pools
                .into_iter()
                .find(|p| p.id == selected)
                .unwrap()
                .addresses
                .remove(0))
        })
        .await
        .unwrap();
    h.f.balances
        .lock()
        .unwrap()
        .insert(wallet.address.clone(), 5_000_000);
    publish_initial_credit(&h.store, &pool, &h.base).await;
    let manager = make_pool(h.store.clone(), pool.clone(), &h.base);
    manager.reconcile().await.unwrap();
    h.client = make_client(h.store.clone(), pool.clone(), &h.base);
    h.f.free.store(true, Ordering::SeqCst);
    h.client.execute(h.route()).await.unwrap();
    h.f.free.store(false, Ordering::SeqCst);
    let selected = pool.clone();
    h.store
        .call(move |s| {
            let status = s.status()?;
            let group = status.pools.iter().find(|p| p.id == selected).unwrap();
            assert!(group.serving_ready);
            assert_eq!(group.addresses.len(), 1);
            assert_eq!(group.addresses[0].role, "READY");
            assert_eq!(group.reserve_claims[0].generation, 0);
            Ok(())
        })
        .await
        .unwrap();
    assert!(h.f.signed.lock().unwrap().is_empty());
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    let (a, b) = tokio::join!(h.client.execute(h.route()), h.client.execute(h.route()));
    a.unwrap();
    b.unwrap();
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    {
        let signed = h.f.signed.lock().unwrap();
        assert_ne!(
            signed[0]["payload"]["authorization"]["nonce"],
            signed[1]["payload"]["authorization"]["nonce"]
        );
        let db = rusqlite::Connection::open(&h.f.db).unwrap();
        let roles: Vec<String> = db.prepare("SELECT e.payer_role FROM payment_admission_evidence e JOIN payment_attempts p ON p.id=e.attempt_id WHERE p.pool_id=?1 ORDER BY p.rowid").unwrap().query_map([&pool], |r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(roles, ["READY", "ACTIVE"]);
    }
    let selected = pool.clone();
    h.store
        .call(move |s| {
            let status = s.status()?;
            let group = status.pools.iter().find(|p| p.id == selected).unwrap();
            assert_eq!(group.addresses.len(), 1);
            assert_eq!(group.addresses[0].role, "ACTIVE");
            assert_eq!(group.reserve_claims[0].generation, 1);
            assert!(group.reserve_claims[0].wallet_id.is_none());
            assert!(!group.inventory_ready);
            Ok(())
        })
        .await
        .unwrap();
    let results = futures_util::future::join_all((0..16).map(|_| {
        let pool = pool.clone();
        h.store.call(move |s| {
            s.allocate_capacity_kind(
                &pool,
                x402_treazury::rotation::store::AllocationKind::Small,
                true,
            )
        })
    }))
    .await;
    assert_eq!(
        results
            .into_iter()
            .map(Result::unwrap)
            .filter(Option::is_some)
            .count(),
        1
    );
    let selected = pool.clone();
    h.store
        .call(move |s| {
            let status = s.status()?;
            let group = status.pools.iter().find(|p| p.id == selected).unwrap();
            assert_eq!(group.addresses.len(), 2);
            assert_eq!(group.reserve_claims[0].generation, 1);
            assert!(group.reserve_claims[0].wallet_id.is_some());
            assert_eq!(
                status
                    .funding_jobs
                    .iter()
                    .filter(|j| j.pool_id == selected)
                    .count(),
                2
            );
            assert!(!status.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    drop(manager);
    h.close().await;
}
