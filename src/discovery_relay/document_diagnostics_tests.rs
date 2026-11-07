use super::*;

#[test]
fn relay_http_status_keeps_specific_categories_through_shared_failure_caching() {
    for (status, kind) in [
        (StatusCode::FORBIDDEN, "access denied"),
        (StatusCode::NOT_FOUND, "not found"),
        (StatusCode::TOO_MANY_REQUESTS, "rate limited"),
        (StatusCode::SERVICE_UNAVAILABLE, "provider unavailable"),
    ] {
        let error = anyhow::Error::new(super::super::RelayFailure::http_status(status.as_u16()))
            .context(crate::payment::ResponseFacts::provider(status))
            .context(crate::payment::ApiExecution { non_read: false });
        let expected = format!("Provider API HTTP {status}: {kind}.");
        assert_eq!(crate::payment::present_api_error(&error).unwrap(), expected);
        let shared = SharedFailure::capture(&error, "cached relay unavailable");
        assert_eq!(shared.status(), Some(status.as_u16()));
        assert_eq!(
            crate::payment::present_api_error(&shared.error()).unwrap(),
            expected
        );
    }
}

#[test]
fn relay_admission_reasons_survive_presentation_and_shared_failure_caching() {
    use crate::{discovery_relay::RelayFailure, rotation::error::AdmissionError};
    for (admission, status, possible, expected) in [
        (
            AdmissionError::PriceLimit("PRIVATE payment cap"),
            Some(StatusCode::PAYMENT_REQUIRED),
            false,
            "Provider API HTTP 402 Payment Required: The requested payment exceeds the authorized amount.",
        ),
        (
            AdmissionError::WalletNotReady("PRIVATE wallet identity"),
            None,
            false,
            "No eligible wallet is ready for this payment.",
        ),
        (
            AdmissionError::OutcomeUnknown("PRIVATE transport context"),
            None,
            true,
            "The payment outcome is unknown; reconciliation is required. Do not retry automatically.",
        ),
    ] {
        let original = anyhow::Error::new(admission).context("PRIVATE upstream prose");
        let safe = RelayFailure::from_error(&original);
        let mut failure = TargetFailure::new(safe.kind(), safe.status(), false);
        failure.response = status.map(crate::payment::ResponseFacts::provider);
        let error = failure.error(possible);
        assert_eq!(crate::payment::present_api_error(&error).unwrap(), expected);
        let shared = SharedFailure::capture(&error, "cached help unavailable");
        assert_eq!(shared.status(), status.map(|status| status.as_u16()));
        assert_eq!(shared.possible_payment(), possible);
        let cached = shared.error();
        assert_eq!(
            crate::payment::present_api_error(&cached).unwrap(),
            expected
        );
        assert!(!format!("{cached:#}").contains("PRIVATE"));
        // A second cache boundary retains the already-redacted RelayFailure too.
        let shared_again = SharedFailure::capture(&cached, "shared acquisition unavailable");
        assert_eq!(
            crate::payment::present_api_error(&shared_again.error()).unwrap(),
            expected
        );
    }
}

fn capture(kind: RequestKind, failure: &Failure, level: tracing::Level) -> String {
    let logs = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(level)
        .with_writer(logs.reopen().unwrap())
        .finish();
    let mut options = Options::new(
        "https://PRIVATE_HOST/PRIVATE_PATH?token=PRIVATE_TOKEN",
        IsolationId::bootstrap(),
        1024,
        "max_help_bytes",
    );
    options.kind = kind;
    tracing::subscriber::with_default(subscriber, || {
        log_origin_failure(&options, failure, (1, 3));
    });
    std::fs::read_to_string(logs.path()).unwrap()
}

#[test]
fn speculative_failures_are_debug_and_hidden_at_info() {
    if crate::test_log_capture::run_in_child(
        "discovery_relay::document::diagnostics_tests::speculative_failures_are_debug_and_hidden_at_info",
    ) {
        return;
    }
    let failure = Failure::status(
        anyhow::anyhow!("PRIVATE_UPSTREAM_PROSE"),
        StatusCode::NOT_FOUND,
        &HeaderMap::new(),
        FallbackPolicy::BlockingFailures,
    );
    for kind in [
        RequestKind::OpenApiProbe,
        RequestKind::DirectoryProbe,
        RequestKind::DocumentationProbe,
    ] {
        assert!(capture(kind, &failure, tracing::Level::INFO).is_empty());
        let logs = capture(kind, &failure, tracing::Level::DEBUG);
        assert!(
            logs.contains("DEBUG") && logs.contains("http_status=404"),
            "{logs}"
        );
        assert!(
            !logs.contains("WARN") && !logs.contains("PRIVATE_"),
            "{logs}"
        );
    }
    let logs = capture(RequestKind::Document, &failure, tracing::Level::INFO);
    assert!(
        logs.contains("INFO") && logs.contains("document does not exist"),
        "{logs}"
    );
    assert!(!logs.contains("PRIVATE_"), "{logs}");
}

#[test]
fn partial_document_reason_is_independent_of_http_status() {
    if crate::test_log_capture::run_in_child(
        "discovery_relay::document::diagnostics_tests::partial_document_reason_is_independent_of_http_status",
    ) {
        return;
    }
    for status in [200, 206] {
        let failure = Failure::validation(IncompleteDocument { status }.into());
        let logs = capture(RequestKind::Document, &failure, tracing::Level::INFO);
        assert!(logs.contains(&format!("http_status={status}")), "{logs}");
        assert!(
            logs.contains("partial response rejected; a complete document is required"),
            "{logs}"
        );
        assert!(logs.contains("relay_fallback_eligible=false"), "{logs}");
        assert!(!logs.contains("unsuccessful HTTP status"), "{logs}");
    }
}

