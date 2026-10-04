//! Real executable, frozen offline catalogs, synthetic registry, no wallet credentials.
#![allow(dead_code, unused_imports)]
#[path = "../examples/live_integration/execution.rs"]
mod execution;
#[path = "../examples/live_integration/files.rs"]
mod files;
#[path = "../examples/live_integration/manifest.rs"]
mod manifest;
#[path = "../examples/live_integration/planner.rs"]
mod planner;
#[path = "../examples/live_integration/preparation.rs"]
mod preparation;
#[path = "../examples/live_integration/process.rs"]
mod process;
#[path = "../examples/live_integration/registry.rs"]
mod registry;
#[path = "../examples/live_integration/schema.rs"]
mod schema;
use serde_json::json;
use std::path::Path;
fn fixture(dir: &Path) -> (manifest::Manifest, std::path::PathBuf) {
    let state = dir.join("state");
    let store =
        x402_treazury::rotation::store::Store::create(&state, &dir.join("key"), 1, b"synthetic")
            .unwrap();
    let id = store.id().to_owned();
    drop(store);
    files::create_dir(&dir.join("evidence")).unwrap();
    std::fs::write(dir.join("spec.json"), json!({"paths":{"/read":{"get":{"parameters":[{"in":"query","name":"n","required":true,"schema":{"type":"integer","minimum":1}}]}}}}).to_string()).unwrap();
    std::fs::write(
        dir.join("deployment.toml"),
        format!(
            r#"version=1
[treasury]
id="{id}"
state_dir="state"
key_file="nonexistent-key"
indexer_url_env="ABSENT_INDEXER"
submission_url_env="ABSENT_SUBMISSION"
daily_input_zec="0.1"
shield_max_fee_zec="0.001"
[wallets.test]
mode="static"
private_key_env="ABSENT_KEY"
[sources.api]
spec="spec.json"
base_url="https://example.invalid"
prefix="api"
probe_pricing=false
[servers.main]
listen="127.0.0.1:18777"
sources=["api"]
wallet="test"
bearer_token_env="TOKEN"
"#
        ),
    )
    .unwrap();
    let m = serde_json::from_value(json!({
        "version":1,"run_id":"prepared","treasury_id":id,"deployment":dir.join("deployment.toml"),
        "binary":env!("CARGO_BIN_EXE_treazury"),"evidence_dir":dir.join("evidence"),"registry_authorization":"reviewed",
        "start":{"mode":"funded_pools","pools":[]},
        "network":{"tor_mode":"direct","confinement":"none","require_isolation_evidence":false},
        "limits":{"api_reservation_usdc":"0","new_funding_jobs":0,"source_exposure_zec":"0","max_in_flight":1,"run_seconds":60,"phase_seconds":30,"call_seconds":10,"cleanup_seconds":10,"result_bytes":10000},
        "catalog":{"execution":"frozen","record_live_discovery":true},
        "windows":[{"id":"now","not_before":1,"not_after":4102444800u64}],
        "cases":[{"id":"read","server":"main","source":"api","tool":"api_read","arguments":{"n":1},"reserve_usdc":"0","reviewed_read_only":true,"unsigned":true}],
        "phases":[{"id":"unsigned","window":"now","cases":["read"],"pools":[],"required":true,"scenario":{"kind":"unsigned"}}]
    })).unwrap();
    (m, state)
}
#[tokio::test]
async fn production_catalogs_are_frozen_validated_and_bound_to_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, state) = fixture(tmp.path());
    let (plan, pins) = preparation::collect_catalogs(&m, &state).await.unwrap();
    assert_eq!(pins.qualification, "catalogs_prepared");
    let artifacts = pins.catalogs.as_ref().unwrap();
    artifacts.verify(&m, &pins).unwrap();
    assert!(!tmp.path().join("nonexistent-key").exists());
    let auth = registry::Authorization {
        version: 1,
        id: "reviewed".into(),
        treasury_id: m.treasury_id.clone(),
        cumulative_api_usdc: "0".into(),
        cumulative_source_zec: "0".into(),
        cumulative_new_jobs: 0,
    };
    let mut r = registry::Registry::authorize(&state, &auth, 2).unwrap();
    r.prepare(&plan, &pins, 2).unwrap();
    assert_eq!(
        r.report(Some(&m.run_id), 3).unwrap()["execution_available"],
        true
    );
    // Vendor changes after preparation do not change the frozen documents.
    std::fs::write(tmp.path().join("spec.json"), "{}").unwrap();
    artifacts.verify(&m, &pins).unwrap();
    std::fs::write(artifacts.directory.join("source-api.json"), "{}").unwrap();
    assert!(
        artifacts
            .verify(&m, &pins)
            .unwrap_err()
            .to_string()
            .contains("artifact changed")
    );
}
#[tokio::test]
async fn filtered_tool_and_invalid_arguments_never_prepare() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut m, state) = fixture(tmp.path());
    m.cases[0].arguments.insert("n".into(), json!(0));
    assert!(
        format!(
            "{:#}",
            preparation::collect_catalogs(&m, &state)
                .await
                .err()
                .unwrap()
        )
        .contains("arguments cannot be certified")
    );
    m.run_id = "unknown".into();
    m.cases[0].tool = "api_missing".into();
    assert!(
        format!(
            "{:#}",
            preparation::collect_catalogs(&m, &state)
                .await
                .err()
                .unwrap()
        )
        .contains("not in its selected")
    );
}
#[tokio::test]
async fn build_info_is_credential_free_and_matches_compiled_library() {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_treazury"))
        .env_clear()
        .arg("build-info")
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let identity: x402_treazury::build_identity::BuildIdentity =
        serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(identity, x402_treazury::build_identity::current());
}

