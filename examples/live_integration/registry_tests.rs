use super::{config, manifest};
use crate::{
    files, planner,
    registry::{Authorization, Outcome, Pins, Registry},
};
use serde_json::{Value, json};
use std::path::PathBuf;
struct Fixture {
    _dir: tempfile::TempDir,
    state: PathBuf,
    auth: Authorization,
    manifest: crate::manifest::Manifest,
    pins: Pins,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let store = x402_treazury::rotation::store::Store::create(
            &state,
            &dir.path().join("key"),
            1,
            b"offline-test-snapshot",
        )
        .unwrap();
        let id = store.id().to_owned();
        drop(store);
        let mut m = manifest();
        m.treasury_id = id.clone();
        let mut config = config();
        config["treasury"]["id"] = json!(id);
        let auth = Authorization {
            version: 1,
            id: m.registry_authorization.clone(),
            treasury_id: id,
            cumulative_api_usdc: "0.04".into(),
            cumulative_source_zec: "0".into(),
            cumulative_new_jobs: 0,
        };
        let pins = Pins {
            binary_sha256: files::hash(b"fixture-binary"),
            cargo_lock_sha256: files::hash(b"fixture-lock"),
            source_revision: "fixture".into(),
            source_dirty: false,
            resolved_config: config,
            qualification: "configuration_only".into(),
            catalogs: None,
        };
        Self {
            _dir: dir,
            state,
            auth,
            manifest: m,
            pins,
        }
    }
    fn open(&self) -> Registry {
        Registry::authorize(&self.state, &self.auth, 2).unwrap()
    }
    fn prepare(&self, r: &mut Registry) {
        r.prepare(
            &planner::build(&self.manifest, &self.pins.resolved_config).unwrap(),
            &self.pins,
            2,
        )
        .unwrap();
    }
}
fn reserved(r: &Registry) -> Value {
    r.report(None, 10).unwrap()["api_reserved_atomic"].clone()
}
#[test]
fn provider_assertions_gate_dependencies_without_rewriting_mcp_or_payment_evidence() {
    for degraded in [true, false] {
        let mut f = Fixture::new();
        f.manifest.start.pools.clear();
        f.manifest.cases[0].unsigned = true;
        f.manifest.cases[0].reserve_usdc = "0".into();
        f.manifest.cases[0].checks = vec![crate::semantics::Check::JsonEquals {
            pointer: "/degraded".into(),
            value: json!(false),
        }];
        let mut next = f.manifest.cases[0].clone();
        next.id = "next".into();
        next.checks.clear();
        f.manifest.cases.push(next);
        f.manifest.phases[0].pools.clear();
        f.manifest.phases[0].scenario = crate::manifest::Scenario::Unsigned {};
        let mut phase = f.manifest.phases[0].clone();
        phase.id = "dependent".into();
        phase.cases = vec!["next".into()];
        phase.depends_on = vec!["smoke".into()];
        f.manifest.phases.push(phase);
        let mut r = f.open();
        f.prepare(&mut r);
        r.reserve_batch("test_run", "one", &["call".into()], 10)
            .unwrap();
        r.dispatching("test_run", "call").unwrap();
        let bytes = serde_json::to_vec(&json!({"result":{"isError":false,"content":[{"type":"text","text":json!({"degraded":degraded}).to_string()}]}})).unwrap();
        r.finish_unsigned("test_run", "call", &bytes, true).unwrap();
        let report = r.report(Some("test_run"), 11).unwrap();
        assert_eq!(report["cases"][0]["semantic"], "PASSED");
        assert_eq!(report["cases"][0]["settlement"], "NOT_SIGNED");
        assert_eq!(
            report["provider_semantics"][0]["status"],
            if degraded { "failed" } else { "passed" }
        );
        assert_eq!(
            r.reserve_batch("test_run", "two", &["next".into()], 12)
                .is_err(),
            degraded
        );
        assert_eq!(
            r.response("test_run", "call").unwrap(),
            serde_json::from_slice::<Value>(&bytes).unwrap()
        );
        let response = f
            .state
            .join("live-integration")
            .join(format!("response-{}.bin", files::hash("test_run:call")));
        std::fs::write(response, b"changed evidence").unwrap();
        let report = r.report(Some("test_run"), 13).unwrap();
        assert_eq!(report["provider_semantics"][0]["status"], "invalid");
        assert_eq!(report["provider_semantics"][0]["required"], true);
        assert_eq!(report["cases"][0]["semantic"], "PASSED");
    }
}
#[test]
fn help_cache_dependency_cannot_pass_from_mcp_success_alone() {
    let mut f = Fixture::new();
    f.manifest.start.pools.clear();
    f.manifest.cases[0].unsigned = true;
    f.manifest.cases[0].reserve_usdc = "0".into();
    f.manifest.cases[0].help_cache = Some(crate::help::ExpectedCache::Hit);
    let mut next = f.manifest.cases[0].clone();
    next.id = "next".into();
    next.help_cache = None;
    f.manifest.cases.push(next);
    f.manifest.phases[0].pools.clear();
    f.manifest.phases[0].scenario = crate::manifest::Scenario::Unsigned {};
    let mut phase = f.manifest.phases[0].clone();
    phase.id = "dependent".into();
    phase.cases = vec!["next".into()];
    phase.depends_on = vec!["smoke".into()];
    f.manifest.phases.push(phase);
    let mut r = f.open();
    f.prepare(&mut r);
    r.reserve_batch("test_run", "one", &["call".into()], 10)
        .unwrap();
    r.dispatching("test_run", "call").unwrap();
    let bytes = serde_json::to_vec(
        &json!({"result":{"isError":false,"content":[{"type":"text","text":"docs"}]}}),
    )
    .unwrap();
    r.finish_unsigned("test_run", "call", &bytes, true).unwrap();
    let report = r.report(Some("test_run"), 11).unwrap();
    assert_eq!(report["cases"][0]["semantic"], "PASSED");
    assert_eq!(report["help_stages"][0]["status"], "incomplete");
    assert!(
        r.reserve_batch("test_run", "two", &["next".into()], 12)
            .unwrap_err()
            .to_string()
            .contains("provider/help assertions")
    );
    assert_eq!(
        r.execution_state("test_run", "next").unwrap(),
        "UNATTEMPTED"
    );
}
#[test]
fn reservations_survive_errors_and_new_runs_do_not_reset_authority() {
    let mut f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    assert!(
        r.prepare(
            &planner::build(&f.manifest, &f.pins.resolved_config).unwrap(),
            &f.pins,
            2
        )
        .is_err()
    );
    r.reserve_batch("test_run", "one", &["call".into()], 10)
        .unwrap();
    assert!(
        r.reserve_batch("test_run", "again", &["call".into()], 10)
            .is_err()
    );
    r.dispatching("test_run", "call").unwrap();
    r.finish("test_run", "call", Outcome::TransportUncertain, None)
        .unwrap();
    assert_eq!(reserved(&r), 20000);
    drop(r);
    let mut r = Registry::open(&f.state, false).unwrap();
    assert_eq!(
        r.report(None, 90000).unwrap()["cases"][0]["eligibility"],
        "observe_only"
    );
    f.manifest.run_id = "second".into();
    f.prepare(&mut r);
    r.reserve_batch("second", "one", &["call".into()], 10)
        .unwrap();
    f.manifest.run_id = "third".into();
    f.prepare(&mut r);
    assert!(
        r.reserve_batch("third", "one", &["call".into()], 10)
            .is_err()
    );
    assert_eq!(reserved(&r), 40000);
    assert!(
        r.report(None, 10)
            .unwrap()
            .to_string()
            .find(&f.auth.treasury_id)
            .is_none()
    );
}
#[test]
fn batch_is_all_or_none_and_windows_are_absolute() {
    let mut f = Fixture::new();
    let mut r = f.open();
    let mut c = f.manifest.cases[0].clone();
    c.id = "second".into();
    f.manifest.cases.push(c);
    f.manifest.phases[0].cases.push("second".into());
    f.manifest.phases[0].scenario = crate::manifest::Scenario::Concurrency {
        batches: vec![vec!["call".into(), "second".into()]],
    };
    f.prepare(&mut r);
    for ids in [
        vec!["call".into(), "unknown".into()],
        vec!["call".into(), "call".into()],
        vec!["call".into()],
    ] {
        assert!(r.reserve_batch("test_run", "bad", &ids, 10).is_err());
        assert_eq!(reserved(&r), 0);
    }
    let ids = vec!["call".into(), "second".into()];
    assert!(r.reserve_batch("test_run", "bad", &ids, 0).is_err());
    assert!(r.reserve_batch("test_run", "bad", &ids, 1000).is_err());
    // Abort on the second row after the first update, proving transactional rollback.
    let db = rusqlite::Connection::open(f.state.join("live-integration/registry.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_second BEFORE UPDATE ON cases WHEN NEW.id='second' BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(r.reserve_batch("test_run", "both", &ids, 1).is_err());
    assert_eq!(reserved(&r), 0);
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM batches", [], |row| row.get(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_second;").unwrap();
    r.reserve_batch("test_run", "both", &ids, 1).unwrap();
    assert_eq!(reserved(&r), 40000);
}
#[test]
fn authorization_revisions_preserve_baseline_and_charged_reservations() {
    let mut f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    // A changed run cannot replace existing pins, even before reservation.
    let mut changed = f.pins.clone();
    changed.binary_sha256 = files::hash(b"new");
    assert!(
        r.prepare(
            &planner::build(&f.manifest, &f.pins.resolved_config).unwrap(),
            &changed,
            10
        )
        .is_err()
    );
    r.reserve_batch("test_run", "one", &["call".into()], 10)
        .unwrap();
    drop(r);
    // An explicit new authorization cannot erase consumption; duplicate authorization IDs fail.
    assert!(Registry::authorize(&f.state, &f.auth, 10).is_err());
    f.auth.id = "review_2".into();
    f.auth.cumulative_api_usdc = "0.01".into();
    assert!(Registry::authorize(&f.state, &f.auth, 10).is_err());
    f.auth.cumulative_api_usdc = "0.05".into();
    let r = Registry::authorize(&f.state, &f.auth, 10).unwrap();
    assert_eq!(reserved(&r), 20000);
}
#[test]
fn ownership_and_unsafe_aliases_are_refused_and_reports_never_overwrite() {
    let f = Fixture::new();
    let r = f.open();
    assert!(Registry::open(&f.state, true).is_err());
    drop(r);
    let path = f.state.join("live-integration/registry.sqlite");
    #[cfg(unix)]
    {
        let link = f.state.join("alias");
        std::fs::hard_link(&path, &link).unwrap();
        assert!(Registry::open(&f.state, false).is_err());
        std::fs::remove_file(link).unwrap();
        std::os::unix::fs::symlink(&path, f.state.join("live-integration/registry.sqlite-wal"))
            .unwrap();
        assert!(Registry::open(&f.state, false).is_err());
        std::fs::remove_file(f.state.join("live-integration/registry.sqlite-wal")).unwrap();
    }
    let output = f.state.join("report.json");
    files::publish(&output, b"one").unwrap();
    assert!(files::publish(&output, b"two").is_err());
    assert_eq!(std::fs::read(output).unwrap(), b"one");
    Registry::open(&f.state, true).unwrap();
}
#[test]
fn body_evidence_is_bounded_private_and_not_settlement_proof() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    r.reserve_batch("test_run", "one", &["call".into()], 10)
        .unwrap();
    r.dispatching("test_run", "call").unwrap();
    assert!(
        r.finish(
            "test_run",
            "call",
            Outcome::ResponseSaved,
            Some(&vec![0; 1001])
        )
        .is_err()
    );
    r.finish("test_run", "call", Outcome::ResponseSaved, Some(b"body"))
        .unwrap();
    assert!(
        r.finish(
            "test_run",
            "call",
            Outcome::ResponseSaved,
            Some(b"replacement")
        )
        .is_err()
    );
    let report = r.report(None, 10).unwrap();
    assert_eq!(report["cases"][0]["result_hash"], files::hash(b"body"));
    assert_eq!(report["cases"][0]["settlement"], "PENDING");
    assert_eq!(reserved(&r), 20000);
}
#[test]
fn committed_reservation_survives_abrupt_process_exit() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    drop(r);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::registry_tests::crash_child",
            "--ignored",
            "--nocapture",
        ])
        .env("TREAZURY_TEST_REGISTRY", &f.state)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(86),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut r = Registry::open(&f.state, false).unwrap();
    assert_eq!(reserved(&r), 20000);
    assert!(
        r.reserve_batch("test_run", "replay", &["call".into()], 10)
            .is_err()
    );
}
#[test]
#[ignore = "subprocess crash helper exercised by parent"]
fn crash_child() {
    let state = std::env::var_os("TREAZURY_TEST_REGISTRY").expect("parent fixture required");
    let mut r = Registry::open(std::path::Path::new(&state), false).unwrap();
    r.reserve_batch("test_run", "committed", &["call".into()], 10)
        .unwrap();
    std::process::exit(86);
}

