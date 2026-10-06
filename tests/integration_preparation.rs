//! Real executable, frozen offline catalogs, synthetic registry, no wallet credentials.
#![allow(dead_code, unused_imports)]
include!("../examples/live_integration/driver.rs");
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
        "binary":env!("CARGO_BIN_EXE_x402_treazury"),"evidence_dir":dir.join("evidence"),"registry_authorization":"reviewed",
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
async fn production_startup_records_pricing_modes_and_rebuilds_inventory_without_extra_probes() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let price = Arc::new(AtomicBool::new(false));
    let offer = price.clone();
    let app = axum::Router::new().route(
        "/read",
        axum::routing::get(move |request:axum::extract::Request| {
            let offer=offer.clone();
            let seen = seen.clone();
            async move {
                assert!(!request.headers().contains_key("payment-signature"));
                seen.fetch_add(1, Ordering::SeqCst);
                use axum::response::IntoResponse;
                use base64::Engine;
                if offer.load(Ordering::SeqCst)&&request.uri().query().is_none() {
                    let challenge=base64::engine::general_purpose::STANDARD.encode(json!({"accepts":[{"scheme":"exact","amount":"14000","asset":"USDC","network":"eip155:8453"}]}).to_string());
                    (axum::http::StatusCode::PAYMENT_REQUIRED,[("payment-required",challenge)],"unpaid").into_response()
                } else {"free result".into_response()}
            }
        }),
    );
    let vendor = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", vendor.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(vendor, app).await.unwrap() });
    for (enabled, nonempty) in [(false, false), (true, false), (true, true)] {
        price.store(nonempty, Ordering::SeqCst);
        let before = count.load(Ordering::SeqCst);
        let tmp = tempfile::tempdir().unwrap();
        let (mut m, state) = fixture(tmp.path());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let text = std::fs::read_to_string(&m.deployment)
            .unwrap()
            .replace("https://example.invalid", &base)
            .replace("127.0.0.1:18777", &addr.to_string())
            .replace("probe_pricing=false", &format!("probe_pricing={enabled}"));
        std::fs::write(&m.deployment, text).unwrap();
        m.pricing_checks.insert(
            "api".into(),
            if nonempty {
                pricing_stages::Expectation::Nonempty
            } else if enabled {
                pricing_stages::Expectation::Empty
            } else {
                pricing_stages::Expectation::Disabled
            },
        );
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
        assert_eq!(execution::run(&state, &m.run_id).await.unwrap(), 0);
        let r = registry::Registry::open(&state, true).unwrap();
        let report = r.report(Some(&m.run_id), 3).unwrap();
        assert_eq!(report["catalog_stages"][0]["phase"], "initial_inspection");
        assert_eq!(report["catalog_stages"][0]["status"], "completed");
        assert_eq!(report["catalog_stages"][1]["phase"], "frozen_reload");
        assert_eq!(
            report["catalog_stages"][1]["sources"][0]["state"],
            "completed"
        );
        assert_eq!(report["pricing_stages"][0]["assessment"], "passed");
        assert_eq!(
            report["pricing_stages"][0]["evidence"]["available_prices"],
            usize::from(nonempty)
        );
        assert_eq!(
            report["pricing_stages"][0]["evidence"]["observed"],
            usize::from(enabled)
        );
        assert_eq!(
            count.load(Ordering::SeqCst) - before,
            1 + usize::from(enabled)
        );
        assert_eq!(report["api_reserved_atomic"], 0);
    }
    server.abort();
    let _ = server.await;
}
#[tokio::test]
async fn production_catalogs_are_frozen_validated_and_bound_to_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, state) = fixture(tmp.path());
    let (plan, pins) = preparation::collect_catalogs(&m, &state).await.unwrap();
    assert_eq!(pins.qualification, "catalogs_prepared");
    let artifacts = pins.catalogs.as_ref().unwrap();
    artifacts.verify(&m, &pins).unwrap();
    let mut observation_only = m.clone();
    observation_only.binary = tmp.path().join("old-executable-no-longer-installed");
    artifacts.verify_archive(&observation_only).unwrap();
    assert!(artifacts.verify(&observation_only, &pins).is_err());
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
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
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
    let cwd = std::env::current_dir().unwrap();
    let tmp = tempfile::tempdir_in(cwd.join("target")).unwrap();
    let (mut m, state) = fixture(tmp.path());
    let state = state.strip_prefix(&cwd).unwrap().to_path_buf();
    assert!(state.is_relative());
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
    let binding = r
        .application_binding(&m.run_id, "path_check", "0".repeat(64), 100)
        .unwrap();
    assert_eq!(
        binding.registry,
        state
            .join("live-integration/registry.sqlite")
            .canonicalize()
            .unwrap()
    );
    drop(r);
    assert_eq!(execution::run(&state, &m.run_id).await.unwrap(), 2);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(maximum.load(Ordering::SeqCst), 2);
    assert!(
        execution::run(&state, &m.run_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("observation only")
    );
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
    assert_eq!(r.application_evidence(&m.run_id).unwrap()["claims"], 2);
    let db = rusqlite::Connection::open(state.join("live-integration/registry.sqlite")).unwrap();
    db.execute("DELETE FROM events WHERE kind='application_finished'", [])
        .unwrap();
    assert!(
        r.application_evidence(&m.run_id)
            .unwrap_err()
            .to_string()
            .contains("lacks application completion")
    );
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
    assert!(
        execution::run(&state, &m.run_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("observation only")
    );
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

/// Explicit offline fixture materialization for the unsigned live-Tor acceptance.
/// This creates an empty synthetic treasury, not an address to fund or a wallet.
#[test]
#[ignore = "opt-in fixture writer; requires TREAZURY_M4_FIXTURE_DIR, performs no network calls"]
fn prepare_real_tor_qualification_fixture() {
    let root = std::path::PathBuf::from(
        std::env::var_os("TREAZURY_M4_FIXTURE_DIR").expect("set a new absolute fixture directory"),
    );
    assert!(root.is_absolute());
    files::create_dir(&root).unwrap();
    let (mut m, state) = fixture(&root);
    let deployment = format!(
        r#"version=1
[treasury]
id="{treasury_id}"
state_dir="state"
key_file="nonexistent-key"
indexer_url_env="ABSENT_INDEXER"
submission_url_env="ABSENT_SUBMISSION"
daily_input_zec="0.1"
shield_max_fee_zec="0.001"
[network]
mode="tor"
socks_endpoint="127.0.0.1:19950"
isolation_namespace="m4_{}"
[wallets.test]
mode="static"
private_key_env="ABSENT_KEY"
[sources.trace]
spec="spec.json"
base_url="https://www.cloudflare.com"
prefix="trace"
help_url="https://www.cloudflare.com/cdn-cgi/trace"
probe_pricing=false
[sources.tor]
spec="spec.json"
base_url="https://www.torproject.org"
prefix="tor"
allow_http1=true
help_url="https://www.torproject.org/"
probe_pricing=false
[sources.fresh]
spec="spec.json"
base_url="https://www.cloudflare.com"
prefix="fresh"
help_url="https://www.cloudflare.com/cdn-cgi/trace?treazury_qualification=uncached"
probe_pricing=false
[sources.catalog]
spec="https://api.exa.ai/openapi.json"
base_url="https://api.exa.ai"
prefix="exa"
probe_pricing=false
[servers.main]
listen="127.0.0.1:19877"
sources=["trace","tor","fresh","catalog"]
wallet="test"
bearer_token_env="TOKEN"
"#,
        uuid::Uuid::new_v4().simple(),
        treasury_id = m.treasury_id
    );
    std::fs::write(&m.deployment, deployment).unwrap();
    m.run_id = "unsigned_tor".into();
    m.network.tor_mode = manifest::TorMode::Owned;
    m.network.confinement = manifest::Confinement::MacosSandbox;
    m.network.require_isolation_evidence = true;
    m.network.tor_binary = Some("/Applications/Tor Browser.app/Contents/MacOS/Tor/tor".into());
    m.limits.run_seconds = 600;
    m.limits.phase_seconds = 300;
    m.limits.call_seconds = 240;
    m.limits.result_bytes = 1024 * 1024;
    m.limits.cleanup_seconds = 20;
    let template = m.cases[0].clone();
    m.cases = [
        ("warm", "trace"),
        ("second", "tor"),
        ("cached", "trace"),
        ("uncached", "fresh"),
    ]
    .iter()
    .map(|(id, source)| {
        let mut c = template.clone();
        c.id = (*id).into();
        c.source = (*source).into();
        c.tool = format!("{source}_help");
        c.arguments.clear();
        c
    })
    .collect();
    m.phases[0].id = "connected".into();
    m.phases[0].cases = vec!["warm".into(), "second".into()];
    let mut outage = m.phases[0].clone();
    outage.id = "outage".into();
    outage.depends_on = vec!["connected".into()];
    outage.cases = vec!["cached".into(), "uncached".into()];
    outage.scenario = manifest::Scenario::TorOutage {
        warm_case: "warm".into(),
        cached_case: "cached".into(),
        uncached_case: "uncached".into(),
    };
    m.phases.push(outage);
    let auth = registry::Authorization {
        version: 1,
        id: "reviewed".into(),
        treasury_id: m.treasury_id.clone(),
        cumulative_api_usdc: "0".into(),
        cumulative_source_zec: "0".into(),
        cumulative_new_jobs: 0,
    };
    drop(
        registry::Registry::authorize(
            &state,
            &auth,
            x402_treazury::rotation::base::now().unwrap() as i64,
        )
        .unwrap(),
    );
    files::publish(
        &root.join("run.toml"),
        toml::to_string_pretty(&m).unwrap().as_bytes(),
    )
    .unwrap();
    println!(
        "Unsigned fixture ready at {}; no live requests performed; never fund this synthetic state",
        root.display()
    );
}

#[test]
fn unsigned_identity_export_keeps_retired_and_replacement_wallets_unpermitted() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, state) = fixture(tmp.path());
    let mut store = x402_treazury::rotation::store::Store::open(
        &state,
        &tmp.path().join("key"),
        &m.treasury_id,
    )
    .unwrap();
    let pool = store.ensure_pool("history", "1").unwrap();
    for address in &store.status().unwrap().pools[0].addresses {
        store
            .record_credit(&address.id, "1000000", "offline", 1)
            .unwrap();
    }
    store.promote(&pool, 0).unwrap();
    drop(store);
    std::fs::remove_file(tmp.path().join("key")).unwrap();
    let snapshot = json!({"deployment":{"network":{"mode":"tor","socks_endpoint":"127.0.0.1:19950","isolation_namespace":"identity-test"}},"sources":{"api":{"settings":{"spec":"https://catalog.example/openapi.json","help_url":"https://docs.example/llms.txt","base_url":"https://api.example"},"base_url":"https://api.example"}},"inventory":[{"server":"main","tools":[{"source":"api","name":"api_read","path":"/read"}]}]});
    let map = tor::identities::unsigned_map(&m, &snapshot, &state).unwrap();
    assert_eq!(map.values().filter(|v| v.kind == "evm").count(), 3);
    assert_eq!(map.values().filter(|v| v.kind == "discovery").count(), 3);
    assert!(
        map.values()
            .any(|v| v.targets.contains("docs.example:443") && !v.required)
    );
    assert!(
        map.values()
            .any(|v| v.targets.contains("catalog.example:443") && v.required)
    );
    let (label, wallet) = map.iter().find(|(_, v)| v.kind == "evm").unwrap();
    assert!(!wallet.permitted && !wallet.required);
    let subset = std::collections::BTreeMap::from([(label.clone(), wallet.clone())]);
    assert_eq!(
        tor::audit::verify(&subset, &[]).unwrap()["identities"][label]["status"],
        "not_observed"
    );
    let event = format!(
        "650 STREAM 1 NEW 0 api.example:443 SOCKS_USERNAME={} SOCKS_PASSWORD={}",
        wallet.user, wallet.password
    );
    assert!(
        tor::audit::verify(&subset, &[event])
            .unwrap_err()
            .to_string()
            .contains("not permitted")
    );
    let mut with_rpc = snapshot.clone();
    with_rpc["deployment"]["funding"] = json!({});
    let rpc_map = tor::identities::unsigned_map(&m, &with_rpc, &state).unwrap();
    assert_eq!(
        rpc_map
            .values()
            .filter(|v| v.kind == "discovery" && !v.permitted && !v.required)
            .count(),
        3
    );
    with_rpc["deployment"]["funding"]["base_rpc_url_env"] = json!("CUSTOM_RPC_NOT_IN_CHILD_ENV");
    assert!(tor::identities::unsigned_map(&m, &with_rpc, &state).is_err());
    let mut wrong = m.clone();
    wrong.treasury_id = uuid::Uuid::new_v4().to_string();
    assert!(tor::identities::unsigned_map(&wrong, &snapshot, &state).is_err());
}

