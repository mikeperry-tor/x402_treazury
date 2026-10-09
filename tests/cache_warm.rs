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
# Direct warming and ordinary inspection must ignore even an enabled relay:
# its bootstrap is deliberately missing and its wallet key is unavailable.
[discovery_relay]
provider='missing-bootstrap.toml'
wallet='w'
serve=true
warm=true
[treasury]
state_dir='state'
daily_treasury_spend_limit_zec='0.01'
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
                r#"{"operations":[{"method":"GET","path":"/read"},{"method":"GET","path":"/excluded"}]}"#,
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
    let warmed = cli(&[
        "catalog",
        "warm",
        "--config",
        path,
        "--source",
        "api",
        "--direct",
        "--discover-pricing",
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
    assert_eq!(report["sources"][0]["fresh_pricing_entries"], 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(tor_calls.load(Ordering::SeqCst), 0);
    assert!(String::from_utf8_lossy(&warmed.stderr).contains("Explicit direct cache warming"));
    // Normal loading still uses Tor policy. Remove the intentionally unselected
    // source; this does not change the selected source's URL/settings/network keys.
    config(Path::new(path), &origin, socks_address, false);
    let inventory = cli(&["catalog", "tools", "--config", path, "--discover-pricing"]).await;
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
    assert_eq!(calls.load(Ordering::SeqCst), 2);
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
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    std::fs::write(
        path,
        original.replace("[sources.api]", "[sources.api]\nhttp_cache_enabled=false"),
    )
    .unwrap();
    let disabled = cli(&["catalog", "warm", "--config", path, "--direct"]).await;
    assert!(!disabled.status.success());
    assert!(String::from_utf8_lossy(&disabled.stderr).contains("http_cache_enabled=false"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
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
    assert_eq!(calls.load(Ordering::SeqCst), 2);
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
    assert_eq!(calls.load(Ordering::SeqCst), 2);
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
    assert_eq!(calls.load(Ordering::SeqCst), 3);
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
