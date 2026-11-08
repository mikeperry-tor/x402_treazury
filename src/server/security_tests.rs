use super::*;

#[test]
fn listener_authority_rejects_empty_tokens_and_missing_host_defense() {
    for hosts in [
        host::HostPolicy::new(None, true).unwrap(),
        host::HostPolicy::new(Some(vec![]), false).unwrap(),
    ] {
        assert!(hosts.validate_auth(None).is_err());
        hosts.validate_auth(Some("fixture")).unwrap();
    }
    let policy = host::HostPolicy::default();
    policy.validate_auth(None).unwrap();
    for token in ["", " \t"] {
        assert!(policy.validate_auth(Some(token)).is_err());
    }
    for token in [None, Some("".to_owned()), Some(" \t".to_owned())] {
        let mut server = Server::new(
            vec![],
            crate::payment::PaidClient::unsigned(),
            "https://example.com".into(),
            None,
        );
        server.host_policy = host::HostPolicy::new(None, true).unwrap();
        assert!(http_app_with_auth(server, token).is_err());
    }
}

#[test]
fn empty_cap_requires_explicit_unlimited_authority() {
    for value in ["", " ", "\t"] {
        assert!(crate::payment::SpendPolicy::dollars(value).is_err());
    }
    for value in ["none", "off", "NONE", "OFF"] {
        assert!(
            crate::payment::SpendPolicy::dollars(value)
                .unwrap()
                .max_atomic
                .is_none()
        );
    }
    assert_eq!(
        crate::payment::SpendPolicy::dollars("0")
            .unwrap()
            .max_atomic,
        Some(alloy_primitives::U256::ZERO)
    );
}