#[test]
fn legacy_baseline_is_immutable_and_active_treasury_is_refused() {
    use std::io::Write;
    let f = Fixture::new();
    let legacy = f.state.join("qualification");
    files::create_dir(&legacy).unwrap();
    files::create_file(&legacy.join("owner.lock")).unwrap();
    let mut file = files::create_file(&legacy.join("run.sqlite")).unwrap();
    file.write_all(b"historic ledger bytes").unwrap();
    file.sync_all().unwrap();
    drop(file);
    let owner = files::lock(&f.state.join("owner.lock")).unwrap();
    assert!(Registry::authorize(&f.state, &f.auth, 2).is_err());
    drop(owner);
    let legacy_owner = files::lock(&legacy.join("owner.lock")).unwrap();
    assert!(Registry::authorize(&f.state, &f.auth, 2).is_err());
    drop(legacy_owner);
    let r = f.open();
    drop(r);
    assert_eq!(
        std::fs::read(legacy.join("run.sqlite")).unwrap(),
        b"historic ledger bytes"
    );
    let db = rusqlite::Connection::open(f.state.join("live-integration/registry.sqlite")).unwrap();
    let raw: String = db
        .query_row("SELECT baseline FROM identity", [], |r| r.get(0))
        .unwrap();
    let baseline: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        baseline["legacy_file_hashes"]["qualification/run.sqlite"],
        files::hash(b"historic ledger bytes")
    );
    assert!(baseline["payment_attempts"].is_array());
    assert!(baseline["source_budget_entries"].is_array());
    let mut auth = f.auth.clone();
    auth.id = "next".into();
    drop(db);
    let r = Registry::authorize(&f.state, &auth, 3).unwrap();
    drop(r);
    let db = rusqlite::Connection::open(f.state.join("live-integration/registry.sqlite")).unwrap();
    assert_eq!(
        raw,
        db.query_row::<String, _, _>("SELECT baseline FROM identity", [], |r| r.get(0))
            .unwrap()
    );
}

