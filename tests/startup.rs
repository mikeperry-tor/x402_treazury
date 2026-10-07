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
                        serde_json::to_vec(&serde_json::json!({"openapi":"3.0.0","servers":[{"url":"https://api.example"}],"paths":{"/read":{"get":{"summary":"Read","responses":{}}},"/other":{"get":{"summary":"Other","responses":{}}}}})).unwrap()
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
async fn default_limit_is_two_and_abort_drops_active_requests() {
    let mut fixture = Fixture::new(3, None).await;
    let dir = tempfile::tempdir().unwrap();
    let path = config(
        dir.path(),
        &format!("http://{}", fixture.address),
        3,
        None,
        "",
    );
    assert_eq!(
        Deployment::show_config(&path).await.unwrap()["startup"]["catalog_concurrency"],
        2
    );
    let task = tokio::spawn(async move { Deployment::load(&path).await });
    let mut seen = Vec::new();
    for _ in 0..2 {
        seen.push(fixture.next().await);
    }
    seen.sort();
    assert_eq!(seen, (0..2).collect::<Vec<_>>());
    assert!(fixture.arrivals.try_recv().is_err());
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    for _ in 0..2 {
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
async fn failed_inspection_finishes_independent_catalogs_with_rolling_bound() {
    let mut fixture = Fixture::new(3, Some(0)).await;
    let dir = tempfile::tempdir().unwrap();
    let path = config(
        dir.path(),
        &format!("http://{}", fixture.address),
        3,
        Some(2),
        "",
    );
    let task = tokio::spawn(async move { Deployment::inspect_catalogs(&path).await });
    let mut first = vec![fixture.next().await, fixture.next().await];
    first.sort();
    assert_eq!(first, [0, 1]);
    assert!(fixture.arrivals.try_recv().is_err());
    fixture.release(0);
    assert_eq!(fixture.next().await, 2);
    assert!(!task.is_finished());
    fixture.release(2);
    fixture.release(1);
    let (result, rows) = task.await.unwrap();
    assert!(result.is_err());
    use x402_treazury::deployment::catalog_evidence::State;
    assert_eq!(rows[0].state, State::Failed);
    assert_eq!(rows[0].stage, "parse");
    assert_eq!(rows[1].state, State::Completed);
    assert_eq!(rows[2].state, State::Completed);
}
#[tokio::test]
async fn failed_inspection_aliases_share_one_failure_without_retry() {
    let mut fixture = Fixture::new(1, Some(0)).await;
    let dir = tempfile::tempdir().unwrap();
    let path = alias_config(dir.path(), &format!("http://{}", fixture.address), 3, 2);
    let task = tokio::spawn(async move { Deployment::inspect_catalogs(&path).await });
    assert_eq!(fixture.next().await, 0);
    fixture.release(0);
    let (result, rows) = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.state
        == x402_treazury::deployment::catalog_evidence::State::Failed
        && row.stage == "parse"));
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
    let path = config(dir.path(), "http://a.test", 4, Some(4), &network);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("http://a.test/2", "http://b.test/2")
        .replace("http://a.test/3", "http://a.test/0");
    std::fs::write(&path, text).unwrap();
    let task = tokio::spawn(async move { Deployment::load(&path).await });
    for _ in 0..3 {
        let id = fixture.next().await;
        fixture.release(id);
    }
    task.await.unwrap().unwrap();
    // Pricing uses the same production discovery factory concurrently, including
    // multiple paths sharing one origin. Unsigned 200s are cached as no price.
    let pricing = tokio::spawn(async {
        let cache = x402_treazury::pricing::PricingCache::default();
        let cfg = x402_treazury::catalog::Config::default();
        let a = serde_json::json!({"paths":{"/0":{"get":{}},"/1":{"get":{}}}});
        let b = serde_json::json!({"paths":{"/2":{"get":{}}}});
        let ta = x402_treazury::catalog::build_tools(&cfg, &a, "a").unwrap();
        let tb = x402_treazury::catalog::build_tools(&cfg, &b, "b").unwrap();
        let (a, b) = tokio::join!(
            cache.discover(&cfg, &a, &ta, "http://a.test"),
            cache.discover(&cfg, &b, &tb, "http://b.test")
        );
        assert!(a.unwrap().is_empty());
        assert!(b.unwrap().is_empty());
    });
    for _ in 0..3 {
        let id = fixture.next().await;
        fixture.release(id);
    }
    pricing.await.unwrap();
    let context = NetworkContext::new(policy).unwrap();
    let records = proxy.records.lock().unwrap();
    assert_eq!(records.len(), 6);
    for record in records.iter() {
        assert_eq!(record.address_type, 3);
        let expected = context
            .credentials(&IsolationId::discovery(&format!("http://{}", record.host)).unwrap());
        assert_eq!((&record.user, &record.password), (&expected.0, &expected.1));
    }
    let a: Vec<_> = records.iter().filter(|r| r.host == "a.test").collect();
    assert_eq!(a.len(), 4);
    assert!(a.iter().all(|r| r.password == a[0].password));
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
    for debug in [false, true] {
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
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
        command.env_remove("RUST_LOG");
        if debug {
            command.env("RUST_LOG", "warn,x402_treazury=debug");
        }
        let output = command
            .args(["catalog", "tools", "--config", path.to_str().unwrap()])
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let inventory: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(inventory[0]["tools"].as_array().unwrap().len(), 4);
        let log = String::from_utf8(output.stderr).unwrap();
        for message in [
            "HTTP response protocol",
            "Catalog load started",
            "Catalog response headers received; reading body",
            "Catalog response body complete",
            "Catalog JSON parse complete",
            "Catalog fetch/parse finished",
            "Catalog generation finished",
        ] {
            assert_eq!(
                log.contains(message),
                debug,
                "unexpected visibility for {message}: {log}"
            );
        }
        for message in ["Catalog ready", "All catalogs loaded"] {
            assert!(log.contains(message), "missing {message}: {log}");
        }
        assert!(log.contains("elapsed_ms"));
        assert_eq!(log.contains("headers_ms"), debug);
        assert_eq!(log.contains("body_ms"), debug);
        assert_eq!(log.contains("parse_ms"), debug);
        assert_eq!(log.contains("catalog_download"), debug);
        assert!(!log.contains(&fixture.address.to_string()));
    }
}