#[test]
fn tor_audit_allows_multiple_circuits_per_identity_without_requiring_unused_history() {
    use std::collections::{BTreeMap, BTreeSet};
    let ids = BTreeMap::from([
        (
            "wallet".into(),
            tor::audit::Identity {
                user: "u".into(),
                password: "w".into(),
                kind: "evm".into(),
                targets: BTreeSet::new(),
                required: true,
                required_targets: Default::default(),
                permitted: true,
            },
        ),
        (
            "historical".into(),
            tor::audit::Identity {
                user: "u".into(),
                password: "old".into(),
                kind: "evm".into(),
                targets: BTreeSet::new(),
                required: false,
                required_targets: Default::default(),
                permitted: false,
            },
        ),
    ]);
    let events = vec![
        "650 STREAM 1 NEW 0 one.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=w".into(),
        "650 STREAM 1 SUCCEEDED 8 192.0.2.1:443".into(),
        "650 STREAM 2 NEW 0 two.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=w".into(),
        "650 STREAM 2 SUCCEEDED 9 192.0.2.2:443".into(),
    ];
    let result = tor::audit::verify(&ids, &events).unwrap();
    assert_eq!(result["identities"]["wallet"]["circuits"], 2);
    assert_eq!(result["identities"]["historical"]["status"], "not_observed");
}

