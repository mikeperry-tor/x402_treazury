fn planner_error(m: &Manifest, cfg: &Value) -> String {
    planner::build(m, cfg)
        .err()
        .expect("invalid fixture accepted")
        .to_string()
}
fn planner_add_case(m: &mut Manifest, id: &str) {
    let mut case = m.cases[0].clone();
    case.id = id.into();
    m.cases.push(case);
    m.phases[0].cases.push(id.into());
}
fn planner_scenario(kind: &str) -> (Manifest, Value, u64, u32) {
    let mut m = manifest();
    let mut cfg = config();
    let mut jobs = 0;
    match kind {
        "smoke" => (),
        "provider_sweep" => m.phases[0].scenario = Scenario::ProviderSweep {},
        "concurrency" => {
            planner_add_case(&mut m, "second");
            m.phases[0].scenario = Scenario::Concurrency {
                batches: vec![m.phases[0].cases.clone()],
            };
        }
        "rotation" | "refill_service" | "lifecycle" => {
            planner_add_case(&mut m, "second");
            jobs = if kind == "lifecycle" { 2 } else { 1 };
            let mut rounds = vec![round("call", "second")];
            if kind == "lifecycle" {
                planner_add_case(&mut m, "third");
                planner_add_case(&mut m, "fourth");
                rounds.push(round("third", "fourth"));
            }
            m.phases[0].scenario = match kind {
                "rotation" => Scenario::Rotation {
                    refill_slots: jobs,
                    rounds,
                },
                "refill_service" => Scenario::RefillService {
                    refill_slots: jobs,
                    rounds,
                },
                _ => Scenario::Lifecycle {
                    refill_slots: jobs,
                    rounds,
                    restart: Restart::SubmittedDeposit,
                },
            };
        }
        "reliability" => {
            m.limits.run_seconds = 172800;
            m.phases[0].scenario = Scenario::Reliability {
                min_spacing_seconds: 60,
                later_utc_day: true,
            };
            planner_add_case(&mut m, "second");
            m.phases[0].cases.pop();
            let mut phase = m.phases[0].clone();
            phase.id = "later".into();
            phase.window = "later".into();
            phase.cases = vec!["second".into()];
            phase.depends_on = vec!["smoke".into()];
            m.phases.push(phase);
            m.windows.push(Window {
                id: "later".into(),
                not_before: 86400,
                not_after: 87000,
            });
        }
        "unsigned" | "tor_outage" => {
            m.start.pools.clear();
            m.phases[0].pools.clear();
            m.phases[0].scenario = Scenario::Unsigned {};
            m.cases[0].unsigned = true;
            m.cases[0].reserve_usdc = "0".into();
            cfg["resolved_wallets"] = json!({"pool":{"mode":"static"}});
            // Neither treasury identity nor confidentiality is needed without selected pools.
            cfg["treasury"] = Value::Null;
            cfg["funding"] = Value::Null;
            if kind == "tor_outage" {
                planner_add_case(&mut m, "cached");
                planner_add_case(&mut m, "fresh");
                m.phases[0].cases = vec!["call".into()];
                let mut phase = m.phases[0].clone();
                phase.id = "outage".into();
                phase.depends_on = vec!["smoke".into()];
                phase.cases = vec!["cached".into(), "fresh".into()];
                phase.scenario = Scenario::TorOutage {
                    warm_case: "call".into(),
                    cached_case: "cached".into(),
                    uncached_case: "fresh".into(),
                };
                m.phases.push(phase);
                m.network.tor_mode = TorMode::Owned;
                m.network.tor_binary = Some("synthetic-tor".into());
                cfg["network"] = json!({"mode":"tor"});
            }
        }
        _ => panic!("unknown fixture"),
    }
    let total = if m.cases[0].unsigned {
        0
    } else {
        m.cases.len() as u64 * 20000
    };
    m.limits.api_reservation_usdc = if total > 40000 { "0.1" } else { "0.04" }.into();
    m.limits.new_funding_jobs = jobs;
    if jobs > 0 {
        m.limits.source_exposure_zec = "0.1".into();
    }
    (m, cfg, total, jobs)
}
fn planner_expected(
    m: &Manifest,
    cfg: &Value,
    total: u64,
    ceiling: u64,
    jobs: u32,
    allowance: u32,
) -> Value {
    let unsigned = m.cases[0].unsigned;
    let bindings:serde_json::Map<String,Value>=m.cases.iter().map(|c|(c.id.clone(),json!({"server":c.server,"source":c.source,"tool":c.tool,"wallet":"pool","unsigned":unsigned,"reservation_atomic":if unsigned {0}else{20000},"catalog_validation":"pending_prepare"}))).collect();
    let phases:Vec<_>=m.phases.iter().map(|p|json!({"id":p.id,"scenario":p.scenario,"depends_on":p.depends_on,"window":m.windows.iter().find(|w|w.id==p.window).unwrap(),"cases":p.cases,"pools":p.pools,"required":p.required,"call_count":p.cases.len()})).collect();
    json!({"version":1,"run_id":m.run_id,"offline":true,"execution_authorized":false,
        "api_reservation_atomic":total,"api_headroom_atomic":ceiling-total,
        "planned_new_funding_jobs":jobs,"funding_job_headroom":allowance-jobs,
        "source_exposure_limit_zatoshis":if allowance==0 {0}else{10000000},
        "funding_feasibility":"unverified: no live balance, quote, fee or baseline reads",
        "prerequisites":[
            "prepare must pin binaries/config/catalogs and validate selected tool schemas and arguments",
            "registry authorization and immutable treasury baseline required before execution",
            "reviewed startup inventory required; treasury-only bootstrap waits for active and configured reserves, while every payment needs eligible single-wallet capacity after liabilities and fresh canonical evidence",
            "source fees and floating swap minimum must fit funding permits; offline targets are not quotes",
            "all declared pools must be preserved; unused pools receive no new allocation authority"],
        "case_bindings":bindings,"expanded_phases":phases,"declared_managed_pools":if unsigned {json!([])}else{json!(["pool","unused"])},
        "network_disclosures":{"policy":cfg["network"],"base_rpc_policy":cfg["base_rpc_policy"],"tor":m.network,"external_requests_performed":0,"provider_origins":"resolve during prepare; no catalog downloads during plan"},"manifest":m})
}
#[test]
fn planner_complete_scenario_and_start_mode_contracts() {
    for kind in [
        "smoke",
        "unsigned",
        "provider_sweep",
        "concurrency",
        "rotation",
        "refill_service",
        "lifecycle",
        "reliability",
        "tor_outage",
    ] {
        for treasury_only in [false, true] {
            let (mut m, mut cfg, total, refills) = planner_scenario(kind);
            if treasury_only {
                m.start.mode = StartMode::TreasuryOnly;
            }
            if treasury_only && m.cases[0].unsigned {
                assert_eq!(
                    planner_error(&m, &cfg),
                    "unsigned suite must not initialize or select managed pools"
                );
                continue;
            }
            let jobs = refills + if treasury_only { 2 } else { 0 };
            let allowance = if jobs > 0 { jobs + 1 } else { 0 };
            m.limits.new_funding_jobs = allowance;
            if allowance > 0 {
                m.limits.source_exposure_zec = "0.1".into();
            }
            if kind == "provider_sweep" {
                m.network.tor_mode = TorMode::External;
                cfg["network"] = json!({"mode":"tor","annotation":"kept"});
                cfg["base_rpc_policy"] = json!({"scope":"fixture"});
            }
            let ceiling = if total > 40000 { 100000 } else { 40000 };
            assert_eq!(
                serde_json::to_value(planner::build(&m, &cfg).unwrap()).unwrap(),
                planner_expected(&m, &cfg, total, ceiling, jobs, allowance),
                "{kind} treasury={treasury_only}"
            );
        }
    }
}
#[test]
fn planner_deployment_validation_preserves_exact_first_failures() {
    let base = manifest();
    let cfg = config();
    let cases = [
        (
            "/servers",
            json!({"main":{"auth":false}}),
            "live qualification requires authenticated listeners",
        ),
        ("/resolved_wallets", Value::Null, "resolved wallets missing"),
        (
            "/resolved_wallets/pool/mode",
            json!("static"),
            "unknown managed pool pool",
        ),
        (
            "/treasury/id",
            json!("different"),
            "treasury UUID does not match deployment",
        ),
        (
            "/funding/confidentiality",
            json!("private"),
            "initial runner supports public swaps only",
        ),
        (
            "/network/mode",
            json!("tor"),
            "manifest/deployment network modes differ",
        ),
    ];
    for (pointer, value, error) in cases {
        let mut c = cfg.clone();
        if pointer == "/servers" {
            c["servers"] = value;
        } else {
            *c.pointer_mut(pointer).unwrap() = value;
        }
        c["wallet_bindings"] = Value::Null;
        assert_eq!(planner_error(&base, &c), error);
    }
    let mut invalid = base.clone();
    invalid.version = 2;
    assert_eq!(
        planner_error(&invalid, &Value::Null),
        "unsupported version or invalid run/authorization identifier"
    );
    let m = base.clone();
    let c = json!({"servers":{"main":{"auth":false}}});
    assert_eq!(
        planner_error(&m, &c),
        "live qualification requires authenticated listeners"
    );
    for auth in [Value::Null, json!("false"), json!(0), json!(true)] {
        let mut c = cfg.clone();
        c["servers"] = json!({"main":{"auth":auth}});
        assert_eq!(
            serde_json::to_value(planner::build(&base, &c).unwrap()).unwrap(),
            planner_expected(&base, &c, 20000, 40000, 0, 0)
        );
    }
}
#[test]
fn planner_case_bindings_cap_fallback_and_reservation_boundaries() {
    let m = manifest();
    let mut cfg = config();
    cfg["resolved_wallets"]["pool"]
        .as_object_mut()
        .unwrap()
        .remove("max_api_payment_usdc");
    cfg["resolved_wallets"]["pool"]["max_price_usd"] = json!("0.02");
    let mut equality = m.clone();
    equality.limits.api_reservation_usdc = "0.02".into();
    assert_eq!(
        serde_json::to_value(planner::build(&equality, &cfg).unwrap()).unwrap(),
        planner_expected(&equality, &cfg, 20000, 20000, 0, 0)
    );
    cfg["resolved_wallets"]["pool"]["max_api_payment_usdc"] = Value::Null;
    assert_eq!(planner_error(&m, &cfg), "wallet cap missing");
    cfg["resolved_wallets"]["pool"]["max_api_payment_usdc"] = json!("0.03");
    assert_eq!(
        planner_error(&m, &cfg),
        "case call reservation is below effective wallet cap"
    );
    let mut c = config();
    c["wallet_bindings"] = Value::Null;
    assert_eq!(
        planner_error(&m, &c),
        "case call: unknown source/listener binding"
    );
    c = config();
    c["wallet_bindings"]["main"]["api"]["wallet"] = json!("unused");
    assert_eq!(
        planner_error(&m, &c),
        "case call must use a selected managed pool"
    );
    let (unsigned, mut c, _, _) = planner_scenario("unsigned");
    c["resolved_wallets"] = json!({"pool":{"mode":"unrecognized","max_api_payment_usdc":null}});
    // Unsigned bindings bypass paid profile-mode and cap validation.
    assert_eq!(
        serde_json::to_value(planner::build(&unsigned, &c).unwrap()).unwrap(),
        planner_expected(&unsigned, &c, 0, 40000, 0, 0)
    );
}
#[test]
fn planner_phase_state_and_scenario_errors_keep_order() {
    let cfg = config();
    for (mut m, error) in {
        let mut rows = Vec::new();
        let mut m = manifest();
        m.phases[0].depends_on = vec!["later".into(), "later".into()];
        m.phases[0].cases.clear();
        rows.push((m, "invalid/duplicate dependency: later"));
        let mut m = manifest();
        m.phases[0].depends_on = vec!["later".into()];
        m.phases[0].pools.clear();
        rows.push((
            m,
            "phase smoke dependency later must precede it (no cycles/forward dependencies)",
        ));
        let mut m = manifest();
        m.phases[0].pools.push("pool".into());
        m.phases[0].cases.clear();
        rows.push((m, "invalid/duplicate phase pool: pool"));
        let mut m = manifest();
        m.phases[0].cases.clear();
        m.phases[0].pools = vec!["absent".into()];
        rows.push((m, "phase smoke has no cases"));
        let mut m = manifest();
        m.phases[0].pools = vec!["unused".into()];
        m.phases[0].window = "missing".into();
        rows.push((m, "phase smoke references unselected pool unused"));
        let mut m = manifest();
        m.phases[0].window = "missing".into();
        m.phases[0].cases = vec!["unknown".into()];
        rows.push((m, "phase has unknown window"));
        let mut m = manifest();
        m.phases[0].cases = vec!["unknown".into()];
        rows.push((m, "unknown phase case unknown"));
        let mut m = manifest();
        m.phases[0].cases.push("call".into());
        rows.push((
            m,
            "case call appears more than once; expand distinct case IDs explicitly",
        ));
        let mut m = manifest();
        m.phases[0].pools.clear();
        rows.push((m, "case call wallet not in phase pools"));
        let mut m = manifest();
        m.phases[0].scenario = Scenario::Unsigned {};
        rows.push((
            m,
            "unsigned phase cannot contain paid cases or funding pools",
        ));
        rows
    } {
        // A later invalid allowance must not mask the phase error.
        m.limits.new_funding_jobs = 1;
        m.limits.source_exposure_zec = "0.1".into();
        assert_eq!(planner_error(&m, &cfg), error);
    }
    let (base, cfg, _, _) = planner_scenario("concurrency");
    for (batches, error) in [
        (vec![], "concurrency requires explicit batches"),
        (
            vec![vec!["call".into()]],
            "batch size exceeds concurrency or is below two",
        ),
        (
            vec![vec!["call".into(), "unknown".into()]],
            "batch contains unknown or repeated case unknown",
        ),
        (
            vec![vec!["call".into(), "call".into()]],
            "batch contains unknown or repeated case call",
        ),
    ] {
        let mut m = base.clone();
        m.phases[0].scenario = Scenario::Concurrency { batches };
        assert_eq!(planner_error(&m, &cfg), error);
    }
    let (mut m, cfg, _, _) = planner_scenario("rotation");
    m.phases[0].scenario = Scenario::Rotation {
        refill_slots: 2,
        rounds: vec![],
    };
    assert_eq!(
        planner_error(&m, &cfg),
        "rotation/refill requires paid cases, pools and exactly one refill slot"
    );
    let (mut m, cfg, _, _) = planner_scenario("lifecycle");
    m.phases[0].scenario = Scenario::Lifecycle {
        refill_slots: 1,
        rounds: vec![],
        restart: Restart::QueuedRefill,
    };
    assert_eq!(
        planner_error(&m, &cfg),
        "lifecycle requires paid cases, pools and exactly two refill slots"
    );
}
#[test]
fn planner_allocation_final_guards_and_sum_overflow_remain_distinct() {
    let cfg = config();
    let mut m = manifest();
    m.limits.api_reservation_usdc = "0.019999".into();
    m.phases[0].cases.clear();
    assert_eq!(
        planner_error(&m, &cfg),
        "expanded API reservations exceed run ceiling"
    );
    let mut m = manifest();
    planner_add_case(&mut m, "second");
    planner_add_case(&mut m, "third");
    for c in &mut m.cases {
        c.reserve_usdc = "9223372036854.775807".into();
    }
    m.limits.api_reservation_usdc = "9223372036854.775807".into();
    assert_eq!(planner_error(&m, &cfg), "reservation sum overflow");
    let mut m = manifest();
    planner_add_case(&mut m, "unused");
    m.phases[0].cases.pop();
    m.limits.new_funding_jobs = 1;
    m.limits.source_exposure_zec = "0.1".into();
    assert_eq!(
        planner_error(&m, &cfg),
        "unused cases: every reviewed case must appear in exactly one phase"
    );
    m.phases[0].scenario = Scenario::Reliability {
        min_spacing_seconds: 60,
        later_utc_day: false,
    };
    assert_eq!(
        planner_error(&m, &cfg),
        "reliability requires at least two identical server/source/tool/arguments/mode/assertion observations"
    );
    let mut m = manifest();
    m.limits.new_funding_jobs = 1;
    m.limits.source_exposure_zec = "0.1".into();
    assert_eq!(
        planner_error(&m, &cfg),
        "funding allowance without a planned allocation is forbidden"
    );
    m.start.mode = StartMode::TreasuryOnly;
    assert_eq!(
        planner_error(&m, &cfg),
        "insufficient funding slots for bootstrap/refills"
    );
    m.limits.new_funding_jobs = 2;
    assert_eq!(
        serde_json::to_value(planner::build(&m, &cfg).unwrap()).unwrap(),
        planner_expected(&m, &cfg, 20000, 40000, 2, 2)
    );
}