fn alias_config(
    dir: &std::path::Path,
    origin: &str,
    count: usize,
    limit: usize,
) -> std::path::PathBuf {
    let path = config(dir, origin, count, Some(limit), "");
    let mut text = std::fs::read_to_string(&path).unwrap();
    for id in 1..count {
        text = text.replace(&format!("{origin}/{id}"), &format!("{origin}/0"));
    }
    std::fs::write(&path, text).unwrap();
    path
}

#[tokio::test]
async fn cli_reports_parse_failure_stage_without_publishing_inventory() {
    let fixture = Fixture::new(1, Some(0)).await;
    fixture.release(0);
    let dir = tempfile::tempdir().unwrap();
    let path = config(
        dir.path(),
        &format!("http://{}", fixture.address),
        1,
        None,
        "",
    );
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_remove("RUST_LOG")
        .args(["catalog", "tools", "--config", path.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let log = String::from_utf8(output.stderr).unwrap();
    assert!(!log.contains("Catalog response body complete"));
    assert!(log.contains("Catalog load failed; complete inventory unavailable"));
    assert!(log.contains("failure_stage=\"parse\""), "{log}");
    assert!(!log.contains("Catalog JSON parse complete"));
    assert!(!log.contains(&fixture.address.to_string()));
}

#[tokio::test]
async fn failed_inspection_retains_structured_stages_but_never_a_partial_inventory() {
    let fixture = Fixture::new(2, Some(0)).await;
    fixture.release(0);
    fixture.release(1);
    let dir = tempfile::tempdir().unwrap();
    let path = config(
        dir.path(),
        &format!("http://{}", fixture.address),
        2,
        None,
        "",
    );
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .args([
            "config",
            "check",
            "--config",
            path.to_str().unwrap(),
            "--qualification-snapshot",
        ])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snapshot["preparation_failed"], true);
    assert_eq!(snapshot["catalog_stages"][0]["stage"], "parse");
    assert_eq!(snapshot["catalog_stages"][0]["state"], "failed");
    assert_eq!(snapshot["catalog_stages"][1]["state"], "completed");
    assert!(snapshot.get("inventory").is_none());
    assert!(snapshot.get("sources").is_none());
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains(&fixture.address.to_string())
    );
}

