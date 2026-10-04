//! Production deployment loading against controlled, unsigned catalogs.
#[path = "support/socks.rs"]
mod socks;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Semaphore, mpsc},
};
use x402_treazury::{
    deployment::Deployment,
    network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
};

struct Fixture {
    address: std::net::SocketAddr,
    gates: Vec<Arc<Semaphore>>,
    arrivals: mpsc::Receiver<usize>,
    cancelled: mpsc::Receiver<usize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(count: usize, bad: Option<usize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let gates: Vec<_> = (0..count).map(|_| Arc::new(Semaphore::new(0))).collect();
        let (arrived, arrivals) = mpsc::channel(64);
        let (closed, cancelled) = mpsc::channel(64);
        let gate_copy = gates.clone();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (gates, arrived, closed) = (gate_copy.clone(), arrived.clone(), closed.clone());
                connections.spawn(async move {
                    let mut request = Vec::new();
                    loop {
                        let byte = socket.read_u8().await.unwrap();
                        request.push(byte);
                        assert!(request.len() < 8192);
                        if request.ends_with(b"\r\n\r\n") { break; }
                    }
                    let text = String::from_utf8(request).unwrap();
                    let id: usize = text.split_whitespace().nth(1).unwrap().trim_start_matches('/').parse().unwrap();
                    let body = if bad == Some(id) { b"invalid".to_vec() } else {
                        serde_json::to_vec(&serde_json::json!({"openapi":"3.0.0","servers":[{"url":"https://api.example"}],"paths":{"/read":{"get":{"summary":"Read","responses":{}}}}})).unwrap()
                    };
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                    arrived.send(id).await.unwrap();
                    let mut byte = [0];
                    tokio::select! {
                        permit = gates[id].acquire() => {
                            permit.unwrap().forget();
                            let _ = socket.write_all(&body).await;
                        }
                        _ = socket.read(&mut byte) => { let _ = closed.send(id).await; }
                    }
                });
            }
        });
        Self {
            address,
            gates,
            arrivals,
            cancelled,
            task,
        }
    }
    async fn next(&mut self) -> usize {
        tokio::time::timeout(Duration::from_secs(5), self.arrivals.recv())
            .await
            .unwrap()
            .unwrap()
    }
    fn release(&self, id: usize) {
        self.gates[id].add_permits(1);
    }
}
fn config(
    dir: &std::path::Path,
    origin: &str,
    count: usize,
    concurrency: Option<usize>,
    network: &str,
) -> std::path::PathBuf {
    let mut value = format!("version=1\n{network}\n");
    if let Some(n) = concurrency {
        value += &format!("[startup]\ncatalog_concurrency={n}\n");
    }
    value +=
        "[wallets.w]\nmode='static'\nprivate_key_env='UNUSED_TEST_KEY'\nmax_price_usd='0.01'\n";
    for id in 0..count {
        value += &format!("[sources.s{id:02}]\nspec='{origin}/{id}'\nprobe_pricing=false\n");
    }
    let names: Vec<_> = (0..count).rev().map(|id| format!("s{id:02}")).collect();
    value += &format!(
        "[servers.test]\nlisten='127.0.0.1:0'\nbearer_token_env='UNUSED_TEST_TOKEN'\nwallet='w'\nsources={}\n",
        serde_json::to_string(&names).unwrap()
    );
    let path = dir.join("deployment.toml");
    std::fs::write(&path, value).unwrap();
    path
}
#[tokio::test]
async fn rolling_slots_and_inventory_match_serial_loading() {
    let mut fixture = Fixture::new(5, None).await;
    let dir = tempfile::tempdir().unwrap();
    let path = config(
        dir.path(),
        &format!("http://{}", fixture.address),
        5,
        Some(2),
        "",
    );
    let load_path = path.clone();
    let task = tokio::spawn(async move { Deployment::load(&load_path).await });
    let mut first = vec![fixture.next().await, fixture.next().await];
    first.sort();
    assert_eq!(first, [0, 1]);
    assert!(fixture.arrivals.try_recv().is_err());
    // Keep source 0 held while successive sources consume the other rolling slot.
    for id in 1..4 {
        fixture.release(id);
        assert_eq!(fixture.next().await, id + 1);
    }
    fixture.release(4);
    fixture.release(0);
    let parallel = task.await.unwrap().unwrap();
    let inventory = serde_json::to_value(parallel.inventory()).unwrap();
    assert_eq!(inventory[0]["tools"][0]["source"], "s00"); // tool-name order, not completion order
    config(
        dir.path(),
        &format!("http://{}", fixture.address),
        5,
        Some(1),
        "",
    );
    for id in 0..5 {
        fixture.release(id);
    }
    let serial = Deployment::load(&path).await.unwrap();
    assert_eq!(inventory, serde_json::to_value(serial.inventory()).unwrap());
}
#[tokio::test]
async fn default_limit_is_sixteen_and_abort_drops_active_requests() {
    let mut fixture = Fixture::new(17, None).await;
    let dir = tempfile::tempdir().unwrap();
    let path = config(
        dir.path(),
        &format!("http://{}", fixture.address),
        17,
        None,
        "",
    );
    assert_eq!(
        Deployment::show_config(&path).await.unwrap()["startup"]["catalog_concurrency"],
        16
    );
    let task = tokio::spawn(async move { Deployment::load(&path).await });
    let mut seen = Vec::new();
    for _ in 0..16 {
        seen.push(fixture.next().await);
    }
    seen.sort();
    assert_eq!(seen, (0..16).collect::<Vec<_>>());
    assert!(fixture.arrivals.try_recv().is_err());
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    for _ in 0..16 {
        tokio::time::timeout(Duration::from_secs(5), fixture.cancelled.recv())
            .await
            .unwrap()
            .unwrap();
    }
    assert!(fixture.arrivals.try_recv().is_err());
}
#[tokio::test]
async fn failure_cancels_other_fetches_and_never_starts_queued_sources() {
    let mut fixture = Fixture::new(3, Some(0)).await;
    let dir = tempfile::tempdir().unwrap();
    let path = config(
        dir.path(),
        &format!("http://{}", fixture.address),
        3,
        Some(2),
        "",
    );
    let task = tokio::spawn(async move { Deployment::load(&path).await });
    let mut first = vec![fixture.next().await, fixture.next().await];
    first.sort();
    assert_eq!(first, [0, 1]);
    fixture.release(0);
    let error = task.await.unwrap().err().unwrap();
    assert!(format!("{error:#}").contains("source s00: loading spec"));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), fixture.cancelled.recv())
            .await
            .unwrap(),
        Some(1)
    );
    assert!(fixture.arrivals.try_recv().is_err());
}
#[tokio::test]
async fn startup_config_is_strict_and_validated_before_fetching() {
    let dir = tempfile::tempdir().unwrap();
    for limit in [0, 65] {
        let path = config(dir.path(), "http://127.0.0.1:1", 1, Some(limit), "");
        assert!(
            Deployment::show_config(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("catalog_concurrency")
        );
        assert!(
            Deployment::load(&path)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("catalog_concurrency")
        );
    }
    let path = config(dir.path(), "http://127.0.0.1:1", 1, Some(64), "");
    assert_eq!(
        Deployment::show_config(&path).await.unwrap()["startup"]["catalog_concurrency"],
        64
    );
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "catalog_concurrency=64",
        "catalog_concurrency=16\nprovider_loading='background'",
    );
    std::fs::write(&path, text).unwrap();
    assert!(Deployment::show_config(&path).await.is_err());
}
#[tokio::test]
async fn parallel_discovery_keeps_origin_credentials() {
    let result = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tor_catalog_child", "--ignored", "--nocapture"])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[tokio::test]
#[ignore = "subprocess helper owns immutable Tor policy"]
async fn tor_catalog_child() {
    let mut fixture = Fixture::new(3, None).await;
    let proxy = socks::Socks::start(
        BTreeMap::from([
            ("a.test".into(), fixture.address),
            ("b.test".into(), fixture.address),
        ]),
        socks::Fault::None,
    )
    .await;
    let policy = NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(proxy.address),
        ..Default::default()
    };
    let dir = tempfile::tempdir().unwrap();
    let network = format!("[network]\nmode='tor'\nsocks_endpoint='{}'", proxy.address);
    let path = config(dir.path(), "http://a.test", 3, Some(3), &network);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("http://a.test/2", "http://b.test/2");
    std::fs::write(&path, text).unwrap();
    let task = tokio::spawn(async move { Deployment::load(&path).await });
    for _ in 0..3 {
        let id = fixture.next().await;
        fixture.release(id);
    }
    task.await.unwrap().unwrap();
    let context = NetworkContext::new(policy).unwrap();
    let records = proxy.records.lock().unwrap();
    assert_eq!(records.len(), 3);
    for record in records.iter() {
        assert_eq!(record.address_type, 3);
        let expected = context
            .credentials(&IsolationId::discovery(&format!("http://{}", record.host)).unwrap());
        assert_eq!((&record.user, &record.password), (&expected.0, &expected.1));
    }
    let a: Vec<_> = records.iter().filter(|r| r.host == "a.test").collect();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].password, a[1].password);
    assert_ne!(
        a[0].password,
        records
            .iter()
            .find(|r| r.host == "b.test")
            .unwrap()
            .password
    );
}

#[tokio::test]
async fn cli_progress_stays_on_stderr_and_stdout_is_inventory_json() {
    let fixture = Fixture::new(2, None).await;
    for id in 0..2 {
        fixture.release(id);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = config(
        dir.path(),
        &format!("http://{}", fixture.address),
        2,
        None,
        "",
    );
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_treazury"))
        .args(["--meta-config", path.to_str().unwrap(), "--list-tools"])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inventory: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inventory[0]["tools"].as_array().unwrap().len(), 2);
    let log = String::from_utf8(output.stderr).unwrap();
    for message in [
        "Catalog load started",
        "Catalog fetch/parse finished",
        "Catalog generation finished",
        "Catalog ready",
        "All catalogs loaded",
    ] {
        assert!(log.contains(message), "missing {message}: {log}");
    }
    assert!(log.contains("elapsed_ms"));
    assert!(!log.contains(&fixture.address.to_string()));
}
