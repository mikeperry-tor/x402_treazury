// The executable's force path must not enter Tokio's blocking-worker join.
#[test]
fn force_exit_with_blocked_worker_child() {
    let Ok(marker) = std::env::var("X402_TEST_FORCE_CHILD") else {
        return;
    };
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
        super::finish(Err(x402_treazury::server::ForcedShutdown.into()));
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