#[test]
fn competing_supervisor_cannot_prepare_or_reserve() {
    let f = Fixture::new();
    let mut owner = f.open();
    f.prepare(&mut owner);
    let state = f.state.clone();
    let thread = std::thread::spawn(move || {
        assert!(Registry::open(&state, false).is_err());
    });
    thread.join().unwrap();
    owner
        .reserve_batch("test_run", "only", &["call".into()], 10)
        .unwrap();
    drop(owner);
    let mut next = Registry::open(&f.state, false).unwrap();
    assert!(
        next.reserve_batch("test_run", "again", &["call".into()], 10)
            .is_err()
    );
    assert_eq!(reserved(&next), 20000);
}

#[tokio::test]
async fn commands_observe_historical_configuration_only_runs_without_network() {
    use clap::Parser;
    let mut f = Fixture::new();
    let dir = f._dir.path();
    let deployment = format!(
        r#"version=1
[treasury]
id="{}"
state_dir="state"
key_file="NEVER_READ_KEY"
indexer_url_env="UNSET_INDEXER"
submission_url_env="UNSET_SUBMISSION"
daily_treasury_spend_limit_zec="0.1"
max_refund_shielding_fee_zec="0.001"
[funding]
confidentiality="public"
[wallets.pool]
mode="zcash_rotation"
max_api_payment_usdc="0.02"
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[sources.api]
spec="https://unreachable.invalid/openapi.json"
[servers.main]
listen="127.0.0.1:0"
bearer_token_env="UNSET_TOKEN"
wallet="pool"
sources=["api"]
"#,
        f.auth.treasury_id
    );
    let now = x402_treazury::rotation::base::now().unwrap();
    f.manifest.windows[0].not_before = now + 60;
    f.manifest.windows[0].not_after = now + 3600;
    f.manifest.binary = dir.join("not-an-executable");
    std::fs::write(&f.manifest.binary, b"this must only be hashed").unwrap();
    f.manifest.deployment = dir.join("deployment.toml");
    std::fs::write(&f.manifest.deployment, deployment).unwrap();
    let auth = dir.join("authorization.toml");
    std::fs::write(&auth, toml::to_string(&f.auth).unwrap()).unwrap();
    let manifest = dir.join("manifest.toml");
    std::fs::write(&manifest, toml::to_string(&f.manifest).unwrap()).unwrap();
    let mut registry = Registry::authorize(&f.state, &f.auth, now as i64).unwrap();
    let (plan, pins) = crate::preparation::collect(&f.manifest, &f.state)
        .await
        .unwrap();
    registry.prepare(&plan, &pins, now as i64).unwrap();
    drop(registry);
    // Historical config-only records remain readable; only complete preparation can run.
    for obsolete in [
        "init",
        "funding-readiness",
        "prepare-catalogs",
        "revise-pins",
        "status",
        "resume",
    ] {
        assert!(crate::Args::try_parse_from(["live_integration", obsolete]).is_err());
    }
    for extra in [vec![], vec!["--run", "test_run", "--eligibility"]] {
        let mut args = vec![
            "live_integration",
            "report",
            "--state-dir",
            f.state.to_str().unwrap(),
        ];
        args.extend(extra);
        let command = crate::Args::try_parse_from(args).unwrap().command;
        assert_eq!(crate::execute(command).await.unwrap(), 0);
    }
    for extra in [
        vec!["--eligibility"],
        vec!["--run", "test_run", "--allow-funding"],
    ] {
        let mut args = vec!["live_integration", "report", "--state-dir", "unused"];
        args.extend(extra);
        assert!(crate::Args::try_parse_from(args).is_err());
    }
    // Reporting a started run retains window observations without suggesting replay.
    let mut r = Registry::open(&f.state, false).unwrap();
    r.begin_execution("test_run", now as i64).unwrap();
    drop(r);
    let output = f.state.join("eligibility.json");
    let command = crate::Args::try_parse_from([
        "live_integration",
        "report",
        "--state-dir",
        f.state.to_str().unwrap(),
        "--run",
        "test_run",
        "--eligibility",
        "--output",
        output.to_str().unwrap(),
    ])
    .unwrap()
    .command;
    assert_eq!(crate::execute(command).await.unwrap(), 0);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(saved["eligibility"]["run_unstarted"], false);
    assert_eq!(saved["eligibility"]["execution_authorized"], false);
    assert_eq!(
        saved["eligibility"]["windows"]["waiting_window"],
        json!(["call"])
    );
    let r = Registry::open(&f.state, true).unwrap();
    assert_eq!(reserved(&r), 0);
    assert_eq!(
        r.report(None, now as i64).unwrap()["execution_available"],
        false
    );
    assert!(!dir.join("NEVER_READ_KEY").exists());
}

