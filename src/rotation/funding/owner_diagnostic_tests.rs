use super::*;

#[test]
fn terminal_owner_failure_reports_typed_causes_without_error_prose() {
    if crate::test_log_capture::run_in_child(
        "rotation::funding::owner_diagnostic_tests::terminal_owner_failure_reports_typed_causes_without_error_prose",
    ) {
        return;
    }
    let sqlite = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER),
        Some("private sqlite credential".into()),
    );
    let io = std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "private path credential",
    );
    let json =
        serde_json::from_str::<serde_json::Value>("private malformed credential").unwrap_err();
    for (error, expected) in [
        (
            anyhow::Error::new(sqlite),
            vec![
                "category=\"sqlite_failure\"",
                "sqlite_code=\"ConstraintViolation\"",
                "sqlite_extended_code=1811",
            ],
        ),
        (
            anyhow::Error::new(io),
            vec!["category=\"io_failure\"", "io_kind=\"PermissionDenied\""],
        ),
        (
            anyhow::Error::new(json),
            vec!["category=\"json_failure\"", "json_category=\"Syntax\""],
        ),
        (
            anyhow::anyhow!("private untyped credential"),
            vec!["category=\"internal_failure\""],
        ),
    ] {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::ERROR)
            .with_writer(log.reopen().unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            report_owner_failure(&error.context("private outer context credential"));
        });
        let logged = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(logged.lines().count(), 1, "{logged}");
        assert!(
            logged.contains("ERROR") && logged.contains("Automatic wallet funding stopped"),
            "{logged}"
        );
        assert!(
            logged.contains("accepted financial work remains recorded"),
            "{logged}"
        );
        for expected in expected {
            assert!(logged.contains(expected), "missing {expected}: {logged}");
        }
        assert!(
            !logged.contains("private") && !logged.contains("credential"),
            "{logged}"
        );
    }
}
