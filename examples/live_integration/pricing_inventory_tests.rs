fn inventory_fixture() -> (Value, Value) {
    let cfg = x402_treazury::catalog::Config::default();
    let document =
        json!({"paths":{"/read":{"get":{"description":"Whole vendor prose.\nSecond paragraph."}}}});
    let tool = x402_treazury::catalog::build_tools(&cfg, &document, "api")
        .unwrap()
        .remove(0);
    let mut selected = serde_json::to_value(tool).unwrap();
    selected["source"] = json!("api");
    selected["help_url"] = Value::Null;
    selected["opaque"] = json!({"retained":[1,2]});
    let snapshot = json!({"sources":{"api":{"settings":cfg,"document":document}},"inventory":[{"server":"main","tools":[selected]}],"opaque":"preserved"});
    let mut data = x402_treazury::pricing::Evidence {
        enabled: true,
        selected_tools: 1,
        eligible: 1,
        observed: 1,
        available_prices: 1,
        ..Default::default()
    };
    data.cache
        .insert(x402_treazury::pricing::CachePath::Initialized, 1);
    data.outcomes
        .insert(x402_treazury::pricing::Outcome::Discovered, 1);
    data.http_statuses.insert(402, 1);
    let events = json!([{"kind":"application_pricing","detail":{"source":"api","session":"s","phase":"completed","evidence":data,"prices":[{"method":"GET","path":"/read","line":"Price: $0.014 per call."}]}}]);
    (snapshot, events)
}

#[test]
fn inventory_filters_irrelevant_events_before_decoding_and_validates_empty_maps() {
    let (snapshot, events) = inventory_fixture();
    let ignored = json!([
        null, {"kind":"other","detail":{"session":"s","phase":"completed"}},
        {"kind":"application_pricing","detail":{"session":"other","phase":"completed"}},
        {"kind":"application_pricing","detail":{"session":"s","phase":"started"}},
        {"kind":"application_pricing","detail":{"session":"s","phase":"failed"}},
        {"kind":"application_pricing","detail":{"session":"s","phase":"unknown"}}
    ]);
    assert_eq!(inventory(&snapshot, &ignored, "s").unwrap(), snapshot);
    let mut combined = ignored;
    combined.as_array_mut().unwrap().push(events[0].clone());
    assert_eq!(
        inventory(&snapshot, &combined, "s").unwrap(),
        inventory(&snapshot, &events, "s").unwrap()
    );
    let mut empty = events;
    empty[0]["detail"]["evidence"]["available_prices"] = json!(0);
    empty[0]["detail"]["prices"] = json!([]);
    // Empty maps do not inspect malformed inventory or document, after evidence
    // and source policy have been checked.
    let mut opaque = snapshot.clone();
    opaque["inventory"] = json!("opaque");
    opaque["sources"]["api"]["document"] = Value::Null;
    assert_eq!(inventory(&opaque, &empty, "s").unwrap(), opaque);
    opaque["sources"]["api"]["settings"]["probe_pricing"] = json!(false);
    assert_eq!(
        inventory(&opaque, &empty, "s").unwrap_err().to_string(),
        "runtime pricing policy changed"
    );
}

#[test]
fn inventory_rejects_selected_evidence_in_original_validation_order() {
    let (snapshot, events) = inventory_fixture();
    assert_eq!(
        inventory(&snapshot, &Value::Null, "s")
            .unwrap_err()
            .to_string(),
        "pricing events missing"
    );
    let mut missing = events.clone();
    missing[0]["detail"]["source"] = Value::Null;
    missing[0]["detail"]["evidence"] = Value::Null;
    assert_eq!(
        inventory(&snapshot, &missing, "s").unwrap_err().to_string(),
        "pricing source missing"
    );
    let mut duplicate = events.clone();
    let mut invalid = events[0].clone();
    invalid["detail"]["evidence"] = Value::Null;
    duplicate.as_array_mut().unwrap().push(invalid);
    assert_eq!(
        inventory(&snapshot, &duplicate, "s")
            .unwrap_err()
            .to_string(),
        "duplicate source pricing completion"
    );
    for (pointer, value, expected) in [
        (
            "/0/detail/evidence/observed",
            json!(2),
            "pricing observation totals disagree",
        ),
        (
            "/0/detail/prices/0/method",
            json!("POST"),
            "invalid recorded discovery price",
        ),
        ("/0/detail/prices", json!([]), "pricing line count differs"),
    ] {
        let mut bad = events.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        let mut bad_snapshot = snapshot.clone();
        bad_snapshot["sources"]["api"]["settings"] = Value::Null;
        assert_eq!(
            inventory(&bad_snapshot, &bad, "s").unwrap_err().to_string(),
            expected
        );
    }
    let mut duplicate = events.clone();
    let row = duplicate[0]["detail"]["prices"][0].clone();
    duplicate[0]["detail"]["prices"]
        .as_array_mut()
        .unwrap()
        .push(row);
    assert_eq!(
        inventory(&snapshot, &duplicate, "s")
            .unwrap_err()
            .to_string(),
        "duplicate recorded discovery price"
    );
    for pointer in ["/0/detail/evidence", "/0/detail/prices"] {
        let mut bad = events.clone();
        *bad.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(inventory(&snapshot, &bad, "s").is_err(), "{pointer}");
    }
    for settings in [Value::Null, json!({"probe_pricing":"invalid"})] {
        let mut bad = snapshot.clone();
        bad["sources"]["api"]["settings"] = settings;
        assert!(inventory(&bad, &events, "s").is_err());
    }
}

