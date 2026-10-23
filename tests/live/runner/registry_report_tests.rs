fn report_db(f: &Fixture) -> rusqlite::Connection {
    rusqlite::Connection::open(f.state.join("live-integration/registry.sqlite")).unwrap()
}
fn report_install_catalogs(f: &mut Fixture) {
    use std::io::Write;
    files::create_dir(&f.manifest.evidence_dir).unwrap();
    let directory = f.manifest.evidence_dir.join(&f.manifest.run_id);
    files::create_dir(&directory).unwrap();
    let snapshot = json!({"version":1,"inventory":[{"server":"main","tools":[{"source":"api","name":"api_read","input_schema":{"type":"object"}}]}]});
    let mut hashes = std::collections::BTreeMap::new();
    for (name, bytes) in [
        ("snapshot.json", serde_json::to_vec(&snapshot).unwrap()),
        (
            "frozen-snapshot.json",
            serde_json::to_vec(&snapshot).unwrap(),
        ),
        ("deployment.toml", b"synthetic".to_vec()),
        ("build.json", b"{}".to_vec()),
    ] {
        files::create_file(&directory.join(name))
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        hashes.insert(name.into(), files::hash(&bytes));
    }
    f.pins.catalogs = Some(crate::preparation::CatalogPins {
        build: x402_treazury::build_identity::current(),
        directory,
        files: hashes,
    });
    f.pins.qualification = "catalogs_prepared".into();
    f.pins.resolved_config["sources"] = json!({"api":{}});
    let raw = serde_json::to_string(&f.pins).unwrap();
    report_db(f)
        .execute(
            "INSERT INTO pins VALUES(?1,2,?2,?3,3)",
            rusqlite::params![f.manifest.run_id, raw, files::hash(raw.as_bytes())],
        )
        .unwrap();
}

