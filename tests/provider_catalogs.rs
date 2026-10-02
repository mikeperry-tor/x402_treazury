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
    assert_eq!(cases.len(), 15);
    let mut total = 0;
    for case in cases {
        let mut command = Command::new(env!("CARGO_BIN_EXE_treazure"));
        command
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
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
    assert_eq!(total, 731);
}

#[tokio::test]
async fn directory_exact_allowlist_excludes_new_write_and_subroutes() {
    use x402_treazure::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/x402-list.toml"))
        .await
        .unwrap()
        .settings;
    assert!(!cfg.probe_pricing);
    let mut doc: Value =
        serde_json::from_str(include_str!("fixtures/x402_list_openapi.json")).unwrap();
    doc["paths"]["/services"]["post"] = serde_json::json!({"description":"new write"});
    doc["paths"]["/services/new-admin-action"] =
        serde_json::json!({"get":{"description":"new route"}});
    let tools = catalog::build_tools(&cfg, &doc, "x402_list").unwrap();
    assert_eq!(tools.len(), 5);
    assert!(tools.iter().all(|t| t.method == "GET"));
    let details = tools.iter().find(|t| t.path == "/services/{slug}").unwrap();
    let route = details
        .route(
            cfg.base_url.as_ref().unwrap(),
            serde_json::json!({"slug":"vendor/test"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        route.url,
        "https://x402-list.com/api/v1/services/vendor%2Ftest"
    );
    let search = tools.iter().find(|t| t.path == "/services").unwrap();
    let route = search
        .route(
            cfg.base_url.as_ref().unwrap(),
            serde_json::json!({"network":"BSE","limit":2})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(route.query["network"], "BSE");
    assert_eq!(route.query["limit"], 2);
}