#[tokio::test]
async fn runner_executes_concurrent_keyless_cases_once_and_retains_failures() {
    use axum::{
        Router,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let count = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let mut app = Router::new();
    for (path, paid) in [("/read", false), ("/paid", true)] {
        let count = count.clone();
        let active = active.clone();
        let maximum = maximum.clone();
        app = app.route(
            path,
            get(move |headers: HeaderMap| {
                let count = count.clone();
                let active = active.clone();
                let maximum = maximum.clone();
                async move {
                    assert!(
                        !headers.contains_key("payment-signature")
                            && !headers.contains_key("x-payment")
                    );
                    count.fetch_add(1, Ordering::SeqCst);
                    let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(n, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    if paid {
                        (StatusCode::PAYMENT_REQUIRED, "payment needed").into_response()
                    } else {
                        "free response".into_response()
                    }
                }
            }),
        );
    }
    let vendor = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", vendor.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(vendor, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let (mut m, state) = fixture(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let text = std::fs::read_to_string(&m.deployment)
        .unwrap()
        .replace("https://example.invalid", &base)
        .replace("127.0.0.1:18777", &addr.to_string());
    std::fs::write(&m.deployment, text).unwrap();
    let spec_path = tmp.path().join("spec.json");
    let mut spec: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&spec_path).unwrap()).unwrap();
    spec["paths"]["/paid"] = json!({"get":{}});
    std::fs::write(spec_path, spec.to_string()).unwrap();
    let mut case = m.cases[0].clone();
    case.id = "rejected".into();
    case.tool = "api_paid".into();
    case.arguments.clear();
    m.cases.push(case);
    m.limits.max_in_flight = 2;
    m.phases[0].cases.push("rejected".into());
    m.phases[0].scenario = manifest::Scenario::Concurrency {
        batches: vec![vec!["read".into(), "rejected".into()]],
    };
    let (plan, pins) = preparation::collect_catalogs(&m, &state).await.unwrap();
    let auth = registry::Authorization {
        version: 1,
        id: "reviewed".into(),
        treasury_id: m.treasury_id.clone(),
        cumulative_api_usdc: "0".into(),
        cumulative_source_zec: "0".into(),
        cumulative_new_jobs: 0,
    };
    let mut r = registry::Registry::authorize(&state, &auth, 2).unwrap();
    r.prepare(&plan, &pins, 2).unwrap();
    drop(r);
    assert_eq!(execution::run(&state, &m.run_id).await.unwrap(), 2);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(maximum.load(Ordering::SeqCst), 2);
    assert_eq!(execution::run(&state, &m.run_id).await.unwrap(), 2);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let r = registry::Registry::open(&state, true).unwrap();
    let report = r.report(Some(&m.run_id), 3).unwrap();
    assert!(
        report["cases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["execution"] == "COMPLETED" && c["settlement"] == "NOT_SIGNED")
    );
    assert_eq!(report["api_reserved_atomic"], 0);
    server.abort();
}

#[tokio::test]
async fn runner_response_limit_and_timeout_are_visible_and_never_replayed() {
    use axum::{Router, routing::get};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let count = Arc::new(AtomicUsize::new(0));
    let a = count.clone();
    let b = count.clone();
    let app = Router::new()
        .route(
            "/read",
            get(move || {
                let a = a.clone();
                async move {
                    a.fetch_add(1, Ordering::SeqCst);
                    "x".repeat(4000)
                }
            }),
        )
        .route(
            "/slow",
            get(move || {
                let b = b.clone();
                async move {
                    b.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    "slow"
                }
            }),
        );
    let vendor = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", vendor.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(vendor, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let (mut m, state) = fixture(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let text = std::fs::read_to_string(&m.deployment)
        .unwrap()
        .replace("https://example.invalid", &base)
        .replace("127.0.0.1:18777", &addr.to_string());
    std::fs::write(&m.deployment, text).unwrap();
    let spec_path = tmp.path().join("spec.json");
    let mut spec: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&spec_path).unwrap()).unwrap();
    spec["paths"]["/slow"] = json!({"get":{}});
    std::fs::write(spec_path, spec.to_string()).unwrap();
    let mut c = m.cases[0].clone();
    c.id = "slow".into();
    c.tool = "api_slow".into();
    c.arguments.clear();
    m.cases.push(c);
    m.phases[0].cases.push("slow".into());
    m.limits.result_bytes = 2000;
    m.limits.call_seconds = 1;
    let (plan, pins) = preparation::collect_catalogs(&m, &state).await.unwrap();
    let auth = registry::Authorization {
        version: 1,
        id: "reviewed".into(),
        treasury_id: m.treasury_id.clone(),
        cumulative_api_usdc: "0".into(),
        cumulative_source_zec: "0".into(),
        cumulative_new_jobs: 0,
    };
    let mut r = registry::Registry::authorize(&state, &auth, 2).unwrap();
    r.prepare(&plan, &pins, 2).unwrap();
    drop(r);
    assert_eq!(execution::run(&state, &m.run_id).await.unwrap(), 3);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let r = registry::Registry::open(&state, true).unwrap();
    let report = r.report(Some(&m.run_id), 3).unwrap();
    assert!(
        report["cases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["execution"] == "TRANSPORT_UNCERTAIN" && c["settlement"] == "NOT_SIGNED")
    );
    let events = report["runtime_events"].to_string();
    assert!(events.contains("result_bytes=2000"));
    assert!(events.contains("mcp_transport_timeout") || events.contains("case_deadline"));
    drop(r);
    assert_eq!(execution::run(&state, &m.run_id).await.unwrap(), 3);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let sessions: Vec<_> = std::fs::read_dir(pins.catalogs.unwrap().directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("session-")
        })
        .collect();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].join("results.json").exists());
    server.abort();
}
