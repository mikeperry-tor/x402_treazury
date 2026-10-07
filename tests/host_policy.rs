use axum::http::StatusCode;
use x402_treazury::server::{Server, host::HostPolicy, http_app};

#[tokio::test]
async fn host_policies_preserve_auth_and_match_hosts_and_ports() {
    for (allowed, disabled, host, expected) in [
        (None, false, "localhost:8123", 200),
        (None, false, "remote.example", 403),
        (Some(vec!["mcp.example"]), false, "mcp.example:8123", 200),
        (Some(vec!["mcp.example"]), false, "localhost", 403),
        (
            Some(vec!["mcp.example:8123"]),
            false,
            "mcp.example:8124",
            403,
        ),
        (
            Some(vec!["mcp.example:8123"]),
            false,
            "mcp.example:8123",
            200,
        ),
        (Some(vec!["::1"]), false, "[::1]:8123", 200),
        (None, true, "remote.example", 200),
        (Some(vec![]), false, "remote.example", 200),
    ] {
        let mut server = Server::from_bindings(vec![], None, None);
        server.host_policy = HostPolicy::new(
            allowed.map(|hosts| hosts.into_iter().map(String::from).collect()),
            disabled,
        )
        .unwrap();
        let app = http_app(server, "fixture-token".into());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        for authenticated in [true, false] {
            let mut request = http
                .post(format!("http://{address}/mcp"))
                .header("host", host)
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream");
            if authenticated {
                request = request.header("authorization", "Bearer fixture-token");
            }
            let response = request.body(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
            ).send().await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::from_u16(if authenticated { expected } else { 401 }).unwrap(),
                "{host}"
            );
        }
        task.abort();
    }
}

#[test]
fn invalid_and_conflicting_policies_are_rejected() {
    assert!(HostPolicy::new(Some(vec![]), true).is_err());
    for entry in [
        "",
        " ",
        "https://mcp.example",
        "*.example",
        "user@host",
        "host/path",
        "host:bad",
        "host:99999",
        "host:",
    ] {
        assert!(
            HostPolicy::new(Some(vec![entry.into()]), false).is_err(),
            "{entry}"
        );
    }
}

#[tokio::test]
async fn standalone_flags_reach_http_and_disabled_check_warns() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        r#"{"paths":{"/read":{"get":{}}}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("provider.toml"),
        "spec='spec.json'\nbase_url='http://127.0.0.1:1'\nprobe_pricing=false\n",
    )
    .unwrap();
    let address_pattern = regex::Regex::new(r"127\.0\.0\.1:\d+").unwrap();
    for flags in [
        vec!["--allowed-hosts", "mcp.example"],
        vec!["--disable-host-check"],
        vec!["--disable-host-check", "--no-auth"],
    ] {
        let auth_flags = if flags.contains(&"--no-auth") {
            vec![]
        } else {
            vec!["--bearer-token", "fixture-token"]
        };
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
            .env("EVM_PRIVATE_KEY", format!("{:064x}", 1))
            .current_dir(dir.path())
            .args([
                "serve",
                "--provider",
                "provider.toml",
                "--transport",
                "http",
                "--port",
                "0",
            ])
            .args(&flags)
            .args(auth_flags)
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
        let address = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let line = lines.next_line().await.unwrap().expect("startup stderr");
                if line.contains("MCP listening") {
                    break address_pattern.find(&line).unwrap().as_str().to_owned();
                }
            }
        })
        .await
        .unwrap();
        let request = reqwest::Client::builder().no_proxy().build().unwrap()
            .post(format!("http://{address}/mcp")).header("host", "mcp.example")
.header("accept", "application/json, text/event-stream")
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}))
            ;
        let request = if flags.contains(&"--no-auth") {
            request
        } else {
            request.bearer_auth("fixture-token")
        };
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 200);
        if flags[0] == "--disable-host-check" {
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    let line = lines.next_line().await.unwrap().expect("warning stderr");
                    if line.contains("MCP Host allowlist is disabled") {
                        break;
                    }
                }
            })
            .await
            .expect("disabled policy must warn");
        }
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }
    for flags in [
        vec!["--disable-host-check"],
        vec![
            "--transport",
            "http",
            "--disable-host-check",
            "--allowed-hosts",
            "example.com",
        ],
    ] {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
            .args(["serve"])
            .args(flags)
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("require --transport http") || stderr.contains("cannot be used with"),
            "{stderr}"
        );
    }
}
