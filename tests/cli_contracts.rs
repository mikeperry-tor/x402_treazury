//! Offline executable boundaries, with synthetic stores and no real wallet material.
#[path = "support/capacity.rs"]
mod capacity_fixture;
#[path = "support/funding_quote.rs"]
mod quote_fixture;
#[path = "support/rpc_evidence.rs"]
mod rpc_evidence;
#[path = "support/socks.rs"]
mod socks;
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
async fn source_inspection_lists_saved_tools_without_tokens_catalog_fetches_or_registry_writes() {
    use sha2::{Digest, Sha256};
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    // Inspection must not apply current admission policy or connect to this URL.
    std::fs::write(dir.join("registry.toml"), "version=1\nservers={}\n[source_management]\nwallet='missing'\nregistry_file='registry.sqlite'\nallowed_origins=['https://different.example.com']\n").unwrap();
    let id = "00000000-0000-0000-0000-000000000001";
    let document = json!({"openapi":"3.0.3","servers":[{"url":"https://offline.invalid"}],"paths":{"/read":{"get":{"description":"PRIVATE_SCHEMA_SENTINEL"}},"/write":{"post":{}}}}).to_string();
    let record = json!({"id":id,"owner":"toolbox","candidate":{"name":"demo","spec_url":"https://offline.invalid/openapi.json","lifetime":"persistent"},"targets":["toolbox"],"revision":2,"document":document,"hash":format!("{:x}",Sha256::digest(document.as_bytes())),"format":1,"accepted_at":1,"updated_at":2,"removed":false,"disabled":null});
    let db = rusqlite::Connection::open(dir.join("registry.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE snapshot(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;",
    )
    .unwrap();
    db.execute(
        "INSERT INTO snapshot VALUES(1,?1)",
        [json!({"generation":2,"records":{id:record},"receipts":{}}).to_string()],
    )
    .unwrap();
    let before = std::fs::read(dir.join("registry.sqlite")).unwrap();
    let output = run(
        dir,
        &["sources", "inspect", "--config", "registry.toml"],
        &[],
    )
    .await;
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_SCHEMA_SENTINEL"));
    let shown = good(output);
    assert_eq!(shown["generation"], 2);
    assert_eq!(shown["sources"].as_array().unwrap().len(), 1);
    assert_eq!(shown["sources"][0]["id"], id);
    assert_eq!(
        shown["sources"][0]["tools"],
        json!([
            {"name":"dyn_demo__read","method":"GET","path":"/read"},
            {"name":"dyn_demo__write","method":"POST","path":"/write"}
        ])
    );
    assert_eq!(std::fs::read(dir.join("registry.sqlite")).unwrap(), before);
    assert!(!dir.join("registry.sqlite.owner.lock").exists());
    assert!(!dir.join("state").exists());
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

#[tokio::test]
async fn recommendation_inspection_is_explicit_typed_and_never_widens_ordinary_filters() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::write(
        dir.join("api.json"),
        json!({"paths":{"/read":{"get":{}},"/other":{"get":{}},"/hidden":{"get":{}}}}).to_string(),
    )
    .unwrap();
    let provider = "spec='api.json'\nbase_url='https://offline.invalid'\nprefix='api'\nrecommended_tools=['api_read']\nuse_recommended_tools=true\nexclude_tools=['api_hidden']\n";
    std::fs::write(dir.join("provider.toml"), provider).unwrap();
    let selected = good(
        run(
            dir,
            &["catalog", "tools", "--provider", "provider.toml"],
            &[],
        )
        .await,
    );
    assert_eq!(selected.as_array().unwrap().len(), 1);
    assert_eq!(selected[0]["name"], "api_read");
    assert_eq!(selected[0]["recommended"], true);
    let all = good(
        run(
            dir,
            &[
                "catalog",
                "tools",
                "--provider",
                "provider.toml",
                "--all-tools",
            ],
            &[],
        )
        .await,
    );
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert!(
        all.as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "api_other" && t["recommended"] == false)
    );
    let conflict = bad(run(
        dir,
        &[
            "catalog",
            "tools",
            "--provider",
            "provider.toml",
            "--all-tools",
            "--discover-pricing",
        ],
        &[],
    )
    .await);
    assert!(conflict.contains("cannot be used with"));
    std::fs::write(dir.join("deployment.toml"), "version=1\n[sources.api]\nextends='provider.toml'\n[wallets.w]\nmode='static'\nprivate_key_env='UNREAD_KEY'\n[servers.main]\nlisten='127.0.0.1:0'\nbearer_token_env='UNREAD_TOKEN'\nwallet='w'\nsources=['api']\nexclude_tools=['api_other']").unwrap();
    let all = good(
        run(
            dir,
            &[
                "catalog",
                "tools",
                "--config",
                "deployment.toml",
                "--all-tools",
            ],
            &[],
        )
        .await,
    );
    assert_eq!(all[0]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(all[0]["tools"][0]["name"], "api_read");
    // Offline config inspection reports empty advice without trying the spec;
    // catalog inspection diagnoses it, and all-tools can inspect the inventory.
    std::fs::write(
        dir.join("provider.toml"),
        provider.replace("['api_read']", "[]"),
    )
    .unwrap();
    let shown = good(run(dir, &["config", "show", "--provider", "provider.toml"], &[]).await);
    assert_eq!(shown["settings"]["use_recommended_tools"], true);
    assert_eq!(shown["settings"]["recommended_tools"], json!([]));
    assert!(
        bad(run(
            dir,
            &["catalog", "tools", "--provider", "provider.toml"],
            &[]
        )
        .await)
        .contains("no tools matched recommended_tools")
    );
    assert_eq!(
        good(
            run(
                dir,
                &[
                    "catalog",
                    "tools",
                    "--provider",
                    "provider.toml",
                    "--all-tools"
                ],
                &[]
            )
            .await
        )
        .as_array()
        .unwrap()
        .len(),
        2
    );
    // Bypass is applied only to typed configuration, never to raw TOML values.
    std::fs::write(
        dir.join("provider.toml"),
        provider.replace(
            "use_recommended_tools=true",
            "use_recommended_tools='malformed'",
        ),
    )
    .unwrap();
    assert!(
        bad(run(
            dir,
            &[
                "catalog",
                "tools",
                "--provider",
                "provider.toml",
                "--all-tools"
            ],
            &[]
        )
        .await)
        .contains("boolean")
    );
    std::fs::write(
        dir.join("provider.toml"),
        provider
            .replace("api_read", "stale_name")
            .replace("use_recommended_tools=true", "use_recommended_tools=false"),
    )
    .unwrap();
    let unused = run(
        dir,
        &["catalog", "tools", "--provider", "provider.toml"],
        &[],
    )
    .await;
    assert!(
        String::from_utf8_lossy(&unused.stderr).contains("Provider tool selector matches nothing")
    );
    assert_eq!(good(unused).as_array().unwrap().len(), 2);
}

fn network_override_fixture(dir: &Path, network: &str) {
    std::fs::write(
        dir.join("deployment.toml"),
        format!(
            r#"version=1
[treasury]
state_dir='state'
[wallets.shared]
mode='static'
private_key_env='UNREAD_KEY'
[sources.api]
spec='http://catalog.invalid/spec'
base_url='https://api.example.com'
probe_pricing=false
[servers.research]
listen='127.0.0.1:0'
bearer_token_env='UNREAD_TOKEN'
wallet='shared'
sources=['api']
{network}
"#
        ),
    )
    .unwrap();
}

#[tokio::test]
async fn deployment_network_override_replaces_policy_without_inspection_io() {
    let dir = tempfile::tempdir().unwrap();
    network_override_fixture(
        dir.path(),
        "[network]\nmode='tor'\nsocks_endpoint='127.0.0.1:1'\nisolation_namespace='embedded'\nread_timeout_seconds=19\ntor_isolation_retries=7\n",
    );
    std::fs::write(
        dir.path().join("network.toml"),
        "[network]\nmode='direct'\n",
    )
    .unwrap();
    let args = [
        "config",
        "show",
        "--config",
        "deployment.toml",
        "--network-config",
        "network.toml",
    ];
    let shown = good(run(dir.path(), &args, &[]).await);
    assert_eq!(shown["network"]["mode"], "direct");
    assert_eq!(shown["network"]["socks_endpoint"], Value::Null);
    assert_eq!(shown["network"]["isolation_namespace"], "x402_treazury");
    assert_eq!(shown["network"]["read_timeout_seconds"], 60);
    assert_eq!(shown["network"]["connect_timeout_seconds"], 15);
    assert_eq!(shown["network"]["tor_isolation_retries"], 2);
    let original = good(
        run(
            dir.path(),
            &["config", "show", "--config", "deployment.toml"],
            &[],
        )
        .await,
    );
    assert_eq!(original["network"]["isolation_namespace"], "embedded");
    assert_eq!(original["network"]["read_timeout_seconds"], 19);
    assert_eq!(original["network"]["tor_isolation_retries"], 7);
    // Replacement precedes semantic validation of the embedded policy.
    network_override_fixture(dir.path(), "[network]\nmode='tor'\n");
    assert_eq!(
        good(run(dir.path(), &args, &[]).await)["network"]["mode"],
        "direct"
    );
    assert!(
        bad(run(
            dir.path(),
            &["config", "show", "--config", "deployment.toml"],
            &[]
        )
        .await)
        .contains("socks_endpoint is required")
    );
    // A partial external Tor policy uses its own defaults, too.
    std::fs::write(
        dir.path().join("network.toml"),
        "[network]\nmode='tor'\nsocks_endpoint='127.0.0.1:2'\n",
    )
    .unwrap();
    let shown = good(run(dir.path(), &args, &[]).await);
    assert_eq!(shown["network"]["mode"], "tor");
    assert_eq!(shown["network"]["connect_timeout_seconds"], 120);
    assert_eq!(shown["network"]["tor_isolation_retries"], 2);
    assert_eq!(shown["network"]["read_timeout_seconds"], 60);
    assert!(!dir.path().join("state").exists());
}

#[tokio::test]
async fn network_override_replaces_relay_at_its_declaration_without_inspection_io() {
    let dir = tempfile::tempdir().unwrap();
    network_override_fixture(
        dir.path(),
        "[discovery_relay]\nprovider='embedded-missing.toml'\nwallet='shared'\nsources=['api']\nserve=true\nwarm=true\nsearch=false\n",
    );
    let external = dir.path().join("network");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(
        external.join("tor.toml"),
        "[network]\nmode='tor'\nsocks_endpoint='127.0.0.1:1'\n[discovery_relay]\nprovider='external-missing.toml'\n",
    )
    .unwrap();
    let args = [
        "config",
        "show",
        "--config",
        "deployment.toml",
        "--network-config",
        "network/tor.toml",
    ];
    let shown = good(run(dir.path(), &args, &[]).await);
    assert_eq!(shown["network"]["mode"], "tor");
    assert_eq!(
        shown["discovery_relay"],
        json!({
            "provider": external.canonicalize().unwrap().join("external-missing.toml"),
            "wallet": null,
            "sources": [],
            "serve": false,
            "warm": false,
            "search": true
        })
    );
    assert_eq!(shown["discovery_wallets"], json!({}));

    // Both serving and warming validate the selected relay against the
    // deployment before loading providers, keys or state. Ignoring the external
    // section would instead use the valid embedded wallet binding.
    std::fs::write(
        external.join("tor.toml"),
        "[network]\nmode='direct'\n[discovery_relay]\nprovider='external-missing.toml'\nwallet='undeclared-network-relay-wallet'\nserve=true\nwarm=true\n",
    )
    .unwrap();
    for command in [vec!["serve"], vec!["catalog", "warm"]] {
        let mut command = command;
        command.extend([
            "--config",
            "deployment.toml",
            "--network-config",
            "network/tor.toml",
        ]);
        assert!(
            bad(run(dir.path(), &command, &[]).await)
                .contains("discovery_relay.wallet must name a declared wallet")
        );
    }

    // A transport-only override preserves the entire deployment relay and its
    // deployment-relative provider path rather than rebasing it to network/.
    std::fs::write(external.join("tor.toml"), "[network]\nmode='direct'\n").unwrap();
    let preserved = good(run(dir.path(), &args, &[]).await);
    let embedded = good(
        run(
            dir.path(),
            &["config", "show", "--config", "deployment.toml"],
            &[],
        )
        .await,
    );
    assert_eq!(preserved["discovery_relay"], embedded["discovery_relay"]);
    assert_eq!(
        preserved["discovery_relay"]["provider"],
        "embedded-missing.toml"
    );
    assert_eq!(preserved["discovery_wallets"], json!({"api":"shared"}));
    assert!(!dir.path().join("embedded-missing.toml").exists());
    assert!(!external.join("external-missing.toml").exists());
    assert!(!dir.path().join("state").exists());
}

#[tokio::test]
async fn network_override_relay_file_is_reusable_by_offline_standalone_and_wallet_commands() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("network.toml"),
        "[network]\nmode='tor'\nsocks_endpoint='127.0.0.1:1'\n[discovery_relay]\nprovider='missing-relay.toml'\nwallet='deployment-only'\nsources=['deployment-only']\nserve=true\nwarm=true\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("provider.toml"),
        "spec='https://catalog.invalid/missing.json'\nbase_url='https://api.example.com'\n",
    )
    .unwrap();
    let shown = good(
        run(
            dir.path(),
            &[
                "config",
                "show",
                "--provider",
                "provider.toml",
                "--network-config",
                "network.toml",
            ],
            &[],
        )
        .await,
    );
    assert_eq!(shown["network"]["mode"], "tor");
    assert!(shown.get("discovery_relay").is_none());
    assert!(!dir.path().join("state").exists());

    let store = Store::create(
        &dir.path().join("wallet-state"),
        &dir.path().join("wallet.key"),
        1,
        b"synthetic snapshot",
    )
    .unwrap();
    let treasury_id = store.id().to_owned();
    drop(store);
    let status = good(
        run(
            dir.path(),
            &[
                "wallet",
                "status",
                "--json",
                "--state-dir",
                "wallet-state",
                "--key-file",
                "wallet.key",
                "--network-config",
                "network.toml",
            ],
            &[],
        )
        .await,
    );
    assert_eq!(status["treasury_id"], treasury_id);
    assert!(!dir.path().join("missing-relay.toml").exists());
    assert!(!dir.path().join("state").exists());
}

