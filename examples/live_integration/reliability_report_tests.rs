const RELIABILITY_SCOPE: &str = "Repeated identical end-to-end cases using driver dispatch-intent wall-clock timestamps, not packet-arrival timing. Failure is not automatically attributed to the provider or Tor; no future reliability guarantee.";

fn expected_sample(m: &Manifest, index: usize, at: Value, status: &str, category: &str) -> Value {
    json!({"case":m.cases[index].id,"phase":m.phases[index].id,"window":m.windows[index].id,
        "dispatch_at":at,"status":status,"category":category,"mcp":"PASSED",
        "provider_semantics":"passed","settlement":"NOT_SIGNED"})
}

#[test]
fn report_complete_output_empty_path_and_group_order() {
    let (mut m, mut evidence) = fixture();
    let expected = json!([{"status":"passed","planned":2,"passed":2,"failed":0,"incomplete":0,
        "samples":[expected_sample(&m,0,json!(20),"passed","observed"),expected_sample(&m,1,json!(86420),"passed","observed")],"scope":RELIABILITY_SCOPE}]);
    assert_eq!(report(&m, &evidence).unwrap(), expected);
    assert_eq!(
        qualification(&json!({"reliability":expected})).unwrap(),
        (true, true)
    );
    let mut earlier = m.cases.clone();
    for case in &mut earlier {
        case.id = format!("early_{}", case.id);
        case.tool = "aaa".into();
    }
    for (index, case) in earlier.iter().enumerate() {
        m.phases[index].cases.push(case.id.clone());
        let mut row = evidence["cases"][index].clone();
        row["case"] = json!(case.id);
        evidence["cases"].as_array_mut().unwrap().push(row);
        evidence["provider_semantics"]
            .as_array_mut()
            .unwrap()
            .push(json!({"case":case.id,"status":"passed"}));
        let mut event = evidence["runtime_events"][index].clone();
        event["detail"]["case"] = json!(case.id);
        evidence["runtime_events"]
            .as_array_mut()
            .unwrap()
            .push(event);
    }
    m.cases.extend(earlier);
    let mut first = expected[0].clone();
    for sample in first["samples"].as_array_mut().unwrap() {
        sample["case"] = json!(format!("early_{}", sample["case"].as_str().unwrap()));
    }
    assert_eq!(report(&m, &evidence).unwrap(), json!([first, expected[0]]));
    evidence["provider_semantics"][2]["status"] = json!("failed");
    first["status"] = json!("failed");
    first["passed"] = json!(1);
    first["failed"] = json!(1);
    first["samples"][0]["status"] = json!("failed");
    first["samples"][0]["provider_semantics"] = json!("failed");
    let mixed = report(&m, &evidence).unwrap();
    assert_eq!(mixed, json!([first, expected[0]]));
    assert_eq!(
        qualification(&json!({"reliability":mixed})).unwrap(),
        (true, false)
    );
    m.phases.clear();
    assert_eq!(report(&m, &Value::Null).unwrap(), json!([]));
    assert_eq!(
        qualification(&json!({"reliability":[]})).unwrap(),
        (true, true)
    );
}

