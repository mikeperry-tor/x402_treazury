fn capacity_fixture() -> (Value, Value) {
    let (mut before, mut after) = promoted();
    before["schema_version"] = json!(2);
    after["schema_version"] = json!(2);
    after["pools"][0]["generation"] = json!(0);
    after["pools"][0]["addresses"][0]["role"] = json!("RETAINED");
    before["pools"][0]["addresses"][1]["destination_recorded"] = json!(true);
    before["pools"][0]["reserve_claims"] = json!([{"slot":"small","enabled":true,"generation":4,"wallet_id":"ready","destination_ready":true}]);
    after["pools"][0]["reserve_claims"] = json!([{"slot":"small","enabled":true,"generation":5,"wallet_id":"new","destination_ready":false}]);
    (before, after)
}

fn capacity_vacant() -> (Value, Value) {
    let (before, mut after) = capacity_fixture();
    after["pools"][0]["addresses"].as_array_mut().unwrap().pop();
    after["pools"][0]["reserve_claims"][0]["wallet_id"] = Value::Null;
    after["funding_jobs"] = json!([]);
    (before, after)
}

#[test]
fn capacity_group_claim_and_inventory_rejections_keep_exact_errors() {
    for (side, pointer, value, error) in [
        (
            0,
            "/schema_version",
            json!(1),
            "capacity lifecycle schema/treasury mismatch",
        ),
        (
            1,
            "/schema_version",
            Value::Null,
            "capacity lifecycle schema/treasury mismatch",
        ),
        (
            1,
            "/treasury_id",
            json!("other"),
            "capacity lifecycle schema/treasury mismatch",
        ),
        (0, "/pools", json!([]), "previous pool missing"),
        (1, "/pools", json!([]), "current pool missing"),
        (
            1,
            "/pools/0/name",
            json!("other"),
            "capacity group identity changed",
        ),
        (
            1,
            "/pools/0/generation",
            json!(1),
            "capacity group identity changed",
        ),
        (
            1,
            "/pools/0/addresses/2/role",
            json!("ACTIVE"),
            "duplicate lifecycle wallet role",
        ),
        (
            1,
            "/pools/0/addresses/1/id",
            json!("unknown"),
            "new payer was not existing inventory",
        ),
        (
            0,
            "/pools/0/addresses/1/role",
            json!("ALLOCATED"),
            "new payer did not consume funded reserve inventory",
        ),
        (
            0,
            "/pools/0/addresses/1/destination_recorded",
            json!(false),
            "new payer did not consume funded reserve inventory",
        ),
        (
            0,
            "/pools/0/reserve_claims",
            Value::Null,
            "reserve claims missing",
        ),
        (
            1,
            "/pools/0/reserve_claims",
            Value::Null,
            "reserve claims missing",
        ),
        (
            0,
            "/pools/0/reserve_claims/0/slot",
            json!("other"),
            "invalid reserve slot",
        ),
        (
            0,
            "/pools/0/reserve_claims/0/wallet_id",
            json!("other"),
            "consumed payer has no original claim",
        ),
        (
            0,
            "/pools/0/reserve_claims/0/enabled",
            json!(false),
            "consumed reserve was not ready",
        ),
        (
            0,
            "/pools/0/reserve_claims/0/destination_ready",
            json!(false),
            "consumed reserve was not ready",
        ),
        (
            1,
            "/pools/0/reserve_claims",
            json!([]),
            "consumed reserve slot disappeared",
        ),
        (
            0,
            "/pools/0/reserve_claims/0/generation",
            json!(u64::MAX),
            "replacement claim generation mismatch",
        ),
        (
            0,
            "/pools/0/reserve_claims/0/generation",
            Value::Null,
            "replacement claim generation mismatch",
        ),
        (
            1,
            "/pools/0/reserve_claims/0/generation",
            Value::Null,
            "replacement claim generation mismatch",
        ),
        (
            1,
            "/pools/0/reserve_claims/0/enabled",
            json!(false),
            "replacement claim generation mismatch",
        ),
        (
            1,
            "/pools/0/addresses/0/id",
            json!("changed"),
            "consumption lost wallet",
        ),
        (
            1,
            "/pools/0/addresses/0/address",
            json!("changed"),
            "consumption changed immutable wallet identity",
        ),
        (
            1,
            "/pools/0/addresses/0/target",
            json!("1"),
            "consumption changed immutable wallet identity",
        ),
        (
            1,
            "/pools/0/addresses/0/role",
            json!("READY"),
            "unexpected consumption role transition",
        ),
    ] {
        let (before, after) = capacity_fixture();
        let mut states = [before, after];
        *states[side].pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            consumed_reserve(&states[0], &states[1], "p")
                .unwrap_err()
                .to_string(),
            error,
            "{side} {pointer}"
        );
    }
    for side in [0, 1] {
        for (pointer, error) in [
            ("/pools", "duplicate lifecycle identity"),
            ("/pools/0/addresses", "duplicate lifecycle identity"),
            ("/pools/0/reserve_claims", "duplicate reserve slot"),
        ] {
            let (before, after) = capacity_fixture();
            let mut states = [before, after];
            let rows = states[side]
                .pointer_mut(pointer)
                .unwrap()
                .as_array_mut()
                .unwrap();
            rows.push(rows[0].clone());
            assert_eq!(
                consumed_reserve(&states[0], &states[1], "p")
                    .unwrap_err()
                    .to_string(),
                error
            );
        }
    }
}