#[tokio::test]
async fn aliases_share_fetch_and_parse_but_keep_tools_overrides_and_wallets() {
    let mut fixture = Fixture::new(1, None).await;
    let dir = tempfile::tempdir().unwrap();
    let path = alias_config(dir.path(), &format!("http://{}", fixture.address), 4, 2);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "[sources.s01]",
            "[sources.s01]\nwallet='other'\nbase_url='https://other.invalid'",
        )
        .replace("[sources.s02]", "[sources.s02]\nexclude=['/read']")
        + "\n[wallets.other]\nmode='static'\nprivate_key_env='UNUSED_OTHER_KEY'\n[sources.s01.overrides.s01_read]\ndescription='Authored alias description'\n";
    std::fs::write(&path, text).unwrap();
    let task = tokio::spawn(async move {
        let mut deployment = Deployment::load(&path).await.unwrap();
        let before = serde_json::to_value(deployment.inventory()).unwrap();
        deployment.discover_prices().await.unwrap(); // Disabled: unchanged definitions, including overrides.
        assert_eq!(
            before,
            serde_json::to_value(deployment.inventory()).unwrap()
        );
        before
    });
    assert_eq!(fixture.next().await, 0);
    fixture.release(0);
    let inventory = task.await.unwrap();
    assert_eq!(inventory[0]["tools"].as_array().unwrap().len(), 7);
    assert!(
        inventory[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "s02_read")
    );
    assert_eq!(
        inventory[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "s01_read")
            .unwrap()["description"],
        "Authored alias description"
    );
    assert_eq!(inventory[0]["wallet_bindings"]["s01"]["wallet"], "other");
    assert_eq!(inventory[0]["wallet_bindings"]["s00"]["wallet"], "w");
    assert!(fixture.arrivals.try_recv().is_err()); // Includes aliases scheduled after the first fetch completed.
}

#[tokio::test]
async fn alias_fetches_do_not_share_different_timeouts_or_bypass_stricter_limits() {
    for setting in [
        "timeout=2",
        "max_spec_bytes=1",
        "allow_http1=true",
        "allow_tls12=true",
    ] {
        let mut fixture = Fixture::new(1, None).await;
        let dir = tempfile::tempdir().unwrap();
        let path = alias_config(dir.path(), &format!("http://{}", fixture.address), 2, 1);
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("[sources.s01]", &format!("[sources.s01]\n{setting}"));
        std::fs::write(&path, text).unwrap();
        let task = tokio::spawn(async move { Deployment::load(&path).await });
        assert_eq!(fixture.next().await, 0);
        fixture.release(0);
        assert_eq!(fixture.next().await, 0);
        fixture.release(0);
        let result = task.await.unwrap();
        if setting.starts_with("max_spec") {
            let message = format!("{:#}", result.err().unwrap());
            assert!(message.contains("max_spec_bytes=1"), "{message}");
        } else {
            assert!(result.is_ok());
        }
    }
}

#[tokio::test]
async fn shared_fetch_cancellation_closes_request_and_next_load_fetches_again() {
    let mut fixture = Fixture::new(1, None).await;
    let dir = tempfile::tempdir().unwrap();
    let path = alias_config(dir.path(), &format!("http://{}", fixture.address), 2, 2);
    let task_path = path.clone();
    let task = tokio::spawn(async move { Deployment::load(&task_path).await });
    assert_eq!(fixture.next().await, 0);
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), fixture.cancelled.recv())
            .await
            .unwrap(),
        Some(0)
    );
    assert!(fixture.arrivals.try_recv().is_err());
    let task = tokio::spawn(async move { Deployment::load(&path).await });
    assert_eq!(fixture.next().await, 0);
    fixture.release(0);
    assert!(task.await.unwrap().is_ok());
    assert!(fixture.arrivals.try_recv().is_err());
}

#[tokio::test]
async fn malformed_shared_catalog_fails_startup_without_alias_retry() {
    let mut fixture = Fixture::new(1, Some(0)).await;
    let dir = tempfile::tempdir().unwrap();
    let path = alias_config(dir.path(), &format!("http://{}", fixture.address), 2, 2);
    let task = tokio::spawn(async move { Deployment::load(&path).await });
    assert_eq!(fixture.next().await, 0);
    fixture.release(0);
    assert!(task.await.unwrap().is_err());
    assert!(fixture.arrivals.try_recv().is_err());
}

#[tokio::test]
async fn inspection_retains_shared_http_rejection_without_retry_or_url() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let app = axum::Router::new().fallback(move || {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            axum::http::StatusCode::FORBIDDEN
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let path = alias_config(dir.path(), &format!("http://{address}"), 3, 2);
    let (result, rows) = Deployment::inspect_catalogs(&path).await;
    assert!(result.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert_eq!(
            row.state,
            x402_treazury::deployment::catalog_evidence::State::Failed
        );
        assert_eq!(row.stage, "headers");
        assert_eq!(row.http_status, Some(403));
    }
    assert!(
        !serde_json::to_string(&rows)
            .unwrap()
            .contains(&address.to_string())
    );
    server.abort();
    let _ = server.await;
}
