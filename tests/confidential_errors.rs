//! Server-held credentials must not become diagnostic content. Vendor bodies are distinct.
use axum::{Router, http::StatusCode, routing::any};
use serde_json::json;
use x402_treazury::{
    catalog::{Config, RoutedRequest, build_tools, load_json},
    payment::{PaidClient, Payer, SpendPolicy},
    rotation::base::{BaseRpc, ChainQuery},
    server::{Server, http_app},
};
const SECRET: &str = "DUMMY_URL_CREDENTIAL_8675309";
fn clean(text: &str) {
    for secret in [SECRET, "DUMMY_PRIVATE_KEY", "DUMMY_BEARER", "DUMMY_SESSION"] {
        assert!(!text.contains(secret), "credential in diagnostic: {text}");
    }
}
fn payer() -> PaidClient {
    PaidClient::new(Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap())
}
#[tokio::test]
async fn fetch_help_payment_and_rpc_errors_exclude_url_credentials() {
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            // Fetches use GET and JSON-RPC uses POST; both must see the intended
            // availability failure rather than an unrelated method rejection.
            Router::new().route("/", any(|| async { StatusCode::BAD_GATEWAY })),
        )
        .await
        .unwrap();
    });
    let url = format!("{origin}/?api_key={SECRET}");
    // Positive control: reqwest's unfiltered status error really contains this sentinel.
    let raw = http
        .get(&url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err();
    assert!(raw.to_string().contains(SECRET));
    clean(&format!("{:#}", load_json(&url, &http).await.unwrap_err()));
    let rpc = BaseRpc::new(&url, 1, 60).unwrap();
    let err = rpc
        .view(ChainQuery {
            wallets: vec![],
            pending: vec![],
            anchor: None,
        })
        .await
        .err()
        .unwrap();
    clean(&format!("{err:#}"));
    assert!(err.to_string().contains("base_rpc_unavailable"));
    // The same adapter error may be retained by funding reconciliation for inspection.
    let state = tempfile::tempdir().unwrap();
    let db = state.path().join("state");
    let key = state.path().join("key");
    let mut store =
        x402_treazury::rotation::store::Store::create(&db, &key, 1, b"fixture").unwrap();
    let id = store.id().to_owned();
    store.ensure_pool("fixture", "5").unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    store
        .defer_funding(&job.id, 100, Some(&err.to_string()), false)
        .unwrap();
    drop(store);
    let mut store = x402_treazury::rotation::store::Store::open(&db, &key, &id).unwrap();
    let jobs = store.funding_jobs().unwrap();
    clean(&serde_json::to_string(&jobs).unwrap());
    assert_eq!(jobs[0].last_error.as_deref(), Some("base_rpc_unavailable"));

    let config = Config {
        prefix: Some("safe".into()),
        help_url: Some(url.clone()),
        ..Default::default()
    };
    let tools = build_tools(
        &config,
        &json!({"openapi":"3.0.0","paths":{"/x":{"get":{"responses":{}}}}}),
        "safe",
    )
    .unwrap();
    let app = http_app(
        Server::new(tools, payer(), origin, None, None),
        "DUMMY_BEARER".into(),
    );
    let mcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/mcp", mcp.local_addr().unwrap());
    let mcp_task = tokio::spawn(async move { axum::serve(mcp, app).await.unwrap() });
    let result = http.post(endpoint).bearer_auth("DUMMY_BEARER")
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"safe_help","arguments":{}}}))
        .send().await.unwrap().text().await.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap()["result"]["isError"],
        true
    );
    clean(&result);
    mcp_task.abort();
    task.abort();
    // Connect failure exercises the initial payment request and spec fetch paths.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/?token={SECRET}", closed.local_addr().unwrap());
    drop(closed);
    let route = RoutedRequest {
        method: "GET".into(),
        url: url.clone(),
        query: Default::default(),
        body: None,
    };
    clean(&format!("{:#}", payer().execute(route).await.unwrap_err()));
    clean(&format!("{:#}", load_json(&url, &http).await.unwrap_err()));
}

#[tokio::test]
async fn executable_startup_and_inspection_do_not_print_environment_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("spec.json"),
        r#"{"openapi":"3.0.0","paths":{"/x":{"get":{"responses":{}}}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("config.toml"),
        "spec = 'spec.json'\nbase_url = 'https://example.com'\nprobe_pricing = false\n",
    )
    .unwrap();
    for inspect in [true, false] {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_treazury"));
        cmd.current_dir(tmp.path())
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
            .env("EVM_PRIVATE_KEY", "DUMMY_PRIVATE_KEY")
            .env("X402_MCP_BEARER_TOKEN", "DUMMY_BEARER")
            .env("NEAR_USER_SESSION", "DUMMY_SESSION")
            .args(["--config", "config.toml"])
            .kill_on_drop(true);
        if inspect {
            cmd.arg("--show-config");
        }
        let out = tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(out.status.success(), inspect);
        clean(&String::from_utf8_lossy(&out.stdout));
        clean(&String::from_utf8_lossy(&out.stderr));
        if !inspect {
            assert!(String::from_utf8_lossy(&out.stderr).contains("invalid EVM private key"));
        }
    }
}
