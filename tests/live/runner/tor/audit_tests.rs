fn optional_identities() -> Identities {
    let (mut ids, _) = fixture();
    for id in ids.values_mut() {
        id.required = false;
    }
    ids
}

fn audit_error(ids: &Identities, lines: &[&str]) -> String {
    verify(
        ids,
        &lines
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>(),
    )
    .unwrap_err()
    .to_string()
}

#[test]
fn complete_audit_counts_streams_once_and_retains_all_owned_circuits() {
    let mut ids = optional_identities();
    ids.get_mut("catalog").unwrap().required_targets =
        BTreeSet::from(["provider.example:443".into()]);
    ids.get_mut("old").unwrap().required = true;
    let events = [
        "650 CIRC 99 ARBITRARY ignored SOCKS_USERNAME=unknown SOCKS_USERNAME=duplicate",
        "650 STREAM 90 SUCCEEDED 1 invalid-target ignored-fragment",
        "650 STREAM 91 NEW 0 invalid-target PURPOSE=OTHER",
        "650 STREAM 1 NEW 0 PROVIDER.EXAMPLE:443 SOCKS_USERNAME=u SOCKS_PASSWORD=old ignored-fragment",
        "650 STREAM 1 SUCCEEDED 1 192.0.2.1:443",
        "650 STREAM 1 SUCCEEDED 2 entirely-ignored-target",
        "650 STREAM 1 CLOSED 3 ignored-target",
        "650 STREAM 2 NEW 0 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=old",
        "650 STREAM 2 SUCCEEDED 1 ignored-target",
        "650 STREAM 3 NEW 0 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=catalog",
        "650 STREAM 3 NEW 0 PROVIDER.EXAMPLE:443 PURPOSE=USER",
        "650 STREAM 3 SUCCEEDED 4 ignored-target",
    ].map(str::to_owned);
    let result = verify(&ids, &events).unwrap();
    assert_eq!(
        result,
        json!({"identities":{
        "catalog":{"kind":"discovery","required":false,"permitted":true,"successful_streams":1,"required_destinations":1,"circuits":1,"status":"observed"},
        "new":{"kind":"evm","required":false,"permitted":true,"successful_streams":0,"required_destinations":0,"circuits":0,"status":"not_observed"},
        "old":{"kind":"evm","required":true,"permitted":true,"successful_streams":2,"required_destinations":0,"circuits":3,"status":"observed"},
        "treasury":{"kind":"treasury","required":false,"permitted":true,"successful_streams":0,"required_destinations":0,"circuits":0,"status":"not_observed"}},
        "scope":"Tor STREAM identity separation; not payment, settlement or complete unlinkability"})
    );
    for private in ["provider.example", "SOCKS_", "192.0.2.1", "unknown"] {
        assert!(!result.to_string().contains(private));
    }
}

#[test]
fn identity_passes_precede_events_and_required_target_validation() {
    let bad_event = ["invalid"];
    assert_eq!(
        audit_error(&Identities::new(), &bad_event),
        "empty Tor identity map"
    );
    let mut ids = optional_identities();
    ids.get_mut("catalog").unwrap().kind = "unknown".into();
    ids.get_mut("catalog").unwrap().targets.clear();
    assert_eq!(audit_error(&ids, &bad_event), "unknown Tor identity kind");
    ids.get_mut("catalog").unwrap().kind = "discovery".into();
    assert_eq!(
        audit_error(&ids, &bad_event),
        "discovery identity lacks destination constraint"
    );
    let mut ids = optional_identities();
    ids.get_mut("catalog")
        .unwrap()
        .required_targets
        .insert("forbidden:443".into());
    ids.get_mut("new").unwrap().password = "catalog".into();
    assert_eq!(
        audit_error(&ids, &bad_event),
        "distinct identity labels share SOCKS credentials"
    );
    ids.get_mut("new").unwrap().password = "new".into();
    assert_eq!(
        audit_error(&ids, &bad_event),
        "required destinations exceed identity permissions"
    );
    ids.get_mut("catalog").unwrap().required_targets.clear();
    assert_eq!(
        audit_error(&ids, &bad_event),
        "non-event in Tor event evidence"
    );
    let oversized = vec![String::new(); 100001];
    assert_eq!(
        verify(&ids, &oversized).unwrap_err().to_string(),
        "Tor event audit exceeds 100000-event limit"
    );
    assert_eq!(
        verify(&Identities::new(), &oversized)
            .unwrap_err()
            .to_string(),
        "empty Tor identity map"
    );
}

