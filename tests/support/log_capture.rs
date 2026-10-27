//! Isolate first-use tracing callsites from subscriberless sibling tests.

/// Returns true in the parent after the exact child test passes. The matching
/// child returns false so the caller executes its original fixture body.
pub fn run_in_child(test_name: &str) -> bool {
    if std::env::var("TREAZURY_LOG_CAPTURE_CHILD").as_deref() == Ok(test_name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env("TREAZURY_LOG_CAPTURE_CHILD", test_name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed; 0 failed"),
        "Isolated log-capture test did not execute successfully: {stdout}\n{stderr}"
    );
    true
}