#[test]
fn inventory_eligibility_and_pinned_identity_rejections_remain_distinct() {
    let (snapshot, events) = inventory_fixture();
    for (pointer, value, error) in [
        (
            "/inventory/0/tools/0/method",
            json!("POST"),
            "price names an ineligible or unselected route",
        ),
        (
            "/inventory/0/tools/0/help_url",
            json!("https://example.invalid/help"),
            "price names an ineligible or unselected route",
        ),
        (
            "/inventory/0/tools/0/source",
            json!("other"),
            "price names an ineligible or unselected route",
        ),
        (
            "/inventory/0/tools/0/path",
            Value::Null,
            "tool route missing",
        ),
        (
            "/inventory/0/tools/0/name",
            Value::Null,
            "tool name missing",
        ),
        (
            "/inventory/0/tools/0/name",
            json!("missing"),
            "priced tool disappeared",
        ),
        (
            "/inventory/0/tools/0/input_schema",
            json!({}),
            "pricing changed a pinned route/schema",
        ),
        ("/inventory", Value::Null, "snapshot inventory missing"),
        ("/inventory/0/tools", Value::Null, "snapshot tools missing"),
    ] {
        let mut bad = snapshot.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            inventory(&bad, &events, "s").unwrap_err().to_string(),
            error,
            "{pointer}"
        );
    }
    let mut template = snapshot.clone();
    template["inventory"][0]["tools"][0]["path"] = json!("/read/{id}");
    let mut prices = events.clone();
    prices[0]["detail"]["prices"][0]["path"] = json!("/read/{id}");
    assert_eq!(
        inventory(&template, &prices, "s").unwrap_err().to_string(),
        "price names an ineligible or unselected route"
    );
    let mut advertised = snapshot.clone();
    advertised["sources"]["api"]["settings"]["pricing_key"] = json!("x-price");
    advertised["sources"]["api"]["document"]["paths"]["/read"]["get"]["x-price"] =
        json!({"price":"1"});
    assert_eq!(
        inventory(&advertised, &events, "s")
            .unwrap_err()
            .to_string(),
        "price names an ineligible or unselected route"
    );
    // An eligible alias cannot conceal identity drift in another selected tool.
    for (field, value) in [("method", json!("POST")), ("path", json!("/other"))] {
        let mut bad = snapshot.clone();
        let mut alias = bad["inventory"][0]["tools"][0].clone();
        alias[field] = value;
        bad["inventory"][0]["tools"]
            .as_array_mut()
            .unwrap()
            .push(alias);
        assert_eq!(
            inventory(&bad, &events, "s").unwrap_err().to_string(),
            "pricing changed a pinned route/schema"
        );
    }
}

#[test]
fn inventory_updates_only_descriptions_across_listeners_sources_and_overrides() {
    let (mut snapshot, mut events) = inventory_fixture();
    let source = snapshot["sources"]["api"].clone();
    snapshot["sources"]["alias"] = source;
    snapshot["sources"]["alias"]["settings"]["prefix"] = json!("custom");
    snapshot["sources"]["alias"]["settings"]["overrides"] =
        json!({"custom_read":{"description":"Authored override remains final."}});
    let cfg: x402_treazury::catalog::Config =
        serde_json::from_value(snapshot["sources"]["alias"]["settings"].clone()).unwrap();
    let mut alias = serde_json::to_value(
        x402_treazury::catalog::build_tools(
            &cfg,
            &snapshot["sources"]["alias"]["document"],
            "custom",
        )
        .unwrap()
        .remove(0),
    )
    .unwrap();
    alias["source"] = json!("alias");
    let mut completion = events[0].clone();
    completion["detail"]["source"] = json!("alias");
    events.as_array_mut().unwrap().push(completion);
    let selected = snapshot["inventory"][0]["tools"][0].clone();
    let local =
        json!({"source":"local","name":"help","description":"Local unchanged","opaque":true});
    snapshot["inventory"]
        .as_array_mut()
        .unwrap()
        .push(json!({"server":"second","tools":[alias,selected,local]}));
    let mut expected = snapshot.clone();
    // Price text is a pure prefix to the existing full vendor instructions.
    let description = snapshot["inventory"][0]["tools"][0]["description"]
        .as_str()
        .unwrap()
        .replace("Cost: unknown.", "Price: $0.014 per call.");
    expected["inventory"][0]["tools"][0]["description"] = json!(description);
    expected["inventory"][1]["tools"][1]["description"] = json!(description);
    assert_eq!(inventory(&snapshot, &events, "s").unwrap(), expected);
}

#[test]
fn inventory_reconstruction_does_not_add_a_required_arguments_eligibility_gate() {
    let (mut snapshot, events) = inventory_fixture();
    snapshot["sources"]["api"]["document"]["paths"]["/read"]["get"]["parameters"] = json!([
        {"name":"query","in":"query","required":true,"schema":{"type":"string"}}
    ]);
    let cfg: x402_treazury::catalog::Config =
        serde_json::from_value(snapshot["sources"]["api"]["settings"].clone()).unwrap();
    let mut tool = serde_json::to_value(
        x402_treazury::catalog::build_tools(&cfg, &snapshot["sources"]["api"]["document"], "api")
            .unwrap()
            .remove(0),
    )
    .unwrap();
    tool["source"] = json!("api");
    snapshot["inventory"][0]["tools"][0] = tool;
    let mut expected = snapshot.clone();
    expected["inventory"][0]["tools"][0]["description"] = json!(
        snapshot["inventory"][0]["tools"][0]["description"]
            .as_str()
            .unwrap()
            .replace("Cost: unknown.", "Price: $0.014 per call.")
    );
    assert_eq!(inventory(&snapshot, &events, "s").unwrap(), expected);
}