#[test]
fn outage_catalog_validation_rejects_different_caches_and_prewarmed_fresh_urls() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut m, _state) = fixture(tmp.path());
    for id in ["cached", "fresh"] {
        let mut case = m.cases[0].clone();
        case.id = id.into();
        if id == "fresh" {
            case.tool = "api_fresh".into();
        }
        m.cases.push(case);
    }
    let mut phase = m.phases[0].clone();
    phase.scenario = manifest::Scenario::TorOutage {
        warm_case: "read".into(),
        cached_case: "cached".into(),
        uncached_case: "fresh".into(),
    };
    m.phases.push(phase);
    let snapshot = json!({"inventory":[{"server":"main","tools":[{"source":"api","name":"api_read","help_url":"https://docs.example/warm"},{"source":"api","name":"api_fresh","help_url":"https://docs.example/fresh"}]}]});
    tor::outage::validate(&m, &snapshot).unwrap();
    let mut bad = m.clone();
    bad.cases[1].server = "another_listener".into();
    assert!(tor::outage::validate(&bad, &snapshot).is_err());
    let mut bad = snapshot.clone();
    bad["inventory"][0]["tools"][1]["help_url"] = json!("https://docs.example/warm");
    assert!(tor::outage::validate(&m, &bad).is_err());
    let mut bad = snapshot.clone();
    bad["inventory"][0]["tools"][1]["help_url"] = json!(null);
    assert!(tor::outage::validate(&m, &bad).is_err());
    let mut bad = m.clone();
    let mut extra = bad.cases[2].clone();
    extra.id = "earlier_fresh".into();
    bad.cases.push(extra);
    assert!(tor::outage::validate(&bad, &snapshot).is_err());
}

