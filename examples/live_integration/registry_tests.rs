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
fn revisions_preserve_baseline_and_refuse_attempted_contract_changes() {
    let mut f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let report = r.report(None, 10).unwrap();
    let old = report["pin_revisions"][0]["digest"].as_str().unwrap();
    let mut pins = f.pins.clone();
    pins.binary_sha256 = files::hash(b"new");
    assert!(r.revise("test_run", &pins, "bad", 10).is_err());
    r.revise("test_run", &pins, old, 10).unwrap();
    let next = r.report(None, 10).unwrap()["pin_revisions"][1]["digest"]
        .as_str()
        .unwrap()
        .to_owned();
    pins.resolved_config["resolved_wallets"]["pool"]["max_price_usd"] = json!("0.04");
    assert!(r.revise("test_run", &pins, &next, 10).is_err());
    r.reserve_batch("test_run", "one", &["call".into()], 10)
        .unwrap();
    assert!(r.revise("test_run", &f.pins, &next, 10).is_err());
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
async fn commands_prepare_and_resume_without_launching_binary_or_fetching_specs() {
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
daily_input_zec="0.1"
shield_max_fee_zec="0.001"
[funding]
confidentiality="public"
[wallets.pool]
mode="zcash_rotation"
max_price_usd="0.02"
max_input_zec="0.02"
max_fee_bps=500
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
    f.manifest.windows[0].not_before = now;
    f.manifest.windows[0].not_after = now + 3600;
    f.manifest.binary = dir.join("not-an-executable");
    std::fs::write(&f.manifest.binary, b"this must only be hashed").unwrap();
    f.manifest.deployment = dir.join("deployment.toml");
    std::fs::write(&f.manifest.deployment, deployment).unwrap();
    let auth = dir.join("authorization.toml");
    std::fs::write(&auth, toml::to_string(&f.auth).unwrap()).unwrap();
    let manifest = dir.join("manifest.toml");
    std::fs::write(&manifest, toml::to_string(&f.manifest).unwrap()).unwrap();
    for (verb, extra, code) in [
        (
            "authorize-registry",
            vec!["--authorization", auth.to_str().unwrap()],
            0,
        ),
        ("prepare", vec!["--manifest", manifest.to_str().unwrap()], 0),
        ("status", vec![], 0),
        ("resume", vec!["--run", "test_run"], 3),
    ] {
        let mut args = vec![
            "live_integration",
            verb,
            "--state-dir",
            f.state.to_str().unwrap(),
        ];
        args.extend(extra);
        let command = crate::Args::try_parse_from(args).unwrap().command;
        assert_eq!(crate::execute(command).await.unwrap(), code);
    }
    let r = Registry::open(&f.state, true).unwrap();
    assert_eq!(reserved(&r), 0);
    assert_eq!(
        r.report(None, now as i64).unwrap()["execution_available"],
        false
    );
    assert!(!dir.join("NEVER_READ_KEY").exists());
}
