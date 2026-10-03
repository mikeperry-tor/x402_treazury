//! Offline executable boundaries, with synthetic stores and no real wallet material.
use serde_json::{Value, json};
use std::{path::Path, process::Output};
use x402_treazury::rotation::store::{Store, funding::FundingPhase};
async fn run(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_treazury"));
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
    let pool = store.ensure_pool("fixture", "5").unwrap();
    let jobs = store.funding_jobs().unwrap();
    let pending = &jobs[1];
    store.save_funding_quote(&pending.id, b"quote").unwrap();
    store
        .advance_funding(&pending.id, FundingPhase::Quoted, FundingPhase::Preparing)
        .unwrap();
    store
        .reserve(&pending.operation_id, Some(&pool), 1, 100, 1000)
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
        json!({"funding_job_reset":true})
    );
    let mut store = Store::open(&state, &key, &id).unwrap();
    let now = store.funding_jobs().unwrap();
    assert_ne!(
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
        good(run(dir, &["wallet", "status", "--state-dir", "state"], &[]).await)["treasury_id"],
        id
    );
    bad(run(dir, &["wallet", "status", "--state-dir", "missing"], &[]).await);
    assert!(!dir.join("missing").exists());
    std::fs::write(
        dir.join("registry.toml"),
        "version = 1\nservers = {}\n[source_management]\nwallet = 'unused'\nregistry_file = 'registry.sqlite'\n",
    )
    .unwrap();
    let inspect = ["sources", "inspect", "--meta-config", "registry.toml"];
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
        good(run(dir, &["--config", "provider.toml", "--list-tools"], &env).await)[0]["name"],
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
                    "--config",
                    "provider.toml",
                    "--env-file",
                    "env",
                    "--list-tools"
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
                "--config",
                "provider.toml",
                "--env-file",
                "env",
                "--prefix",
                "cli",
                "--base-url",
                "https://cli.example.com",
                "--route-tool",
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
            &["--config", "provider.toml", "--include", "", "--list-tools"],
            &[],
        )
        .await,
    );
    assert_eq!(cleared.as_array().unwrap().len(), 2);
    for invalid in ["0", "NaN", "inf", "-1"] {
        bad(run(
            dir,
            &[
                "--config",
                "provider.toml",
                "--timeout",
                invalid,
                "--list-tools",
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
                &["wallet", command, "--meta-config", "missing.toml"],
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
                "--meta-config",
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
            &["--meta-config", "servers.toml", "--transport", "stdio"],
            &[]
        )
        .await)
        .contains("cannot be combined")
    );
    let key = format!("{:064x}", 1);
    let error = bad(run(
        dir,
        &["--meta-config", "servers.toml"],
        &[("KEY", &key), ("TOKEN", "test-token")],
    )
    .await);
    assert!(error.contains("cannot bind"));
    assert!(!error.contains("MCP listening"));
    let _released = tokio::net::TcpListener::bind(one).await.unwrap();
}