#[test]
fn execution_is_single_use_even_at_the_same_timestamp() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let end = r.begin_execution(&f.manifest.run_id, 10).unwrap();
    assert_eq!(end, 910);
    assert!(r.begin_execution(&f.manifest.run_id, 10).is_err());
    assert!(r.require_unstarted(&f.manifest.run_id).is_err());
    let report = r.report(Some(&f.manifest.run_id), 10).unwrap();
    let starts = report["runtime_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "execution_started")
        .count();
    assert_eq!(
        starts, 1,
        "same-second resume must not duplicate the initial boundary"
    );
    assert!(r.begin_execution(&f.manifest.run_id, 20).is_err());
    assert!(r.begin_execution(&f.manifest.run_id, 19).is_err());
    assert!(r.begin_execution(&f.manifest.run_id, end).is_err());
    drop(r);
    let mut replacement = f.auth.clone();
    replacement.id = "new_authority".into();
    let mut r = Registry::authorize(&f.state, &replacement, 30).unwrap();
    assert!(r.begin_execution(&f.manifest.run_id, 30).is_err());
    assert!(
        r.reserve_batch(&f.manifest.run_id, "revoked", &["call".into()], 30)
            .is_err()
    );
    assert_eq!(reserved(&r), 0);
}

#[test]
fn funding_reservations_survive_reopen_and_authority_cannot_erase_them() {
    use x402_treazury::qualification::funding::{self, Limits, Request};
    let mut f = Fixture::new();
    f.auth.cumulative_source_zec = "0.001".into();
    f.auth.cumulative_new_jobs = 2;
    let mut registry = f.open();
    f.prepare(&mut registry);
    // Exercise the ledger independently of execution, which remains disabled.
    let mut db =
        rusqlite::Connection::open(f.state.join("live-integration/registry.sqlite")).unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    funding::reserve(
        &mut tx,
        "test_run",
        &[Request {
            intent: "bootstrap_a".into(),
            pool: "pool".into(),
            source_bound: 100,
        }],
        Limits {
            jobs: 2,
            source_zatoshis: 100_000,
        },
        Limits {
            jobs: 2,
            source_zatoshis: 100_000,
        },
        3,
    )
    .unwrap();
    tx.commit().unwrap();
    drop(db);
    drop(registry);
    let registry = Registry::open(&f.state, true).unwrap();
    let report = registry.report(None, 10).unwrap();
    assert_eq!(report["funding_jobs_reserved"], 1);
    assert_eq!(report["source_reserved_zatoshis"], 100);
    assert_eq!(report["funding_permits_available"], false);
    drop(registry);
    f.auth.id = "revoke".into();
    f.auth.cumulative_source_zec = "0".into();
    f.auth.cumulative_new_jobs = 0;
    assert!(Registry::authorize(&f.state, &f.auth, 11).is_err());
}

