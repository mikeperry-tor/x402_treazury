// The executable's force path must not enter Tokio's blocking-worker join.
#[test]
fn force_exit_with_blocked_worker_child() {
    let Ok(marker) = std::env::var("X402_TEST_FORCE_CHILD") else {
        return;
    };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let (_hold, wait) = std::sync::mpsc::channel::<()>();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (started, ready) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            std::fs::write(marker, "blocking worker started").unwrap();
            started.send(()).unwrap();
            // Mirrors a live scan/prover that cannot be aborted by Tokio.
            let _ = wait.recv();
        });
        ready.await.unwrap();
        let _ = super::finish(Err(x402_treazury::server::ForcedShutdown.into()));
    });
    // If finish merely returns, teardown waits for the live blocking worker.
    drop(runtime);
}

#[tokio::test]
async fn explicit_force_exits_even_with_blocking_work() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("worker");
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::force_exit_with_blocked_worker_child",
            "--nocapture",
        ])
        .env("X402_TEST_FORCE_CHILD", &marker)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
        .await
        .expect("explicit force must bypass blocking-worker drain")
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        std::fs::read_to_string(marker).unwrap(),
        "blocking worker started"
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains(x402_treazury::server::FORCE_SHUTDOWN_MESSAGE)
    );
}

#[test]
fn terminal_diagnostics_use_command_scope_and_never_log_private_error_chains() {
    if std::env::var("TREAZURY_TERMINAL_LOG_CHILD").is_err() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::terminal_diagnostics_use_command_scope_and_never_log_private_error_chains",
                "--nocapture",
            ])
            .env("TREAZURY_TERMINAL_LOG_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    for (error, level, category) in [
        (
            anyhow::anyhow!("private https://token@example.invalid/path")
                .context(super::ServingFailure),
            "ERROR",
            "operation_failed",
        ),
        (
            anyhow::anyhow!("private https://token@example.invalid/path"),
            "WARN",
            "operation_failed",
        ),
        (
            anyhow::Error::new(x402_treazury::server::ForcedShutdown)
                .context(super::ServingFailure),
            "WARN",
            "forced_shutdown",
        ),
    ] {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, || super::report_terminal_failure(&error));
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains(level) && text.contains(category), "{text}");
        for private in ["private", "token", "example.invalid"] {
            assert!(!text.contains(private), "{text}");
        }
    }
}

#[tokio::test]
async fn standalone_environment_pricing_replaces_the_file_declaration() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("provider.toml");
    std::fs::write(
        &file,
        "spec='catalog.json'\n[pricing]\nsource='catalog'\nformat='x402'\nkey='x-file'\nunit='call'\n",
    ).unwrap();
    let args = super::Args {
        config: Some(file.to_str().unwrap().to_owned()),
        ..Default::default()
    };
    let environment = std::collections::BTreeMap::from([(
        "X402_MCP_GENERIC_PRICING_KEY".into(),
        "x-environment".into(),
    )]);
    let cfg = super::standalone_config(&args, &environment).await.unwrap();
    assert!(cfg.pricing.is_none());
    let document = serde_json::json!({"paths":{"/items":{"get":{
        "x-file":{"price":"$0.02"},
        "x-environment":{"amount":"0.03","currency":"USD","unit":"request"}
    }}}});
    let tool = super::catalog::build_tools(&cfg, &document, "sample")
        .unwrap()
        .remove(0);
    assert!(tool.description.contains("$0.03/request"));
    assert!(!tool.description.contains("$0.02"));
}