#[test]
fn capacity_preserves_unrelated_claims_and_existing_generation_option_semantics() {
    let (mut before, mut after) = capacity_fixture();
    let other = json!({"slot":"large","enabled":false,"generation":7,"wallet_id":null,"destination_ready":false});
    after["pools"][0]["reserve_claims"]
        .as_array_mut()
        .unwrap()
        .push(other.clone());
    // Extra new claims are not interpreted as a change to an old claim.
    assert_eq!(
        consumed_reserve(&before, &after, "p").unwrap(),
        Some("replacement".into())
    );
    before["pools"][0]["reserve_claims"]
        .as_array_mut()
        .unwrap()
        .push(other);
    assert_eq!(
        consumed_reserve(&before, &after, "p").unwrap(),
        Some("replacement".into())
    );
    after["pools"][0]["reserve_claims"][1]["enabled"] = json!(true);
    assert_eq!(
        consumed_reserve(&before, &after, "p")
            .unwrap_err()
            .to_string(),
        "unrelated reserve changed during measured consumption"
    );
    let (mut before, mut after) = capacity_fixture();
    after["pools"][0]["reserve_claims"][0]["generation"] = Value::Null;
    for generation in [Value::Null, json!(u64::MAX)] {
        before["pools"][0]["reserve_claims"][0]["generation"] = generation;
        // Preserve the existing None == None comparison; this refactor does
        // not tighten schema acceptance.
        assert_eq!(
            consumed_reserve(&before, &after, "p").unwrap(),
            Some("replacement".into())
        );
    }
}

#[test]
fn capacity_retirement_requires_exact_bounded_remainder_evidence() {
    let (before, mut after) = capacity_fixture();
    after["pools"][0]["addresses"][0]["role"] = json!("RETIRED");
    assert_eq!(
        consumed_reserve(&before, &after, "p")
            .unwrap_err()
            .to_string(),
        "retirement support missing"
    );
    for (confirmed, latest) in [("2", "3"), ("3", "2"), ("3", "3")] {
        after["pools"][0]["addresses"][0]["retirement"] = json!({"confirmed_amount":confirmed,"latest_amount":latest,"observed_bound":"3","accepted_maximum":"3"});
        assert_eq!(
            consumed_reserve(&before, &after, "p").unwrap(),
            Some("replacement".into())
        );
    }
    for field in [
        "confirmed_amount",
        "latest_amount",
        "observed_bound",
        "accepted_maximum",
    ] {
        let mut bad = after.clone();
        bad["pools"][0]["addresses"][0]["retirement"][field] = Value::Null;
        assert_eq!(
            consumed_reserve(&before, &bad, "p")
                .unwrap_err()
                .to_string(),
            "retirement support missing"
        );
        bad["pools"][0]["addresses"][0]["retirement"][field] = json!("invalid");
        assert!(consumed_reserve(&before, &bad, "p").is_err());
    }
    for field in ["observed_bound", "accepted_maximum"] {
        let mut bad = after.clone();
        bad["pools"][0]["addresses"][0]["retirement"][field] = json!("2");
        assert_eq!(
            consumed_reserve(&before, &bad, "p")
                .unwrap_err()
                .to_string(),
            "retirement remainder support mismatch"
        );
    }
}