#[tokio::test]
async fn network_override_routes_catalog_warming_and_serving_through_the_selected_proxy() {
    use std::{collections::BTreeMap, sync::atomic::Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/spec",
        axum::routing::get(|| async { axum::Json(json!({"paths":{"/read":{"get":{}}}})) }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let authored = socks::Socks::start(BTreeMap::new(), socks::Fault::Refuse).await;
    let selected = socks::Socks::start(
        BTreeMap::from([("catalog.invalid".into(), address)]),
        socks::Fault::None,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("state")).unwrap();
    let embedded = format!(
        "[network]\nmode='tor'\nsocks_endpoint='{}'\nisolation_namespace='embedded'\n",
        authored.address
    );
    std::fs::write(
        dir.path().join("network.toml"),
        format!(
            "[network]\nmode='tor'\nsocks_endpoint='{}'\nisolation_namespace='override'\n",
            selected.address
        ),
    )
    .unwrap();
    for command in [
        ["catalog", "tools"],
        ["catalog", "tags"],
        ["config", "check"],
        ["catalog", "warm"],
    ] {
        network_override_fixture(dir.path(), &embedded);
        let file = dir.path().join("deployment.toml");
        let text = std::fs::read_to_string(&file).unwrap().replace(
            "catalog.invalid/spec'",
            &format!("catalog.invalid/spec?{}'", command[1]),
        );
        std::fs::write(&file, text).unwrap();
        let before = selected.records.lock().unwrap().len();
        let output = run(
            dir.path(),
            &[
                command[0],
                command[1],
                "--config",
                "deployment.toml",
                "--network-config",
                "network.toml",
            ],
            &[],
        )
        .await;
        assert!(
            output.status.success(),
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(selected.records.lock().unwrap().len() > before);
    }
    network_override_fixture(dir.path(), &embedded);
    // Static serving needs no treasury; warming used the empty directory only for cache storage.
    let file = dir.path().join("deployment.toml");
    std::fs::write(
        &file,
        std::fs::read_to_string(&file)
            .unwrap()
            .replace("[treasury]\nstate_dir='state'\n", ""),
    )
    .unwrap();
    let before = selected.records.lock().unwrap().len();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .current_dir(dir.path())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .env("UNREAD_KEY", format!("{:064x}", 1))
        .env("UNREAD_TOKEN", "fixture-token")
        .args([
            "serve",
            "--config",
            "deployment.toml",
            "--network-config",
            "network.toml",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while selected.records.lock().unwrap().len() == before {
            if let Some(status) = child.try_wait().unwrap() {
                use tokio::io::AsyncReadExt;
                let mut stderr = String::new();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .await
                    .unwrap();
                panic!("serve exited before discovery ({status}): {stderr}");
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    child.kill().await.unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(output.stdout.is_empty());
    assert_eq!(authored.accepted.load(Ordering::SeqCst), 0);
    for record in selected.records.lock().unwrap().iter() {
        assert_eq!(record.host, "catalog.invalid");
        assert_eq!(record.address_type, 3);
        assert_eq!(record.user, "<torS0X>0");
        assert!(record.password.starts_with("override:v1:"));
    }
    assert!(!dir.path().join("state/wallet.key").exists());
    server.abort();
}

#[tokio::test]
async fn invalid_network_overrides_and_unpinned_qualification_fail_before_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    network_override_fixture(dir.path(), "");
    for invalid in [
        "[network]\nmode='direct'\n[unrelated]\nenabled=true\n",
        "[network]\nmode='direct'\n[discovery_relay]\nprovider='missing.toml'\nserrve=true\n",
    ] {
        std::fs::write(dir.path().join("invalid.toml"), invalid).unwrap();
        assert!(
            bad(run(
                dir.path(),
                &[
                    "config",
                    "show",
                    "--config",
                    "deployment.toml",
                    "--network-config",
                    "invalid.toml",
                ],
                &[],
            )
            .await)
            .contains("unknown field")
        );
    }
    std::fs::write(
        dir.path().join("invalid.toml"),
        "[network]\nmode='tor'\nsocks_endpoint='1.1.1.1:9150'\n",
    )
    .unwrap();
    for command in [
        vec!["serve"],
        vec!["config", "check"],
        vec!["config", "show"],
        vec!["catalog", "tools"],
        vec!["catalog", "warm"],
        vec![
            "sources",
            "refresh",
            "--server",
            "research",
            "--source-id",
            "missing",
        ],
    ] {
        let mut args = command;
        args.extend([
            "--config",
            "deployment.toml",
            "--network-config",
            "invalid.toml",
        ]);
        assert!(
            bad(run(dir.path(), &args, &[]).await)
                .contains("SOCKS endpoint must be a literal loopback")
        );
    }
    for mut args in [
        vec!["config", "check", "--qualification-snapshot"],
        vec![
            "serve",
            "--qualification-unsigned",
            "--qualification-parent-stdin",
        ],
        vec![
            "serve",
            "--qualification-managed",
            "--qualification-parent-stdin",
            "--qualification-binding",
            "missing-binding.json",
        ],
    ] {
        args.extend([
            "--config",
            "missing.toml",
            "--network-config",
            "missing-network.toml",
        ]);
        assert!(
            bad(run(dir.path(), &args, &[]).await)
                .contains("qualification requires network policy in the pinned deployment")
        );
    }
    assert!(!dir.path().join("state").exists());
    assert!(!dir.path().join("registry.sqlite").exists());
}
