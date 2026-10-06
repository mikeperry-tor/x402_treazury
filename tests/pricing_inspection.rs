//! Real CLI inspection: pricing discovery is opt-in, unsigned and wallet-free.
use axum::{Router, extract::Request, http::StatusCode, response::IntoResponse, routing::any};
use base64::{Engine, engine::general_purpose::STANDARD};
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
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}
fn output(o: std::process::Output) -> Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}
#[tokio::test]
async fn inspection_discovery_is_opt_in_unsigned_and_respects_source_policy() {
    let counts = Arc::new(AtomicUsize::new(0));
    let observed = counts.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(any(move |req: Request| {
        let counts = observed.clone();
        async move {
            assert_eq!(req.method(), "GET");
            for header in ["authorization", "payment-signature", "x-payment"] { assert!(req.headers().get(header).is_none()); }
            counts.fetch_add(1, Ordering::SeqCst);
            if req.uri().path() == "/fail" { return StatusCode::TOO_MANY_REQUESTS.into_response(); }
            (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(json!({"accepts":[{"scheme":"exact","amount":"15000","asset":"USDC","network":"eip155:8453"}]}).to_string()))]).into_response()
        }
    }));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        json!({"servers":[{"url":base}],"paths":{
            "/read":{"get":{}},"/fail":{"get":{}},"/write":{"post":{}},"/item/{id}":{"get":{}}
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("provider.toml"),
        "spec='spec.json'\nprefix='api'\n",
    )
    .unwrap();
    let ordinary = output(
        cli(
            dir.path(),
            &["catalog", "tools", "--provider", "provider.toml"],
        )
        .await,
    );
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
    assert_eq!(counts.load(Ordering::SeqCst), 0);
    assert!(
        ordinary[0]["description"]
            .as_str()
            .unwrap()
            .contains("Cost: unknown.")
    );
    let priced = output(
        cli(
            dir.path(),
            &[
                "catalog",
                "tools",
                "--provider",
                "provider.toml",
                "--discover-pricing",
            ],
        )
        .await,
    );
    assert_eq!(counts.load(Ordering::SeqCst), 2);
    let read = priced
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["path"] == "/read")
        .unwrap();
    assert!(
        read["description"]
            .as_str()
            .unwrap()
            .contains("Cost: ~$0.015/call [x402 probe].")
    );
    let failed = priced
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["path"] == "/fail")
        .unwrap();
    assert!(
        failed["description"]
            .as_str()
            .unwrap()
            .contains("Cost: unknown.")
    );
    std::fs::write(
        dir.path().join("provider.toml"),
        "spec='spec.json'\nprefix='api'\nprobe_pricing=false\n",
    )
    .unwrap();
    output(
        cli(
            dir.path(),
            &[
                "catalog",
                "tools",
                "--provider",
                "provider.toml",
                "--discover-pricing",
            ],
        )
        .await,
    );
    assert_eq!(counts.load(Ordering::SeqCst), 2);
    assert!(
        !cli(
            dir.path(),
            &["serve", "--provider", "provider.toml", "--discover-pricing"]
        )
        .await
        .status
        .success()
    );
    std::fs::write(
        dir.path().join("deployment.toml"),
        r#"version=1
[treasury]
state_dir='must-not-exist'
daily_input_zec='0.01'
shield_max_fee_zec='0.001'
[funding]
confidentiality='public'
[wallets.pool]
mode='zcash_rotation'
max_input_zec='0.01'
max_fee_bps=500
[sources.api]
spec='spec.json'
prefix='api'
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
    let deployment = output(
        cli(
            dir.path(),
            &[
                "catalog",
                "tools",
                "--config",
                "deployment.toml",
                "--discover-pricing",
            ],
        )
        .await,
    );
    assert_eq!(counts.load(Ordering::SeqCst), 3); // one selected route, shared by both listeners
    assert_eq!(deployment.as_array().unwrap().len(), 2);
    for server in deployment.as_array().unwrap() {
        assert!(
            server["tools"][0]["description"]
                .as_str()
                .unwrap()
                .contains("Cost: ~$0.015/call [x402 probe].")
        );
    }
    assert!(!dir.path().join("must-not-exist").exists());
    task.abort();
}