// Complete expectations are authored here, independently of report generators.
// Only pin digests derive from the exact synthetic bytes installed by the fixture.
fn report_expected_base() -> Value {
    json!({"version":1,"authorization":"review_1","api_reserved_atomic":0,
        "api_ceiling_atomic":40000,"api_headroom_atomic":40000,
        "source_ceiling_zatoshis":0,"new_job_ceiling":0,
        "funding_jobs_reserved":0,"source_reserved_zatoshis":0,
        "cases":[],"pin_revisions":[],"runtime_events":[],
        "execution_available":false,"application_evidence":null,
        "qualification_scope":"offline_configuration","tor_isolation":"not_qualified",
        "funding_permits_available":false,
        "baseline":"immutable private evidence; live reconciliation required",
        "next_step":"prepare is required before execution"})
}
fn report_expected_case(run: &str, reservation: i64) -> Value {
    json!({"run":run,"case":"call","reservation_atomic":reservation,"charged_reservation_atomic":0,
        "execution":"UNATTEMPTED","semantic":"UNOBSERVED","settlement":"UNOBSERVED",
        "result_hash":null,"eligibility":"requires_execution_qualification"})
}
fn report_expected_pin(run: &str, revision: i64, pins: &Pins) -> Value {
    json!({"run":run,"revision":revision,"digest":files::hash(serde_json::to_string(pins).unwrap().as_bytes())})
}
fn report_expected_selected(pins: &Pins) -> Value {
    let mut report = report_expected_base();
    report["cases"] = json!([report_expected_case("test_run", 20000)]);
    report["pin_revisions"] = json!([report_expected_pin("test_run", 1, pins)]);
    report["treasury_accounting"] = json!({"status":"unobserved","observations":[]});
    report["catalog_stages"] = json!([]);
    report["provider_semantics"] = json!([{"case":"call","category":"response_not_completed","required":false,"status":"unobserved"}]);
    report["canonical_api_debits"] = json!({"status":"validated","cases":{}});
    report["help_stages"] = json!([]);
    report["fee_observations"] = json!({"asset":"Base USDC","threshold_atomic":10000,
        "admissions_status":"invalid_or_incomplete","challenges_status":"invalid_or_incomplete",
        "cases":[{"case":"call","admitted_above_one_cent":null,"admitted_atomic":null,"challenge_observations":[],"verified_debit_above_one_cent":null,"verified_debit_atomic":null}],
        "meaning":"Observed challenge offers, managed admissions and canonical debits are separate evidence. Offers may be rejected or unselected and grant no payment authority. Admission precedes signing and is not a debit. Missing evidence is not zero; no provider-wide maximum or minimum wallet size is inferred. Headerless legacy body offers are not captured."});
    report["reliability"] = json!([]);
    report["rotation"] = json!([]);
    report["concurrency"] = json!([]);
    report["summary"] = json!({"version":1,"network":"direct","lifecycle":[],"source_inventory_complete":false,
        "source_funding":{"observations":0,"reserved_jobs":0,"reserved_zatoshis":0,"saved_accounting":"unobserved"},
        "providers":[{"source":"api","cases":[{"case":"call","execution":"not_attempted","mcp":"unobserved","payment":"not_attempted","semantics":"unobserved","verified_atomic":null}],
        "catalog":[{"http_status":null,"scope":"no_observation","state":"unobserved"}],
        "help":[{"http_status":null,"scope":"no_observation","state":"not_attempted"}]}]});
    report
}
fn report_prepared_fixture(managed: bool) -> (Fixture, Registry, Value) {
    let mut f = Fixture::new();
    f.manifest.evidence_dir = f._dir.path().join("evidence");
    if !managed {
        f.manifest.start.pools.clear();
        f.manifest.phases[0].pools.clear();
        f.manifest.phases[0].scenario = crate::manifest::Scenario::Unsigned {};
        f.manifest.cases[0].unsigned = true;
        f.manifest.cases[0].reserve_usdc = "0".into();
    }
    let mut r = f.open();
    f.prepare(&mut r);
    let mut expected = report_expected_selected(&f.pins);
    report_install_catalogs(&mut f);
    expected["pin_revisions"]
        .as_array_mut()
        .unwrap()
        .push(report_expected_pin("test_run", 2, &f.pins));
    expected["execution_available"] = json!(true);
    expected["application_evidence"] = json!({"claims":0,"finished":0,"without_completion":[],"payment_attempts_correlated":0,"verified_debits":{},
        "scope":"application acceptance/completion, versioned admission-intent correlation and canonical receipt debit proofs; intent alone does not prove financial commit, settlement or Tor isolation",
        "seller_receipts":{"classifications":{},"recorded":0,"without_observation":0,"meaning":"seller claims only; transaction receipt and USDC debit still require chain verification"}});
    expected["catalog_stages"] = json!([{"phase":"initial_inspection","sources":[],"status":"unobserved"},{"phase":"frozen_reload","sources":[],"status":"unobserved"}]);
    expected["fee_observations"]["admissions_status"] = json!("validated");
    expected["fee_observations"]["challenges_status"] = json!("validated");
    expected["summary"]["source_inventory_complete"] = json!(true);
    if managed {
        expected["qualification_scope"] = json!("managed_mcp_with_authorization_evidence");
        expected["funding_permits_available"] = json!(true);
        expected["next_step"] = json!(
            "positive funding requires --allow-funding and registry authority; unresolved authorizations or missing debit proofs remain incomplete"
        );
    } else {
        expected["cases"][0]["reservation_atomic"] = json!(0);
        expected["summary"]["providers"][0]["cases"][0]["payment"] = json!("not_requested");
        expected["qualification_scope"] = json!("unsigned_mcp_only");
        expected["next_step"] = json!(
            "run dispatches only unattempted unsigned cases; no payment or Tor isolation qualification"
        );
    }
    (f, r, expected)
}
#[test]
fn report_composition_empty_configuration_and_prepared_modes_are_complete() {
    let f = Fixture::new();
    let mut r = f.open();
    assert_eq!(r.report(None, 10).unwrap(), report_expected_base());
    assert_eq!(
        r.report(Some("missing"), 10).unwrap_err().to_string(),
        "Query returned no rows"
    );
    f.prepare(&mut r);
    let expected = report_expected_selected(&f.pins);
    assert_eq!(r.report(Some("test_run"), 10).unwrap(), expected);
    for (now, eligibility) in [
        (0, "waiting_window"),
        (1, "requires_execution_qualification"),
        (999, "requires_execution_qualification"),
        (1000, "expired_window"),
    ] {
        let mut expected = expected.clone();
        expected["cases"][0]["eligibility"] = json!(eligibility);
        assert_eq!(r.report(Some("test_run"), now).unwrap(), expected);
    }
    for managed in [false, true] {
        let (_f, r, expected) = report_prepared_fixture(managed);
        assert_eq!(r.report(Some("test_run"), 10).unwrap(), expected);
    }
}
#[test]
fn report_composition_selected_rows_retain_global_charges_latest_authority_and_funding() {
    use x402_treazury::qualification::funding::{self, Limits, Request};
    let mut f = Fixture::new();
    f.auth.cumulative_source_zec = "0.001".into();
    f.auth.cumulative_new_jobs = 2;
    let mut r = f.open();
    f.prepare(&mut r);
    r.reserve_batch("test_run", "first", &["call".into()], 10)
        .unwrap();
    let mut db = report_db(&f);
    let mut tx = db.transaction().unwrap();
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
            source_zatoshis: 100000,
        },
        Limits {
            jobs: 2,
            source_zatoshis: 100000,
        },
        11,
    )
    .unwrap();
    tx.commit().unwrap();
    let mut second = f.manifest.clone();
    second.run_id = "another".into();
    r.prepare(
        &planner::build(&second, &f.pins.resolved_config).unwrap(),
        &f.pins,
        12,
    )
    .unwrap();
    // Insertion order differs from timestamp order; irrelevant malformed prose is ignored.
    r.event("test_run", "cover_observed", &json!({"marker":"first"}), 20)
        .unwrap();
    r.event("another", "cover_observed", &json!({"marker":"second"}), 15)
        .unwrap();
    db.execute("INSERT INTO events(run,kind,detail,at) VALUES('another','not_a_runtime_kind','not JSON',14)",[]).unwrap();
    drop(r);
    f.auth.id = "review_2".into();
    f.auth.cumulative_api_usdc = "0.08".into();
    let r = Registry::authorize(&f.state, &f.auth, 30).unwrap();
    let mut expected = report_expected_base();
    expected["authorization"] = json!("review_2");
    expected["api_ceiling_atomic"] = json!(80000);
    expected["api_reserved_atomic"] = json!(20000);
    expected["api_headroom_atomic"] = json!(60000);
    expected["source_ceiling_zatoshis"] = json!(100000);
    expected["new_job_ceiling"] = json!(2);
    expected["funding_jobs_reserved"] = json!(1);
    expected["source_reserved_zatoshis"] = json!(100);
    let mut reserved = report_expected_case("test_run", 20000);
    reserved["execution"] = json!("RESERVED");
    reserved["charged_reservation_atomic"] = json!(20000);
    reserved["eligibility"] = json!("observe_only");
    expected["cases"] = json!([report_expected_case("another", 20000), reserved]);
    expected["pin_revisions"] = json!([
        report_expected_pin("another", 1, &f.pins),
        report_expected_pin("test_run", 1, &f.pins)
    ]);
    expected["runtime_events"] = json!([
        {"run":"test_run","kind":"cover_observed","detail":{"marker":"first"},"at":20},
        {"run":"another","kind":"cover_observed","detail":{"marker":"second"},"at":15}]);
    assert_eq!(r.report(None, 40).unwrap(), expected);
    let mut selected = report_expected_selected(&f.pins);
    for key in [
        "authorization",
        "api_ceiling_atomic",
        "api_reserved_atomic",
        "api_headroom_atomic",
        "source_ceiling_zatoshis",
        "new_job_ceiling",
        "funding_jobs_reserved",
        "source_reserved_zatoshis",
    ] {
        selected[key] = expected[key].clone();
    }
    selected["cases"] = json!([report_expected_case("another", 20000)]);
    selected["pin_revisions"] = json!([report_expected_pin("another", 1, &f.pins)]);
    selected["runtime_events"] =
        json!([{"run":"another","kind":"cover_observed","detail":{"marker":"second"},"at":15}]);
    selected["summary"]["source_funding"]["reserved_jobs"] = json!(1);
    selected["summary"]["source_funding"]["reserved_zatoshis"] = json!(100);
    assert_eq!(r.report(Some("another"), 40).unwrap(), selected);
}

