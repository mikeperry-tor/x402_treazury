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
    let logs = capture(RequestKind::Document, &failure, tracing::Level::WARN);
    assert!(
        logs.contains("WARN") && logs.contains("document does not exist"),
        "{logs}"
    );
    assert!(!logs.contains("PRIVATE_"), "{logs}");
}

#[test]
fn partial_document_reason_is_independent_of_http_status() {
    for status in [200, 206] {
        let failure = Failure::validation(IncompleteDocument { status }.into());
        let logs = capture(RequestKind::Document, &failure, tracing::Level::WARN);
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
        assert_eq!(
            logs.contains("WARN"),
            kind == RequestKind::Document,
            "{logs}"
        );
    }
}
