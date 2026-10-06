use serde_json::{Value, json};
use x402_treazury::deployment::Deployment;
fn config(scope: &str) -> String {
    format!(
        r#"version=1
[wallet_assignment]
scope="{scope}"
template="small"
[wallet_templates.small]
mode="zcash_rotation"
deposit_size="5.000001"
max_input_zec="0.02"
max_fee_bps=500
[treasury]
id="11111111-1111-4111-8111-111111111111"
state_dir="state"
key_file="key"
indexer_url_env="INDEXER"
submission_url_env="SUBMISSION"
daily_input_zec="0.1"
shield_max_fee_zec="0.001"
[funding]
base_rpc_url_env="BASE"
[sources.a]
spec="spec.json"
probe_pricing=false
[sources.b]
spec="spec.json"
probe_pricing=false
[sources.unreferenced]
spec="spec.json"
[servers.one]
listen="127.0.0.1:0"
bearer_token_env="TOKEN"
sources=["a","b"]
[servers.two]
listen="127.0.0.1:0"
bearer_token_env="TOKEN"
sources=["a","b"]
"#
    )
}
async fn show(dir: &std::path::Path, text: &str) -> Value {
    let path = dir.join("servers.toml");
    std::fs::write(&path, text).unwrap();
    Deployment::show_config(&path).await.unwrap()
}
#[tokio::test]
async fn scopes_resolve_exact_sharing_counts_and_capital_without_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    for (scope, count, capital) in [
        ("deployment", 1, "10.000002"),
        ("server", 2, "20.000004"),
        ("source", 2, "20.000004"),
        ("binding", 4, "40.000008"),
    ] {
        let shown = show(dir.path(), &config(scope)).await;
        assert_eq!(shown["wallet_summary"]["managed_pool_count"], count);
        assert_eq!(shown["wallet_summary"]["generated_pool_count"], count);
        assert_eq!(
            shown["wallet_summary"]["active_and_standby_target_usdc"],
            capital
        );
        assert_eq!(
            shown["generated_wallets"].as_object().unwrap().len(),
            count as usize
        );
        assert!(shown["wallets"].as_object().unwrap().is_empty());
        let b = &shown["wallet_bindings"];
        assert_eq!(b["one"]["a"]["origin"], "wallet_assignment");
        assert_eq!(b["one"]["a"]["template"], "small");
        assert_eq!(b["one"]["a"]["scope"], scope);
        assert_eq!(
            b["one"]["a"]["wallet"] == b["one"]["b"]["wallet"],
            matches!(scope, "deployment" | "server")
        );
        assert_eq!(
            b["one"]["a"]["wallet"] == b["two"]["a"]["wallet"],
            matches!(scope, "deployment" | "source")
        );
        let name = b["one"]["a"]["wallet"].as_str().unwrap();
        assert_eq!(shown["resolved_wallets"][name]["deposit_size"], "5.000001");
        assert!(!dir.path().join("state").exists());
        assert!(!dir.path().join("key").exists());
    }
    // config show runs without credentials or even a spec file; config check loads
    // the catalog but still creates no pool. Inventory carries template provenance.
    let path = dir.path().join("servers.toml");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .args(["config", "show", "--config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(
        dir.path().join("spec.json"),
        r#"{"servers":[{"url":"https://example.invalid"}],"paths":{"/pay":{"get":{}}}}"#,
    )
    .unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let inv = serde_json::to_value(deployment.inventory()).unwrap();
    assert_eq!(inv[0]["wallet_bindings"]["a"]["scope"], "binding");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .args(["config", "check", "--config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("4 managed pools (4 automatic); active + standby target: 40.000008 USDC")
    );
    assert!(!dir.path().join("state").exists());
}
#[tokio::test]
async fn explicit_exceptions_win_and_templates_alone_never_allocate() {
    let dir = tempfile::tempdir().unwrap();
    let text = config("binding")
        .replace("[sources.a]", "[sources.a]\nwallet='source_wallet'")
        .replace("[servers.one]", "[servers.one]\nwallet='server_wallet'")
        + r#"
[wallets.source_wallet]
mode='static'
private_key_env='SOURCE_KEY'
[wallets.server_wallet]
mode='static'
private_key_env='SERVER_KEY'
[wallets.unused_managed]
mode='zcash_rotation'
deposit_size='3'
max_input_zec='0.01'
max_fee_bps=100
[wallet_templates.unused]
mode='zcash_rotation'
deposit_size='100'
max_input_zec='0.1'
max_fee_bps=100
"#;
    let shown = show(dir.path(), &text).await;
    assert_eq!(
        shown["wallet_bindings"]["one"]["a"],
        json!({"wallet":"source_wallet","origin":"sources.a.wallet"})
    );
    assert_eq!(
        shown["wallet_bindings"]["one"]["b"]["wallet"],
        "server_wallet"
    );
    assert_eq!(
        shown["wallet_bindings"]["two"]["a"]["wallet"],
        "source_wallet"
    );
    assert_eq!(shown["wallet_summary"]["generated_pool_count"], 1);
    assert_eq!(shown["wallet_summary"]["managed_pool_count"], 2);
    assert_eq!(
        shown["wallet_summary"]["active_and_standby_target_usdc"],
        "16.000002"
    );
    let text = config("source")
        .replace("[servers.one]", "[servers.one]\nwallet='shared'")
        .replace("[servers.two]", "[servers.two]\nwallet='shared'")
        + "\n[wallets.shared]\nmode='static'\nprivate_key_env='KEY'\n";
    let mut parsed: toml::Table = toml::from_str(&text).unwrap();
    parsed.remove("treasury");
    parsed.remove("funding");
    let shown = show(dir.path(), &toml::to_string(&parsed).unwrap()).await;
    assert_eq!(shown["wallet_summary"]["managed_pool_count"], 0);
    assert_eq!(shown["generated_wallets"], json!({}));
    parsed.remove("wallet_assignment");
    assert_eq!(
        show(dir.path(), &toml::to_string(&parsed).unwrap()).await["wallet_summary"]["managed_pool_count"],
        0
    );
}
#[tokio::test]
async fn identities_ignore_template_edits_paths_order_and_avoid_binding_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let first = show(dir.path(), &config("source")).await;
    let edited = config("source")
        .replace("small", "renamed_template")
        .replace("5.000001", "7")
        .replace("sources=[\"a\",\"b\"]", "sources=[\"b\",\"a\"]");
    let next = show(other.path(), &edited).await;
    assert_eq!(
        first["wallet_bindings"]["one"]["a"]["wallet"],
        next["wallet_bindings"]["one"]["a"]["wallet"]
    );
    let collision = config("binding")
        .replace("[sources.a]", "[sources.b_c]")
        .replace("[sources.b]", "[sources.c]")
        .replace("[servers.one]", "[servers.a]")
        .replace("[servers.two]", "[servers.a_b]")
        .replace("sources=[\"a\",\"b\"]", "sources=[\"b_c\",\"c\"]");
    let shown = show(dir.path(), &collision).await;
    assert_ne!(
        shown["wallet_bindings"]["a"]["b_c"]["wallet"],
        shown["wallet_bindings"]["a_b"]["c"]["wallet"]
    );
}
#[tokio::test]
async fn invalid_policies_templates_risk_limits_and_reserved_names_fail_offline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("servers.toml");
    for bad in [
        config("unknown"),
        config("source").replace("template=\"small\"", "template='missing'"),
        config("source").replace("scope=\"source\"", "scope='source'\nunknown=true"),
        config("source").replace("max_input_zec=\"0.02\"", "max_input_zec='-1'"),
        config("source").replace(
            "mode=\"zcash_rotation\"",
            "mode='static'\nprivate_key_env='KEY'",
        ),
        config("source").replace("[sources.a]", "[sources.a]\nwallet='small'"),
        config("source") + "\n[wallets.auto_v1_source_a]\nmode='static'\nprivate_key_env='KEY'\n",
        config("source").replace("[treasury]", "[not_treasury]"),
        config("source").replace(
            "deposit_size=\"5.000001\"",
            &format!("deposit_size='{}'", alloy_primitives::U256::MAX),
        ),
    ] {
        std::fs::write(&path, &bad).unwrap();
        assert!(
            Deployment::show_config(&path).await.is_err(),
            "accepted {bad}"
        );
        assert!(!dir.path().join("state").exists());
    }
}

