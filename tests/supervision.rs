//! Real production child, keyless MCP calls, and a parent-owned pipe; localhost only.
#![cfg(unix)]
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

const TOKEN: &str = "local-supervisor-test-token";
#[derive(Clone, Default)]
struct Vendor {
    calls: Arc<AtomicUsize>,
    signed: Arc<AtomicUsize>,
    arrived: Arc<Notify>,
    release: Arc<Notify>,
}
async fn vendor() -> (String, Vendor, tokio::task::JoinHandle<()>) {
    let v = Vendor::default();
    let paid = v.clone();
    let slow = v.clone();
    let app = Router::new()
        .route("/free", get(|| async { "free result" }))
        .route(
            "/paid",
            get(move |headers: HeaderMap| {
                let v = paid.clone();
                async move {
                    v.calls.fetch_add(1, Ordering::SeqCst);
                    if headers.contains_key("payment-signature")
                        || headers.contains_key("x-payment")
                    {
                        v.signed.fetch_add(1, Ordering::SeqCst);
                    }
                    // Refusal must precede parsing even a malformed or zero-cost challenge.
                    (
                        StatusCode::PAYMENT_REQUIRED,
                        [("payment-required", "not-base64")],
                        Json(json!({"x402Version":1,"accepts":[]})),
                    )
                        .into_response()
                }
            }),
        )
        .route(
            "/slow",
            get(move || {
                let v = slow.clone();
                async move {
                    v.arrived.notify_one();
                    v.release.notified().await;
                    "finished slow call"
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (
        base,
        v,
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }),
    )
}
fn config(dir: &Path, base: &str, remote_spec: bool) {
    std::fs::write(
        dir.join("spec.json"),
        json!({"paths":{"/free":{"get":{}},"/paid":{"get":{}},"/slow":{"get":{}}}}).to_string(),
    )
    .unwrap();
    let spec = if remote_spec {
        format!("{base}/slow")
    } else {
        "spec.json".into()
    };
    // These missing paths and absent private-key environment prove no wallet access.
    let text = format!(
        r#"version=1
[treasury]
id="00000000-0000-4000-8000-000000000001"
state_dir="must-not-be-created"
key_file="must-not-be-opened"
indexer_url_env="ABSENT_INDEXER"
submission_url_env="ABSENT_SUBMISSION"
daily_input_zec="0.1"
shield_max_fee_zec="0.001"
[wallets.test]
mode="static"
private_key_env="ABSENT_KEY"
[sources.test]
spec="{spec}"
base_url="{base}"
prefix="test"
probe_pricing=false
[servers.one]
listen="127.0.0.1:0"
sources=["test"]
wallet="test"
bearer_token_env="TOKEN"
"#
    );
    std::fs::write(dir.join("config.toml"), text).unwrap();
}
fn spawn(dir: &Path, piped: bool) -> tokio::process::Child {
    let mut c = tokio::process::Command::new(env!("CARGO_BIN_EXE_treazury"));
    c.env_clear()
        .current_dir(dir)
        .env("TOKEN", TOKEN)
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--config",
            "config.toml",
            "--qualification-unsigned",
            "--qualification-parent-stdin",
        ])
        .stdin(if piped { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::from(
            std::fs::File::create(dir.join("stdout")).unwrap(),
        ))
        .stderr(Stdio::from(
            std::fs::File::create(dir.join("stderr")).unwrap(),
        ))
        .kill_on_drop(true);
    c.spawn().unwrap()
}
fn log(dir: &Path) -> String {
    use std::io::Read;
    let mut b = Vec::new();
    let file = match std::fs::File::open(dir.join("stderr")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return String::new(),
        Err(e) => panic!("cannot read fixture stderr: {e}"),
    };
    file.take(262145).read_to_end(&mut b).unwrap();
    assert!(
        b.len() <= 262144,
        "fixture stderr exceeds explicit 262144-byte bound"
    );
    String::from_utf8(b).unwrap()
}
async fn wait_log(dir: &Path, marker: &str) -> String {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let text = log(dir);
            if text.contains(marker) {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("missing {marker}: {}", log(dir)))
}
fn endpoint(log: &str) -> String {
    let line = log.lines().find(|l| l.contains("MCP listening")).unwrap();
    format!(
        "http://{}/mcp",
        regex::Regex::new(r"127\.0\.0\.1:\d+")
            .unwrap()
            .find(line)
            .unwrap()
            .as_str()
    )
}
async fn call(endpoint: &str, tool: &str, token: &str) -> reqwest::Response {
    reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build().unwrap()
        .post(endpoint).bearer_auth(token).header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":{}}}))
        .send().await.unwrap()
}
async fn exit(child: &mut tokio::process::Child) -> std::process::ExitStatus {
    tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .expect("child did not terminate")
        .unwrap()
}
#[tokio::test]
async fn unsigned_child_enforces_auth_refuses_payment_and_drains_on_parent_eof() {
    let (base, v, server) = vendor().await;
    let dir = tempfile::tempdir().unwrap();
    config(dir.path(), &base, false);
    let mut child = spawn(dir.path(), true);
    let pipe = child.stdin.take().unwrap();
    let url = endpoint(&wait_log(dir.path(), "MCP listening").await);
    assert_eq!(call(&url, "test_free", "bad").await.status(), 401);
    let free: Value = call(&url, "test_free", TOKEN).await.json().await.unwrap();
    assert_eq!(free["result"]["isError"], false, "{free}");
    let denied: Value = call(&url, "test_paid", TOKEN).await.json().await.unwrap();
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    assert!(
        denied.to_string().contains("qualification_payment_denied"),
        "{denied}"
    );
    assert_eq!(v.calls.load(Ordering::SeqCst), 1);
    assert_eq!(v.signed.load(Ordering::SeqCst), 0);
    let task = tokio::spawn(async move {
        call(&url, "test_slow", TOKEN)
            .await
            .json::<Value>()
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(5), v.arrived.notified())
        .await
        .unwrap();
    drop(pipe);
    wait_log(dir.path(), "draining in-flight requests").await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "child abandoned an in-flight request"
    );
    v.release.notify_one();
    let response = task.await.unwrap();
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(exit(&mut child).await.success());
    let logs = log(dir.path());
    assert!(logs.contains("qualification_payment_denied"));
    assert!(logs.contains("supervisor connection closed"));
    assert!(!dir.path().join("must-not-be-created").exists());
    assert_eq!(
        std::fs::metadata(dir.path().join("stdout")).unwrap().len(),
        0
    );
    server.abort();
}
#[tokio::test]
async fn eof_interrupts_catalog_startup_and_invalid_control_data_fails_closed() {
    for invalid_data in [false, true] {
        let (base, v, server) = vendor().await;
        let dir = tempfile::tempdir().unwrap();
        config(dir.path(), &base, true);
        let mut child = spawn(dir.path(), true);
        let mut pipe = child.stdin.take().unwrap();
        tokio::time::timeout(Duration::from_secs(10), v.arrived.notified())
            .await
            .unwrap();
        if invalid_data {
            use tokio::io::AsyncWriteExt;
            pipe.write_all(b"x").await.unwrap();
        } else {
            drop(pipe);
        }
        assert!(!exit(&mut child).await.success());
        let logs = log(dir.path());
        assert!(!logs.contains("MCP listening"));
        assert!(
            logs.contains(if invalid_data {
                "accepts EOF only"
            } else {
                "closed during catalog startup"
            }),
            "{logs}"
        );
        server.abort();
    }
}
#[tokio::test]
async fn parent_pipe_is_required_and_signal_shutdown_does_not_wait_for_pipe_eof() {
    let (base, _v, server) = vendor().await;
    let dir = tempfile::tempdir().unwrap();
    config(dir.path(), &base, false);
    let mut invalid = spawn(dir.path(), false);
    assert!(!exit(&mut invalid).await.success());
    assert!(log(dir.path()).contains("requires a readable stdin pipe"));
    let mut child = spawn(dir.path(), true);
    let pipe = child.stdin.take().unwrap();
    wait_log(dir.path(), "MCP listening").await;
    let pid = child.id().unwrap();
    assert!(child.try_wait().unwrap().is_none());
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGTERM) }, 0);
    assert!(exit(&mut child).await.success());
    assert!(log(dir.path()).contains("draining in-flight requests"));
    drop(pipe);
    server.abort();
}
#[tokio::test]
async fn actual_parent_exit_closes_lifetime_channel_without_destructor_cleanup() {
    let (base, _v, server) = vendor().await;
    let dir = tempfile::tempdir().unwrap();
    config(dir.path(), &base, false);
    let mut parent = tokio::process::Command::new(std::env::current_exe().unwrap());
    parent
        .args([
            "--ignored",
            "--exact",
            "orphan_parent_helper",
            "--nocapture",
        ])
        .env_clear()
        .env("CHILD_DIR", dir.path())
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut parent = parent.spawn().unwrap();
    let text = wait_log(dir.path(), "MCP listening").await;
    let url = endpoint(&text);
    let address = url
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/mcp")
        .unwrap();
    std::fs::write(dir.path().join("exit-parent"), b"exit").unwrap();
    assert_eq!(exit(&mut parent).await.code(), Some(86));
    wait_log(dir.path(), "supervisor connection closed").await;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if tokio::net::TcpListener::bind(address).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    server.abort();
}
#[test]
#[ignore = "subprocess parent-death fixture, invoked by parent test"]
fn orphan_parent_helper() {
    let Some(dir) = std::env::var_os("CHILD_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut child = spawn(&dir, true);
        let _pipe = child.stdin.take().unwrap();
        for _ in 0..1500 {
            if dir.join("exit-parent").exists() {
                std::process::exit(86);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("parent fixture deadline exceeded");
    });
}

#[tokio::test]
async fn unsigned_mode_rejects_managed_funding_and_source_management_configs() {
    let (base, _v, server) = vendor().await;
    for kind in ["managed", "auto_fund", "source_management"] {
        let dir = tempfile::tempdir().unwrap();
        config(dir.path(), &base, false);
        let path = dir.path().join("config.toml");
        let mut document: toml::Value =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        match kind {
            "managed" => {
                document["wallets"]["test"] = toml::from_str::<toml::Value>(
                    "mode='zcash_rotation'\nmax_input_zec='0.02'\nmax_fee_bps=500",
                )
                .unwrap();
                document.as_table_mut().unwrap().insert(
                    "funding".into(),
                    toml::from_str::<toml::Value>("confidentiality='public'").unwrap(),
                );
            }
            "auto_fund" => {
                document.as_table_mut().unwrap().insert(
                    "funding".into(),
                    toml::from_str::<toml::Value>("auto_fund=true").unwrap(),
                );
            }
            _ => {
                document.as_table_mut().unwrap().insert(
                    "source_management".into(),
                    toml::from_str::<toml::Value>(
                        "wallet='test'\nregistry_file='must-not-be-created.sqlite'",
                    )
                    .unwrap(),
                );
            }
        }
        std::fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
        let mut child = spawn(dir.path(), true);
        let _pipe = child.stdin.take().unwrap();
        assert!(!exit(&mut child).await.success());
        let text = log(dir.path());
        assert!(
            text.contains("unsigned qualification forbids"),
            "{kind}: {text}"
        );
        assert!(!dir.path().join("must-not-be-created").exists());
        assert!(!dir.path().join("must-not-be-created.sqlite").exists());
    }
    server.abort();
}

#[path = "support/socks.rs"]
mod socks;
#[tokio::test]
async fn unsigned_child_uses_discovery_isolation_and_has_no_direct_fallback() {
    use std::collections::BTreeMap;
    use x402_treazury::network::{IsolationId, NetworkContext, NetworkPolicy};
    for fault in [socks::Fault::None, socks::Fault::Refuse] {
        let (base, v, server) = vendor().await;
        let destination = base.strip_prefix("http://").unwrap().parse().unwrap();
        let proxy = socks::Socks::start(
            BTreeMap::from([("unsigned.invalid".into(), destination)]),
            fault,
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        config(dir.path(), "http://unsigned.invalid", false);
        let path = dir.path().join("config.toml");
        let network = format!(
            "[network]\nmode='tor'\nsocks_endpoint='{}'\nconnect_timeout_seconds=1\nrequest_timeout_seconds=1\n",
            proxy.address
        );
        let text = std::fs::read_to_string(&path).unwrap() + &network;
        std::fs::write(&path, text).unwrap();
        let mut child = spawn(dir.path(), true);
        let pipe = child.stdin.take().unwrap();
        let url = endpoint(&wait_log(dir.path(), "MCP listening").await);
        let result: Value = call(&url, "test_paid", TOKEN).await.json().await.unwrap();
        assert_eq!(result["result"]["isError"], true);
        let expected = usize::from(matches!(fault, socks::Fault::None));
        assert_eq!(v.calls.load(Ordering::SeqCst), expected);
        assert_eq!(v.signed.load(Ordering::SeqCst), 0);
        let policy: NetworkPolicy = toml::from_str::<toml::Value>(&network).unwrap()["network"]
            .clone()
            .try_into()
            .unwrap();
        let credentials = NetworkContext::new(policy)
            .unwrap()
            .credentials(&IsolationId::discovery("http://unsigned.invalid/paid").unwrap());
        {
            let records = proxy.records.lock().unwrap();
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.address_type, 3);
            assert_eq!(record.host, "unsigned.invalid");
            assert_eq!((record.user.clone(), record.password.clone()), credentials);
        }
        drop(pipe);
        assert!(exit(&mut child).await.success());
        server.abort();
    }
}
