//! Offline executable boundaries, with synthetic stores and no real wallet material.
#[path = "support/capacity.rs"]
mod capacity_fixture;
#[path = "support/funding_quote.rs"]
mod quote_fixture;
#[path = "support/rpc_evidence.rs"]
mod rpc_evidence;
use serde_json::{Value, json};
use std::{path::Path, process::Output};
use x402_treazury::rotation::store::{Store, funding::FundingPhase};
async fn run(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
    command
        .current_dir(dir)
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .env("RUST_BACKTRACE", "1")
        .env("RUST_LIB_BACKTRACE", "1")
        .args(args)
        .kill_on_drop(true);
    for (key, value) in env {
        command.env(key, value);
    }
    tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
        .await
        .unwrap()
        .unwrap()
}
fn good(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn bad(output: Output) -> String {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("error:"));
    assert!(!error.contains("stack backtrace"));
    assert!(!error.contains("panicked at"));
    error
}
#[tokio::test]
async fn backup_recovery_and_registry_inspection_are_offline_and_non_destructive() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let state = dir.join("state");
    let key = dir.join("key");
    let mut store = Store::create(&state, &key, 1, b"synthetic snapshot").unwrap();
    let id = store.id().to_owned();
    let pool = crate::capacity_fixture::declare_and_allocate!(
        store,
        "fixture",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let jobs = store.funding_jobs().unwrap();
    let pending = &jobs[1];
    store
        .save_funding_quote(
            &pending.id,
            &quote_fixture::bound(&pending.target, &pending.recipient, "quote"),
        )
        .unwrap();
    store
        .advance_funding(&pending.id, FundingPhase::Quoted, FundingPhase::Preparing)
        .unwrap();
    store
        .accept_preparation(&pending.operation_id, Some(&pool), 100, None)
        .unwrap();
    store
        .prepare(
            &pending.operation_id,
            1,
            b"prepared snapshot",
            b"signed bytes",
        )
        .unwrap();
    let backup = [
        "wallet",
        "backup",
        "--state-dir",
        "state",
        "--key-file",
        "key",
        "--treasury-id",
        &id,
        "--destination",
        "backup",
    ];
    assert!(bad(run(dir, &backup, &[]).await).contains("state_in_use"));
    assert!(!dir.join("backup").exists());
    drop(store);
    let mut wrong = backup;
    wrong[7] = "wrong-uuid";
    assert!(bad(run(dir, &wrong, &[]).await).contains("treasury ID does not match"));
    assert!(!dir.join("backup").exists());
    assert_eq!(
        good(run(dir, &backup, &[]).await),
        json!({"backup_complete":true})
    );
    bad(run(dir, &backup, &[]).await);
    let restored = Store::open(&dir.join("backup"), &dir.join("backup/key"), &id).unwrap();
    assert_eq!(
        restored
            .prepared_bytes(&pending.operation_id)
            .unwrap()
            .as_slice(),
        b"signed bytes"
    );
    drop(restored);
    let mut recover = vec![
        "wallet",
        "recover-unprepared",
        "--state-dir",
        "state",
        "--key-file",
        "key",
        "--treasury-id",
        id.as_str(),
        "--job-id",
        pending.id.as_str(),
    ];
    bad(run(dir, &recover, &[]).await);
    recover[9] = jobs[0].id.as_str();
    assert_eq!(
        good(run(dir, &recover, &[]).await),
        json!({"funding_attempt_closed":true})
    );
    let store = Store::open(&state, &key, &id).unwrap();
    let now = store.funding_jobs().unwrap();
    assert_eq!(
        now.iter()
            .find(|j| j.id == jobs[0].id)
            .unwrap()
            .operation_id,
        jobs[0].operation_id
    );
    assert_eq!(
        store
            .prepared_bytes(&pending.operation_id)
            .unwrap()
            .as_slice(),
        b"signed bytes"
    );
    drop(store);
    assert_eq!(
        good(
            run(
                dir,
                &["wallet", "status", "--json", "--state-dir", "state"],
                &[]
            )
            .await
        )["treasury_id"],
        id
    );
    bad(run(dir, &["wallet", "status", "--state-dir", "missing"], &[]).await);
    assert!(!dir.join("missing").exists());
    std::fs::write(
        dir.join("registry.toml"),
        "version = 1\nservers = {}\n[source_management]\nwallet = 'unused'\nregistry_file = 'registry.sqlite'\n",
    )
    .unwrap();
    let inspect = ["sources", "inspect", "--config", "registry.toml"];
    bad(run(dir, &inspect, &[]).await);
    assert!(!dir.join("registry.sqlite").exists());
    let db = rusqlite::Connection::open(dir.join("registry.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE snapshot(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;",
    )
    .unwrap();
    db.execute(
        "INSERT INTO snapshot VALUES(1,?1)",
        [json!({"generation":7,"records":{},"receipts":{}}).to_string()],
    )
    .unwrap();
    drop(db);
    let before = std::fs::read(dir.join("registry.sqlite")).unwrap();
    assert_eq!(
        good(run(dir, &inspect, &[]).await),
        json!({"generation":7,"sources":[]})
    );
    assert_eq!(std::fs::read(dir.join("registry.sqlite")).unwrap(), before);
    assert!(!dir.join("registry.sqlite.owner.lock").exists());
}

#[tokio::test]
async fn standalone_precedence_and_invalid_options_need_no_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::write(
        dir.join("spec.json"),
        json!({"paths":{"/a":{"get":{}},"/b":{"get":{}}}}).to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("provider.toml"),"spec = 'spec.json'\nbase_url = 'https://config.example.com'\nprefix = 'config'\ninclude = ['/a']\nprobe_pricing = false\n").unwrap();
    let env = [
        ("X402_MCP_GENERIC_PREFIX", "environment"),
        ("X402_MCP_GENERIC_BASE_URL", "https://env.example.com"),
    ];
    assert_eq!(
        good(
            run(
                dir,
                &["catalog", "tools", "--provider", "provider.toml"],
                &env
            )
            .await
        )[0]["name"],
        "environment_root"
    );
    std::fs::write(
        dir.join("env"),
        "X402_MCP_GENERIC_PREFIX=overlay\nX402_MCP_GENERIC_BASE_URL=https://overlay.example.com\n",
    )
    .unwrap();
    assert_eq!(
        good(
            run(
                dir,
                &[
                    "catalog",
                    "tools",
                    "--provider",
                    "provider.toml",
                    "--env-file",
                    "env"
                ],
                &env
            )
            .await
        )[0]["name"],
        "overlay_root"
    );
    let routed = good(
        run(
            dir,
            &[
                "catalog",
                "route",
                "--provider",
                "provider.toml",
                "--env-file",
                "env",
                "--prefix",
                "cli",
                "--base-url",
                "https://cli.example.com",
                "cli_root",
            ],
            &env,
        )
        .await,
    );
    assert_eq!(routed["url"], "https://cli.example.com/a");
    let cleared = good(
        run(
            dir,
            &[
                "catalog",
                "tools",
                "--provider",
                "provider.toml",
                "--include",
                "",
            ],
            &[],
        )
        .await,
    );
    assert_eq!(cleared.as_array().unwrap().len(), 2);
    for invalid in ["0", "NaN", "inf", "-1"] {
        bad(run(
            dir,
            &[
                "catalog",
                "tools",
                "--provider",
                "provider.toml",
                "--timeout",
                invalid,
            ],
            &[],
        )
        .await);
    }
    #[cfg(not(feature = "zcash"))]
    {
        let error = bad(run(
            dir,
            &[
                "wallet",
                "init",
                "--state-dir",
                "wallet",
                "--key-file",
                "key",
                "--birthday",
                "2000000",
            ],
            &[],
        )
        .await);
        assert!(error.contains("features zcash"));
        assert!(!dir.join("wallet").exists());
    }
}