#[test]
fn managed_application_report_distinguishes_admission_from_settlement_and_refuses_duplicates() {
    let f = Fixture::new();
    let mut registry = f.open();
    f.prepare(&mut registry);
    registry
        .reserve_batch("test_run", "batch", &["call".into()], 10)
        .unwrap();
    registry.dispatching("test_run", "call").unwrap();
    registry.event("test_run", "application_claim", &json!({"case":"call","session":"session","server":"main","source":"api","tool":"api_read","mode":"managed","started_micros":0}), 10).unwrap();
    let event = json!({"case":"call","session":"session","stage":"admitted_before_signing","amount":"1","attempt_id":"attempt"});
    registry
        .event("test_run", "application_payment", &event, 10)
        .unwrap();
    let evidence = registry.application_evidence("test_run").unwrap();
    assert_eq!(evidence["payment_attempts_correlated"], 1);
    assert!(evidence["scope"].as_str().unwrap().contains("settlement"));
    registry
        .event("test_run", "application_payment", &event, 10)
        .unwrap();
    assert!(registry.application_evidence("test_run").is_err());
}

#[test]
fn managed_response_never_establishes_settlement_or_refunds_reservation() {
    for body in [None, Some(b"{\"result\":{\"isError\":false}}".as_slice())] {
        let f = Fixture::new();
        let mut r = f.open();
        f.prepare(&mut r);
        r.reserve_batch("test_run", "batch", &["call".into()], 10)
            .unwrap();
        r.dispatching("test_run", "call").unwrap();
        r.finish_managed("test_run", "call", body, true).unwrap();
        let report = r.report(Some("test_run"), 11).unwrap();
        assert_eq!(report["cases"][0]["settlement"], "PENDING");
        assert_eq!(
            report["cases"][0]["execution"],
            if body.is_some() {
                "COMPLETED"
            } else {
                "TRANSPORT_UNCERTAIN"
            }
        );
        assert_eq!(reserved(&r), 20000);
        assert!(r.finish_managed("test_run", "call", body, true).is_err());
        drop(r);
        let r = Registry::open(&f.state, false).unwrap();
        assert_eq!(
            r.report(Some("test_run"), 12).unwrap()["cases"][0]["settlement"],
            "PENDING"
        );
    }
}

#[test]
fn settlement_observation_requires_canonical_reason_or_drained_unsigned_evidence() {
    for (state, reason, closed, expected) in [
        ("RESOLVED", Some("USED"), false, "USED"),
        ("RESOLVED", Some("EXPIRED_UNUSED"), false, "EXPIRED_UNUSED"),
        ("RESOLVED", None, true, "PENDING"),
        ("POSSIBLY_SUBMITTED", None, true, "PENDING"),
        ("ADMITTED", None, false, "PENDING"),
        ("ADMITTED", None, true, "NOT_SIGNED"),
        ("MISSING", None, false, "PENDING"),
        ("MISSING", None, true, "NOT_SIGNED"),
    ] {
        let f = Fixture::new();
        let mut r = f.open();
        f.prepare(&mut r);
        r.reserve_batch("test_run", "batch", &["call".into()], 10)
            .unwrap();
        r.dispatching("test_run", "call").unwrap();
        r.event("test_run", "application_claim", &json!({"case":"call","session":"session","server":"main","source":"api","tool":"api_read","mode":"managed","started_micros":0}), 10).unwrap();
        r.event(
            "test_run",
            "application_finished",
            &json!({"case":"call","session":"session","is_error":false,"finished_micros":1}),
            10,
        )
        .unwrap();
        r.event("test_run", "application_payment", &json!({"case":"call","session":"session","stage":"admitted_before_signing","amount":"1","attempt_id":"attempt","pool":"p","wallet":"w","generation":0}), 10).unwrap();
        r.finish_managed("test_run", "call", Some(b"{}"), true)
            .unwrap();
        let mut snapshot = json!({"treasury_status":{"treasury_id":f.manifest.treasury_id},"payment_attempts":[],"payment_resolutions":[]});
        if state != "MISSING" {
            snapshot["payment_attempts"] = json!([{"id":"attempt","pool":"p","wallet":"w","generation":0,"amount":"1","state":state}]);
        }
        if let Some(reason) = reason {
            snapshot["payment_resolutions"] = json!([{"attempt_id":"attempt","outcome":reason,"height":12,"hash":"block","block_time":100}]);
            let mut wrong = snapshot.clone();
            wrong["payment_attempts"][0]["wallet"] = json!("different");
            assert!(r.apply_settlement("test_run", &wrong, closed).is_err());
        }
        r.apply_settlement("test_run", &snapshot, closed).unwrap();
        r.apply_settlement("test_run", &snapshot, closed).unwrap();
        let report = r.report(Some("test_run"), 12).unwrap();
        assert_eq!(
            report["cases"][0]["settlement"], expected,
            "{state}/{reason:?}/{closed}"
        );
        let events = report["runtime_events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "payment_observed")
            .count();
        assert_eq!(events, usize::from(expected != "PENDING"));
        assert_eq!(reserved(&r), 20000);
    }
}

#[test]
fn seller_receipts_are_correlated_but_never_promoted_to_chain_evidence() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    r.reserve_batch("test_run", "batch", &["call".into()], 10)
        .unwrap();
    r.dispatching("test_run", "call").unwrap();
    let payer = format!("0x{:040x}", 1);
    r.event("test_run","application_claim",&json!({"case":"call","session":"s","server":"main","source":"api","tool":"api_read","mode":"managed","started_micros":0}),10).unwrap();
    r.event("test_run","application_payment",&json!({"case":"call","session":"s","stage":"admitted_before_signing","amount":"1","attempt_id":"a","address":payer}),10).unwrap();
    let receipt = json!({"case":"call","session":"s","attempt_id":"a","receipt":{"classification":"seller_success","network":"eip155:8453","transaction":format!("0x{:064x}",2),"payer":payer}});
    r.event("test_run", "application_receipt", &receipt, 10)
        .unwrap();
    let evidence = r.application_evidence("test_run").unwrap();
    assert_eq!(evidence["seller_receipts"]["recorded"], 1);
    assert_eq!(
        evidence["seller_receipts"]["classifications"]["seller_success"],
        1
    );
    let report = r.report(Some("test_run"), 10).unwrap();
    assert_ne!(report["cases"][0]["settlement"], "USED");
    assert!(r.verified_debits("test_run").unwrap().is_empty());
    let proof = json!({"case":"call","session":"s","attempt_id":"a","status":"verified","proof":{
        "transaction":format!("0x{:064x}",2),"block_height":42,"block_hash":format!("0x{:064x}",4),
        "payer":payer,"payee":format!("0x{:040x}",5),"nonce":format!("0x{:064x}",6),"amount_atomic":"1"}});
    let before = r.report(Some("test_run"), 10).unwrap();
    let mut wrong = proof.clone();
    wrong["proof"]["amount_atomic"] = json!("2");
    assert!(
        r.record_receipt_observations("test_run", &[wrong], 10)
            .is_err()
    );
    assert_eq!(r.report(Some("test_run"), 10).unwrap(), before);
    r.record_receipt_observations("test_run", std::slice::from_ref(&proof), 10)
        .unwrap();
    let after = r.report(Some("test_run"), 10).unwrap();
    r.record_receipt_observations("test_run", std::slice::from_ref(&proof), 11)
        .unwrap();
    assert_eq!(r.report(Some("test_run"), 10).unwrap(), after);
    assert_eq!(reserved(&r), 20000);
    assert_eq!(before["cases"], after["cases"]);
    assert_eq!(r.verified_debits("test_run").unwrap().len(), 1);
    let db = rusqlite::Connection::open(f.state.join("live-integration/registry.sqlite")).unwrap();
    db.execute("UPDATE events SET detail=json_set(detail,'$.proof.amount_atomic','2') WHERE kind='application_debit_verified'",[]).unwrap();
    assert!(r.verified_debits("test_run").is_err());
    db.execute(
        "UPDATE events SET detail=?1 WHERE kind='application_debit_verified'",
        [proof.to_string()],
    )
    .unwrap();
    db.execute("UPDATE events SET detail=json_set(detail,'$.receipt.payer',?1) WHERE kind='application_receipt'",
        [format!("0x{:040x}",3)]).unwrap();
    assert!(r.application_evidence("test_run").is_err());
    db.execute(
        "UPDATE events SET detail=?1 WHERE kind='application_receipt'",
        [receipt.to_string()],
    )
    .unwrap();
    r.event("test_run", "application_receipt", &receipt, 10)
        .unwrap();
    assert!(r.application_evidence("test_run").is_err());
}