#[tokio::test]
async fn managed_tor_map_preserves_wallet_scope_and_requires_paid_provider_destination() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut m, state) = fixture(tmp.path());
    m.start.pools = vec!["alpha".into()];
    m.cases[0].unsigned = false;
    let mut store = x402_treazury::rotation::store::Store::open(
        &state,
        &tmp.path().join("key"),
        &m.treasury_id,
    )
    .unwrap();
    let alpha = store.ensure_pool("alpha", "1").unwrap();
    store.ensure_pool("beta", "1").unwrap();
    let wallets = store
        .status()
        .unwrap()
        .pools
        .into_iter()
        .find(|p| p.name == "alpha")
        .unwrap()
        .addresses;
    for wallet in &wallets {
        store
            .record_credit(&wallet.id, "1000000", "offline", 1)
            .unwrap();
    }
    // Exercise the real snapshot shape. `--show-config` has extra resolved fields
    // which are intentionally absent from the configuration used for execution.
    let deployment = format!(
        r#"version=1
[network]
mode="tor"
socks_endpoint="127.0.0.1:19950"
isolation_namespace="managed-map"
[treasury]
id="{}"
state_dir="state"
key_file="key"
indexer_url_env="INDEXER"
submission_url_env="SUBMISSION"
daily_input_zec="1"
shield_max_fee_zec="0.001"
[funding]
auto_fund=false
confidentiality="public"
[wallets.alpha]
mode="zcash_rotation"
deposit_size="1"
max_input_zec="0.01"
max_fee_bps=500
[wallets.beta]
mode="zcash_rotation"
deposit_size="1"
max_input_zec="0.01"
max_fee_bps=500
[sources.api]
spec="spec.json"
base_url="https://api.example"
prefix="api"
wallet="alpha"
probe_pricing=false
[sources.other]
spec="spec.json"
base_url="https://other.example"
prefix="other"
wallet="beta"
probe_pricing=false
[servers.main]
listen="127.0.0.1:18777"
sources=["api","other"]
bearer_token_env="TOKEN"
"#,
        m.treasury_id
    );
    std::fs::write(&m.deployment, deployment).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .arg("--config")
        .arg(&m.deployment)
        .arg("--qualification-snapshot")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(snapshot["deployment"].get("resolved_wallets").is_none());
    assert!(snapshot["deployment"].get("wallet_bindings").is_none());
    let lookup = |name: &str| match name {
        "INDEXER" => Some("https://indexer.example".into()),
        "SUBMISSION" => Some("https://submission.example".into()),
        _ => None,
    };
    let before = tor::identities::managed_map(&m, &snapshot, &state, lookup, &[]).unwrap();
    let mut wrong_bindings = snapshot.clone();
    wrong_bindings["inventory"][0]["wallet_bindings"]["api"]["wallet"] = json!("beta");
    assert!(tor::identities::managed_map(&m, &wrong_bindings, &state, lookup, &[]).is_err());
    assert_eq!(before.values().filter(|i| i.kind == "evm").count(), 4);
    store.promote(&alpha, 0).unwrap();
    drop(store);
    std::fs::remove_file(tmp.path().join("key")).unwrap();
    let payment =
        json!({"case":"read","pool":alpha,"wallet":wallets[0].id,"address":wallets[0].address});
    let after = tor::identities::managed_map(
        &m,
        &snapshot,
        &state,
        lookup,
        std::slice::from_ref(&payment),
    )
    .unwrap();
    assert_eq!(after.values().filter(|i| i.kind == "evm").count(), 5);
    for (label, old) in &before {
        assert_eq!(
            (&old.user, &old.password),
            (&after[label].user, &after[label].password)
        );
    }
    for (label, identity) in after.iter().filter(|(_, i)| i.kind == "evm") {
        assert!(identity.permitted);
        if label.starts_with("evm_alpha_") {
            assert!(identity.targets.contains("api.example:443"));
            assert!(!identity.targets.contains("other.example:443"));
        } else {
            assert!(identity.targets.contains("other.example:443"));
            assert!(!identity.targets.contains("api.example:443"));
        }
    }
    let (label, wallet) = after
        .iter()
        .find(|(_, i)| i.kind == "evm" && i.required)
        .unwrap();
    assert!(wallet.required_targets.contains("api.example:443"));
    let subset = std::collections::BTreeMap::from([(label.clone(), wallet.clone())]);
    let events = |host: &str| {
        vec![
            format!(
                "650 STREAM 1 NEW 0 {host} SOCKS_USERNAME={} SOCKS_PASSWORD={}",
                wallet.user, wallet.password
            ),
            "650 STREAM 1 SUCCEEDED 1 192.0.2.1:443".into(),
        ]
    };
    assert!(tor::audit::verify(&subset, &events("indexer.example:443")).is_err());
    tor::audit::verify(&subset, &events("api.example:443")).unwrap();
    assert!(tor::audit::verify(&subset, &events("other.example:443")).is_err());
    let mut wrong = payment;
    wrong["address"] = json!("0x0000000000000000000000000000000000000000");
    assert!(tor::identities::managed_map(&m, &snapshot, &state, lookup, &[wrong]).is_err());
}

