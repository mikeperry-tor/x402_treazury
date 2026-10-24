//! Startup discovery resumes in the same serving process when the proxy appears.
use super::*;
use axum::{
    Json, Router,
    http::HeaderMap,
    routing::{get, post},
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tracing_subscriber::prelude::*;

struct ProxyFailure(Arc<tokio::sync::Notify>);
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ProxyFailure {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() == "x402_treazury::network::startup_socks"
            && *event.metadata().level() == tracing::Level::ERROR
        {
            self.0.notify_one();
        }
    }
}

#[tokio::test]
async fn late_socks_endpoint_recovers_serving_discovery() {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "deployment::deferred::startup_socks_tests::late_socks_child",
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "isolated Tor policy; invoked by parent"]
async fn late_socks_child() {
    use crate::rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{Store, StoreHandle},
    };
    let unreachable = Arc::new(tokio::sync::Notify::new());
    tracing_subscriber::registry()
        .with(ProxyFailure(unreachable.clone()))
        .init();
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = reserved.local_addr().unwrap();
    drop(reserved);
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_address = origin.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let balances = Arc::new(AtomicUsize::new(0));
    let scans = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/spec", get({ let calls=calls.clone(); move |headers: HeaderMap| { let calls=calls.clone(); async move {
            assert!(!headers.contains_key("payment-signature") && !headers.contains_key("x-payment"));
            calls.fetch_add(1,Ordering::SeqCst);
            Json(serde_json::json!({"operations":[{"method":"GET","path":"/read"}]}))
        }}}))
        .route("/rpc", post({ let balances=balances.clone(); move |headers: HeaderMap, Json(value): Json<serde_json::Value>| { let balances=balances.clone(); async move {
            assert!(!headers.contains_key("payment-signature") && !headers.contains_key("x-payment"));
            assert_eq!(value["method"], "eth_call");
            balances.fetch_add(1,Ordering::SeqCst);
            Json(serde_json::json!({"jsonrpc":"2.0","id":1,"result":format!("0x{:064x}",17)}))
        }}}))
        .fallback({ let scans=scans.clone(); move || { let scans=scans.clone(); async move {
            scans.fetch_add(1,Ordering::SeqCst);
            ([("content-type","application/grpc"),("grpc-status","14")], Vec::<u8>::new())
        }}});
    let api = tokio::spawn(async move { axum::serve(origin, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let group = crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "startup",
        crate::rotation::test_capacity::two_reserves("5"),
        None,
        0
    )
    .unwrap();
    for job in state.funding_jobs().unwrap() {
        state
            .record_credit(&job.wallet_id, "5000000", "fixture", 100)
            .unwrap();
    }
    let (store, worker) = StoreHandle::spawn(state);
    let env = BTreeMap::from([
        ("KEY".into(), format!("{:064x}", 2)),
        ("TOKEN".into(), "fixture".into()),
    ]);
    std::fs::write(
        dir.path().join("local.json"),
        r#"{"operations":[{"method":"GET","path":"/local"}]}"#,
    )
    .unwrap();
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, format!("version=1\n[network]\nmode='tor'\nsocks_endpoint='{proxy_address}'\ncover_traffic_enabled=false\n[wallets.w]\nmode='static'\nprivate_key_env='KEY'\n[sources.local]\nspec='local.json'\nbase_url='http://catalog.example.com'\n[sources.remote]\nspec='http://catalog.example.com/spec'\nbase_url='http://catalog.example.com'\n[servers.one]\nwait_for_catalogs=false\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='w'\nsources=['local','remote']\n")).unwrap();
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    // An idle treasury also retains its startup pass, without enabled funding demand.
    let mut treasury = crate::treasury::Treasury::create(
        dir.path().join("treasury"), dir.path().join("treasury-key"), 2_000_000,
        Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into())),
    ).await.unwrap();
    let indexer = format!("http://127.0.0.1:{}", origin_address.port());
    treasury.configure_sync(crate::treasury::SyncSettings::new(indexer.clone(), 3).unwrap());
    let (handle, commands) = crate::treasury::actor::channel();
    let treasury_stop = CancellationToken::new();
    let treasury_task = tokio::spawn(treasury.run_commands(
        commands,
        crate::treasury::submission::GrpcSubmission::new(indexer.clone(), indexer).unwrap(),
        treasury_stop.clone(),
    ));
    let mut running = deployment.bind(&env).await.unwrap();
    let catalog = running.deferred.as_ref().unwrap().catalog.clone();
    let mcp_address = running.addresses()[0].1;
    running.managed_pools.push(Arc::new(
        ManagedPool::new(
            store.clone(),
            group.clone(),
            BaseRpc::new(
                &format!("http://127.0.0.1:{}/rpc", origin_address.port()),
                12,
            )
            .unwrap(),
            crate::payment::SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap(),
    ));
    let stop = CancellationToken::new();
    let serving = tokio::spawn(running.serve(stop.clone()));
    // The local catalog and authenticated listener remain usable before proxy recovery.
    assert_eq!(catalog.read().views["one"].len(), 1);
    let local_network = crate::network::NetworkContext::new(Default::default()).unwrap();
    let mcp_url = format!("http://{mcp_address}/mcp");
    let response = local_network.http(&crate::network::IsolationId::bootstrap(), &mcp_url, None).unwrap()
        .post(format!("http://{mcp_address}/mcp"))
        .bearer_auth("fixture").header("accept","application/json, text/event-stream")
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}))
        .send().await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    tokio::time::timeout(Duration::from_secs(5), unreachable.notified())
        .await
        .expect("startup must observe and report the unavailable proxy before recovery");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(balances.load(Ordering::SeqCst), 0);
    assert_eq!(scans.load(Ordering::SeqCst), 0);
    let socks = crate::test_socks::Socks::start_on(
        TcpListener::bind(proxy_address).await.unwrap(),
        BTreeMap::from([
            ("catalog.example.com".into(), origin_address),
            ("127.0.0.1".into(), origin_address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let group = group.clone();
            let observed = store
                .call(move |s| {
                    Ok(s.status()?
                        .pools
                        .into_iter()
                        .find(|p| p.id == group)
                        .unwrap()
                        .addresses
                        .iter()
                        .all(|a| a.confirmed_balance == "17"))
                })
                .await
                .unwrap();
            if catalog.read().views["one"].len() == 2
                && observed
                && scans.load(Ordering::SeqCst) > 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("catalog and finite wallet discovery must resume without restart");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(balances.load(Ordering::SeqCst), 2);
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    for table in ["outgoing", "treasury_operations", "funding_recovery"] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "startup created financial work in {table}");
    }
    drop(db);
    stop.cancel();
    treasury_stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), treasury_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(handle);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(socks);
    drop(store);
    worker.await.unwrap();
    api.abort();
}