fn rotation_fixture() -> Fixture {
    let mut f = Fixture::new();
    for id in ["extra", "service"] {
        let mut c = f.manifest.cases[0].clone();
        c.id = id.into();
        f.manifest.cases.push(c);
        f.manifest.phases[0].cases.push(id.into());
    }
    f.manifest.limits.api_reservation_usdc = "0.06".into();
    f.manifest.limits.new_funding_jobs = 1;
    f.manifest.limits.source_exposure_zec = "0.1".into();
    f.auth.cumulative_api_usdc = "0.06".into();
    f.auth.cumulative_new_jobs = 1;
    f.auth.cumulative_source_zec = "0.1".into();
    f.manifest.phases[0].scenario = crate::manifest::Scenario::Rotation {
        refill_slots: 1,
        rounds: vec![crate::manifest::RotationRound {
            pool: "pool".into(),
            expected_price_usdc: "0.014".into(),
            depletion_cases: vec!["call".into(), "extra".into()],
            service_cases: vec!["service".into()],
        }],
    };
    f
}
fn rotation_states(treasury: &str) -> (Value, Value) {
    let wallet = |id, n, role| json!({"id":id,"address":format!("0x{n:040x}"),"role":role,"target":"2000000"});
    let before = json!({"treasury_id":treasury,"pools":[{"id":"p","name":"pool","generation":0,"addresses":[wallet("a",1,"ACTIVE"),wallet("b",2,"READY")]}],"funding_jobs":[]});
    let after = json!({"treasury_id":treasury,"pools":[{"id":"p","name":"pool","generation":1,"addresses":[wallet("a",1,"RETIRED"),wallet("b",2,"ACTIVE"),wallet("c",3,"ALLOCATED")]}],"funding_jobs":[{"id":"job","pool_id":"p","wallet_id":"c","recipient":format!("0x{:040x}",3),"target":"2000000","phase":"ALLOCATED"}]});
    (before, after)
}
#[test]
fn rotation_boundary_durably_skips_only_unused_tail_without_refunding_attempts() {
    let f = rotation_fixture();
    let mut r = f.open();
    f.prepare(&mut r);
    let run = &f.manifest.run_id;
    let (before, after) = rotation_states(&f.manifest.treasury_id);
    assert!(
        r.stop_depletion(run, "smoke", 0, &before, &after, 10)
            .is_err()
    );
    r.reserve_batch(run, "first", &["call".into()], 10).unwrap();
    assert!(
        r.stop_depletion(run, "smoke", 0, &before, &after, 10)
            .is_err()
    );
    r.dispatching(run, "call").unwrap();
    assert!(
        r.stop_depletion(run, "smoke", 0, &before, &after, 10)
            .is_err()
    );
    r.finish_managed(run, "call", Some(b"{}"), true).unwrap();
    let mut bad = after.clone();
    bad["pools"][0]["generation"] = json!(2);
    assert!(
        r.stop_depletion(run, "smoke", 0, &before, &bad, 10)
            .is_err()
    );
    assert_eq!(r.execution_state(run, "extra").unwrap(), "UNATTEMPTED");
    assert_eq!(
        r.stop_depletion(run, "smoke", 0, &before, &after, 10)
            .unwrap(),
        vec!["extra"]
    );
    assert_eq!(reserved(&r), 20000);
    drop(r);
    let mut r = Registry::open(&f.state, false).unwrap();
    assert!(r.validated_skips(run).unwrap().contains("extra"));
    assert!(
        r.stop_depletion(run, "smoke", 0, &before, &after, 11)
            .is_err()
    );
    assert!(
        r.reserve_batch(run, "replay", &["extra".into()], 11)
            .is_err()
    );
    assert!(r.dispatching(run, "extra").is_err());
    let report = r.report(Some(run), 11).unwrap();
    let skipped = report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["case"] == "extra")
        .unwrap();
    assert_eq!(skipped["reservation_atomic"], 20000);
    assert_eq!(skipped["charged_reservation_atomic"], 0);
    assert_eq!(skipped["semantic"], "NOT_EXECUTED");
    r.reserve_batch(run, "service_batch", &["service".into()], 11)
        .unwrap();
    assert_eq!(reserved(&r), 40000);
    r.event(run, "application_claim", &json!({"case":"extra"}), 12)
        .unwrap();
    assert!(r.validated_skips(run).is_err());
    assert!(r.report(None, 12).is_err());
}
#[test]
fn rotation_boundary_refuses_inflight_tail_and_premature_service_atomically() {
    for id in ["extra", "service"] {
        let f = rotation_fixture();
        let mut r = f.open();
        f.prepare(&mut r);
        let run = &f.manifest.run_id;
        r.reserve_batch(run, "first", &["call".into()], 10).unwrap();
        r.dispatching(run, "call").unwrap();
        r.finish_managed(run, "call", None, false).unwrap();
        r.reserve_batch(run, "next", &[id.into()], 10).unwrap();
        let (before, after) = rotation_states(&f.manifest.treasury_id);
        assert!(
            r.stop_depletion(run, "smoke", 0, &before, &after, 10)
                .is_err()
        );
        assert!(r.validated_skips(run).unwrap().is_empty());
        assert_eq!(reserved(&r), 40000);
        assert_eq!(r.execution_state(run, id).unwrap(), "RESERVED");
    }
}