#[test]
fn report_composition_strict_event_errors_precede_later_integrity_failures() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let db = report_db(&f);
    db.execute(
        "INSERT INTO events(run,kind,detail,at) VALUES('test_run','cover_observed','not JSON',10)",
        [],
    )
    .unwrap();
    db.execute("UPDATE identity SET baseline_hash='changed'", [])
        .unwrap();
    // Runtime JSON decoding occurs before the selected-run accounting pass.
    assert_eq!(
        r.report(Some("test_run"), 10).unwrap_err().to_string(),
        "expected ident at line 1 column 2"
    );
    db.execute("DELETE FROM events WHERE kind='cover_observed'", [])
        .unwrap();
    assert_eq!(
        r.report(Some("test_run"), 10).unwrap_err().to_string(),
        "accounting baseline hash mismatch"
    );
    // Selected rows validate their manifest before any runtime JSON is decoded.
    db.execute("UPDATE runs SET manifest='not JSON'", [])
        .unwrap();
    db.execute(
        "INSERT INTO events(run,kind,detail,at) VALUES('test_run','cover_observed','[',10)",
        [],
    )
    .unwrap();
    assert_eq!(
        r.report(Some("test_run"), 10).unwrap_err().to_string(),
        "expected ident at line 1 column 2"
    );
}
#[test]
fn report_composition_application_wrapper_does_not_hide_later_fatal_help_evidence() {
    let (_f, r, _expected) = report_prepared_fixture(true);
    r.event("test_run", "application_claim", &json!({}), 10)
        .unwrap();
    assert_eq!(
        r.report(Some("test_run"), 10).unwrap_err().to_string(),
        "help claim identity missing"
    );
}
#[test]
fn report_composition_application_and_debit_failures_remain_nonfatal() {
    for kind in ["application_claim", "application_debit_verified"] {
        let (_f, r, mut expected) = report_prepared_fixture(true);
        let detail = if kind == "application_claim" {
            json!({"case":"call"})
        } else {
            json!({})
        };
        r.event("test_run", kind, &detail, 10).unwrap();
        expected["runtime_events"] =
            json!([{"run":"test_run","kind":kind,"detail":detail,"at":10}]);
        expected["application_evidence"] = json!({"status":"invalid_or_incomplete","reason":if kind=="application_claim" {"application event session missing/invalid"}else{"debit proof case missing"}});
        expected["fee_observations"]["admissions_status"] = json!("invalid_or_incomplete");
        expected["fee_observations"]["challenges_status"] = json!("invalid_or_incomplete");
        if kind == "application_debit_verified" {
            expected["canonical_api_debits"] = json!({"status":"invalid_or_incomplete","cases":{}});
            expected["rotation"] =
                json!([{"status":"invalid","reason":"debit proof case missing"}]);
            expected["summary"]["lifecycle"] =
                json!([{"http_status":null,"scope":"rotation_0","state":"incomplete"}]);
        }
        assert_eq!(r.report(Some("test_run"), 10).unwrap(), expected);
    }
}
#[test]
fn report_composition_case_and_pin_bounds_preserve_order_and_exact_limits() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let db = report_db(&f);
    db.execute_batch("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<10000) INSERT INTO cases(run,id,reservation,window_start,window_end,phase) SELECT 'test_run','extra_'||n,0,1,1000,'smoke' FROM numbers;
        WITH RECURSIVE numbers(n) AS (VALUES(2) UNION ALL SELECT n+1 FROM numbers WHERE n<10001) INSERT INTO pins(run,revision,payload,digest,at) SELECT 'test_run',n,'{}','fixture',2 FROM numbers;").unwrap();
    assert_eq!(
        r.report(None, 10).unwrap_err().to_string(),
        "report exceeds 10000 cases; select a single run instead"
    );
    db.execute("DELETE FROM cases WHERE id='extra_10000'", [])
        .unwrap();
    assert_eq!(
        r.report(None, 10).unwrap_err().to_string(),
        "report exceeds 10000 pin revisions; select a single run instead"
    );
    db.execute("DELETE FROM pins WHERE revision=10001", [])
        .unwrap();
    let report = r.report(None, 10).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 10000);
    assert_eq!(report["pin_revisions"].as_array().unwrap().len(), 10000);
}
#[test]
fn report_composition_event_and_complete_document_bounds_are_not_partial_reports() {
    let f = Fixture::new();
    let mut r = f.open();
    f.prepare(&mut r);
    let db = report_db(&f);
    // The event count guard precedes parsing every selected detail.
    db.execute_batch("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<50001) INSERT INTO events(run,kind,detail,at) SELECT 'test_run','cover_observed','not JSON',n FROM numbers").unwrap();
    assert_eq!(
        r.report(None, 10).unwrap_err().to_string(),
        "report exceeds 50000 runtime events; select a single run"
    );
    db.execute(
        "DELETE FROM events WHERE kind='cover_observed' AND at=50001",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE events SET detail='{}' WHERE kind='cover_observed'",
        [],
    )
    .unwrap();
    assert_eq!(
        r.report(None, 10).unwrap()["runtime_events"]
            .as_array()
            .unwrap()
            .len(),
        50000
    );
    db.execute("DELETE FROM events WHERE kind='cover_observed'", [])
        .unwrap();
    let mut expected = report_expected_base();
    expected["cases"] = json!([report_expected_case("test_run", 20000)]);
    expected["pin_revisions"] = json!([report_expected_pin("test_run", 1, &f.pins)]);
    expected["runtime_events"] =
        json!([{"run":"test_run","kind":"cover_observed","detail":"","at":10}]);
    let overhead = serde_json::to_vec(&expected).unwrap().len();
    let payload = "x".repeat(files::DOCUMENT_BYTES - overhead);
    expected["runtime_events"][0]["detail"] = json!(payload);
    let detail = serde_json::to_string(&payload).unwrap();
    db.execute(
        "INSERT INTO events(run,kind,detail,at) VALUES('test_run','cover_observed',?1,10)",
        [detail],
    )
    .unwrap();
    let exact = r.report(None, 10).unwrap();
    assert_eq!(
        serde_json::to_vec(&exact).unwrap().len(),
        files::DOCUMENT_BYTES
    );
    assert_eq!(exact, expected);
    db.execute(
        "UPDATE events SET detail=?1 WHERE kind='cover_observed'",
        [serde_json::to_string(&(payload + "x")).unwrap()],
    )
    .unwrap();
    assert_eq!(
        r.report(None, 10).unwrap_err().to_string(),
        "registry document exceeds 16777216 byte limit"
    );
}