#[cfg(feature = "zcash")]
#[tokio::test]
async fn serving_reuses_generated_pool_identity_and_retains_old_scopes() {
    use x402_treazury::{rotation::store::status, treasury::Treasury};
    let dir = tempfile::tempdir().unwrap();
    let owner=Treasury::create(dir.path().join("state"),dir.path().join("key"),2_000_000,Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
    let id = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        r#"{"servers":[{"url":"https://example.invalid"}],"paths":{"/pay":{"get":{}}}}"#,
    )
    .unwrap();
    let base = config("source").replace("11111111-1111-4111-8111-111111111111", &id);
    let edited = base
        .replace("5.000001", "7")
        .replace("small", "another_template");
    let added = edited.replace("sources=[\"a\",\"b\"]", "sources=[\"a\",\"b\",\"c\"]")
        + "\n[sources.c]\nspec='spec.json'\nprobe_pricing=false\n";
    let scopes = [
        base.clone(),
        edited,
        added.clone(),
        added.replace("scope=\"source\"", "scope='binding'"),
        base,
    ];
    let env = std::collections::BTreeMap::from([
        ("TOKEN".into(), "test-token".into()),
        ("INDEXER".into(), "https://example.invalid".into()),
        ("SUBMISSION".into(), "https://example.invalid".into()),
        ("BASE".into(), "https://example.invalid".into()),
    ]);
    let path = dir.path().join("servers.toml");
    let mut original_id = String::new();
    let mut addresses = Vec::new();
    for (phase, text) in scopes.iter().enumerate() {
        std::fs::write(&path, text).unwrap();
        let deployment = Deployment::load(&path).await.unwrap();
        let expected = [2, 2, 3, 6, 2][phase];
        assert_eq!(deployment.wallet_summary().managed_pool_count, expected);
        let running = deployment.bind(&env).await.unwrap();
        let state = status(&dir.path().join("state")).unwrap();
        assert_eq!(state.pools.iter().filter(|p| p.enabled).count(), expected);
        let a = state
            .pools
            .iter()
            .find(|p| p.name == "auto_v1_source_a")
            .unwrap();
        if phase == 0 {
            original_id = a.id.clone();
            addresses = a
                .addresses
                .iter()
                .map(|a| a.address.clone())
                .collect::<Vec<_>>();
        }
        assert_eq!(a.id, original_id);
        assert_eq!(
            a.addresses
                .iter()
                .map(|a| a.address.clone())
                .collect::<Vec<_>>(),
            addresses
        );
        assert!(a.addresses.iter().all(|a| a.target == "5000001"));
        if phase == 1 {
            assert_eq!(a.deposit_atomic, "7000000");
        }
        if phase == 3 {
            assert!(!a.enabled);
            assert_eq!(state.pools.len(), 9);
        }
        if phase == 4 {
            assert!(a.enabled);
            assert_eq!(state.pools.len(), 9);
        }
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), running.serve(stop))
            .await
            .unwrap()
            .unwrap();
    }
}