#[tokio::test]
async fn large_catalog_evidence_preserves_full_documents_without_raising_result_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, state) = fixture(tmp.path());
    let mut document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tmp.path().join("spec.json")).unwrap()).unwrap();
    document["paths"]["/read"]["get"]["responses"] = json!({
        "200":{"description":"x".repeat(files::DOCUMENT_BYTES + 1)}
    });
    std::fs::write(
        tmp.path().join("spec.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    let (_, pins) = preparation::collect_catalogs(&m, &state).await.unwrap();
    let artifacts = pins.catalogs.as_ref().unwrap();
    artifacts.verify(&m, &pins).unwrap();
    for name in ["snapshot.json", "frozen-snapshot.json"] {
        let path = artifacts.directory.join(name);
        // Normal result/document reads retain their smaller explicit bound.
        let error = files::read(&path).unwrap_err().to_string();
        assert!(error.contains("16777216 byte limit"));
        let snapshot: serde_json::Value =
            serde_json::from_slice(&files::read_catalog(&path).unwrap()).unwrap();
        assert_eq!(snapshot["sources"]["api"]["document"], document);
    }
    let frozen: serde_json::Value = serde_json::from_slice(
        &files::read_catalog(&artifacts.directory.join("source-api.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(frozen, document);
    assert_eq!(m.limits.result_bytes, 10000);
}

#[cfg(feature = "zcash")]
#[tokio::test]
async fn production_wallet_sync_failure_is_not_reported_as_absent_funds() {
    let dir = tempfile::tempdir().unwrap();
    let treasury = x402_treazury::treasury::Treasury::create(
        dir.path().join("state"),
        dir.path().join("key"),
        2_000_000,
        None,
    )
    .await
    .unwrap();
    let id = treasury.status().await.unwrap().treasury_id;
    treasury.close().await.unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    let app = axum::Router::new().fallback(move || {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { axum::http::StatusCode::SERVICE_UNAVAILABLE }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let config = dir.path().join("readiness.toml");
    std::fs::write(
        &config,
        format!(
            r#"version=1
servers={{}}
[treasury]
id="{id}"
state_dir="state"
key_file="key"
indexer_url_env="INDEXER"
submission_url_env="ABSENT_SUBMISSION"
daily_input_zec="0.1"
shield_max_fee_zec="0.001"
[sources.unused]
spec="missing.json"
"#
        ),
    )
    .unwrap();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
            .kill_on_drop(true)
            .env_clear()
            .env("INDEXER", &endpoint)
            .args(["wallet", "sync", "--config"])
            .arg(&config)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "failed sync must not publish successful state"
    );
    assert!(!output.stderr.is_empty());
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) > 0);
    let reopened = x402_treazury::treasury::Treasury::open(
        dir.path().join("state"),
        dir.path().join("key"),
        id,
    )
    .await
    .unwrap();
    assert!(reopened.status().await.unwrap().pools.is_empty());
    reopened.close().await.unwrap();
    server.abort();
}
