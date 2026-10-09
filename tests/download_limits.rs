use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use x402_treazury::mcp_wire::McpResponse;
use x402_treazury::{
    catalog::{Config, build_tools},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, http_app},
};
#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn payer() -> PaidClient {
    PaidClient::new(Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap())
}
async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() }),
    )
}
async fn call(http: &reqwest::Client, url: &str, name: &str) -> Value {
    http.post(format!("{url}/mcp")).bearer_auth("fixture").header("accept","application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{}}}))
        .send().await.unwrap().mcp_json::<Value>().await.unwrap()["result"].clone()
}
#[tokio::test]
async fn bounds_are_explicit_in_errors_logs_and_agent_results_without_partial_success() {
    let logs = Capture(Arc::default());
    let sink = logs.clone();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish(),
    )
    .unwrap();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    // Exercise Content-Length and chunked bodies at exactly seven bytes and one over.
    for chunked in [false, true] {
        for body in ["雪🙂", "雪🙂!"] {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", l.local_addr().unwrap());
            let task = tokio::spawn(async move {
                let (mut s, _) = l.accept().await.unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(s.read_u8().await.unwrap());
                }
                let wire = if chunked {
                    format!(
                        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\n雪\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                        body.len() - 3,
                        &body[3..]
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = s.write_all(wire.as_bytes()).await;
            });
            let result = x402_treazury::limits::read(
                http.get(url).send().await.unwrap(),
                7,
                "fixture document",
                "max_spec_bytes",
            )
            .await;
            if body.len() == 7 {
                assert_eq!(result.unwrap(), body.as_bytes());
            } else {
                let error = result.unwrap_err().to_string();
                assert!(error.contains("max_spec_bytes=7 bytes"));
                assert!(error.contains("no partial content returned"));
            }
            task.await.unwrap();
        }
    }
    let signed = Arc::new(AtomicUsize::new(0));
    let unsigned = Arc::new(AtomicUsize::new(0));
    let help = Arc::new(AtomicUsize::new(0));
    let (s, u, h) = (signed.clone(), unsigned.clone(), help.clone());
    let (vendor,v)=serve(Router::new().route("/pay",get(move |headers:HeaderMap| {let(s,u)=(s.clone(),u.clone());async move {
        if headers.contains_key("payment-signature") {s.fetch_add(1,Ordering::SeqCst);return "PRIVATE-CONTENT".into_response();}
        u.fetch_add(1,Ordering::SeqCst);
        let challenge=json!({"x402Version":2,"resource":{"url":"http://localhost/pay","description":"fixture".repeat(100),"mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":x402_treazury::payment::USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
        (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(challenge.to_string()))]).into_response()
    }})).route("/help",get(move||{let h=h.clone();async move{h.fetch_add(1,Ordering::SeqCst);"雪🙂"}}))
        .route("/spec",get(||async {json!({"paths":{"/unused":{"get":{}}}}).to_string()}))).await;
    let cfg = Config {
        help_url: Some(format!("{vendor}/help")),
        ..Default::default()
    };
    let tools = build_tools(&cfg, &json!({"paths":{"/pay":{"get":{}}}}), "t").unwrap();
    let help_tool = tools.iter().find(|t| t.help_url.is_some()).unwrap().clone();
    let mut small_help = help_tool.clone();
    small_help.name = "small_help".into();
    let mut bindings: Vec<_> = tools
        .into_iter()
        .map(|t| (t, payer().with_download_limits(7, 7), vendor.clone()))
        .collect();
    bindings.push((
        small_help,
        payer().with_download_limits(7, 6),
        vendor.clone(),
    ));
    let mut s = Server::from_bindings(bindings, None, Some(2));
    // Download success then explicit display truncation, not a download error.
    let (base, server) = serve(http_app(s.clone(), "fixture".into())).await;
    let result = call(&http, &base, "t_help").await;
    assert_ne!(result["isError"], true);
    assert_eq!(result["content"][0]["text"], "雪🙂");
    for _ in 0..2 {
        let result = call(&http, &base, "small_help").await;
        assert_eq!(result["isError"], true);
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("max_help_bytes=6 bytes")
        );
    }
    assert_eq!(
        help.load(Ordering::SeqCst),
        3,
        "different help limits cannot share cached success; rejected reads can retry"
    );
    let result = call(&http, &base, "t_pay").await;
    assert_eq!(result["isError"], true);
    let error = result["content"][0]["text"].as_str().unwrap();
    assert!(error.contains("max_response_bytes=7 bytes"));
    assert!(error.contains("may already have settled"));
    assert!(!error.contains("PRIVATE-CONTENT"));
    assert_eq!(signed.load(Ordering::SeqCst), 1);
    assert_eq!(unsigned.load(Ordering::SeqCst), 1);
    s.max_response_chars = Some(1);
    assert_eq!(
        s.invoke("t_help", &Default::default()).await.unwrap(),
        "雪\n[truncated by --max-response-chars]"
    );
    assert!(
        format!(
            "{:#}",
            x402_treazury::catalog::load_json_with_limit(&format!("{vendor}/spec"), &http, 8)
                .await
                .unwrap_err()
        )
        .contains("max_spec_bytes=8 bytes")
    );
    // The SDK's incoming request limit remains a visible HTTP error, with a log.
    let rejected = http
        .post(format!("{base}/mcp"))
        .bearer_auth("fixture")
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .body(" ".repeat(4 * 1024 * 1024 + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 413);
    assert!(
        rejected
            .text()
            .await
            .unwrap()
            .contains("exceeds 4194304 bytes")
    );
    let cfg = Config {
        max_description_chars: Some(1),
        ..Default::default()
    };
    let descriptions = build_tools(
        &cfg,
        &json!({"paths":{"/a":{"get":{"description":"雪🙂"}}}}),
        "t",
    )
    .unwrap();
    assert!(
        descriptions[0]
            .description
            .contains("[truncated by max_description_chars=1]")
    );
    let capped = Config {
        probe_max_endpoints: 0,
        ..Default::default()
    };
    let spec = json!({"paths":{"/skipped":{"get":{}}}});
    let tools = build_tools(&capped, &spec, "t").unwrap();
    assert!(
        x402_treazury::pricing::PricingCache::default()
            .discover(&capped, &spec, &tools, &vendor)
            .await
            .unwrap()
            .is_empty()
    );
    let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for evidence in [
        "max_response_bytes",
        "max_help_bytes",
        "max_spec_bytes",
        "no partial content returned",
        "tool output truncated",
        "tool description truncated",
        "MCP request rejected",
        "pricing discovery capped",
        "x402 challenge description truncated",
    ] {
        assert!(log.contains(evidence), "missing {evidence}: {log}");
    }
    assert!(!log.contains("PRIVATE-CONTENT"));
    server.abort();
    v.abort();
}

#[tokio::test]
async fn static_spec_limits_are_visible_in_cli_and_cli_overrides_toml() {
    let (vendor, task) = serve(Router::new().route(
        "/spec",
        get(|| async { json!({"paths":{"/hello":{"get":{}}}}).to_string() }),
    ))
    .await;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("provider.toml"),format!("spec = '{vendor}/spec'\nbase_url = '{vendor}'\nprefix = 'test'\nmax_spec_bytes = 8\nprobe_pricing = false\n")).unwrap();
    for override_limit in [false, true] {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
        command
            .env_clear()
            .current_dir(tmp.path())
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
            .args(["catalog", "tools", "--provider", "provider.toml"])
            .kill_on_drop(true);
        if override_limit {
            command.args(["--max-spec-bytes", "1024"]);
        }
        let output = tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.status.success(), override_limit);
        if override_limit {
            let tools: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(tools[0]["name"], "test_hello");
        } else {
            assert!(output.stdout.is_empty());
            let logs = String::from_utf8(output.stderr).unwrap();
            assert!(logs.contains("download limit exceeded"));
            assert!(logs.contains("max_spec_bytes=8 bytes"));
            assert!(logs.contains("no partial content returned"));
        }
    }
    let spec_url = format!("{vendor}/spec");
    std::fs::write(tmp.path().join("servers.toml"),format!("version = 1\n[sources.test]\nspec = '{spec_url}'\nbase_url = '{vendor}'\nmax_spec_bytes = 8\n[wallets.shared]\nmode = 'static'\nprivate_key_env = 'MUST_NOT_LOAD'\n[servers.test]\nlisten = '127.0.0.1:0'\nwallet = 'shared'\nsources = ['test']\nbearer_token_env = 'MUST_NOT_LOAD'\n")).unwrap();
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .current_dir(tmp.path())
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args(["config", "check", "--config", "servers.toml"])
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("max_spec_bytes=8 bytes")
    );
    task.abort();
}