#[test]
fn capacity_vacant_and_allocated_preserve_funding_history_and_scope() {
    for vacant in [false, true] {
        let (mut before, mut after) = if vacant {
            capacity_vacant()
        } else {
            capacity_fixture()
        };
        let history = json!({"id":"history","pool_id":"other","wallet_id":"historic","recipient":"opaque","target":"17","phase":"COMPLETE"});
        before["funding_jobs"]
            .as_array_mut()
            .unwrap()
            .push(history.clone());
        after["funding_jobs"].as_array_mut().unwrap().push(history);
        let expected = if vacant {
            None
        } else {
            Some("replacement".into())
        };
        assert_eq!(consumed_reserve(&before, &after, "p").unwrap(), expected);
        let index = usize::from(!vacant);
        for field in ["pool_id", "wallet_id", "recipient", "target"] {
            let mut bad = after.clone();
            bad["funding_jobs"][index][field] = json!("changed");
            assert_eq!(
                consumed_reserve(&before, &bad, "p")
                    .unwrap_err()
                    .to_string(),
                if vacant {
                    "vacant consumption changed funding identity"
                } else {
                    "funding identity changed"
                }
            );
        }
        let mut missing = after.clone();
        missing["funding_jobs"]
            .as_array_mut()
            .unwrap()
            .remove(index);
        assert_eq!(
            consumed_reserve(&before, &missing, "p")
                .unwrap_err()
                .to_string(),
            if vacant {
                "vacant consumption lost funding history"
            } else {
                "consumption lost funding job"
            }
        );
        after["funding_jobs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"unrelated","pool_id":"other"}));
        assert_eq!(consumed_reserve(&before, &after, "p").unwrap(), expected);
        after["funding_jobs"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["pool_id"] = json!("p");
        assert_eq!(
            consumed_reserve(&before, &after, "p")
                .unwrap_err()
                .to_string(),
            if vacant {
                "vacant consumption unexpectedly allocated selected inventory"
            } else {
                "replacement funding correlation differs"
            }
        );
    }
    let (before, mut vacant) = capacity_vacant();
    vacant["pools"][0]["addresses"]
        .as_array_mut()
        .unwrap()
        .push(wallet("extra", 9, "ALLOCATED"));
    assert_eq!(
        consumed_reserve(&before, &vacant, "p")
            .unwrap_err()
            .to_string(),
        "vacant consumption changed inventory"
    );
    let (mut before, mut vacant) = capacity_vacant();
    before["pools"][0]["addresses"][0]["address"] = json!("opaque");
    vacant["pools"][0]["addresses"][0]["address"] = json!("opaque");
    assert_eq!(consumed_reserve(&before, &vacant, "p").unwrap(), None);
}

#[test]
fn capacity_replacement_identity_correlation_and_readiness_matrix() {
    for (pointer, value, error) in [
        (
            "/pools/0/reserve_claims/0/wallet_id",
            json!(7),
            "replacement not yet allocated",
        ),
        (
            "/pools/0/reserve_claims/0/wallet_id",
            json!("missing"),
            "replacement wallet missing",
        ),
        (
            "/pools/0/reserve_claims/0/wallet_id",
            json!("old"),
            "consumption did not retain inventory and allocate exactly its replacement",
        ),
        (
            "/pools/0/addresses/2/address",
            Value::Null,
            "replacement address missing",
        ),
        (
            "/funding_jobs",
            json!([]),
            "replacement funding job missing",
        ),
        (
            "/funding_jobs/0/wallet_id",
            json!("other"),
            "replacement funding correlation differs",
        ),
        (
            "/funding_jobs/0/recipient",
            json!("other"),
            "replacement funding correlation differs",
        ),
        (
            "/funding_jobs/0/target",
            json!("1"),
            "replacement funding correlation differs",
        ),
        (
            "/pools/0/addresses/2/role",
            json!("RETIRED"),
            "invalid replacement role",
        ),
        (
            "/pools/0/addresses/2/role",
            json!("READY"),
            "replacement readiness lacks exact destination support",
        ),
    ] {
        let (before, mut after) = capacity_fixture();
        *after.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            consumed_reserve(&before, &after, "p")
                .unwrap_err()
                .to_string(),
            error,
            "{pointer}"
        );
    }
    let (before, mut after) = capacity_fixture();
    after["pools"][0]["addresses"][2]["address"] = json!("invalid");
    assert!(consumed_reserve(&before, &after, "p").is_err());
    let (before, mut after) = capacity_fixture();
    after["pools"][0]["addresses"]
        .as_array_mut()
        .unwrap()
        .push(wallet("extra", 9, "ALLOCATED"));
    assert_eq!(
        consumed_reserve(&before, &after, "p")
            .unwrap_err()
            .to_string(),
        "consumption did not retain inventory and allocate exactly its replacement"
    );
    for role in ["ALLOCATED", "READY"] {
        for recorded in [false, true] {
            for ready in [false, true] {
                let (before, mut after) = capacity_fixture();
                after["pools"][0]["addresses"][2]["role"] = json!(role);
                after["pools"][0]["addresses"][2]["destination_recorded"] = json!(recorded);
                after["pools"][0]["reserve_claims"][0]["destination_ready"] = json!(ready);
                after["funding_jobs"][0]["phase"] = json!("SWAPPING");
                if (role == "READY") == recorded && (role != "READY" || ready) {
                    assert_eq!(
                        consumed_reserve(&before, &after, "p").unwrap(),
                        Some("replacement".into())
                    );
                } else {
                    assert_eq!(
                        consumed_reserve(&before, &after, "p")
                            .unwrap_err()
                            .to_string(),
                        "replacement readiness lacks exact destination support"
                    );
                }
            }
        }
    }
}

#[test]
fn capacity_competing_failures_preserve_validation_order() {
    let (mut before, mut after) = capacity_fixture();
    before["schema_version"] = json!(1);
    after["pools"] = Value::Null;
    assert_eq!(
        consumed_reserve(&before, &after, "p")
            .unwrap_err()
            .to_string(),
        "capacity lifecycle schema/treasury mismatch"
    );
    let (mut before, mut after) = capacity_fixture();
    before["pools"][0]["reserve_claims"][0]["enabled"] = json!(false);
    after["pools"][0]["addresses"][0]["address"] = json!("changed");
    after["pools"][0]["addresses"][2]["address"] = json!("invalid");
    assert_eq!(
        consumed_reserve(&before, &after, "p")
            .unwrap_err()
            .to_string(),
        "consumed reserve was not ready"
    );
    before["pools"][0]["reserve_claims"][0]["enabled"] = json!(true);
    assert_eq!(
        consumed_reserve(&before, &after, "p")
            .unwrap_err()
            .to_string(),
        "consumption changed immutable wallet identity"
    );
    let (mut before, mut after) = capacity_fixture();
    before["pools"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"other","addresses":[wallet("history",3,"RETIRED")]}));
    after["funding_jobs"][0]["recipient"] = json!("wrong");
    after["pools"][0]["addresses"][2]["role"] = json!("READY");
    assert_eq!(
        consumed_reserve(&before, &after, "p")
            .unwrap_err()
            .to_string(),
        "replacement reused an address"
    );
    before["pools"][1]["addresses"][0] = wallet("history", 4, "RETIRED");
    assert_eq!(
        consumed_reserve(&before, &after, "p")
            .unwrap_err()
            .to_string(),
        "replacement funding correlation differs"
    );
}