#[test]
fn report_evidence_errors_preserve_validation_and_index_order() {
    let (m, r) = fixture();
    let mut invalid = m.clone();
    invalid.phases.pop();
    assert_eq!(
        report(&invalid, &Value::Null).unwrap_err().to_string(),
        "reliability requires at least two identical server/source/tool/arguments/mode/assertion observations"
    );
    let cases = [
        ("/cases", Value::Null, "reliability evidence array missing"),
        (
            "/cases/0/case",
            Value::Null,
            "reliability evidence identity missing",
        ),
        (
            "/cases/1/case",
            json!("call"),
            "duplicate reliability evidence",
        ),
        (
            "/provider_semantics",
            Value::Null,
            "reliability evidence array missing",
        ),
        (
            "/provider_semantics/0/case",
            Value::Null,
            "reliability evidence identity missing",
        ),
        (
            "/provider_semantics/1/case",
            json!("call"),
            "duplicate reliability evidence",
        ),
        (
            "/runtime_events",
            Value::Null,
            "reliability runtime events missing",
        ),
        (
            "/runtime_events/0/detail/case",
            Value::Null,
            "dispatch case missing",
        ),
        (
            "/runtime_events/0/detail/case",
            json!("unknown"),
            "dispatch names an unknown case",
        ),
        (
            "/runtime_events/0/at",
            json!(-1),
            "dispatch timestamp invalid",
        ),
        (
            "/runtime_events/0/at",
            Value::Null,
            "dispatch timestamp invalid",
        ),
    ];
    for (pointer, value, error) in cases {
        let mut r = r.clone();
        *r.pointer_mut(pointer).unwrap() = value;
        assert_eq!(report(&m, &r).unwrap_err().to_string(), error, "{pointer}");
    }
    let mut duplicate = r.clone();
    let event = duplicate["runtime_events"][0].clone();
    duplicate["runtime_events"]
        .as_array_mut()
        .unwrap()
        .push(event);
    assert_eq!(
        report(&m, &duplicate).unwrap_err().to_string(),
        "duplicate dispatch intent"
    );
    duplicate["runtime_events"][2]["at"] = json!(-1);
    assert_eq!(
        report(&m, &duplicate).unwrap_err().to_string(),
        "dispatch timestamp invalid"
    );
    duplicate["cases"] = Value::Null;
    duplicate["provider_semantics"] = Value::Null;
    assert_eq!(
        report(&m, &duplicate).unwrap_err().to_string(),
        "reliability evidence array missing"
    );
    let mut ignored = r.clone();
    ignored["runtime_events"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"unrelated","at":-1}));
    assert_eq!(report(&m, &ignored).unwrap(), report(&m, &r).unwrap());
    for (field, error) in [
        ("cases", "reliability case evidence missing"),
        (
            "provider_semantics",
            "reliability semantic evidence missing",
        ),
    ] {
        let mut missing = r.clone();
        missing[field].as_array_mut().unwrap().pop();
        missing["runtime_events"].as_array_mut().unwrap().pop();
        assert_eq!(report(&m, &missing).unwrap_err().to_string(), error);
    }
}

fn three_samples() -> (Manifest, Value) {
    let (mut m, mut r) = fixture();
    let mut case = m.cases[1].clone();
    case.id = "third".into();
    m.cases.push(case);
    let mut phase = m.phases[1].clone();
    phase.id = "third_phase".into();
    phase.window = "third_window".into();
    phase.cases = vec!["third".into()];
    m.phases.push(phase);
    let mut window = m.windows[1].clone();
    window.id = "third_window".into();
    m.windows.push(window);
    for (i, window) in m.windows.iter_mut().enumerate() {
        window.not_before = 10 + i as u64 * 100;
        window.not_after = 20 + i as u64 * 100;
    }
    for phase in &mut m.phases {
        phase.scenario = Scenario::Reliability {
            min_spacing_seconds: 60,
            later_utc_day: false,
        };
    }
    r["cases"].as_array_mut().unwrap().push(json!({"case":"third","execution":"COMPLETED","semantic":"PASSED","settlement":"NOT_SIGNED"}));
    r["provider_semantics"]
        .as_array_mut()
        .unwrap()
        .push(json!({"case":"third","status":"passed"}));
    r["runtime_events"][0]["at"] = json!(10);
    r["runtime_events"][1]["at"] = json!(110);
    r["runtime_events"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"mcp_dispatch_intent","detail":{"case":"third"},"at":210}));
    (m, r)
}