#[test]
fn byte_limit_rejection_retains_safe_fields_at_the_callers_severity() {
    if crate::test_log_capture::run_in_child(
        "discovery_relay::document::diagnostics_tests::byte_limit_rejection_retains_safe_fields_at_the_callers_severity",
    ) {
        return;
    }
    let failure = Failure::body(
        crate::limits::DownloadLimit {
            kind: "document",
            setting: "max_help_bytes",
            limit: 7,
        }
        .into(),
    );
    for kind in [RequestKind::Document, RequestKind::DocumentationProbe] {
        let logs = capture(kind, &failure, tracing::Level::DEBUG);
        for expected in [
            "document exceeds the download limit",
            "limit_bytes=7",
            "setting=\"max_help_bytes\"",
            "relay_fallback_eligible=false",
        ] {
            assert!(logs.contains(expected), "{logs}");
        }
        assert!(!logs.contains("WARN"), "{logs}");
        assert_eq!(
            logs.contains("INFO"),
            kind == RequestKind::Document,
            "{logs}"
        );
    }
}

#[test]
fn document_warning_waits_for_the_complete_fallback_outcome() {
    const CHILD: &str = "TREAZURY_DOCUMENT_DIAGNOSTICS_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fallback_diagnostics_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "discovery_relay::document::diagnostics_tests::document_warning_waits_for_the_complete_fallback_outcome", "--nocapture"])
        .env(CHILD, "1").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn fallback_diagnostics_fixture() {
    use crate::network::{Mode, NetworkContext, NetworkPolicy};
    use axum::{
        Router,
        http::StatusCode,
        routing::{get, post},
    };
    use std::collections::BTreeMap;
    use tracing::instrument::WithSubscriber;

    let app = Router::new()
        .route("/read", get(|| async { StatusCode::FORBIDDEN }))
        .route("/failed", post(|| async { StatusCode::NOT_IMPLEMENTED }))
        .route(
            "/success",
            post(|| async {
                axum::Json(serde_json::json!({
                    "url":"https://api.example.com/read", "method":"GET",
                    "statusCode":200, "headers":{}, "body":"complete",
                    "redirectChain":[], "error":null
                }))
            }),
        );
    let (address, server) = crate::test_tls::serve(app).await;
    let proxy = crate::test_socks::Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address.into()),
            tor_isolation_retries: Some(0),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    for (scenario, kind, debug) in [
        ("success", RequestKind::Document, false),
        ("success", RequestKind::Document, true),
        ("success", RequestKind::OpenApiProbe, false),
        ("success", RequestKind::OpenApiProbe, true),
        ("exhausted", RequestKind::Document, false),
        ("absent", RequestKind::Document, false),
        ("disabled", RequestKind::Document, false),
        ("no_authority", RequestKind::Document, false),
    ] {
        let logs = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_env_filter(if debug {
                "warn,x402_treazury=debug"
            } else {
                "warn,x402_treazury=info"
            })
            .with_writer(logs.reopen().unwrap())
            .finish();
        let mut entries = vec![(
            "curl".into(),
            "https://api.example.com/failed".into(),
            crate::catalog::Config::default(),
        )];
        if scenario == "success" {
            entries.push((
                "curl".into(),
                "https://api.example.com/success".into(),
                crate::catalog::Config::default(),
            ));
        }
        let mut relay =
            super::super::SearchRelay::fixture_methods(BTreeMap::from([("GET".into(), entries)]))
                .unwrap();
        if scenario == "disabled" {
            relay.policy.provider_metadata = false;
        }
        let client = if scenario == "no_authority" {
            PaidClient::unsigned()
        } else {
            PaidClient::new(
                crate::payment::Payer::new(
                    &format!("{:064x}", 91),
                    crate::payment::SpendPolicy::dollars("0.01").unwrap(),
                )
                .unwrap(),
            )
        }
        .with_document_relay((scenario != "absent").then(|| Arc::new(relay)));
        let mut options = Options::new(
            "https://api.example.com/read",
            IsolationId::bootstrap(),
            4096,
            "max_help_bytes",
        );
        options.kind = kind;
        let result = read(&client, &options).with_subscriber(subscriber).await;
        let logs = std::fs::read_to_string(logs.path()).unwrap();
        if kind == RequestKind::Document {
            assert!(
                logs.contains("INFO") && logs.contains("http_status=403"),
                "{scenario}: {logs}"
            );
        } else {
            assert!(!logs.contains("INFO"), "{logs}");
        }
        if scenario == "success" {
            assert_eq!(&*result.unwrap().body, b"complete");
            assert!(!logs.contains("WARN"), "{logs}");
            for detail in [
                "Relay payment attempt failed",
                "Configured relay request failed",
                "Payment call completed",
                "Requesting the target response from a configured paid relay",
            ] {
                assert_eq!(logs.contains(detail), debug, "{logs}");
            }
            assert_eq!(
                logs.contains(
                    "Configured paid relay selected as fallback to retrieve this document"
                ),
                kind == RequestKind::Document,
                "{logs}"
            );
        } else {
            assert!(result.is_err(), "{scenario}");
            let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
            assert_eq!(warnings.len(), 1, "{scenario}: {logs}");
            assert!(
                warnings[0].contains("paid fallback unavailable or failed"),
                "{logs}"
            );
        }
        assert!(!logs.contains("api.example.com"), "{logs}");
        client.drain().await;
    }
    server.abort();
}
