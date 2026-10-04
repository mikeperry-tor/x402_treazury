use super::{manifest::*, planner};
use serde_json::{Value, json};
fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "version":1,"run_id":"test_run","treasury_id":"11111111-1111-4111-8111-111111111111",
        "deployment":"deploy.toml","binary":"treazury","evidence_dir":"evidence","registry_authorization":"review_1",
        "start":{"mode":"funded_pools","pools":["pool"]},
        "network":{"tor_mode":"direct","confinement":"none","require_isolation_evidence":false},
        "limits":{"api_reservation_usdc":"0.04","new_funding_jobs":0,"source_exposure_zec":"0","max_in_flight":2,"run_seconds":900,"phase_seconds":600,"call_seconds":300,"cleanup_seconds":900,"result_bytes":1000},
        "catalog":{"execution":"frozen","record_live_discovery":true},
        "windows":[{"id":"now","not_before":1,"not_after":1000}],
        "cases":[{"id":"call","server":"main","source":"api","tool":"api_read","arguments":{},"reserve_usdc":"0.02","reviewed_read_only":true}],
        "phases":[{"id":"smoke","window":"now","cases":["call"],"pools":["pool"],"required":true,"scenario":{"kind":"smoke"}}]
    })).unwrap()
}
fn config() -> Value {
    json!({"resolved_wallets":{"pool":{"mode":"zcash_rotation","max_price_usd":"0.02"},"unused":{"mode":"zcash_rotation","max_price_usd":"0.02"}},"wallet_bindings":{"main":{"api":{"wallet":"pool"}}},"treasury":{"id":"11111111-1111-4111-8111-111111111111"},"funding":{"confidentiality":"public"},"network":{"mode":"direct"}})
}
#[test]
fn zero_amounts_and_exact_decimal_boundaries() {
    for zero in ["0", "0.0", "00.00000000"] {
        assert_eq!(zec(zero).unwrap(), 0);
    }
    for invalid in ["", ".", "-0", "0.000000000", "0e0", "NaN"] {
        assert!(zec(invalid).is_err(), "{invalid}");
    }
    assert_eq!(zec("0.00000001").unwrap(), 1);
    assert_eq!(usdc("0.000001").unwrap(), 1);
    for invalid in ["off", "none", "", "0.0000001", "9223372036854775808"] {
        assert!(usdc(invalid).is_err());
    }
}
#[test]
fn smoke_has_no_execution_authority_or_hidden_allocation() {
    let p = planner::build(&manifest(), &config()).unwrap();
    assert!(p.offline && !p.execution_authorized);
    assert_eq!(p.api_reservation_atomic, 20000);
    assert_eq!(p.api_headroom_atomic, 20000);
    assert_eq!(p.planned_new_funding_jobs, 0);
    assert_eq!(p.declared_managed_pools, vec!["pool", "unused"]);
    assert_eq!(p.expanded_phases[0]["call_count"], 1);
}
#[test]
fn strict_variants_and_read_only_review() {
    let base = serde_json::to_value(manifest()).unwrap();
    for (pointer, value) in [
        ("/unexpected", json!(true)),
        ("/phases/0/scenario/refill_slots", json!(1)),
        ("/limits/call_seconds", json!(0)),
        ("/cases/0/reviewed_read_only", json!(false)),
    ] {
        let mut v = base.clone();
        if pointer == "/unexpected" {
            v["unexpected"] = value;
        } else if pointer.ends_with("refill_slots") {
            v["phases"][0]["scenario"]["refill_slots"] = value;
        } else {
            *v.pointer_mut(pointer).unwrap() = value;
        }
        let r = serde_json::from_value::<Manifest>(v);
        assert!(r.is_err() || r.unwrap().validate().is_err(), "{pointer}");
    }
}
#[test]
fn cases_dependencies_budgets_and_bindings_fail_closed() {
    for mode in 0..8 {
        let mut m = manifest();
        match mode {
            0 => m.cases[0].reserve_usdc = "0.01".into(),
            1 => m.limits.api_reservation_usdc = "0.01".into(),
            2 => m.phases[0].depends_on = vec!["smoke".into()],
            3 => m.phases[0].cases.push("call".into()),
            4 => m.cases[0].source = "missing".into(),
            5 => {
                let mut c = m.cases[0].clone();
                c.id = "unused".into();
                m.cases.push(c);
            }
            6 => m.phases[0].pools = vec!["unused".into()],
            _ => {
                m.limits.new_funding_jobs = 1;
                m.limits.source_exposure_zec = "0.01".into();
            }
        }
        assert!(planner::build(&m, &config()).is_err(), "mode {mode}");
    }
}
#[test]
fn explicit_concurrency_and_bootstrap_refill_slots() {
    let mut m = manifest();
    let mut second = m.cases[0].clone();
    second.id = "second".into();
    m.cases.push(second);
    m.phases[0].cases.push("second".into());
    m.phases[0].scenario = Scenario::Concurrency {
        batches: vec![vec!["call".into(), "second".into()]],
    };
    assert_eq!(
        planner::build(&m, &config())
            .unwrap()
            .api_reservation_atomic,
        40000
    );
    m.limits.max_in_flight = 1;
    assert!(planner::build(&m, &config()).is_err());
    m.phases[0].scenario = Scenario::Lifecycle {
        refill_slots: 2,
        restart: Restart::QueuedRefill,
    };
    m.start.mode = StartMode::TreasuryOnly;
    m.limits.new_funding_jobs = 3;
    m.limits.source_exposure_zec = "0.1".into();
    assert!(planner::build(&m, &config()).is_err());
    m.limits.new_funding_jobs = 4;
    assert_eq!(
        planner::build(&m, &config())
            .unwrap()
            .planned_new_funding_jobs,
        4
    );
}
#[test]
fn reliability_windows_do_not_reset_budget_or_reuse_case_ids() {
    let mut m = manifest();
    m.phases[0].scenario = Scenario::Reliability {
        min_spacing_seconds: 60,
        later_utc_day: true,
    };
    let mut c = m.cases[0].clone();
    c.id = "tomorrow_call".into();
    m.cases.push(c);
    let mut p = m.phases[0].clone();
    p.id = "tomorrow".into();
    p.window = "tomorrow".into();
    p.cases = vec!["tomorrow_call".into()];
    p.depends_on = vec!["smoke".into()];
    m.phases.push(p);
    m.windows.push(Window {
        id: "tomorrow".into(),
        not_before: 86400,
        not_after: 87000,
    });
    assert_eq!(
        planner::build(&m, &config())
            .unwrap()
            .api_reservation_atomic,
        40000
    );
    m.windows[1].not_before = 1060;
    m.windows[1].not_after = 2000;
    assert!(planner::build(&m, &config()).is_err());
}
#[tokio::test]
async fn offline_planner_resolves_production_config_without_catalog_keys_or_wallet_state() {
    let dir = tempfile::tempdir().unwrap();
    let config = r#"version=1
[treasury]
id="11111111-1111-4111-8111-111111111111"
state_dir="absent_state"
key_file="absent_key"
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
"#;
    std::fs::write(dir.path().join("deploy.toml"), config).unwrap();
    let m = manifest();
    std::fs::write(dir.path().join("run.toml"), toml::to_string(&m).unwrap()).unwrap();
    let loaded = Manifest::load(&dir.path().join("run.toml")).await.unwrap();
    assert!(loaded.binary.is_absolute());
    assert!(planner::plan(&loaded).await.unwrap().offline);
    assert!(!dir.path().join("absent_state").exists());
    assert!(!dir.path().join("evidence").exists());
}
