use super::{manifest::*, planner};
use serde_json::{Value, json};
pub(super) fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "version":1,"run_id":"test_run","treasury_id":"11111111-1111-4111-8111-111111111111",
        "deployment":"deploy.toml","binary":"x402_treazury","evidence_dir":"evidence","registry_authorization":"review_1",
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
    json!({"resolved_wallets":{"pool":{"mode":"zcash_rotation","max_api_payment_usdc":"0.02"},"unused":{"mode":"zcash_rotation","max_api_payment_usdc":"0.02"}},"wallet_bindings":{"main":{"api":{"wallet":"pool"}}},"treasury":{"id":"11111111-1111-4111-8111-111111111111"},"funding":{"confidentiality":"public"},"network":{"mode":"direct"}})
}
#[test]
fn qualification_rejects_explicitly_unauthenticated_listeners() {
    let mut config = config();
    config["servers"] = json!({"main":{"auth":false}});
    assert!(planner::build(&manifest(), &config).err().unwrap().to_string().contains("authenticated listeners"));
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
    for id in ["third", "fourth"] {
        let mut case = m.cases[0].clone();
        case.id = id.into();
        m.cases.push(case);
        m.phases[0].cases.push(id.into());
    }
    m.limits.api_reservation_usdc = "0.08".into();
    m.phases[0].scenario = Scenario::Lifecycle {
        refill_slots: 2,
        restart: Restart::QueuedRefill,
        rounds: vec![round("call", "second"), round("third", "fourth")],
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
    m.limits.run_seconds = 172800;
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

#[path = "registry_tests.rs"]
mod registry_tests;

#[test]
fn outage_plan_requires_owned_tor_final_phase_and_warm_dependency() {
    let mut m = manifest();
    m.start.pools.clear();
    m.cases[0].unsigned = true;
    m.cases[0].reserve_usdc = "0".into();
    m.phases[0].pools.clear();
    m.phases[0].scenario = Scenario::Unsigned {};
    for id in ["cached", "fresh"] {
        let mut c = m.cases[0].clone();
        c.id = id.into();
        m.cases.push(c);
    }
    let mut phase = m.phases[0].clone();
    phase.id = "outage".into();
    phase.cases = vec!["cached".into(), "fresh".into()];
    phase.depends_on = vec!["smoke".into()];
    phase.scenario = Scenario::TorOutage {
        warm_case: "call".into(),
        cached_case: "cached".into(),
        uncached_case: "fresh".into(),
    };
    m.phases.push(phase);
    m.network.tor_mode = TorMode::Owned;
    m.network.tor_binary = Some("/tor".into());
    let mut cfg = config();
    cfg["network"]["mode"] = json!("tor");
    let error = planner::build(&m, &cfg).err().unwrap();
    assert!(error.to_string().contains("no managed wallets"));
    cfg["resolved_wallets"] = json!({"pool":{"mode":"static","max_api_payment_usdc":"0.02"}});
    planner::build(&m, &cfg).unwrap();
    let mut bad_config = cfg.clone();
    bad_config["resolved_wallets"]["unused"] =
        json!({"mode":"zcash_rotation","max_api_payment_usdc":"0.02"});
    assert!(planner::build(&m, &bad_config).is_err());
    let mut bad_config = cfg.clone();
    bad_config["funding"]["auto_fund"] = json!(true);
    assert!(planner::build(&m, &bad_config).is_err());
    let mut managed = m.clone();
    managed.start.pools.push("pool".into());
    let mut managed_config = cfg.clone();
    managed_config["resolved_wallets"]["pool"]["mode"] = json!("zcash_rotation");
    assert!(planner::build(&managed, &managed_config).is_err());
    let mut bad = m.clone();
    bad.phases[1].depends_on.clear();
    assert!(planner::build(&bad, &cfg).is_err());
    let mut bad = m.clone();
    bad.phases.swap(0, 1);
    assert!(planner::build(&bad, &cfg).is_err());
    let mut bad = m.clone();
    bad.network.tor_mode = TorMode::External;
    bad.network.tor_binary = None;
    assert!(planner::build(&bad, &cfg).is_err());
    let mut bad = m.clone();
    bad.phases[1].cases.reverse();
    assert!(planner::build(&bad, &cfg).is_err());
}

#[test]
fn settling_interval_is_explicit_and_bounded_by_phase() {
    let mut m = manifest();
    assert_eq!(m.limits.post_batch_wait_ms, 0);
    m.limits.post_batch_wait_ms = 60_000;
    m.validate().unwrap();
    m.limits.post_batch_wait_ms = 60_001;
    assert!(m.validate().is_err());
    m.limits.post_batch_wait_ms = 2_000;
    m.limits.phase_seconds = 1;
    m.limits.call_seconds = 1;
    assert!(m.validate().is_err());
}

#[test]
fn paid_success_requires_actual_debit_but_unsigned_help_does_not() {
    let mut m = manifest();
    let proof = std::collections::BTreeMap::from([("call".into(), json!({}))]);
    let empty = std::collections::BTreeMap::new();
    for (settlement, semantic, with_proof, expected) in [
        ("USED", "PASSED", false, false),
        ("USED", "PASSED", true, true),
        ("NOT_SIGNED", "PASSED", false, false),
        ("EXPIRED_UNUSED", "PASSED", false, false),
        ("NOT_SIGNED", "FAILED", false, true),
        ("EXPIRED_UNUSED", "FAILED", false, true),
    ] {
        let case = json!({"case":"call","settlement":settlement,"semantic":semantic});
        assert_eq!(
            super::execution::debit_complete(&case, &m, if with_proof { &proof } else { &empty }),
            expected
        );
    }
    m.cases[0].unsigned = true;
    assert!(super::execution::debit_complete(
        &json!({"case":"call","settlement":"NOT_SIGNED","semantic":"PASSED"}),
        &m,
        &empty
    ));
}

#[test]
fn funding_launch_requires_explicit_flag_and_zero_authority_never_enables_it() {
    let mut m = manifest();
    let mut config: x402_treazury::deployment::MetaConfig =
        toml::from_str(include_str!("../deployments/public-swap-demo.toml")).unwrap();
    config.funding.as_mut().unwrap().auto_fund = true;
    assert!(!super::execution::funding_enabled(&m, &config, true).unwrap());
    m.limits.new_funding_jobs = 2;
    assert!(super::execution::funding_enabled(&m, &config, false).is_err());
    assert!(super::execution::funding_enabled(&m, &config, true).unwrap());
    config.funding.as_mut().unwrap().auto_fund = false;
    assert!(super::execution::funding_enabled(&m, &config, true).is_err());
    config.funding.as_mut().unwrap().auto_fund = true;
    m.start.pools.clear();
    assert!(super::execution::funding_enabled(&m, &config, true).is_err());
}

#[test]
fn payment_wait_observes_successful_peers_without_waiting_for_undispatched_work() {
    let m = manifest();
    let mut report = json!({"cases":[
        {"case":"call","execution":"COMPLETED","semantic":"PASSED","settlement":"PENDING"},
        {"case":"other","execution":"COMPLETED","semantic":"FAILED","settlement":"PENDING"}],
        "runtime_events":[{"kind":"application_payment","detail":{"case":"call"}}]});
    let empty = std::collections::BTreeMap::new();
    let waiting = |r: &Value, phase, proofs| {
        super::execution::bootstrap::waiting_count(r, &m, phase, proofs).unwrap()
    };
    assert_eq!(waiting(&report, None, &empty), 1);
    let mut dependent = m.phases[0].clone();
    dependent.depends_on = vec!["smoke".into()];
    assert_eq!(waiting(&report, Some(&dependent), &empty), 1);
    report["cases"][0]["settlement"] = json!("USED");
    assert_eq!(waiting(&report, None, &empty), 1);
    let verified = std::collections::BTreeMap::from([("call".into(), json!({}))]);
    assert_eq!(waiting(&report, None, &verified), 0);
    for terminal in ["EXPIRED_UNUSED", "NOT_SIGNED"] {
        report["cases"][0]["settlement"] = json!(terminal);
        assert_eq!(waiting(&report, None, &empty), 0);
        assert_eq!(waiting(&report, Some(&dependent), &empty), 0);
        assert!(!super::execution::debit_complete(&report["cases"][0], &m, &empty));
    }
    // Missing evidence for an actually consumed authorization still needs observation.
    report["cases"][0]["settlement"] = json!("USED");
    assert_eq!(waiting(&report, None, &empty), 1);
    report["runtime_events"] = json!([]);
    assert_eq!(waiting(&report, None, &empty), 0);
    report["cases"][0]["execution"] = json!("UNATTEMPTED");
    assert_eq!(waiting(&report, Some(&dependent), &empty), 0);
}

fn round(depletion: &str, service: &str) -> RotationRound {
    RotationRound {
        pool: "pool".into(),
        expected_price_usdc: "0.014".into(),
        depletion_cases: vec![depletion.into()],
        service_cases: vec![service.into()],
    }
}

#[test]
fn rotation_rounds_require_distinct_ordered_paid_calls_and_exact_wallet_scope() {
    let mut m = manifest();
    for id in ["deplete_more", "service", "peer_service"] {
        let mut c = m.cases[0].clone();
        c.id = id.into();
        m.cases.push(c);
        m.phases[0].cases.push(id.into());
    }
    m.start.pools.push("unused".into());
    m.phases[0].pools.push("unused".into());
    m.cases[3].source = "peer".into();
    let mut cfg = config();
    cfg["wallet_bindings"]["main"]["peer"] = json!({"wallet":"unused"});
    m.limits.api_reservation_usdc = "0.08".into();
    m.limits.new_funding_jobs = 1;
    m.limits.source_exposure_zec = "0.1".into();
    let mut r = round("call", "service");
    r.depletion_cases.push("deplete_more".into());
    r.service_cases.push("peer_service".into());
    m.phases[0].scenario = Scenario::Rotation {
        refill_slots: 1,
        rounds: vec![r],
    };
    planner::build(&m, &cfg).unwrap();
    let mut high_reservation = m.clone();
    high_reservation.limits.api_reservation_usdc = "0.4".into();
    for case in &mut high_reservation.cases {
        case.reserve_usdc = "0.1".into();
    }
    if let Scenario::Rotation { rounds, .. } = &mut high_reservation.phases[0].scenario {
        rounds[0].expected_price_usdc = "0.03".into();
    }
    let error = planner::build(&high_reservation, &cfg).err().unwrap();
    assert!(error.to_string().contains("effective wallet cap"));
    let base = serde_json::to_value(&m).unwrap();
    for (pointer, value) in [
        ("/phases/0/scenario/rounds", json!([])),
        ("/phases/0/scenario/rounds/0/pool", json!("absent")),
        (
            "/phases/0/scenario/rounds/0/expected_price_usdc",
            json!("0"),
        ),
        (
            "/phases/0/scenario/rounds/0/expected_price_usdc",
            json!("0.03"),
        ),
        ("/phases/0/scenario/rounds/0/depletion_cases", json!([])),
        ("/phases/0/scenario/rounds/0/service_cases", json!([])),
        (
            "/phases/0/scenario/rounds/0/service_cases",
            json!(["call", "peer_service"]),
        ),
        (
            "/phases/0/scenario/rounds/0/service_cases",
            json!(["peer_service", "service"]),
        ),
        ("/cases/1/arguments", json!({"different":true})),
        ("/cases/1/source", json!("peer")),
        ("/cases/1/unsigned", json!(true)),
    ] {
        let mut v = base.clone();
        *v.pointer_mut(pointer).unwrap() = value;
        let bad: Manifest = serde_json::from_value(v).unwrap();
        assert!(planner::build(&bad, &cfg).is_err(), "{pointer}");
    }
    let mut missing = base.clone();
    missing["phases"][0]["scenario"]
        .as_object_mut()
        .unwrap()
        .remove("rounds");
    assert!(serde_json::from_value::<Manifest>(missing).is_err());
    let mut unknown = base;
    unknown["phases"][0]["scenario"]["rounds"][0]["repeat_forever"] = json!(true);
    assert!(serde_json::from_value::<Manifest>(unknown).is_err());
}
