//! Exercise the actual CLI process and wire framing without a Python MCP client.
use serde_json::{Value, json};
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn cli_stdio_initializes_lists_calls_and_exits_cleanly() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let vendor = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/hello",
                axum::routing::get(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        "hello over stdio"
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        json!({"paths":{"/hello":{"get":{}}}}).to_string(),
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
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_treazure"))
        .env_clear()
        .env("EVM_PRIVATE_KEY", format!("{:064x}", 1))
        .arg("--config")
        .arg(&config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"test_hello","arguments":{}}}),
    ];
    for (index, message) in messages.iter().enumerate() {
        input
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        let line = tokio::time::timeout(Duration::from_secs(15), output.next_line())
            .await
            .unwrap()
            .unwrap()
            .expect("stdout closed before response");
        let response: Value =
            serde_json::from_str(&line).expect("stdout must contain only MCP JSON");
        assert_eq!(response["id"], message["id"]);
        assert!(response.get("error").is_none(), "{response}");
        match index {
            0 => {
                assert!(response["result"]["serverInfo"]["version"].is_string());
                input
                    .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
                    .await
                    .unwrap();
            }
            1 => {
                assert_eq!(response["result"]["tools"][0]["name"], "test_hello");
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
            2 => assert_eq!(response["result"]["content"][0]["text"], "hello over stdio"),
            _ => unreachable!(),
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(input);
    let result = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    vendor.abort();
}
