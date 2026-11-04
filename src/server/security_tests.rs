use super::*;

#[tokio::test]
async fn disabled_auth_and_host_checks_emit_separate_warnings() {
    if crate::test_log_capture::run_in_child(
        "server::security_tests::disabled_auth_and_host_checks_emit_separate_warnings",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(log.reopen().unwrap())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let mut server = Server::new(
            vec![],
            crate::payment::PaidClient::unsigned(),
            "https://example.com".into(),
            None,
        );
        server.host_policy = host::HostPolicy::new(None, true).unwrap();
        let _app = http_app_with_auth(server, None);
    });
    let text = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(text.matches("http_auth_disabled").count(), 1, "{text}");
    assert_eq!(
        text.matches("http_host_check_disabled").count(),
        1,
        "{text}"
    );
    assert_eq!(text.matches("WARN").count(), 2, "{text}");
}

#[test]
fn empty_cap_warns_without_changing_explicit_limits() {
    if crate::test_log_capture::run_in_child(
        "server::security_tests::empty_cap_warns_without_changing_explicit_limits",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(log.reopen().unwrap())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        for value in ["", "none", "off", "NONE", "OFF"] {
            assert!(
                crate::payment::SpendPolicy::dollars(value)
                    .unwrap()
                    .max_atomic
                    .is_none()
            );
        }
        assert_eq!(
            crate::payment::SpendPolicy::dollars("1.00")
                .unwrap()
                .max_atomic,
            Some(alloy_primitives::U256::from(1_000_000))
        );
        assert!(crate::payment::SpendPolicy::dollars("invalid").is_err());
        assert!(crate::rotation::config::positive_usdc("").is_err());
        assert!(crate::rotation::config::retirement_remainder("").is_err());
        assert!(crate::qualification::budget::atomic("").is_err());
    });
    let text = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(
        text.matches("implicit_unlimited_payment_cap").count(),
        1,
        "{text}"
    );
    assert!(text.contains("none") && text.contains("off"), "{text}");
}

#[test]
fn listener_addresses_require_loopback_for_both_ip_families() {
    for address in ["127.0.0.1", "127.0.0.2", "::1"] {
        host::require_loopback(address.parse().unwrap()).unwrap();
    }
    for address in ["0.0.0.0", "192.168.1.1", "::", "2001:db8::1"] {
        assert!(host::require_loopback(address.parse().unwrap()).is_err());
    }
}