#[test]
fn report_previous_timestamp_tracks_invalid_but_not_absent_or_unattempted_samples() {
    let (m, base) = three_samples();
    for (unattempted, missing, second, third_status) in [
        (false, false, u64::MAX, "invalid"),
        (true, false, u64::MAX, "passed"),
        (false, true, 110, "passed"),
        (false, false, 151, "invalid"),
    ] {
        let mut r = base.clone();
        r["runtime_events"][1]["at"] = json!(second);
        if unattempted {
            r["cases"][1]["execution"] = json!("UNATTEMPTED");
        }
        if missing {
            r["runtime_events"].as_array_mut().unwrap().remove(1);
        }
        let second_status = if missing { "incomplete" } else { "invalid" };
        let second_category = if missing {
            "dispatch_not_observed"
        } else if unattempted {
            "unattempted_case_has_dispatch"
        } else {
            "dispatch_outside_reviewed_window_or_spacing"
        };
        let third_category = if third_status == "passed" {
            "observed"
        } else {
            "dispatch_outside_reviewed_window_or_spacing"
        };
        let passed = if third_status == "passed" { 2 } else { 1 };
        let expected = json!([{"status":if missing {"incomplete"} else {"invalid"},"planned":3,"passed":passed,"failed":0,"incomplete":3-passed,"scope":RELIABILITY_SCOPE,
            "samples":[expected_sample(&m,0,json!(10),"passed","observed"),expected_sample(&m,1,if missing {Value::Null}else{json!(second)},second_status,second_category),expected_sample(&m,2,json!(210),third_status,third_category)]}]);
        assert_eq!(report(&m, &r).unwrap(), expected);
        assert_eq!(
            qualification(&json!({"reliability":expected})).unwrap(),
            (false, false)
        );
    }
}

#[test]
fn report_window_spacing_and_day_boundaries() {
    let (mut m, mut r) = three_samples();
    m.phases.pop();
    m.cases.pop();
    m.windows.pop();
    r["cases"].as_array_mut().unwrap().pop();
    r["provider_semantics"].as_array_mut().unwrap().pop();
    r["runtime_events"].as_array_mut().unwrap().pop();
    m.windows[1].not_before = 80;
    m.windows[1].not_after = 90;
    for (first, second, status) in [
        (10, 80, "passed"),
        (20, 80, "invalid"),
        (10, 90, "invalid"),
        (21, 80, "invalid"),
    ] {
        r["runtime_events"][0]["at"] = json!(first);
        r["runtime_events"][1]["at"] = json!(second);
        let output = report(&m, &r).unwrap();
        assert_eq!(output[0]["status"], status);
        if first == 20 {
            assert_eq!(output[0]["samples"][1]["status"], "passed");
        }
        if first == 21 {
            assert_eq!(output[0]["samples"][1]["status"], "invalid");
        }
    }
    m.windows[0].not_before = 86300;
    m.windows[0].not_after = 86340;
    m.windows[1].not_before = 86400;
    m.windows[1].not_after = 86410;
    for phase in &mut m.phases {
        phase.scenario = Scenario::Reliability {
            min_spacing_seconds: 60,
            later_utc_day: true,
        };
    }
    r["runtime_events"][0]["at"] = json!(86339);
    r["runtime_events"][1]["at"] = json!(86400);
    assert_eq!(report(&m, &r).unwrap()[0]["status"], "passed");
    r["runtime_events"][1]["at"] = json!(86399);
    assert_eq!(report(&m, &r).unwrap()[0]["status"], "invalid");
}

