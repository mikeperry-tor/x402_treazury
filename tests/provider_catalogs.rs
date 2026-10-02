//! Frozen independent catalog expectations; no Python or live vendor needed.
use serde::Deserialize;
use serde_json::Value;
use std::{path::Path, process::Command};
#[derive(Deserialize)]
struct Case {
    name: String,
    provider: String,
    spec: Option<String>,
}
#[test]
fn bundled_catalogs_match_reviewed_tool_contracts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = root;
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/catalogs/cases.json")).unwrap();
    assert_eq!(cases.len(), 14);
    let mut total = 0;
    for case in cases {
        let mut command = Command::new(env!("CARGO_BIN_EXE_x402-mcp-prototype"));
        command
            .env_clear()
            .current_dir(repo)
            .arg("--config")
            .arg(&case.provider)
            .arg("--list-tools");
        if let Some(spec) = case.spec {
            command.arg("--spec").arg(spec);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            case.name,
            String::from_utf8_lossy(&output.stderr)
        );
        let mut actual: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
        actual.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let expected: Vec<Value> = serde_json::from_slice(
            &std::fs::read(root.join(format!("tests/fixtures/catalogs/{}.json", case.name)))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(actual.len(), expected.len(), "{} tool count", case.name);
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(actual, expected, "{}: {}", case.name, expected["name"]);
        }
        total += actual.len();
    }
    assert_eq!(total, 726);
}