#[test]
fn planner_scenario_specific_guards_preserve_ordered_rejections() {
    let (outage, cfg, _, _) = planner_scenario("tor_outage");
    let mut c = cfg.clone();
    c["resolved_wallets"]["unselected"] = json!({"mode":"zcash_rotation"});
    let mut m = outage.clone();
    m.network.tor_mode = TorMode::External;
    m.network.tor_binary = None;
    assert_eq!(
        planner_error(&m, &c),
        "Tor outage requires a separate keyless deployment with no managed wallets or automatic funding"
    );
    assert_eq!(
        planner_error(&m, &cfg),
        "Tor outage requires owned Tor and unsigned cases"
    );
    let mut m = outage.clone();
    m.phases[1].cases.reverse();
    m.phases[1].depends_on.clear();
    assert_eq!(
        planner_error(&m, &cfg),
        "Tor outage requires ordered cached/uncached cases exactly once"
    );
    let mut m = outage.clone();
    m.phases[1].depends_on.clear();
    assert_eq!(
        planner_error(&m, &cfg),
        "outage requires an earlier warm phase as an explicit dependency"
    );
    let mut m = outage.clone();
    if let Scenario::TorOutage { warm_case, .. } = &mut m.phases[1].scenario {
        *warm_case = "unknown".into();
    }
    assert_eq!(planner_error(&m, &cfg), "outage warm case missing");
    let mut m = outage.clone();
    let mut later = m.phases[0].clone();
    later.id = "later".into();
    m.phases.push(later);
    assert_eq!(
        planner_error(&m, &cfg),
        "Tor outage must be the final phase"
    );
    let (mut m, mut cfg, _, _) = planner_scenario("lifecycle");
    m.start.pools.push("unused".into());
    m.phases[0].pools.push("unused".into());
    cfg["wallet_bindings"]["main"]["peer"] = json!({"wallet":"unused"});
    m.cases[2].source = "peer".into();
    m.cases[3].source = "peer".into();
    if let Scenario::Lifecycle { rounds, .. } = &mut m.phases[0].scenario {
        rounds[1].pool = "unused".into();
    }
    assert_eq!(
        planner_error(&m, &cfg),
        "lifecycle must rotate the same selected pool twice"
    );
    let (mut m, cfg, _, _) = planner_scenario("concurrency");
    planner_add_case(&mut m, "third");
    m.limits.api_reservation_usdc = "0.06".into();
    assert_eq!(
        planner_error(&m, &cfg),
        "batches must cover phase cases exactly once"
    );
    m.phases[0].scenario = Scenario::Smoke {};
    assert_eq!(
        planner_error(&m, &cfg),
        "smoke requires one paid case and pool"
    );
}