#[test]
fn report_observation_and_aggregate_precedence_preserve_counts_and_qualification() {
    let (base_m, base) = fixture();
    for (unsigned, execution, settlement, mcp, semantic, debits, status) in [
        (
            true,
            "COMPLETED",
            "NOT_SIGNED",
            "PASSED",
            "passed",
            false,
            "passed",
        ),
        (
            true,
            "COMPLETED",
            "EXPIRED_UNUSED",
            "PASSED",
            "passed",
            false,
            "passed",
        ),
        (
            false,
            "COMPLETED",
            "NOT_SIGNED",
            "PASSED",
            "passed",
            false,
            "incomplete",
        ),
        (
            false,
            "COMPLETED",
            "USED",
            "PASSED",
            "passed",
            true,
            "passed",
        ),
        (
            false,
            "COMPLETED",
            "USED",
            "PASSED",
            "passed",
            false,
            "incomplete",
        ),
        (
            true,
            "DISPATCHING",
            "NOT_SIGNED",
            "FAILED",
            "failed",
            false,
            "incomplete",
        ),
        (
            true,
            "COMPLETED",
            "UNKNOWN",
            "FAILED",
            "failed",
            false,
            "incomplete",
        ),
        (
            true,
            "COMPLETED",
            "NOT_SIGNED",
            "FAILED",
            "passed",
            false,
            "failed",
        ),
        (
            true,
            "COMPLETED",
            "NOT_SIGNED",
            "PASSED",
            "failed",
            false,
            "failed",
        ),
        (
            true,
            "COMPLETED",
            "NOT_SIGNED",
            "PASSED",
            "manual_review_required",
            false,
            "incomplete",
        ),
    ] {
        let mut m = base_m.clone();
        for c in &mut m.cases {
            c.unsigned = unsigned;
        }
        let mut r = base.clone();
        r["cases"][0]["execution"] = json!(execution);
        r["cases"][0]["settlement"] = json!(settlement);
        r["cases"][0]["semantic"] = json!(mcp);
        r["provider_semantics"][0]["status"] = json!(semantic);
        // Keep the peer a passing observation for both paid and unsigned cases.
        if !unsigned {
            r["cases"][1]["settlement"] = json!("USED");
            r["canonical_api_debits"]["cases"]["later"] = json!("1");
        }
        if debits {
            r["canonical_api_debits"]["cases"]["call"] = json!("1");
        }
        let mut expected_first = expected_sample(&m, 0, json!(20), status, "observed");
        expected_first["mcp"] = json!(mcp);
        expected_first["provider_semantics"] = json!(semantic);
        expected_first["settlement"] = json!(settlement);
        let mut peer = expected_sample(&m, 1, json!(86420), "passed", "observed");
        if !unsigned {
            peer["settlement"] = json!("USED");
        }
        let expected = json!([{"status":status,"planned":2,"passed":if status=="passed" {2}else{1},"failed":u8::from(status=="failed"),"incomplete":u8::from(status=="incomplete"),"samples":[expected_first,peer],"scope":RELIABILITY_SCOPE}]);
        assert_eq!(report(&m, &r).unwrap(), expected);
        assert_eq!(
            qualification(&json!({"reliability":expected})).unwrap(),
            (status != "incomplete", status == "passed")
        );
        if debits {
            r["canonical_api_debits"]["status"] = json!("invalid_or_incomplete");
            let output = report(&m, &r).unwrap();
            assert_eq!(output[0]["status"], "incomplete");
            assert_eq!(output[0]["incomplete"], 2);
            assert_eq!(
                qualification(&json!({"reliability":output})).unwrap(),
                (false, false)
            );
        }
    }
    let (m, mut r) = three_samples();
    r["cases"][0]["semantic"] = json!("FAILED");
    r["provider_semantics"][1]["status"] = json!("manual_review_required");
    let output = report(&m, &r).unwrap();
    assert_eq!(
        (
            output[0]["status"].clone(),
            output[0]["failed"].clone(),
            output[0]["incomplete"].clone()
        ),
        (json!("incomplete"), json!(1), json!(1))
    );
    r["runtime_events"][2]["at"] = json!(220);
    let output = report(&m, &r).unwrap();
    assert_eq!(
        (
            output[0]["status"].clone(),
            output[0]["failed"].clone(),
            output[0]["incomplete"].clone()
        ),
        (json!("invalid"), json!(1), json!(2))
    );
}