#[test]
fn stream_shape_attributes_credentials_and_permissions_keep_error_order() {
    let ids = optional_identities();
    for (event, expected) in [
        ("650 OTHER", "unrequested or malformed Tor event"),
        ("650 CIRC 1", "malformed Tor CIRC event"),
        ("650 CIRC 0 BUILT", "noncanonical Tor identifier"),
        ("650 STREAM", "malformed Tor STREAM event"),
        (
            "650 STREAM 01 NEW 0 bad SOCKS_USERNAME=u SOCKS_USERNAME=u",
            "noncanonical Tor identifier",
        ),
        (
            "650 STREAM 1 NEW 0 bad SOCKS_USERNAME=u SOCKS_USERNAME=u",
            "duplicate Tor STREAM attribute",
        ),
        (
            "650 STREAM 1 NEW 0 bad SOCKS_PASSWORD=old",
            "Tor stream missing SOCKS username",
        ),
        (
            "650 STREAM 1 NEW 0 bad SOCKS_USERNAME=u",
            "Tor stream missing SOCKS password",
        ),
        (
            "650 STREAM 1 NEW 0 bad SOCKS_USERNAME=u SOCKS_PASSWORD=unknown",
            "authenticated Tor stream has an unknown identity",
        ),
        (
            "650 STREAM 1 NEW 0 bad PURPOSE=USER",
            "Tor USER stream lacks isolation credentials",
        ),
        (
            "650 STREAM 1 SUCCEEDED 1 bad SOCKS_USERNAME=u SOCKS_PASSWORD=old",
            "successful stream lacks remote-host/circuit evidence",
        ),
    ] {
        assert_eq!(audit_error(&ids, &[event]), expected, "{event}");
    }
    let first = "650 STREAM 1 NEW 0 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=old";
    assert_eq!(
        audit_error(
            &ids,
            &[
                first,
                "650 STREAM 1 CLOSED 1 ignored SOCKS_USERNAME=u SOCKS_PASSWORD=unknown"
            ]
        ),
        "Tor stream credentials changed"
    );
    assert_eq!(
        audit_error(&ids, &[first, "650 STREAM 1 SUCCEEDED 0 ignored"]),
        "successful stream lacks remote-host/circuit evidence"
    );
    let mut forbidden = ids.clone();
    forbidden.get_mut("old").unwrap().permitted = false;
    assert_eq!(
        audit_error(
            &forbidden,
            &["650 STREAM 1 ARBITRARY 1 bad SOCKS_USERNAME=u SOCKS_PASSWORD=old"]
        ),
        "known Tor identity is not permitted in this phase"
    );
    // Merely retaining a forbidden historical identity is still accepted.
    assert_eq!(
        verify(&forbidden, &[]).unwrap()["identities"]["old"]["permitted"],
        false
    );
}

#[test]
fn destinations_and_circuit_ownership_precede_success_and_completeness() {
    let ids = optional_identities();
    let first = "650 STREAM 1 NEW 1 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=catalog";
    for (target, expected) in [
        ("provider.example", "Tor stream target lacks port"),
        ("provider.example:0", "invalid Tor stream target"),
        ("provider.example:65536", "invalid Tor stream target"),
        (
            "192.0.2.1:443",
            "Tor stream used a locally resolved address or noncanonical hostname",
        ),
        (
            "other.example:443",
            "Tor identity used an unexpected destination",
        ),
    ] {
        let event = format!("650 STREAM 1 NEW 0 {target} SOCKS_USERNAME=u SOCKS_PASSWORD=catalog");
        assert_eq!(audit_error(&ids, &[&event]), expected);
    }
    assert_eq!(
        audit_error(
            &ids,
            &[
                first,
                "650 STREAM 2 SUCCEEDED 1 ignored SOCKS_USERNAME=u SOCKS_PASSWORD=old"
            ]
        ),
        "distinct identities shared a Tor circuit"
    );
    let mut broader = ids.clone();
    broader
        .get_mut("catalog")
        .unwrap()
        .targets
        .insert("other.example:443".into());
    assert_eq!(
        audit_error(&broader, &[first, "650 STREAM 1 NEW 0 other.example:443"]),
        "Tor stream destination changed"
    );
    // Permission rejects an unlisted replacement before changed-target checking.
    assert_eq!(
        audit_error(&ids, &[first, "650 STREAM 1 NEW 0 other.example:443"]),
        "Tor identity used an unexpected destination"
    );
}

#[test]
fn required_success_and_destinations_are_checked_in_label_order() {
    let mut ids = optional_identities();
    let catalog = ids.get_mut("catalog").unwrap();
    catalog.required = true;
    catalog
        .required_targets
        .insert("provider.example:443".into());
    ids.get_mut("old").unwrap().required = true;
    assert_eq!(
        audit_error(&ids, &[]),
        "required Tor identity catalog has no successful remote-host stream"
    );
    ids.get_mut("catalog").unwrap().required = false;
    assert_eq!(
        audit_error(&ids, &[]),
        "required Tor identity catalog lacks a successful stream to a required destination"
    );
    let catalog = ids.get_mut("catalog").unwrap();
    catalog.targets.insert("other.example:443".into());
    let events = [
        "650 STREAM 1 NEW 0 other.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=catalog",
        "650 STREAM 1 SUCCEEDED 1 ignored",
    ];
    assert_eq!(
        audit_error(&ids, &events),
        "required Tor identity catalog lacks a successful stream to a required destination"
    );
    let events = [
        "650 STREAM 1 NEW 0 provider.example:443 SOCKS_USERNAME=u SOCKS_PASSWORD=catalog",
        "650 STREAM 1 SUCCEEDED 1 ignored",
    ];
    assert_eq!(
        audit_error(&ids, &events),
        "required Tor identity old has no successful remote-host stream"
    );
}