#[test]
fn rotation_boundaries_require_new_jobs_and_consecutive_generations() {
    let mut f = rotation_fixture();
    let mut second = match &f.manifest.phases[0].scenario {
        crate::manifest::Scenario::Rotation { rounds, .. } => rounds[0].clone(),
        _ => unreachable!(),
    };
    second.depletion_cases = vec!["second".into()];
    second.service_cases = vec!["second_service".into()];
    for id in ["second", "second_service"] {
        let mut c = f.manifest.cases[0].clone();
        c.id = id.into();
        f.manifest.cases.push(c);
        f.manifest.phases[0].cases.push(id.into());
    }
    let first = match &f.manifest.phases[0].scenario {
        crate::manifest::Scenario::Rotation { rounds, .. } => rounds[0].clone(),
        _ => unreachable!(),
    };
    f.manifest.phases[0].scenario = crate::manifest::Scenario::Lifecycle {
        refill_slots: 2,
        restart: crate::manifest::Restart::QueuedRefill,
        rounds: vec![first, second],
    };
    f.auth.cumulative_api_usdc = "0.1".into();
    f.auth.cumulative_new_jobs = 2;
    f.manifest.limits.api_reservation_usdc = "0.1".into();
    f.manifest.limits.new_funding_jobs = 2;
    let mut r = f.open();
    f.prepare(&mut r);
    let run = &f.manifest.run_id;
    let (before, after) = rotation_states(&f.manifest.treasury_id);
    let finish = |r: &mut Registry, id: &str| {
        r.reserve_batch(run, id, &[id.into()], 10).unwrap();
        r.dispatching(run, id).unwrap();
        r.finish_managed(run, id, Some(b"{}"), true).unwrap();
    };
    finish(&mut r, "call");
    r.stop_depletion(run, "smoke", 0, &before, &after, 10)
        .unwrap();
    finish(&mut r, "service");
    finish(&mut r, "second");
    let mut reused = after.clone();
    reused["funding_jobs"][0]["id"] = json!("different_job");
    assert!(
        r.stop_depletion(run, "smoke", 1, &before, &reused, 10)
            .is_err()
    );
    let mut ready = after;
    ready["pools"][0]["addresses"][2]["role"] = json!("READY");
    ready["funding_jobs"][0]["phase"] = json!("COMPLETE");
    let mut rotated = ready.clone();
    rotated["pools"][0]["generation"] = json!(2);
    rotated["pools"][0]["addresses"][1]["role"] = json!("RETIRED");
    rotated["pools"][0]["addresses"][2]["role"] = json!("ACTIVE");
    let address = format!("0x{:040x}", 4);
    rotated["pools"][0]["addresses"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"d","address":address,"role":"ALLOCATED","target":"2000000"}));
    rotated["funding_jobs"].as_array_mut().unwrap().push(json!({"id":"second_job","pool_id":"p","wallet_id":"d","recipient":address,"target":"2000000","phase":"ALLOCATED"}));
    assert!(
        r.stop_depletion(run, "smoke", 1, &ready, &rotated, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(r.validated_skips(run).unwrap().len(), 1);
    assert_eq!(reserved(&r), 60000);
}

#[test]
fn pool_observation_requests_are_single_pending_and_responses_are_correlated() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let run = &f.manifest.run_id;
    let binding = x402_treazury::qualification::Binding {
        version: 1,
        run: run.clone(),
        session: "session".into(),
        registry: f.state.join("live-integration/registry.sqlite"),
        pin_digest: "fixture".into(),
        config_sha256: "fixture".into(),
        expires_at: 100,
    };
    r.event(
        run,
        "application_session",
        &serde_json::to_value(binding).unwrap(),
        3,
    )
    .unwrap();
    assert!(
        r.request_pool_observation(run, "p", "unselected", 10)
            .is_err()
    );
    assert!(r.request_pool_observation(run, "p", "pool", 100).is_err());
    let request = r.request_pool_observation(run, "p", "pool", 10).unwrap();
    assert!(r.pool_observation(run, &request).unwrap().is_none());
    assert!(r.request_pool_observation(run, "p", "pool", 11).is_err());
    let response = json!({"request":request,"pool":"p","pool_name":"pool","session":"session",
        "query_started_micros":10,"observed_micros":20,"state":{"treasury_id":f.manifest.treasury_id}});
    r.event(run, "application_pool_observation", &response, 12)
        .unwrap();
    assert_eq!(
        r.pool_observation(run, &request).unwrap().unwrap(),
        response
    );
    let next = r.request_pool_observation(run, "p", "pool", 13).unwrap();
    assert_ne!(next, request);
    assert!(r.pool_observation(run, &next).unwrap().is_none());
    let mut wrong = response.clone();
    wrong["request"] = json!(next);
    wrong["session"] = json!("stale_session");
    r.event(run, "application_pool_observation", &wrong, 14)
        .unwrap();
    assert!(r.pool_observation(run, &next).is_err());
    r.event(run, "application_pool_observation", &response, 15)
        .unwrap();
    assert!(r.pool_observation(run, &request).is_err());
}