#[tokio::test]
async fn wallet_recovery_argument_errors_are_legible_without_submission_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    for (command, flag) in [
        ("reconcile", "--operation-id"),
        ("recover-expired", "--operation-id"),
        ("shield-refunds", "--job-id"),
    ] {
        assert!(
            bad(run(
                tmp.path(),
                &["wallet", command, "--config", "missing.toml"],
                &[]
            )
            .await)
            .contains(flag)
        );
        let error = bad(run(
            tmp.path(),
            &[
                "wallet",
                command,
                "--config",
                "missing.toml",
                flag,
                "00000000-0000-0000-0000-000000000000",
            ],
            &[],
        )
        .await);
        #[cfg(not(feature = "zcash"))]
        assert!(error.contains("features zcash"));
        #[cfg(feature = "zcash")]
        assert!(!error.contains("submission token"));
    }
}

#[tokio::test]
async fn executable_bind_failure_rolls_back_prior_ports_and_rejects_meta_overrides() {
    let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let one = first.local_addr().unwrap();
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let two = occupied.local_addr().unwrap();
    drop(first);
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::write(
        dir.join("spec.json"),
        json!({"paths":{"/hello":{"get":{}}}}).to_string(),
    )
    .unwrap();
    let mut config="version = 1\n[sources.test]\nspec = 'spec.json'\nbase_url = 'https://example.com'\nprobe_pricing = false\n[wallets.shared]\nmode = 'static'\nprivate_key_env = 'KEY'\n".to_owned();
    for (id, address) in [("one", one), ("two", two)] {
        config += &format!(
            "[servers.{id}]\nlisten = '{address}'\nsources = ['test']\nwallet = 'shared'\nbearer_token_env = 'TOKEN'\n"
        );
    }
    std::fs::write(dir.join("servers.toml"), config).unwrap();
    assert!(
        bad(run(
            dir,
            &["serve", "--config", "servers.toml", "--transport", "stdio"],
            &[]
        )
        .await)
        .contains("cannot be combined")
    );
    let key = format!("{:064x}", 1);
    let error = bad(run(
        dir,
        &["serve", "--config", "servers.toml"],
        &[("KEY", &key), ("TOKEN", "test-token")],
    )
    .await);
    assert!(error.contains("cannot bind"));
    assert!(!error.contains("MCP listening"));
    let _released = tokio::net::TcpListener::bind(one).await.unwrap();
}

