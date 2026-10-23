//! The real inspection command accounts for its terminating newline at the byte bound.
use std::{io::Write, path::Path, process::Stdio, time::Duration};
use x402_treazury::deployment::QUALIFICATION_SNAPSHOT_BYTES;

fn write_spec(path: &Path, padding: usize) {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    file.write_all(b"{\"paths\":{\"/read\":{\"get\":{\"responses\":{\"200\":{\"description\":\"")
        .unwrap();
    let block = [b'x'; 64 * 1024];
    for _ in 0..padding / block.len() {
        file.write_all(&block).unwrap();
    }
    file.write_all(&block[..padding % block.len()]).unwrap();
    file.write_all(b"\"}}}}}}").unwrap();
    file.flush().unwrap();
}

async fn inspect(dir: &Path) -> std::process::ExitStatus {
    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .current_dir(dir)
        .args([
            "config",
            "check",
            "--config",
            "config.toml",
            "--qualification-snapshot",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            std::fs::File::create(dir.join("stdout")).unwrap(),
        ))
        .stderr(Stdio::from(
            std::fs::File::create(dir.join("stderr")).unwrap(),
        ))
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .expect("offline snapshot inspection did not finish")
        .unwrap()
        .status
}

#[tokio::test]
async fn snapshot_byte_limit_includes_newline_and_rejects_without_partial_stdout() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!(
            r#"version=1
[treasury]
id="00000000-0000-4000-8000-000000000001"
state_dir="must-not-be-created"
key_file="must-not-be-opened"
indexer_url_env="ABSENT_INDEXER"
submission_url_env="ABSENT_SUBMISSION"
max_refund_shielding_fee_zec="0.001"
[wallets.test]
mode="static"
private_key_env="ABSENT_KEY"
[sources.test]
spec="spec.json"
base_url="https://snapshot.invalid"
prefix="test"
max_spec_bytes={}
[servers.one]
listen="127.0.0.1:0"
sources=["test"]
wallet="test"
bearer_token_env="ABSENT_TOKEN"
"#,
            QUALIFICATION_SNAPSHOT_BYTES + 1024
        ),
    )
    .unwrap();
    let spec = dir.path().join("spec.json");
    let stdout = dir.path().join("stdout");
    write_spec(&spec, 0);
    assert!(inspect(dir.path()).await.success());
    let baseline = std::fs::read(&stdout).unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&baseline).unwrap();
    assert_eq!(
        snapshot["sources"]["test"]["document"]["paths"]["/read"]["get"]["responses"]["200"]["description"],
        ""
    );
    assert_eq!(baseline.last(), Some(&b'\n'));
    let padding = QUALIFICATION_SNAPSHOT_BYTES
        .checked_sub(baseline.len())
        .unwrap();

    write_spec(&spec, padding);
    assert!(
        inspect(dir.path()).await.success(),
        "{}",
        std::fs::read_to_string(dir.path().join("stderr")).unwrap()
    );
    assert_eq!(
        std::fs::metadata(&stdout).unwrap().len(),
        QUALIFICATION_SNAPSHOT_BYTES as u64
    );
    // Read only the suffix of the large output: the final newline is part of the bound.
    use std::io::{Read, Seek, SeekFrom};
    let mut output = std::fs::File::open(&stdout).unwrap();
    output.seek(SeekFrom::End(-2)).unwrap();
    let mut ending = [0; 2];
    output.read_exact(&mut ending).unwrap();
    assert_eq!(&ending, b"}\n");
    drop(output);

    write_spec(&spec, padding + 1);
    assert!(!inspect(dir.path()).await.success());
    assert_eq!(std::fs::metadata(&stdout).unwrap().len(), 0);
    let stderr = std::fs::read_to_string(dir.path().join("stderr")).unwrap();
    assert!(
        stderr.contains("qualification snapshot exceeds 134217728-byte limit including newline"),
        "{stderr}"
    );
    assert!(!stderr.contains("MCP listening"));
    assert!(!dir.path().join("must-not-be-created").exists());
    assert!(!dir.path().join("must-not-be-opened").exists());
}
