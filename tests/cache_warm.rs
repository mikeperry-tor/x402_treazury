//! Explicit direct warming in a separate process, with no wallet keys or paid I/O.
use axum::{
    Router,
    extract::{Request, State},
    response::IntoResponse,
    routing::any,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::process::Command;

async fn cli(args: &[&str]) -> std::process::Output {
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
            .args(args)
            .env_remove("RUST_LOG")
            .env("RUST_BACKTRACE", "0")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("CLI timeout")
    .unwrap()
}
fn config(path: &Path, origin: &str, socks: std::net::SocketAddr, extra: bool) {
    let extra = if extra {
        format!("[sources.unselected]\nspec='{origin}/must-not-fetch'\n")
    } else {
        String::new()
    };
    std::fs::write(
        path,
        format!(
            r#"
version=1
[network]
mode='tor'
socks_endpoint='{socks}'
isolation_namespace='direct-warm-fixture'
# Explicit direct warming bypasses relay bootstrap and payment:
# its provider is deliberately missing and its wallet key is unavailable.
[relay_fallback]
relays.GET=['missing-bootstrap.toml']
wallet='w'
[treasury]
state_dir='state'
max_refund_shielding_fee_zec='0.001'
[wallets.w]
mode='static'
private_key_env='UNUSED_KEY'
max_api_payment_usdc='0.01'
[sources.api]
spec='{origin}/spec'
base_url='{origin}'
{extra}
[servers.test]
listen='127.0.0.1:0'
bearer_token_env='UNUSED_TOKEN'
wallet='w'
sources=['api']
exclude_tools=['api_excluded']
"#
        ),
    )
    .unwrap();
}

#[tokio::test]
async fn direct_warm_is_scoped_and_reused_by_tor_but_never_becomes_live_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("state")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    async fn handler(State(calls): State<Arc<AtomicUsize>>, req: Request) -> impl IntoResponse {
        assert_eq!(req.method(), "GET");
        assert!(!req.headers().contains_key("payment-signature"));
        assert!(!req.headers().contains_key("x-payment"));
        assert!(!req.headers().contains_key("authorization"));
        calls.fetch_add(1, Ordering::SeqCst);
        let price = STANDARD.encode(r#"{"accepts":[{"amount":"1000","asset":"USDC"}]}"#);
        match req.uri().path() {
            "/spec" => (
                axum::http::StatusCode::OK,
                [
                    ("cache-control", "max-age=600".into()),
                    ("etag", "\"one\"".into()),
                ],
                r#"{"operations":[{"method":"GET","path":"/read","pricing":{"amount":"0.001","currency":"USD"}},{"method":"GET","path":"/excluded"}]}"#,
            ),
            "/read" => (
                axum::http::StatusCode::PAYMENT_REQUIRED,
                [
                    ("cache-control", "max-age=600".into()),
                    ("payment-required", price),
                ],
                "",
            ),
            other => panic!("unselected request: {other}"),
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .fallback(any(handler))
        .with_state(calls.clone());
    let origin_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let socks = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socks_address = socks.local_addr().unwrap();
    let tor_calls = Arc::new(AtomicUsize::new(0));
    let connections = tor_calls.clone();
    let socks_task = tokio::spawn(async move {
        loop {
            let (socket, _) = socks.accept().await.unwrap();
            connections.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });
    let path = dir.path().join("deployment.toml");
    config(&path, &origin, socks_address, true);
    let path = path.to_str().unwrap();
    let unknown = cli(&[
        "catalog", "warm", "--config", path, "--source", "missing", "--direct",
    ])
    .await;
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown cache-warming source"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let network_path = dir.path().join("network.toml");
    std::fs::write(&network_path, format!("[network]\nmode='tor'\nsocks_endpoint='{socks_address}'\nisolation_namespace='direct-warm-fixture'\n")).unwrap();
    let warmed = cli(&[
        "catalog",
        "warm",
        "--config",
        path,
        "--network-config",
        network_path.to_str().unwrap(),
        "--source",
        "api",
        "--direct",
    ])
    .await;
    assert!(
        warmed.status.success(),
        "{}",
        String::from_utf8_lossy(&warmed.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&warmed.stdout).unwrap();
    assert_eq!(report["direct"], true);
    assert_eq!(report["sources"].as_array().unwrap().len(), 1);
    assert_eq!(report["sources"][0]["catalog_cache"], "fresh");
    assert!(report["sources"][0].get("fresh_pricing_entries").is_none());
    assert!(report["sources"][0].get("pricing").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(tor_calls.load(Ordering::SeqCst), 0);
    assert!(String::from_utf8_lossy(&warmed.stderr).contains("Explicit direct cache warming"));
    // Normal loading still uses Tor policy. Remove the intentionally unselected
    // source; this does not change the selected source's URL/settings/network keys.
    config(Path::new(path), &origin, socks_address, false);
    // Ordinary catalog loading validates configured local relay metadata even
    // when the catalog is cached. It still must not load the unavailable key,
    // contact the relay, or create wallet state.
    let configured = std::fs::read_to_string(path).unwrap().replace(
        "missing-bootstrap.toml",
        concat!(env!("CARGO_MANIFEST_DIR"), "/providers/curl/provider.toml"),
    );
    std::fs::write(path, configured).unwrap();
    let inventory = cli(&["catalog", "tools", "--config", path]).await;
    assert!(
        inventory.status.success(),
        "{}",
        String::from_utf8_lossy(&inventory.stderr)
    );
    assert!(String::from_utf8_lossy(&inventory.stdout).contains("$0.001"));
    assert!(
        String::from_utf8_lossy(&inventory.stderr)
            .contains("Using explicitly directly warmed discovery cache")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(tor_calls.load(Ordering::SeqCst), 0);
    assert!(!dir.path().join("state/state.sqlite").exists());
    assert!(!dir.path().join("state/wallet.key").exists());
    // The opt-in belongs to the exact configured network/isolation policy.
    let original = std::fs::read_to_string(path).unwrap();
    std::fs::write(
        path,
        original.replace("direct-warm-fixture", "different-namespace"),
    )
    .unwrap();
    let different = cli(&["catalog", "tools", "--config", path]).await;
    assert!(!different.status.success());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    std::fs::write(
        path,
        original.replace("[sources.api]", "[sources.api]\nhttp_cache_enabled=false"),
    )
    .unwrap();
    let disabled = cli(&["catalog", "warm", "--config", path, "--direct"]).await;
    assert!(!disabled.status.success());
    assert!(String::from_utf8_lossy(&disabled.stderr).contains("http_cache_enabled=false"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    std::fs::write(
        path,
        original.replace(
            "[sources.api]",
            "[sources.api]\ncatalog_cache_ttl_seconds=0",
        ),
    )
    .unwrap();
    let disabled = cli(&["catalog", "warm", "--config", path, "--direct"]).await;
    assert!(!disabled.status.success());
    assert!(String::from_utf8_lossy(&disabled.stderr).contains("catalog_cache_ttl_seconds=0"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    std::fs::write(path, original).unwrap();
    // Expired direct entries cannot be revalidated over Tor or trigger direct I/O.
    let db = rusqlite::Connection::open(dir.path().join("state/http-cache/discovery-v1.sqlite"))
        .unwrap();
    db.execute(
        "UPDATE entries SET metadata=json_set(metadata,'$.expires',0)",
        [],
    )
    .unwrap();
    let expired = cli(&["catalog", "tools", "--config", path]).await;
    assert!(!expired.status.success());
    assert!(tor_calls.load(Ordering::SeqCst) > 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Refresh is a separate explicit direct command.
    let refreshed = cli(&[
        "catalog", "warm", "--config", path, "--source", "api", "--direct",
    ])
    .await;
    assert!(
        refreshed.status.success(),
        "{}",
        String::from_utf8_lossy(&refreshed.stderr)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    origin_task.abort();
    socks_task.abort();
}

#[tokio::test]
async fn warming_requires_existing_state_and_cannot_be_selected_for_serving() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    config(
        &path,
        "http://127.0.0.1:1",
        "127.0.0.1:2".parse().unwrap(),
        false,
    );
    let path = path.to_str().unwrap();
    let missing = cli(&["catalog", "warm", "--config", path, "--direct"]).await;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("existing treasury state directory"));
    assert!(!dir.path().join("state").exists());
    let forbidden = cli(&["serve", "--config", path, "--direct"]).await;
    assert!(!forbidden.status.success());
    assert!(String::from_utf8_lossy(&forbidden.stderr).contains("unexpected argument '--direct'"));
    let standalone = cli(&["catalog", "warm", "--provider", "unused.toml", "--direct"]).await;
    assert!(!standalone.status.success());
}

#[tokio::test]
async fn warming_sorts_local_and_shared_remote_catalogs_without_endpoint_reads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("state")).unwrap();
    let catalog_calls = Arc::new(AtomicUsize::new(0));
    let price_calls = Arc::new(AtomicUsize::new(0));
    let counts = (catalog_calls.clone(), price_calls.clone());
    async fn handler(
        State((catalog, pricing)): State<(Arc<AtomicUsize>, Arc<AtomicUsize>)>,
        req: Request,
    ) -> impl IntoResponse {
        assert_eq!(req.method(), "GET");
        assert!(!req.headers().contains_key("payment-signature"));
        assert!(!req.headers().contains_key("x-payment"));
        match req.uri().path() {
            "/spec" => {
                catalog.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::OK,
                    [
                        ("cache-control", "max-age=600".to_owned()),
                        ("etag", "\"shared\"".to_owned()),
                    ],
                    r#"{"operations":[{"method":"GET","path":"/read"}]}"#,
                )
            }
            "/read" => {
                pricing.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::PAYMENT_REQUIRED,
                    [
                        ("cache-control", "max-age=600".to_owned()),
                        (
                            "payment-required",
                            STANDARD.encode(r#"{"accepts":[{"amount":"1000","asset":"USDC"}]}"#),
                        ),
                    ],
                    "",
                )
            }
            other => panic!("unexpected request {other}"),
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(any(handler)).with_state(counts),
        )
        .await
        .unwrap()
    });
    std::fs::write(
        dir.path().join("local.json"),
        r#"{"operations":[{"method":"GET","path":"/read"}]}"#,
    )
    .unwrap();
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        format!(
            r#"
version=1
[startup]
catalog_concurrency=2
[treasury]
state_dir='state'
max_refund_shielding_fee_zec='0.001'
[wallets.w]
mode='static'
private_key_env='UNUSED_KEY'
max_api_payment_usdc='0.01'
[sources.z_remote]
spec='{origin}/spec'
base_url='{origin}'
[sources.a_remote]
spec='{origin}/spec'
base_url='{origin}'
[sources.m_local]
spec='local.json'
base_url='{origin}'
[servers.test]
listen='127.0.0.1:0'
bearer_token_env='UNUSED_TOKEN'
wallet='w'
sources=['z_remote','m_local','a_remote']
"#
        ),
    )
    .unwrap();
    let path = path.to_str().unwrap();
    let run = || async move {
        let args = ["catalog", "warm", "--config", path, "--direct"];
        let output = cli(&args).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let warmed = run().await;
    let rows = warmed["sources"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["source"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a_remote", "m_local", "z_remote"]
    );
    assert_eq!(rows[0]["catalog_cache"], "fresh");
    assert_eq!(rows[1]["catalog_cache"], "local_file");
    assert_eq!(rows[2]["catalog_cache"], "fresh");
    assert!(
        rows.iter()
            .all(|r| r.get("pricing").is_none() && r.get("fresh_pricing_entries").is_none())
    );
    assert_eq!(
        catalog_calls.load(Ordering::SeqCst),
        1,
        "shared catalog fetched once"
    );
    assert_eq!(price_calls.load(Ordering::SeqCst), 0);

    for _ in 0..2 {
        let repeated = run().await;
        assert_eq!(
            repeated["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| (&row["source"], &row["catalog_cache"]))
                .collect::<Vec<_>>(),
            rows.iter()
                .map(|row| (&row["source"], &row["catalog_cache"]))
                .collect::<Vec<_>>()
        );
        assert_eq!(catalog_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            price_calls.load(Ordering::SeqCst),
            0,
            "warming never acquires endpoint prices"
        );
    }
    let db = rusqlite::Connection::open(dir.path().join("state/http-cache/discovery-v1.sqlite"))
        .unwrap();
    // Expire cached catalogs; the next explicit warm refreshes the shared document
    // while the local source stays local.
    db.execute(
        "UPDATE entries SET metadata=json_set(metadata,'$.expires',0)",
        [],
    )
    .unwrap();
    let refreshed = run().await;
    assert!(
        refreshed["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("fresh_pricing_entries").is_none() && r.get("pricing").is_none())
    );
    assert_eq!(catalog_calls.load(Ordering::SeqCst), 2);
    assert_eq!(price_calls.load(Ordering::SeqCst), 0);
    assert!(!dir.path().join("state/state.sqlite").exists());
    assert!(!dir.path().join("state/wallet.key").exists());
    task.abort();
}

#[tokio::test]
async fn warming_rolls_forward_within_source_bound_and_sorts_completion_order() {
    use tokio::sync::{Semaphore, mpsc};
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("state")).unwrap();
    let gates: Vec<_> = (0..3).map(|_| Arc::new(Semaphore::new(0))).collect();
    let (send, mut arrivals) = mpsc::channel(3);
    async fn handler(
        State((gates, arrivals)): State<(Vec<Arc<Semaphore>>, mpsc::Sender<usize>)>,
        req: Request,
    ) -> impl IntoResponse {
        assert_eq!(req.method(), "GET");
        assert!(!req.headers().contains_key("payment-signature"));
        let id: usize = req.uri().path().trim_start_matches('/').parse().unwrap();
        arrivals.send(id).await.unwrap();
        gates[id].acquire().await.unwrap().forget();
        r#"{"operations":[{"method":"GET","path":"/read"}]}"#
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = (gates.clone(), send);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(any(handler)).with_state(state),
        )
        .await
        .unwrap()
    });
    let path = dir.path().join("deployment.toml");
    let mut config = String::from(
        "version=1\n[startup]\ncatalog_concurrency=2\n[treasury]\nstate_dir='state'\nmax_refund_shielding_fee_zec='0.001'\n[wallets.w]\nmode='static'\nprivate_key_env='UNUSED_KEY'\nmax_api_payment_usdc='0.01'\n",
    );
    for id in 0..3 {
        config.push_str(&format!(
            "[sources.s{id}]\nspec='{origin}/{id}'\nbase_url='{origin}'\n"
        ));
    }
    config.push_str("[servers.test]\nlisten='127.0.0.1:0'\nbearer_token_env='UNUSED_TOKEN'\nwallet='w'\nsources=['s0','s1','s2']\n");
    std::fs::write(&path, config).unwrap();
    let warming = tokio::spawn(async move {
        cli(&[
            "catalog",
            "warm",
            "--config",
            path.to_str().unwrap(),
            "--direct",
        ])
        .await
    });
    let timeout = std::time::Duration::from_secs(10);
    let first = tokio::time::timeout(timeout, arrivals.recv())
        .await
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(timeout, arrivals.recv())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    assert!(first < 2 && second < 2);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), arrivals.recv())
            .await
            .is_err(),
        "third catalog must wait for a source slot"
    );
    // Leave the first source blocked: completing its peer must advance the
    // rolling window, rather than waiting for the entire first batch.
    gates[second].add_permits(1);
    assert_eq!(
        tokio::time::timeout(timeout, arrivals.recv())
            .await
            .unwrap()
            .unwrap(),
        2
    );
    gates[2].add_permits(1);
    gates[first].add_permits(1);
    let output = warming.await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["source"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["s0", "s1", "s2"]
    );
    assert!(
        arrivals.try_recv().is_err(),
        "exactly one request per catalog"
    );
    server.abort();
}