#[tokio::test]
async fn qualification_restriction_requires_serving_meta_config() {
    let tmp = tempfile::tempdir().unwrap();
    let flag = "--qualification-no-new-funding";
    assert!(bad(run(tmp.path(), &["serve", flag], &[]).await).contains("--config"));
    for command in [
        ["config", "check"],
        ["config", "show"],
        ["catalog", "tools"],
        ["catalog", "tags"],
    ] {
        let error = bad(run(
            tmp.path(),
            &[command[0], command[1], "--config", "missing.toml", flag],
            &[],
        )
        .await);
        assert!(error.contains("unexpected argument"), "{error}");
    }
}

#[tokio::test]
async fn unsigned_qualification_cannot_use_stdio_or_dotenv_or_inspection_modes() {
    let tmp = tempfile::tempdir().unwrap();
    let unsigned = "--qualification-unsigned";
    assert!(
        bad(run(tmp.path(), &["serve", unsigned], &[]).await)
            .contains("--qualification-parent-stdin")
    );
    for command in [
        ["catalog", "tools"],
        ["config", "show"],
        ["config", "check"],
    ] {
        let error = bad(run(
            tmp.path(),
            &[command[0], command[1], "--config", "missing.toml", unsigned],
            &[],
        )
        .await);
        assert!(error.contains("unexpected argument"), "{error}");
    }
    let error = bad(run(
        tmp.path(),
        &[
            "serve",
            "--config",
            "missing.toml",
            unsigned,
            "--qualification-parent-stdin",
            "--env-file",
            "must-not-be-read",
        ],
        &[],
    )
    .await);
    assert!(error.contains("cannot be used with"), "{error}");
}

