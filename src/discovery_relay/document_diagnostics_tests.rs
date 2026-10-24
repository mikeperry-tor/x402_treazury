use super::*;

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
            socks_endpoint: Some(proxy.address),
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
