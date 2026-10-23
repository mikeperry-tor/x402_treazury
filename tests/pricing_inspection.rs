//! Catalog inspection uses authored prices and never requests API endpoints.
use axum::{Router, extract::Request, http::StatusCode, routing::any};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
async fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
    tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
            .current_dir(dir)
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}
fn output(result: std::process::Output) -> Value {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
#[tokio::test]
async fn inspection_keeps_authored_prices_unknown_cost_and_zero_endpoint_io() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(any(move |request: Request| {
        seen.fetch_add(1, Ordering::SeqCst);
        assert!(!request.headers().contains_key("payment-signature"));
        async { StatusCode::PAYMENT_REQUIRED }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        json!({"servers":[{"url":base}],"paths":{
            "/read":{"get":{"pricing":{"amount":"0.015","currency":"USD"}}},
            "/unknown":{"get":{}}, "/write":{"post":{}}
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("provider.toml"),
        "spec='spec.json'\nprefix='api'\npricing_key='pricing'\n",
    )
    .unwrap();
    let tools = output(
        cli(
            dir.path(),
            &["catalog", "tools", "--provider", "provider.toml"],
        )
        .await,
    );
    let description = |path: &str| {
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["path"] == path)
            .unwrap()["description"]
            .as_str()
            .unwrap()
    };
    assert!(description("/read").contains("Cost: ~$0.015/call [spec]."));
    assert!(description("/unknown").contains("Cost: unknown."));
    let checked = cli(
        dir.path(),
        &["config", "check", "--provider", "provider.toml"],
    )
    .await;
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(String::from_utf8_lossy(&checked.stdout).contains("tools validated"));
    for command in ["tools", "warm"] {
        let selector = if command == "warm" {
            "--config"
        } else {
            "--provider"
        };
        let removed = cli(
            dir.path(),
            &[
                "catalog",
                command,
                selector,
                "provider.toml",
                "--discover-pricing",
            ],
        )
        .await;
        assert!(!removed.status.success());
        assert!(String::from_utf8_lossy(&removed.stderr).contains("--discover-pricing"));
    }
    std::fs::write(
        dir.path().join("deployment.toml"),
        r#"version=1
[treasury]
state_dir='must-not-exist'
[funding]
confidentiality='public'
[wallets.pool]
mode='zcash_rotation'
max_api_payment_usdc='1'
reserves={small={funding_amount_usdc='2'}}
max_conversion_overhead_percent=5
[sources.api]
spec='spec.json'
prefix='api'
pricing_key='pricing'
[servers.first]
listen='127.0.0.1:0'
bearer_token_env='ABSENT_TOKEN'
wallet='pool'
sources=['api']
include_tools=['api_read']
[servers.second]
listen='127.0.0.1:0'
bearer_token_env='ALSO_ABSENT'
wallet='pool'
sources=['api']
include_tools=['api_read']
"#,
    )
    .unwrap();
    let inventory = output(
        cli(
            dir.path(),
            &["catalog", "tools", "--config", "deployment.toml"],
        )
        .await,
    );
    assert_eq!(inventory.as_array().unwrap().len(), 2);
    for listener in inventory.as_array().unwrap() {
        assert!(
            listener["tools"][0]["description"]
                .as_str()
                .unwrap()
                .contains("Cost: ~$0.015/call [spec].")
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "metadata inspection never reads or signs an API route"
    );
    assert!(!dir.path().join("must-not-exist").exists());
    server.abort();
}