#[cfg(feature = "zcash")]
#[path = "support/signatures.rs"]
mod signatures;
#[cfg(feature = "zcash")]
#[tokio::test]
async fn automatic_scope_bindings_match_actual_payment_signers() {
    use axum::{
        Json,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::sync::{Arc, Mutex};
    use x402_treazury::{
        rotation::{base::now, store::status},
        treasury::Treasury,
    };
    let signed = Arc::new(Mutex::new(vec![]));
    let logs = signed.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let rpc = |Json(request): Json<Value>| async move {
        let result = match request["method"].as_str().unwrap() {
            "eth_chainId" => json!("0x2105"),
            "eth_getBlockByNumber" => {
                let tag = request["params"][0].as_str().unwrap();
                json!({"number":if tag=="latest" {"0x100"} else {tag},"timestamp":format!("0x{:x}",now().unwrap()),"hash":format!("0x{:064x}",7)})
            }
            "eth_call" => {
                let data = request["params"][0]["data"].as_str().unwrap();
                json!(format!(
                    "0x{:064x}",
                    if data.starts_with("0x70a08231") {
                        5_000_001u64
                    } else {
                        0
                    }
                ))
            }
            other => panic!("unexpected RPC {other}"),
        };
        Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
    };
    let vendor = move |headers: HeaderMap| {
        let logs = logs.clone();
        async move {
            if let Some(h) = headers.get("payment-signature") {
                let payload: Value =
                    serde_json::from_slice(&STANDARD.decode(h.as_bytes()).unwrap()).unwrap();
                let address = signatures::recover_exact(&payload);
                logs.lock().unwrap().push(address.to_string());
                return Json(json!({"payer":address.to_string()})).into_response();
            }
            let challenge = json!({"x402Version":2,"resource":{"url":"https://fixture.example.com/pay","description":"pay","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":x402_treazury::payment::USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (
                StatusCode::PAYMENT_REQUIRED,
                [(
                    "payment-required",
                    STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
                )],
            )
                .into_response()
        }
    };
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/rpc", post(rpc))
                .route("/pay", get(vendor)),
        )
        .await
        .unwrap()
    });
    for scope in ["deployment", "server", "source", "binding", "overrides"] {
        let dir = tempfile::tempdir().unwrap();
        let owner=Treasury::create(dir.path().join("state"),dir.path().join("key"),2_000_000,Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        let id = owner.status().await.unwrap().treasury_id;
        owner.close().await.unwrap();
        std::fs::write(
            dir.path().join("spec.json"),
            json!({"servers":[{"url":base}],"paths":{"/pay":{"get":{}}}}).to_string(),
        )
        .unwrap();
        let text = config(if scope == "overrides" {
            "binding"
        } else {
            scope
        })
        .replace("11111111-1111-4111-8111-111111111111", &id);
        let text = if scope == "overrides" {
            text.replace("[sources.a]", "[sources.a]\nwallet='source_override'")
                .replace("[servers.one]", "[servers.one]\nwallet='server_override'")
                + "\n[wallets.source_override]\nmode='static'\nprivate_key_env='SOURCE_KEY'\n[wallets.server_override]\nmode='static'\nprivate_key_env='SERVER_KEY'\n"
        } else {
            text
        };
        let path = dir.path().join("servers.toml");
        std::fs::write(&path, text).unwrap();
        let shown = Deployment::show_config(&path).await.unwrap();
        let running = Deployment::load(&path)
            .await
            .unwrap()
            .bind(&std::collections::BTreeMap::from([
                ("TOKEN".into(), "scope-token".into()),
                ("SOURCE_KEY".into(), format!("{:064x}", 1)),
                ("SERVER_KEY".into(), format!("{:064x}", 2)),
                ("INDEXER".into(), "http://127.0.0.1:1".into()),
                ("SUBMISSION".into(), "http://127.0.0.1:1".into()),
                ("BASE".into(), format!("{base}/rpc")),
            ]))
            .await
            .unwrap();
        let state = status(&dir.path().join("state")).unwrap();
        let ports = running.addresses();
        let stop = tokio_util::sync::CancellationToken::new();
        let serve = tokio::spawn(running.serve(stop.clone()));
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for (server, address) in ports {
            for source in ["a", "b"] {
                let response:Value=client.post(format!("http://{address}/mcp")).bearer_auth("scope-token").header("accept","application/json, text/event-stream").json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":format!("{source}_pay"),"arguments":{}}})).send().await.unwrap().json().await.unwrap();
                assert_ne!(
                    response["result"]["isError"], true,
                    "{scope} {server}/{source}: {response}"
                );
                let body: Value = serde_json::from_str(
                    response["result"]["content"][0]["text"].as_str().unwrap(),
                )
                .unwrap();
                let profile = shown["wallet_bindings"][&server][source]["wallet"]
                    .as_str()
                    .unwrap();
                let expected = match profile {
                    "source_override" | "server_override" => {
                        let key = if profile == "source_override" { 1 } else { 2 };
                        let signer: alloy_signer_local::PrivateKeySigner =
                            format!("{key:064x}").parse().unwrap();
                        signer.address().to_string()
                    }
                    _ => state
                        .pools
                        .iter()
                        .find(|p| p.name == profile)
                        .unwrap()
                        .addresses[0]
                        .address
                        .clone(),
                };
                assert_eq!(
                    body["payer"].as_str().unwrap().to_lowercase(),
                    expected.to_lowercase()
                );
            }
        }
        stop.cancel();
        serve.await.unwrap().unwrap();
        let owner = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
            .await
            .unwrap();
        let reopened = owner.status().await.unwrap();
        owner.close().await.unwrap();
        assert_eq!(
            reopened.pools.iter().map(|p| &p.id).collect::<Vec<_>>(),
            state.pools.iter().map(|p| &p.id).collect::<Vec<_>>()
        );
    }
    assert_eq!(signed.lock().unwrap().len(), 20);
    task.abort();
}
