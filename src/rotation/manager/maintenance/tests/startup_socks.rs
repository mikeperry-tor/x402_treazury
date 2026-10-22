//! Requested balance ownership remains independent of finite startup connectivity.
use super::*;
use std::collections::BTreeMap;

#[tokio::test]
async fn requested_refresh_bypasses_absent_startup_socks_without_resetting_cooldown() {
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(std::env::current_exe().unwrap())
            .kill_on_drop(true)
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
            .args([
                "--ignored",
                "--exact",
                "rotation::manager::maintenance::tests::startup_socks::requested_refresh_child",
                "--nocapture",
            ])
            .output(),
    )
    .await
    .expect("requested-refresh child exceeded its test deadline")
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[tokio::test]
#[ignore = "isolated Tor policy; invoked by parent"]
async fn requested_refresh_child() {
    let log = tempfile::NamedTempFile::new().unwrap();
    tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .init();
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = reserved.local_addr().unwrap();
    drop(reserved);
    crate::network::install(crate::network::NetworkPolicy {
        mode: crate::network::Mode::Tor,
        socks_endpoint: Some(endpoint),
        connect_timeout_seconds: Some(1),
        cover_traffic_enabled: Some(false),
        ..Default::default()
    })
    .unwrap();
    // This fixture has one durable requested refresh, independently of startup.
    let f = fixture(false, false).await;
    let pool = f.pool.clone();
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let running = tokio::spawn(async move { pool.run_reconciliation(stopped).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.status().await.balance_refresh_error.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("requested refresh must execute even while startup waits for SOCKS");
    f.wait_completion().await;
    let due = f.store.balance_due(&f.wallet).await;
    assert!(due > Instant::now());
    assert!(f.status().await.low_balance);
    assert_eq!(f.rpc.calls.load(Ordering::SeqCst), 0);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !std::fs::read_to_string(log.path())
        .unwrap()
        .contains("tor_socks_unreachable")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "startup did not report the absent SOCKS endpoint"
        );
        tokio::task::yield_now().await;
    }

    tokio::time::pause();
    // Keep real socket/Store work runnable without automatically jumping clocks.
    let runnable = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let origin = f.endpoint.strip_prefix("http://").unwrap().parse().unwrap();
    let socks = crate::test_socks::Socks::start_on(
        tokio::net::TcpListener::bind(endpoint).await.unwrap(),
        BTreeMap::from([("127.0.0.1".into(), origin)]),
        crate::test_socks::Fault::None,
    )
    .await;
    runnable.abort();
    tokio::time::resume();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(7),
            crate::network::global().wait_for_startup_socks()
        )
        .await
        .is_ok(),
        "SOCKS recovery was not observed: {}",
        std::fs::read_to_string(log.path()).unwrap()
    );
    tokio::time::pause();
    let runnable = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..100 {
        f.pool.note_insufficient(f.wallet.clone()).await.unwrap();
        tokio::task::yield_now().await;
    }
    assert_eq!(f.store.balance_due(&f.wallet).await, due);
    assert_eq!(f.rpc.calls.load(Ordering::SeqCst), 0);
    // Startup and new notifications cannot turn the prior failure into a fast retry.
    tokio::time::advance(due.duration_since(Instant::now()) - Duration::from_secs(1)).await;
    f.pool.note_insufficient(f.wallet.clone()).await.unwrap();
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(f.store.balance_due(&f.wallet).await, due);
    assert_eq!(f.rpc.calls.load(Ordering::SeqCst), 0);
    tokio::time::advance(Duration::from_secs(1) + CADENCE).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while f.status().await.low_balance {
        assert!(
            std::time::Instant::now() < deadline,
            "requested retry did not run after cooldown"
        );
        tokio::task::yield_now().await;
    }
    assert_eq!(f.rpc.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.status().await.confirmed_balance, "17");
    assert!(f.status().await.balance_refresh_error.is_none());
    assert!(f.rpc.signed.lock().unwrap().is_empty());
    stop.cancel();
    running.await.unwrap().unwrap();
    let db = rusqlite::Connection::open(f.dir.path().join("state/state.sqlite")).unwrap();
    for table in ["outgoing", "treasury_operations", "funding_recovery"] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "balance discovery created financial work in {table}"
        );
    }
    drop(db);
    runnable.abort();
    tokio::time::resume();
    drop(socks);
    f.close().await;
}