#[tokio::test]
async fn deployment_sources_sharing_a_wallet_keep_distinct_response_caps() {
    use std::collections::BTreeMap;
    use x402_treazury::deployment::Deployment;
    let (vendor, v) = serve(Router::new().route("/hello", get(|| async { "123456789" }))).await;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("spec.json"),
        json!({"paths":{"/hello":{"get":{}}}}).to_string(),
    )
    .unwrap();
    let mut config =
        "version = 1\n[wallets.shared]\nmode = 'static'\nprivate_key_env = 'KEY'\n".to_owned();
    for (id, limit) in [("small", 8), ("large", 9)] {
        config += &format!(
            "[sources.{id}]\nspec = 'spec.json'\nbase_url = '{vendor}'\nprefix = 'test'\nmax_response_bytes = {limit}\nprobe_pricing = false\n[servers.{id}]\nlisten = '127.0.0.1:0'\nwallet = 'shared'\nsources = ['{id}']\nbearer_token_env = 'TOKEN'\n"
        );
    }
    let path = tmp.path().join("servers.toml");
    std::fs::write(&path, config).unwrap();
    let running = Deployment::load(&path)
        .await
        .unwrap()
        .bind(&BTreeMap::from([
            ("KEY".into(), format!("{:064x}", 1)),
            ("TOKEN".into(), "fixture".into()),
        ]))
        .await
        .unwrap();
    let addresses = running.addresses();
    let stop = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    for (name, address) in addresses {
        let result = call(&http, &format!("http://{address}"), "test_hello").await;
        if name == "small" {
            assert_eq!(result["isError"], true);
            assert!(
                result["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("max_response_bytes=8 bytes")
            );
        } else {
            assert_ne!(result["isError"], true);
            assert_eq!(result["content"][0]["text"], "123456789");
        }
    }
    stop.cancel();
    task.await.unwrap().unwrap();
    v.abort();
}