#[test]
fn accounting_capture_requires_clean_session_and_exclusive_owner_and_survives_reopen() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let run = &f.manifest.run_id;
    assert!(r.capture_accounting(run, "session", 10).is_err());
    r.event(run, "application_session", &json!({"session":"session"}), 3)
        .unwrap();
    assert!(r.capture_accounting(run, "session", 10).is_err());
    r.event(
        run,
        "child_finished",
        &json!({"session":"session","success":true,"forced_kill":false,"valid_output":true}),
        8,
    )
    .unwrap();
    assert!(r.capture_accounting(run, "session", 7).is_err());
    let owner = files::lock(&f.state.join("owner.lock")).unwrap();
    assert!(r.capture_accounting(run, "session", 10).is_err());
    drop(owner);
    r.capture_accounting(run, "session", 10).unwrap();
    assert!(r.capture_accounting(run, "session", 11).is_err());
    drop(r);
    let r = Registry::open(&f.state, true).unwrap();
    let accounting = r.report(Some(run), 12).unwrap()["treasury_accounting"].clone();
    assert_eq!(accounting["status"], "recorded");
    assert_eq!(
        accounting["observations"][0]["baseline_comparison"]["scope"],
        "registry_authorization_to_snapshot"
    );
    assert_eq!(
        accounting["observations"][0]["run_source_accounting"]["scope"],
        "run_funding_permits"
    );
    assert_eq!(
        accounting["observations"][0]["run_source_accounting"]["operation_count"],
        0
    );
    assert_eq!(accounting["observations"].as_array().unwrap().len(), 1);
    assert_eq!(
        accounting["observations"][0]["summary"]["fresh_chain_read"],
        false
    );
}

#[test]
fn source_attribution_is_bound_to_run_and_frozen_at_capture() {
    let mut f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let run = f.manifest.run_id.clone();
    f.manifest.run_id = "other_run".into();
    f.prepare(&mut r);
    let db = rusqlite::Connection::open(f.state.join("live-integration/registry.sqlite")).unwrap();
    for (id, owner) in [("mine", run.as_str()), ("theirs", "other_run")] {
        db.execute("INSERT INTO funding_permits(intent,run,pool,source_bound,job,at) VALUES(?1,?2,'pool',110,?1,3)", rusqlite::params![id,owner]).unwrap();
        db.execute(
            "INSERT INTO funding_source_attempts(operation,job,source_bound) VALUES(?1,?1,110)",
            [id],
        )
        .unwrap();
    }
    r.event(&run, "application_session", &json!({"session":"s"}), 3)
        .unwrap();
    r.event(
        &run,
        "child_finished",
        &json!({"session":"s","success":true,"forced_kill":false,"valid_output":true}),
        4,
    )
    .unwrap();
    r.capture_accounting(&run, "s", 5).unwrap();
    db.execute(
        "UPDATE funding_source_attempts SET released=1 WHERE operation='mine'",
        [],
    )
    .unwrap();
    let report = r.report(Some(&run), 6).unwrap();
    let attributed = &report["treasury_accounting"]["observations"][0]["run_source_accounting"];
    assert_eq!(attributed["operation_count"], 1);
    assert_eq!(attributed["registry_unreleased_bounds_zatoshis"], 110);
    assert_eq!(attributed["attempts_without_treasury_budget"], 1);
    db.execute("UPDATE events SET detail=json_remove(detail,'$.source_attempts') WHERE kind='accounting_observed'",[]).unwrap();
    assert_eq!(
        r.report(Some(&run), 6).unwrap()["treasury_accounting"]["observations"][0]["run_source_accounting"]
            ["status"],
        "unobserved"
    );
}

#[test]
fn accounting_report_rejects_changed_authorization_baseline() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let db = rusqlite::Connection::open(f.state.join("live-integration/registry.sqlite")).unwrap();
    db.execute("UPDATE identity SET baseline='{}'", []).unwrap();
    let error = r.report(Some(&f.manifest.run_id), 5).unwrap_err();
    assert!(error.to_string().contains("baseline hash mismatch"));
}
