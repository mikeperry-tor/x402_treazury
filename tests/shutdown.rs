//! Actual CLI signals while an unfunded, locally signed payment is in flight.
#![cfg(unix)]
use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Notify,
};

#[derive(Clone, Default)]
struct Vendor {
    unsigned: Arc<AtomicUsize>,
    signed: Arc<AtomicUsize>,
    arrived: Arc<Notify>,
    release: Arc<Notify>,
    pause_unsigned: Arc<AtomicBool>,
    unsigned_arrived: Arc<Notify>,
    unsigned_release: Arc<Notify>,
}
async fn payment(
    axum::extract::State(v): axum::extract::State<Vendor>,
    headers: HeaderMap,
) -> Response {
    if headers.contains_key("payment-signature") {
        v.signed.fetch_add(1, Ordering::SeqCst);
        v.arrived.notify_one();
        v.release.notified().await;
        return "completed payment".into_response();
    }
    v.unsigned.fetch_add(1, Ordering::SeqCst);
    if v.pause_unsigned.load(Ordering::SeqCst) {
        v.unsigned_arrived.notify_one();
        v.unsigned_release.notified().await;
    }
    let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/pay","description":"fixture","mimeType":"text/plain"},
        "accepts":[{"scheme":"exact","network":"eip155:8453","asset":x402_treazury::payment::USDC,"amount":"14000",
        "payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
    )
        .into_response()
}

async fn scenario(meta: bool, signal: &str, finish: bool, disconnect: bool) {
    let vendor = Vendor::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/pay", post(payment))
        .with_state(vendor.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("spec.json"),
        json!({"paths":{"/pay":{"post":{}}}}).to_string(),
    )
    .unwrap();
    let source = format!(
        "spec = 'spec.json'\nbase_url = '{base}'\nprefix = 'test'\nprobe_pricing = false\ntimeout = 60\n"
    );
    let config = if meta {
        format!(
            "version = 1\n[sources.test]\n{source}\n[wallets.shared]\nmode = 'static'\nprivate_key_env = 'EVM_PRIVATE_KEY'\n[servers.one]\nlisten = '127.0.0.1:0'\nsources = ['test']\nwallet = 'shared'\nbearer_token_env = 'TEST_TOKEN'\n[servers.two]\nlisten = '127.0.0.1:0'\nsources = ['test']\nwallet = 'shared'\nbearer_token_env = 'TEST_TOKEN'\n"
        )
    } else {
        source
    };
    std::fs::write(tmp.path().join("config.toml"), config).unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
    command
        .env_clear()
        .current_dir(tmp.path())
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .env("EVM_PRIVATE_KEY", format!("{:064x}", 1))
        .env("TEST_TOKEN", "fixture-token")
        .env("X402_MCP_BEARER_TOKEN", "fixture-token")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if meta {
        command.args(["serve", "--config", "config.toml"]);
    } else {
        command.args([
            "serve",
            "--provider",
            "config.toml",
            "--transport",
            "http",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
        ]);
    }
    let mut child = command.spawn().unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let pattern = regex::Regex::new(r"127\.0\.0\.1:\d+").unwrap();
    let mut addresses = Vec::new();
    let mut logs = String::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        while addresses.len() < if meta { 2 } else { 1 } {
            let mut line = String::new();
            assert_ne!(
                stderr.read_line(&mut line).await.unwrap(),
                0,
                "startup failed: {logs}"
            );
            logs.push_str(&line);
            if line.contains("MCP listening") {
                addresses.push(pattern.find(&line).unwrap().as_str().to_owned());
            }
        }
    })
    .await
    .expect("startup deadline");
    let (draining, mut drain_notice) = tokio::sync::mpsc::unbounded_channel();
    let logs = tokio::spawn(async move {
        loop {
            let mut line = String::new();
            if stderr.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            if line.contains("draining accepted work") {
                let _ = draining.send(());
            }
            logs.push_str(&line);
        }
        logs
    });
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap();
    let request = http.post(format!("http://{}/mcp", addresses[0])).bearer_auth("fixture-token")
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"test_pay","arguments":{}}}));
    let mut pending = tokio::spawn(async move { request.send().await });
    tokio::time::timeout(Duration::from_secs(10), vendor.arrived.notified())
        .await
        .expect("signed request must arrive");
    let started = std::time::Instant::now();
    assert!(
        std::process::Command::new("/bin/kill")
            .args([signal, &child.id().unwrap().to_string()])
            .status()
            .unwrap()
            .success()
    );
    tokio::time::timeout(Duration::from_secs(3), drain_notice.recv())
        .await
        .expect("drain notice must be printed promptly")
        .expect("stderr closed before drain notice");
    tokio::time::timeout(Duration::from_secs(3), async {
        for address in &addresses {
            while tokio::net::TcpStream::connect(address).await.is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    })
    .await
    .expect("all listeners must stop accepting before drain finishes");
    assert!(
        child.try_wait().unwrap().is_none(),
        "in-flight call must be allowed to drain"
    );
    if disconnect {
        pending.abort();
    }
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "no automatic ten-second abandonment"
    );
    if finish {
        vendor.release.notify_one();
        if !disconnect {
            let response: Value = (&mut pending).await.unwrap().unwrap().json().await.unwrap();
            assert_eq!(
                response["result"]["content"][0]["text"],
                "completed payment"
            );
        }
    } else {
        assert!(
            std::process::Command::new("/bin/kill")
                .args([signal, &child.id().unwrap().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
    let output = tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
        .await
        .expect("bounded shutdown")
        .unwrap();
    pending.abort();
    let logs = logs.await.unwrap();
    assert_eq!(logs.matches("draining accepted work").count(), 1, "{logs}");
    assert!(
        logs.contains("pending payments finish safely. Please wait."),
        "{logs}"
    );
    assert_eq!(output.status.success(), finish, "{logs}");
    assert!(
        output.stdout.is_empty(),
        "HTTP server must not write protocol/log text to stdout"
    );
    if !finish {
        assert!(started.elapsed() >= Duration::from_secs(9));
        assert!(
            logs.contains("forced shutdown; unfinished paid operations may have unknown outcomes"),
            "{logs}"
        );
    }
    for address in &addresses {
        tokio::net::TcpListener::bind(address)
            .await
            .expect("listener port released");
    }
    assert_eq!(vendor.unsigned.load(Ordering::SeqCst), 1);
    assert_eq!(vendor.signed.load(Ordering::SeqCst), 1);
    vendor.release.notify_one();
    task.abort();
}

#[tokio::test]
async fn http_signals_drain_signed_calls_and_release_all_ports() {
    for meta in [false, true] {
        for signal in ["-INT", "-TERM"] {
            scenario(meta, signal, true, false).await;
        }
    }
}
#[tokio::test]
async fn http_explicit_force_reports_ambiguous_paid_calls_without_replay() {
    for meta in [false, true] {
        scenario(meta, "-TERM", false, false).await;
    }
}

#[tokio::test]
async fn http_disconnect_does_not_abandon_accepted_payment() {
    for meta in [false, true] {
        scenario(meta, "-TERM", true, true).await;
    }
}

#[tokio::test]
async fn stdio_eof_and_signal_preserve_accepted_calls_before_and_after_signing() {
    for (before_signing, signal) in [(true, false), (false, false), (false, true)] {
        let vendor = Vendor::default();
        vendor
            .pause_unsigned
            .store(before_signing, Ordering::SeqCst);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/pay", post(payment))
            .with_state(vendor.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("spec.json"),
            json!({"paths":{"/pay":{"post":{}}}}).to_string(),
        )
        .unwrap();
        let config = dir.path().join("provider.toml");
        std::fs::write(
            &config,
            format!(
                "spec = 'spec.json'\nbase_url = '{base}'\nprefix = 'test'\nprobe_pricing = false\n"
            ),
        )
        .unwrap();
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
            .env("EVM_PRIVATE_KEY", format!("{:064x}", 1))
            .args(["serve", "--provider"])
            .arg(&config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
        input
            .write_all(format!("{init}\n").as_bytes())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(15), output.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        input.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"test_pay\",\"arguments\":{}}}\n").await.unwrap();
        let arrived = if before_signing {
            &vendor.unsigned_arrived
        } else {
            &vendor.arrived
        };
        tokio::time::timeout(Duration::from_secs(10), arrived.notified())
            .await
            .unwrap();
        if signal {
            assert!(
                std::process::Command::new("/bin/kill")
                    .args(["-TERM", &child.id().unwrap().to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
        } else {
            drop(input);
        }
        // Exceed rmcp's EOF response-drain allowance. Application work is owned
        // separately even when the transport can no longer deliver its response.
        tokio::time::sleep(Duration::from_secs(6)).await;
        assert!(
            child.try_wait().unwrap().is_none(),
            "EOF abandoned accepted work"
        );
        if before_signing {
            assert_eq!(vendor.signed.load(Ordering::SeqCst), 0);
            vendor.unsigned_release.notify_one();
            tokio::time::timeout(Duration::from_secs(10), vendor.arrived.notified())
                .await
                .unwrap();
        }
        vendor.release.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(vendor.unsigned.load(Ordering::SeqCst), 1);
        assert_eq!(vendor.signed.load(Ordering::SeqCst), 1);
        task.abort();
    }
}