#[tokio::test]
async fn oversized_legacy_challenge_is_rejected_before_signing() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let (url, task) = serve(Router::new().route(
        "/pay",
        get(move |headers: HeaderMap| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                assert!(!headers.contains_key("x-payment"));
                (
                    StatusCode::PAYMENT_REQUIRED,
                    "oversized legacy challenge body",
                )
            }
        }),
    ))
    .await;
    let tool = build_tools(
        &Config::default(),
        &json!({"paths":{"/pay":{"get":{}}}}),
        "t",
    )
    .unwrap()
    .remove(0);
    let error = payer()
        .with_download_limits(7, 7)
        .execute(tool.route(&url, &Default::default()).unwrap())
        .await
        .unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("payment challenge rejected before signing"));
    assert!(error.contains("max_response_bytes=7 bytes"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn catalog_failure_stages_preserve_http_status_without_urls_or_document_text() {
    use x402_treazury::catalog::{LoadStage, load_json_with_limit};
    let (base, task) = serve(
        Router::new()
            .route(
                "/rejected",
                get(|| async { (StatusCode::TOO_MANY_REQUESTS, "PRIVATE-CONTENT") }),
            )
            .route("/malformed", get(|| async { "PRIVATE-CONTENT" }))
            .route("/large", get(|| async { "\"PRIVATE-CONTENT\"" })),
    )
    .await;
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    for (path, limit, stage, status) in [
        ("rejected", 32, LoadStage::Headers, Some(429)),
        ("malformed", 32, LoadStage::Parse, None),
        ("large", 4, LoadStage::Body, None),
    ] {
        let error =
            load_json_with_limit(&format!("{base}/{path}?secret=PRIVATE-TOKEN"), &http, limit)
                .await
                .unwrap_err();
        assert_eq!(error.downcast_ref::<LoadStage>(), Some(&stage));
        assert_eq!(
            error
                .downcast_ref::<reqwest::Error>()
                .and_then(|e| e.status())
                .map(|s| s.as_u16()),
            status
        );
        let text = format!("{error:#}");
        for private in [&base, "PRIVATE-TOKEN", "PRIVATE-CONTENT"] {
            assert!(!text.contains(private), "private catalog detail in error");
        }
    }
    task.abort();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("PRIVATE-PATH.json");
    let error = load_json_with_limit(path.to_str().unwrap(), &http, 32)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<LoadStage>(),
        Some(&LoadStage::LocalRead)
    );
    assert!(!format!("{error:#}").contains("PRIVATE-PATH"));
    std::fs::write(&path, "PRIVATE-CONTENT").unwrap();
    let error = load_json_with_limit(path.to_str().unwrap(), &http, 32)
        .await
        .unwrap_err();
    assert_eq!(error.downcast_ref::<LoadStage>(), Some(&LoadStage::Parse));
    // Local operator-authored specs deliberately retain the documented exemption
    // from the remote download cap; typed staging must not change that policy.
    std::fs::write(&path, "{\"paths\":{}}").unwrap();
    assert_eq!(
        load_json_with_limit(path.to_str().unwrap(), &http, 1)
            .await
            .unwrap(),
        json!({"paths":{}})
    );
}
