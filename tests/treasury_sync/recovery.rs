//! Actual recovery command failure/cancellation must release the encrypted owner.
use super::*;
use std::{path::Path, process::Stdio, time::Duration};

fn config(dir: &Path, id: &str) -> std::path::PathBuf {
    let path = dir.join("recovery.toml");
    std::fs::write(
        &path,
        format!(
            r#"version=1
servers={{}}
[treasury]
id="{id}"
state_dir="state"
key_file="key"
indexer_url_env="INDEXER"
submission_url_env="UNUSED_SUBMISSION"
[sources.unloaded]
spec="nonexistent.json"
"#
        ),
    )
    .unwrap();
    path
}

fn command(config: &Path, endpoint: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
    command
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .env("INDEXER", endpoint)
        .args(["wallet", "recover-expired", "--operation-id"])
        .arg("11111111-1111-4111-8111-111111111111")
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

#[tokio::test]
async fn missing_operation_after_canonical_sync_releases_owner_without_submission() {
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, _) = mock("main").await;
    let treasury = wallet(dir.path()).await;
    let before = treasury.status().await.unwrap();
    let addresses = treasury.addresses().await.unwrap();
    treasury.close().await.unwrap();
    let config = config(dir.path(), &before.treasury_id);
    // The deadline bounds a child process, including cold executable validation;
    // the local scan itself has no injected delay, proofs or funding work.
    let output = tokio::time::timeout(
        Duration::from_secs(120),
        command(&config, &endpoint)
            .spawn()
            .unwrap()
            .wait_with_output(),
    )
    .await
    .expect("recovery command did not exit")
    .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("transaction is not pending"), "{stderr}");
    assert!(calls.lock().unwrap().iter().any(|m| m == "GetBlockRange"));
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    let reopened = Treasury::open(
        dir.path().join("state"),
        dir.path().join("key"),
        before.treasury_id,
    )
    .await
    .unwrap();
    let after = reopened.status().await.unwrap();
    let saved = after.sync.unwrap();
    assert_eq!(
        saved.phase,
        SyncPhase::Offline,
        "failure cleanup closes the SDK"
    );
    assert_eq!(saved.height, Some(TIP));
    assert_eq!(saved.observed_tip_height, Some(TIP));
    assert!(saved.checked_at.is_some() && saved.last_error.is_none());
    assert!(stderr.contains("Zcash treasury wallet synchronized; progress saved"));
    assert!(!after.sync_complete);
    assert!(after.snapshot_revision > before.snapshot_revision);
    assert_eq!(reopened.addresses().await.unwrap(), addresses);
    assert!(!after.outgoing_pending && after.treasury_operations.is_empty());
    assert!(after.funding_jobs.is_empty());
    reopened.close().await.unwrap();
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn first_signal_cancels_recovery_scan_and_releases_owner() {
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, stalled) = mock("main").await;
    stalled.store(true, Ordering::SeqCst);
    let treasury = wallet(dir.path()).await;
    let before = treasury.status().await.unwrap();
    let addresses = treasury.addresses().await.unwrap();
    treasury.close().await.unwrap();
    let config = config(dir.path(), &before.treasury_id);
    let mut child = command(&config, &endpoint).spawn().unwrap();
    // An observed SDK block request proves the command registered its signal
    // receiver and accepted the scan. Wait for that phase, not elapsed time.
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            assert!(
                child.try_wait().unwrap().is_none(),
                "recovery exited before scan"
            );
            if calls.lock().unwrap().iter().any(|m| m == "GetBlockRange") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("recovery did not reach the stalled scan");
    assert!(
        Treasury::open(
            dir.path().join("state"),
            dir.path().join("key"),
            before.treasury_id.clone(),
        )
        .await
        .is_err(),
        "the scanning command must still own the wallet"
    );
    assert!(
        std::process::Command::new("/bin/kill")
            .args(["-TERM", &child.id().unwrap().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("recovery did not finish its cancellation and close")
        .unwrap();
    assert!(!output.status.success());
    assert!(
        output.status.code().is_some(),
        "command was killed instead of draining"
    );
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Draining accepted work"), "{stderr}");
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    let reopened = Treasury::open(
        dir.path().join("state"),
        dir.path().join("key"),
        before.treasury_id,
    )
    .await
    .unwrap();
    let after = reopened.status().await.unwrap();
    let saved = after.sync.unwrap();
    assert_eq!(saved.phase, SyncPhase::Offline);
    assert_eq!(saved.last_error.as_deref(), Some("treasury sync cancelled"));
    assert!(!after.sync_complete);
    assert!(after.snapshot_revision > before.snapshot_revision);
    assert_eq!(reopened.addresses().await.unwrap(), addresses);
    assert!(!after.outgoing_pending && after.treasury_operations.is_empty());
    assert!(after.funding_jobs.is_empty());
    reopened.close().await.unwrap();
    server.abort();
}