#[cfg(feature = "zcash")]
#[tokio::test]
async fn bootstrap_is_catalog_free_requires_policy_and_resumes_completed_pairs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let config = r#"version=1
[treasury]
state_dir="state"
max_refund_shielding_fee_zec="0.001"
indexer_url="https://127.0.0.1:1"
[funding]
auto_fund=false
base_rpc_url_env="BASE"
base_rpc_fallback_url_envs=[]
[wallets.web]
mode="zcash_rotation"
max_api_payment_usdc="1"
reserves={small={funding_amount_usdc="1"},large={funding_amount_usdc="2"}}
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[wallets.static_unused]
mode="static"
private_key_env="UNSET_STATIC_KEY"
[sources.api]
spec="missing-catalog.json"
probe_pricing=false
[servers.web]
listen="127.0.0.1:0"
sources=["api"]
wallet="web"
bearer_token_env="UNSET_LISTENER_TOKEN"
"#;
    std::fs::write(dir.join("deployment.toml"), config).unwrap();
    let args = ["wallet", "bootstrap", "--config", "deployment.toml"];
    let error = bad(run(dir, &args, &[]).await);
    assert!(error.contains("auto_fund=true"), "{error}");
    assert!(!dir.join("state").exists());
    assert!(bad(run(dir, &["wallet", "bootstrap"], &[]).await).contains("--config"));
    // A new, unfunded test-only treasury: explicit birthday prevents network I/O.
    let output = run(
        dir,
        &[
            "wallet",
            "init",
            "--config",
            "deployment.toml",
            "--birthday",
            "2000000",
        ],
        &[],
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state = dir.join("state");
    let id = x402_treazury::rotation::store::status(&state)
        .unwrap()
        .treasury_id;
    let mut store = Store::open(&state, &state.join("wallet.key"), &id).unwrap();
    crate::capacity_fixture::declare_and_allocate!(
        store,
        "web",
        crate::capacity_fixture::two_reserves("2"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let recovery_job = store.funding_jobs().unwrap().remove(0);
    store
        .advance_funding(
            &recovery_job.id,
            FundingPhase::Allocated,
            FundingPhase::RecoveryRequired,
        )
        .unwrap();
    store
        .defer_funding(
            &recovery_job.id,
            0,
            Some("quote_refresh_exhausted; no preparation started"),
            false,
        )
        .unwrap();
    drop(store);
    // Recovery works with auto_fund=false and unavailable catalog/RPC URLs;
    // Safe unprepared closure requires no network, listener token or new transfer.
    // Paused funding cannot authorize a replacement operation.
    let report = good(
        run(
            dir,
            &["wallet", "recover", "--config", "deployment.toml"],
            &[],
        )
        .await,
    );
    assert_eq!(
        report["recovery"][0]["outcome"]["status"],
        "operator_required"
    );
    assert_eq!(
        report["recovery"][0]["outcome"]["reason"],
        "funding_disabled"
    );
    let mut store = Store::open(&state, &state.join("wallet.key"), &id).unwrap();
    assert!(store.status().unwrap().treasury_operations.is_empty());
    assert_eq!(store.funding_recovery_count(&recovery_job.id).unwrap(), 1);
    assert_eq!(
        store
            .funding_job(&recovery_job.id)
            .unwrap()
            .unwrap()
            .operation_id,
        recovery_job.operation_id
    );
    for job in store.funding_jobs().unwrap() {
        // Synthetic confirmed credit, never a real transfer or live RPC.
        rpc_evidence::credit(&mut store, &job.wallet_id, "2123456", 13).unwrap();
    }
    let jobs = store.status().unwrap().funding_jobs.len();
    drop(store);
    std::fs::write(
        dir.join("deployment.toml"),
        config.replace("auto_fund=false", "auto_fund=true"),
    )
    .unwrap();
    for json_output in [false, true] {
        let mut args = args.to_vec();
        if json_output {
            args.push("--json");
        }
        let output = run(dir, &args, &[("BASE", "https://127.0.0.1:1")]).await;
        if json_output {
            let summary = good(output);
            assert_eq!(summary["captured_completed_groups"], json!(["web"]));
            assert_eq!(
                summary["balance_estimates"],
                json!([
                    {"wallet_name":"web", "wallet_type":"large", "balance_estimate_usdc":"2.123456"},
                    {"wallet_name":"web", "wallet_type":"small", "balance_estimate_usdc":"2.123456"}
                ])
            );
        } else {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout).unwrap();
            assert_eq!(
                text,
                "Wallet bootstrap completed.\n\nAdvisory wallet balance estimates (USDC):\n  web (large): 2.123456 USDC\n  web (small): 2.123456 USDC\n"
            );
        }
        // Closed ownership is immediately available and no new jobs were queued.
        let store = Store::open(&state, &state.join("wallet.key"), &id).unwrap();
        assert_eq!(store.status().unwrap().funding_jobs.len(), jobs);
    }
    let serving = ["serve", "--config", "deployment.toml"];
    // Serving validates auth before bootstrap, then recognizes the funded
    // identity before trying the deliberately missing catalog. No listener starts.
    let error = bad(run(dir, &serving, &[("BASE", "https://127.0.0.1:1")]).await);
    assert!(error.contains("UNSET_LISTENER_TOKEN"), "{error}");
    assert!(!error.contains("bootstrap already complete"));
    let env = [
        ("BASE", "https://127.0.0.1:1"),
        ("UNSET_LISTENER_TOKEN", "fixture-token"),
    ];
    let error = bad(run(dir, &serving, &env).await);
    assert!(error.contains("catalog local_read failed"), "{error}");
    std::fs::write(dir.join("deployment.toml"), config).unwrap();
    let error = bad(run(dir, &serving, &env).await);
    assert!(error.contains("catalog local_read failed"), "{error}");
    let store = Store::open(&state, &state.join("wallet.key"), &id).unwrap();
    assert_eq!(store.status().unwrap().funding_jobs.len(), jobs);
}

#[tokio::test]
async fn payment_and_wallet_funding_help_use_explicit_usdc_names() {
    let dir = tempfile::tempdir().unwrap();
    for (args, current, obsolete) in [(
        vec!["serve", "--help"],
        "--max-api-payment-usdc",
        "--max-price-usd",
    )] {
        let output = run(dir.path(), &args, &[]).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains(current), "{help}");
        assert!(!help.contains(obsolete), "{help}");
    }
}