#[tokio::test]
async fn historical_pricing_evidence_is_readable_without_live_manifest_authority() {
    let (f, r, _) = report_prepared_fixture(true);
    let db = report_db(&f);
    let mut historical = serde_json::to_value(&f.manifest).unwrap();
    historical["pricing_checks"] = json!({"api":"nonempty"});
    let raw = historical.to_string();
    db.execute(
        "UPDATE runs SET manifest=?1 WHERE id=?2",
        rusqlite::params![raw, f.manifest.run_id],
    )
    .unwrap();
    let event = json!({"session":"retained", "source":"api", "phase":"completed", "evidence":{"available_prices":1}});
    r.event(&f.manifest.run_id, "application_pricing", &event, 3)
        .unwrap();
    let before = r.report(Some(&f.manifest.run_id), 3).unwrap();
    assert!(before.get("pricing_stages").is_none());
    assert!(
        before["runtime_events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "application_pricing" && e["detail"] == event)
    );
    assert!(
        serde_json::from_value::<crate::manifest::Manifest>(historical)
            .unwrap_err()
            .to_string()
            .contains("pricing_checks")
    );
    assert!(
        r.manifest(&f.manifest.run_id)
            .unwrap_err()
            .to_string()
            .contains("pricing_checks")
    );
    drop(r);
    // Exercise the real offline interface: a historical manifest must reach
    // the original-run Tor constraint, never fail on its retired pricing field.
    let audit_output = f.manifest.evidence_dir.join("audit.json");
    let audit_error = crate::tor::archive::audit_run(&f.state, &f.manifest.run_id, &audit_output)
        .unwrap_err()
        .to_string();
    assert_eq!(
        audit_error,
        "offline Tor audit requires an originally owned-Tor run"
    );
    assert!(
        !audit_output.exists(),
        "a direct archive must not be Tor-qualified"
    );
    let report_output = f.manifest.evidence_dir.join("historical-report.json");
    assert_eq!(
        crate::execute(crate::Command::Report {
            state_dir: f.state.clone(),
            run: Some(f.manifest.run_id.clone()),
            output: Some(report_output.clone()),
            format: crate::reporting::Format::Json,
            eligibility: true,
        })
        .await
        .unwrap(),
        0
    );
    let projection: Value = serde_json::from_slice(&std::fs::read(report_output).unwrap()).unwrap();
    assert_eq!(projection["eligibility"]["status"], "refused");
    assert_eq!(projection["eligibility"]["execution_authorized"], false);
    assert!(
        projection["eligibility"]["reason"]
            .as_str()
            .unwrap()
            .contains("pricing_checks")
    );
    assert!(
        projection["runtime_events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "application_pricing" && e["detail"] == event)
    );
    let error = crate::execution::run(&f.state, &f.manifest.run_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("pricing_checks"), "{error:#}");
    let r = Registry::open(&f.state, true).unwrap();
    assert_eq!(r.report(Some(&f.manifest.run_id), 3).unwrap(), before);
    assert_eq!(
        db.query_row(
            "SELECT manifest FROM runs WHERE id=?1",
            [&f.manifest.run_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM events WHERE kind IN ('execution_started','application_claim','application_payment','funding_execution')", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(before["api_reserved_atomic"], 0);
}
