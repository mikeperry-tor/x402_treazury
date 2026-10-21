fn expected_inspection(
    eligible: &[&str],
    waiting: &[&str],
    expired: &[&str],
    observed: usize,
    deadline: Option<u64>,
) -> Value {
    json!({"eligible":eligible,"waiting_window":waiting,"expired_window":expired,
        "observe_only_count":observed,"original_deadline":deadline,
        "scope":"offline eligibility only; unchanged provenance, authority, clean prior processes, production state and supervised network qualification must still pass before dispatch"})
}
#[test]
fn inspection_inventory_and_ordered_evidence_errors_are_exact() {
    let (m, r) = input();
    for (cases, error) in [
        (Value::Null, "resume cases missing"),
        (json!([{}]), "resume case ID missing"),
        (
            json!([{"case":"call"},{"case":"call"}]),
            "duplicate resume case",
        ),
        (json!([]), "resume case inventory differs from manifest"),
        (
            json!([{"case":"call"},{"case":"extra"}]),
            "resume case inventory differs from manifest",
        ),
    ] {
        let mut bad = r.clone();
        bad["cases"] = cases;
        bad["runtime_events"] = Value::Null;
        assert_eq!(inspect(&m, &bad, 90000).unwrap_err().to_string(), error);
    }
    let mut bad = r.clone();
    bad["runtime_events"] = Value::Null;
    bad["canonical_api_debits"] = Value::Null;
    assert_eq!(
        inspect(&m, &bad, 90000).unwrap_err().to_string(),
        "resume runtime events missing"
    );
    for ids in [vec![], vec!["call", "call"], vec!["other"]] {
        let mut m = m.clone();
        m.phases[0].cases = ids.into_iter().map(str::to_owned).collect();
        let mut bad = r.clone();
        bad["canonical_api_debits"] = Value::Null;
        assert_eq!(
            inspect(&m, &bad, 90000).unwrap_err().to_string(),
            "resume phase case inventory differs from manifest"
        );
    }
    for (proofs, error) in [
        (
            json!({"status":"invalid_or_incomplete","cases":[]}),
            "resume canonical accounting is invalid or incomplete",
        ),
        (
            json!({"status":"validated","cases":[]}),
            "resume debit proofs missing",
        ),
        (
            json!({"status":"validated","cases":{"other":null}}),
            "resume debit names unknown case",
        ),
    ] {
        let mut bad = r.clone();
        bad["canonical_api_debits"] = proofs;
        bad["cases"][0]["execution"] = Value::Null;
        assert_eq!(inspect(&m, &bad, 90000).unwrap_err().to_string(), error);
    }
    let mut m = m;
    let mut other = m.cases[0].clone();
    other.id = "other".into();
    m.cases.push(other);
    m.phases[0].cases.push("other".into());
    let mut bad = r;
    bad["cases"] =
        json!([{"case":"other","execution":null},{"case":"call","execution":"RESERVED"}]);
    assert_eq!(
        inspect(&m, &bad, 90000).unwrap_err().to_string(),
        "prior dispatch remains unresolved; observation/reconciliation only"
    );
    m.cases.reverse();
    assert_eq!(
        inspect(&m, &bad, 90000).unwrap_err().to_string(),
        "resume execution missing"
    );
}
#[test]
fn inspection_event_timestamps_and_original_deadline_preserve_precedence() {
    let (mut m, r) = input();
    for (events, error) in [
        (
            json!([{"kind":"unrelated"}]),
            "resume event timestamp missing",
        ),
        (
            json!([{"kind":"unrelated","at":-1}]),
            "resume event timestamp missing",
        ),
        (
            json!([{"kind":"unrelated","at":90001}]),
            "wall clock moved backwards; resume refused",
        ),
        (
            json!([{"kind":"execution_started","at":1},{"kind":"execution_started","at":2}]),
            "duplicate initial execution timestamp",
        ),
        (
            json!([{"kind":"execution_started","at":1},{"kind":"execution_started","at":90001}]),
            "wall clock moved backwards; resume refused",
        ),
        (
            json!([{"kind":"execution_started","at":1},{"kind":"execution_started"}]),
            "resume event timestamp missing",
        ),
    ] {
        let mut bad = r.clone();
        bad["runtime_events"] = events;
        bad["canonical_api_debits"] = Value::Null;
        assert_eq!(inspect(&m, &bad, 90000).unwrap_err().to_string(), error);
    }
    let mut no_start = r.clone();
    no_start["runtime_events"] = json!([{"kind":"other","at":90000},{"at":2}]);
    assert_eq!(
        inspect(&m, &no_start, 90000).unwrap(),
        expected_inspection(&["call"], &[], &[], 0, None)
    );
    no_start["runtime_events"] = json!([]);
    assert_eq!(
        inspect(&m, &no_start, 90000).unwrap(),
        expected_inspection(&["call"], &[], &[], 0, None)
    );
    let mut unordered = r.clone();
    unordered["runtime_events"] =
        json!([{"kind":"other","at":90000},{"kind":"execution_started","at":1}]);
    assert_eq!(
        inspect(&m, &unordered, 90000).unwrap(),
        expected_inspection(&["call"], &[], &[], 0, Some(200001))
    );
    m.limits.run_seconds = 90000;
    assert_eq!(
        inspect(&m, &r, 90000).unwrap(),
        expected_inspection(&["call"], &[], &[], 0, Some(90001))
    );
    assert_eq!(
        inspect(&m, &r, 90001).unwrap(),
        expected_inspection(&[], &[], &["call"], 0, Some(90001))
    );
    m.limits.run_seconds = u64::MAX;
    m.phases[0].cases.clear();
    assert_eq!(
        inspect(&m, &r, 90000).unwrap_err().to_string(),
        "resume deadline overflow"
    );
}
#[test]
fn inspection_prior_execution_settlement_and_proof_matrix_preserves_observation_only() {
    let (m, r) = input();
    let unused = "unused payment has a debit proof";
    let missing = "used authorization lacks canonical debit proof";
    let unresolved = "prior payment remains unresolved; observation/reconciliation only";
    // Proof content is deliberately opaque here; upstream report verification owns it.
    for state in ["COMPLETED", "TRANSPORT_UNCERTAIN"] {
        for (settlement, proof, unsigned, error) in [
            ("NOT_SIGNED", false, false, None),
            ("EXPIRED_UNUSED", false, false, None),
            ("NOT_SIGNED", false, true, None),
            ("EXPIRED_UNUSED", false, true, None),
            ("NOT_SIGNED", true, false, Some(unused)),
            ("EXPIRED_UNUSED", true, false, Some(unused)),
            ("USED", true, false, None),
            ("USED", false, false, Some(missing)),
            ("USED", true, true, Some(missing)),
            ("USED", false, true, Some(missing)),
            ("PENDING", false, false, Some(unresolved)),
            ("UNOBSERVED", false, false, Some(unresolved)),
            ("UNKNOWN", false, false, Some(unresolved)),
        ] {
            let mut m = m.clone();
            m.cases[0].unsigned = unsigned;
            let mut r = r.clone();
            r["cases"][0]["execution"] = json!(state);
            r["cases"][0]["settlement"] = json!(settlement);
            if proof {
                r["canonical_api_debits"]["cases"]["call"] = Value::Null;
            }
            match error {
                Some(error) => assert_eq!(
                    inspect(&m, &r, 90000).unwrap_err().to_string(),
                    error,
                    "{state} {settlement} {proof} {unsigned}"
                ),
                None => assert_eq!(
                    inspect(&m, &r, 90000).unwrap(),
                    expected_inspection(&[], &[], &[], 1, Some(200001))
                ),
            }
        }
        let mut bad = r.clone();
        bad["cases"][0]["execution"] = json!(state);
        bad["cases"][0]["settlement"] = Value::Null;
        assert_eq!(
            inspect(&m, &bad, 200001).unwrap_err().to_string(),
            unresolved
        );
    }
    for (state, error) in [
        (
            json!("RESERVED"),
            "prior dispatch remains unresolved; observation/reconciliation only",
        ),
        (
            json!("DISPATCHING"),
            "prior dispatch remains unresolved; observation/reconciliation only",
        ),
        (json!("RESPONSE_SAVED"), "unknown resume execution state"),
        (json!("UNKNOWN"), "unknown resume execution state"),
        (Value::Null, "resume execution missing"),
        (json!(false), "resume execution missing"),
    ] {
        let mut bad = r.clone();
        bad["cases"][0]["execution"] = state;
        // An expired run still validates every prior financial state.
        assert_eq!(inspect(&m, &bad, 200001).unwrap_err().to_string(), error);
    }
    for (settlement, proof) in [("NOT_SIGNED", false), ("UNOBSERVED", true)] {
        let mut bad = r.clone();
        bad["cases"][0]["settlement"] = json!(settlement);
        if proof {
            bad["canonical_api_debits"]["cases"]["call"] = Value::Null;
        }
        assert_eq!(
            inspect(&m, &bad, 90000).unwrap_err().to_string(),
            "untouched case has payment evidence"
        );
    }
    let mut skipped = r;
    skipped["cases"][0] = json!({"case":"call","execution":"SKIPPED_TARGET_REACHED"});
    skipped["canonical_api_debits"]["cases"]["call"] = json!({"opaque":"upstream-owned"});
    assert_eq!(
        inspect(&m, &skipped, 90000).unwrap(),
        expected_inspection(&[], &[], &[], 1, Some(200001))
    );
}
#[test]
fn inspection_application_evidence_is_required_only_after_observed_work() {
    let (mut m, r) = input();
    for evidence in [
        Value::Null,
        json!({"payment_attempts_correlated":"0","without_completion":[]}),
        json!({"payment_attempts_correlated":0,"without_completion":null}),
        json!({"payment_attempts_correlated":0,"without_completion":["call"]}),
    ] {
        let mut report = r.clone();
        report["application_evidence"] = evidence;
        assert_eq!(
            inspect(&m, &report, 90000).unwrap(),
            expected_inspection(&["call"], &[], &[], 0, Some(200001))
        );
        report["cases"][0]["execution"] = json!("COMPLETED");
        report["cases"][0]["settlement"] = json!("NOT_SIGNED");
        assert_eq!(
            inspect(&m, &report, 90000).unwrap_err().to_string(),
            "prior application identity/completion evidence is invalid or incomplete"
        );
        m.windows.clear();
        assert_eq!(
            inspect(&m, &report, 90000).unwrap_err().to_string(),
            "prior application identity/completion evidence is invalid or incomplete"
        );
        m = input().0;
    }
    for count in [0, u64::MAX] {
        let mut report = r.clone();
        report["cases"][0]["execution"] = json!("COMPLETED");
        report["cases"][0]["settlement"] = json!("NOT_SIGNED");
        report["application_evidence"]["payment_attempts_correlated"] = json!(count);
        assert_eq!(
            inspect(&m, &report, 90000).unwrap(),
            expected_inspection(&[], &[], &[], 1, Some(200001))
        );
    }
}
#[test]
fn inspection_mixed_windows_preserve_phase_and_case_order_in_complete_output() {
    let (mut m, mut r) = input();
    let original_case = m.cases[0].clone();
    let original_phase = m.phases[0].clone();
    let original_window = m.windows[0].clone();
    m.cases.clear();
    m.phases.clear();
    m.windows.clear();
    r["cases"] = json!([]);
    for (name, start, end, ids, observed) in [
        ("open", 100, 200, vec!["z_open", "a_open"], false),
        ("waiting", 101, 200, vec!["z_wait", "a_wait"], false),
        ("closed", 50, 100, vec!["closed"], false),
        ("observed", 100, 200, vec!["observed"], true),
        ("second_open", 100, 200, vec!["middle_open"], false),
    ] {
        let mut phase = original_phase.clone();
        phase.id = name.into();
        phase.window = name.into();
        phase.cases = ids.iter().map(|id| (*id).into()).collect();
        m.phases.push(phase);
        let mut window = original_window.clone();
        window.id = name.into();
        window.not_before = start;
        window.not_after = end;
        m.windows.push(window);
        for id in ids {
            let mut case = original_case.clone();
            case.id = id.into();
            m.cases.push(case);
            r["cases"].as_array_mut().unwrap().push(json!({"case":id,"execution":if observed{"COMPLETED"}else{"UNATTEMPTED"},"settlement":if observed{"NOT_SIGNED"}else{"UNOBSERVED"}}));
        }
    }
    m.cases.reverse();
    r["cases"].as_array_mut().unwrap().reverse();
    assert_eq!(
        inspect(&m, &r, 100).unwrap(),
        expected_inspection(
            &["z_open", "a_open", "middle_open"],
            &["z_wait", "a_wait"],
            &["closed"],
            1,
            Some(200001)
        )
    );
    assert_eq!(
        inspect(&m, &r, 101).unwrap(),
        expected_inspection(
            &["z_open", "a_open", "z_wait", "a_wait", "middle_open"],
            &[],
            &["closed"],
            1,
            Some(200001)
        )
    );
    m.limits.run_seconds = 99;
    assert_eq!(
        inspect(&m, &r, 100).unwrap(),
        expected_inspection(
            &[],
            &[],
            &[
                "z_open",
                "a_open",
                "z_wait",
                "a_wait",
                "closed",
                "middle_open"
            ],
            1,
            Some(100)
        )
    );
}
#[test]
fn inspection_partial_lifecycle_refusal_precedes_even_expired_window_classification() {
    let (mut m, mut r) = input();
    let mut later = m.cases[0].clone();
    later.id = "later".into();
    m.cases.push(later);
    m.phases[0].cases.push("later".into());
    r["cases"]
        .as_array_mut()
        .unwrap()
        .push(json!({"case":"later","execution":"UNATTEMPTED","settlement":"UNOBSERVED"}));
    for scenario in [
        Scenario::Rotation {
            refill_slots: 1,
            rounds: vec![],
        },
        Scenario::RefillService {
            refill_slots: 1,
            rounds: vec![],
        },
        Scenario::Lifecycle {
            refill_slots: 1,
            restart: crate::manifest::Restart::QueuedRefill,
            rounds: vec![],
        },
    ] {
        // Scenario validation belongs upstream; this isolates inspect's partial-phase rule.
        let mut m = m.clone();
        m.phases[0].scenario = scenario;
        assert_eq!(
            inspect(&m, &r, 90000).unwrap(),
            expected_inspection(&["call", "later"], &[], &[], 0, Some(200001))
        );
        let mut partial = r.clone();
        partial["cases"][0]["execution"] = json!("COMPLETED");
        partial["cases"][0]["settlement"] = json!("NOT_SIGNED");
        for now in [90000, 200001] {
            assert_eq!(
                inspect(&m, &partial, now).unwrap_err().to_string(),
                "partially dispatched rotation/lifecycle remains observation-only"
            );
        }
        partial["cases"][1]["execution"] = json!("COMPLETED");
        partial["cases"][1]["settlement"] = json!("NOT_SIGNED");
        assert_eq!(
            inspect(&m, &partial, 90000).unwrap(),
            expected_inspection(&[], &[], &[], 2, Some(200001))
        );
        partial["cases"][1]["execution"] = json!("UNATTEMPTED");
        partial["cases"][1]["settlement"] = json!("UNOBSERVED");
        m.windows.clear();
        assert_eq!(
            inspect(&m, &partial, 90000).unwrap_err().to_string(),
            "resume phase window missing"
        );
    }
}