#[tokio::test]
async fn source_maintenance_has_explicit_operator_commands() {
    let dir = tempfile::tempdir().unwrap();
    for command in ["refresh", "remove"] {
        let output = run(dir.path(), &["sources", command, "--help"], &[]).await;
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        for option in ["--config", "--server", "--source-id"] {
            assert!(help.contains(option), "{help}");
        }
        bad(run(
            dir.path(),
            &[
                "sources",
                command,
                "--config",
                "missing.toml",
                "--server",
                "research",
                "--source-id",
                "missing",
            ],
            &[],
        )
        .await);
    }
}

#[tokio::test]
async fn removed_wallet_pool_fails_before_opening_state_with_migration_guidance() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        &[
            "wallet",
            "pool",
            "--name",
            "old",
            "--funding-amount-usdc",
            "2",
        ],
        &[],
    )
    .await;
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("wallet pool was removed"), "{error}");
    assert!(error.contains("wallet bootstrap --config FILE"), "{error}");
}

#[tokio::test]
async fn standalone_modes_preserve_inspection_and_unsigned_probe_order() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
        assert_eq!(request.method(), "GET");
        assert!(request.headers().get("payment-signature").is_none());
        assert!(request.headers().get("x-payment").is_none());
        calls.fetch_add(1, Ordering::SeqCst);
        async { "unsigned" }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        r#"{"paths":{"/read":{"get":{"tags":["read"]}}}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("provider.toml"),
        "spec='spec.json'\nprefix='api'\n",
    )
    .unwrap();
    let tags = run(
        dir.path(),
        &["catalog", "tags", "--provider", "provider.toml"],
        &[],
    )
    .await;
    assert_eq!(good(tags), json!({"read":1}));
    let error = bad(run(
        dir.path(),
        &["catalog", "tools", "--provider", "provider.toml"],
        &[],
    )
    .await);
    assert!(error.contains("base_url is required"), "{error}");
    std::fs::write(
        dir.path().join("provider.toml"),
        format!("spec='spec.json'\nprefix='api'\nbase_url='{base}'\n"),
    )
    .unwrap();
    let tools = good(
        run(
            dir.path(),
            &["catalog", "tools", "--provider", "provider.toml"],
            &[],
        )
        .await,
    );
    assert_eq!(tools.as_array().unwrap().len(), 1);
    assert_eq!(tools[0]["name"], "api_read");
    assert_eq!(
        tools[0]["input_schema"],
        json!({"type":"object","properties":{}})
    );
    let checked = run(
        dir.path(),
        &["config", "check", "--provider", "provider.toml"],
        &[],
    )
    .await;
    assert!(checked.status.success());
    assert_eq!(checked.stdout, b"1 tools validated\n");
    let routed = good(
        run(
            dir.path(),
            &[
                "catalog",
                "route",
                "--provider",
                "provider.toml",
                "api_read",
            ],
            &[],
        )
        .await,
    );
    assert_eq!(
        routed,
        json!({"method":"GET","url":format!("{base}/read"),"query":{},"body":null})
    );
    let unknown = bad(run(
        dir.path(),
        &[
            "catalog",
            "route",
            "--provider",
            "provider.toml",
            "missing",
            "--args",
            "invalid",
        ],
        &[],
    )
    .await);
    assert!(unknown.contains("unknown tool"), "{unknown}");
    let invalid = bad(run(
        dir.path(),
        &[
            "serve",
            "--provider",
            "provider.toml",
            "--max-api-payment-usdc",
            "invalid",
        ],
        &[],
    )
    .await);
    assert!(
        invalid.contains("price must be a nonnegative decimal"),
        "{invalid}"
    );
    let missing = bad(run(dir.path(), &["serve", "--provider", "provider.toml"], &[]).await);
    assert!(missing.contains("EVM_PRIVATE_KEY required"), "{missing}");
    assert_eq!(count.load(Ordering::SeqCst), 0);
    good(
        run(
            dir.path(),
            &[
                "catalog",
                "tools",
                "--provider",
                "provider.toml",
                "--discover-pricing",
            ],
            &[],
        )
        .await,
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let key = "0000000000000000000000000000000000000000000000000000000000000001";
    let auth = bad(run(
        dir.path(),
        &[
            "serve",
            "--provider",
            "provider.toml",
            "--transport",
            "http",
        ],
        &[("EVM_PRIVATE_KEY", key)],
    )
    .await);
    assert!(auth.contains("HTTP transport requires"), "{auth}");
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "serving discovery precedes HTTP auth but follows valid payer setup"
    );
    server.abort();
}
