//! Safe observations only; these fields never grant payment or retry authority.
use super::SubmissionProgress;
use anyhow::Result;
use std::{cell::Cell, future::Future};

/// Provider-reported categories are diagnostic claims, never replay authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProviderFailure {
    PaymentSettlementFailed,
}

impl ProviderFailure {
    pub(super) fn from_body(body: &[u8]) -> Option<Self> {
        let body: serde_json::Value = serde_json::from_slice(body).ok()?;
        match body.pointer("/error/code")?.as_str()? {
            "payment_settlement_failed" => Some(Self::PaymentSettlementFailed),
            _ => None,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::PaymentSettlementFailed => "payment_settlement_failed",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::PaymentSettlementFailed => {
                "The provider reported payment settlement failure; the result remains unverified"
            }
        }
    }
}

/// Admission errors can include configuration detail: emit only fixed categories.
pub(super) fn cover_admission(error: &anyhow::Error) -> (tracing::Level, &'static str) {
    match error.to_string().as_str() {
        "cover_owner_limit" => (tracing::Level::WARN, "cover_owner_limit"),
        "cover_active_episode_limit" => (tracing::Level::WARN, "cover_active_episode_limit"),
        "cover_owner_draining" => (tracing::Level::DEBUG, "cover_owner_draining"),
        "cover_shutdown" => (tracing::Level::DEBUG, "cover_shutdown"),
        _ => (tracing::Level::INFO, "cover_configuration_unavailable"),
    }
}

#[derive(Clone, Copy)]
struct Observation {
    stage: &'static str,
    last_http_status: Option<u16>,
    unsigned_attempt: Option<usize>,
    max_unsigned_attempts: Option<usize>,
}
tokio::task_local! {
    static OBSERVATION: Cell<Observation>;
}

/// Scope inside retained work so cancellation of its waiter cannot lose context.
pub(crate) async fn scope<T>(work: impl Future<Output = T>) -> T {
    OBSERVATION
        .scope(
            Cell::new(Observation {
                stage: "preparation",
                last_http_status: None,
                unsigned_attempt: None,
                max_unsigned_attempts: None,
            }),
            work,
        )
        .await
}

pub(crate) fn span(source: &str, method: &str) -> tracing::Span {
    // Keep correlation on WARN events when the operator filters out INFO.
    // Creating a span does not itself emit an event.
    tracing::warn_span!("payment_call", call_id = %uuid::Uuid::new_v4(), source, method)
}

pub(crate) fn stage(stage: &'static str) {
    let _ = OBSERVATION.try_with(|cell| {
        cell.set(Observation {
            stage,
            ..cell.get()
        })
    });
}

pub(crate) fn unsigned_attempt(attempt: usize, max_attempts: usize) {
    let _ = OBSERVATION.try_with(|cell| {
        cell.set(Observation {
            unsigned_attempt: Some(attempt),
            max_unsigned_attempts: Some(max_attempts),
            ..cell.get()
        });
    });
}

/// Fixed categories only: error display/source text can contain credentials.
pub(crate) fn transport_reason(error: &reqwest::Error) -> &'static str {
    if crate::network::tls_failure(error) {
        "tls_failure"
    } else if error.is_timeout() && error.is_connect() {
        "connection_timeout"
    } else if error.is_timeout() {
        "read_inactivity"
    } else if error.is_connect() {
        "connection"
    } else if error.is_body() {
        "body"
    } else if error.is_decode() {
        "decode"
    } else if error.is_builder() {
        "request_construction"
    } else {
        "transport"
    }
}

pub(crate) fn response(response: &reqwest::Response) {
    let _ = OBSERVATION.try_with(|cell| {
        cell.set(Observation {
            last_http_status: Some(response.status().as_u16()),
            ..cell.get()
        });
    });
}

pub(crate) fn finish<T>(result: &Result<T>, progress: &SubmissionProgress, relay_call: bool) {
    let kind = match result {
        Err(error) if error.is::<super::PrivatePayerHandoff>() => "private_payer_handoff",
        Err(error) if error.is::<super::HttpFailure>() => "http_status",
        Err(error) if error.is::<crate::rotation::error::AdmissionError>() => {
            "payment_admission_or_dispatch"
        }
        Err(error) if error.is::<reqwest::Error>() => "http_transport_or_body",
        _ => "operation",
    };
    let transport = result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<reqwest::Error>());
    let reason = result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<crate::rotation::error::AdmissionError>())
        .map(admission_reason);
    let provider_failure = result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<super::HttpFailure>())
        .and_then(|error| error.provider_failure);
    emit(
        result.is_ok(),
        kind,
        transport,
        progress,
        relay_call,
        reason.or_else(|| provider_failure.map(ProviderFailure::reason)),
        provider_failure.map(ProviderFailure::code),
    );
}

/// Direct callers own body consumption; observe only the returned headers here.
pub(crate) fn finish_response(response: &reqwest::Response, progress: &SubmissionProgress) {
    self::response(response);
    emit(
        response.status().is_success(),
        "http_status",
        None,
        progress,
        false,
        None,
        None,
    );
}

/// Only the typed category is safe here; even static error detail can contain
/// provider-authored or credential-bearing text.
fn admission_reason(error: &crate::rotation::error::AdmissionError) -> &'static str {
    use crate::rotation::error::AdmissionError;
    match error {
        AdmissionError::PayerChanged => {
            "The wallet assigned to this payment changed before authorization"
        }
        AdmissionError::UnsupportedPayment(_) => {
            "The server requested an unsupported payment format or requirement"
        }
        AdmissionError::PriceLimit(_) => "The requested payment exceeds the authorized amount",
        AdmissionError::PaymentPending(_) => "A payment is still pending",
        AdmissionError::WalletNotReady(_) => "No eligible wallet is ready for this payment",
        AdmissionError::FundingUnavailable(_) => "Wallet funding is unavailable",
        AdmissionError::FundingRestricted | AdmissionError::FundingPermitDenied => {
            "This operation does not have authority to start new wallet funding"
        }
        AdmissionError::OutcomeUnknown(_) => {
            "The payment outcome is unknown; reconciliation is required"
        }
        AdmissionError::ChainRecoveryRequired(_) => {
            "Blockchain reconciliation is required before this wallet can continue"
        }
    }
}

/// Human explanations complement stable diagnostic codes without exposing errors.
fn failure_reason(kind: &str, transport: Option<&str>) -> &'static str {
    match (kind, transport) {
        (_, Some("tls_failure")) => "The secure connection failed TLS validation or negotiation",
        (_, Some("connection_timeout")) => "The connection to the server or proxy timed out",
        (_, Some("read_inactivity")) => {
            "The server stopped sending data before the response was complete"
        }
        (_, Some("connection")) => "Could not connect to the server or configured proxy",
        (_, Some("body")) => "The response body could not be read completely",
        (_, Some("decode")) => "The server response could not be decoded",
        (_, Some("request_construction")) => "The HTTP request could not be constructed",
        (_, Some(_)) => "The HTTP request or response failed during transport",
        ("http_status", None) => "The server returned an unsuccessful HTTP status",
        ("private_payer_handoff", None) => {
            "Private request headers cannot be sent under another wallet; this call has stopped"
        }
        _ => "The operation failed; no further cause information is available",
    }
}

fn emit(
    success: bool,
    failure_kind: &'static str,
    transport: Option<&reqwest::Error>,
    progress: &SubmissionProgress,
    relay_call: bool,
    reason: Option<&'static str>,
    provider_failure: Option<&'static str>,
) {
    let observation = OBSERVATION.with(Cell::get);
    let stage = observation.stage;
    let last_http_status = observation.last_http_status;
    let unsigned_attempt = observation.unsigned_attempt;
    let max_unsigned_attempts = observation.max_unsigned_attempts;
    let transport_reason = transport.map(transport_reason);
    let reason = reason.unwrap_or_else(|| failure_reason(failure_kind, transport_reason));
    let io = transport.and_then(|error| {
        let mut cause = std::error::Error::source(error);
        while let Some(error) = cause {
            if let Some(io) = error.downcast_ref::<std::io::Error>() {
                return Some(io);
            }
            cause = error.source();
        }
        None
    });
    let io_kind = io.map(std::io::Error::kind);
    let os_error = io.and_then(std::io::Error::raw_os_error);
    let authorization_may_charge = progress.possible();
    if success {
        tracing::debug!(
            stage,
            last_http_status,
            authorization_may_charge,
            "Payment call completed; completion does not establish settlement"
        );
    } else if relay_call && !authorization_may_charge {
        tracing::debug!(
            stage,
            last_http_status,
            failure_kind,
            provider_failure,
            reason,
            transport_reason,
            io_kind = ?io_kind,
            os_error,
            unsigned_attempt,
            max_unsigned_attempts,
            authorization_may_charge,
            "Relay payment attempt failed before any potentially chargeable authorization"
        );
    } else {
        tracing::warn!(
            stage,
            last_http_status,
            failure_kind,
            provider_failure,
            reason,
            transport_reason,
            io_kind = ?io_kind,
            os_error,
            unsigned_attempt,
            max_unsigned_attempts,
            authorization_may_charge,
            "{}",
            if authorization_may_charge {
                "Payment call failed; do not automatically repeat a call whose authorization may charge"
            } else {
                "Payment call failed before any potentially chargeable authorization"
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::instrument::WithSubscriber;

    #[test]
    fn provider_failure_requires_an_allowlisted_structured_code() {
        for body in [
            r#"{"error":"payment_settlement_failed"}"#,
            r#"{"error":{"message":"payment_settlement_failed"}}"#,
            r#"{"error":{"code":"unknown_private_code"}}"#,
            r#"{"error":{"code":["payment_settlement_failed"]}}"#,
            r#"{"code":"payment_settlement_failed"}"#,
            "payment_settlement_failed",
            "{}",
        ] {
            assert_eq!(ProviderFailure::from_body(body.as_bytes()), None, "{body}");
        }
        assert_eq!(
            ProviderFailure::from_body(br#"{"error":{"code":"payment_settlement_failed","message":"PRIVATE_PROVIDER_MESSAGE"}}"#),
            Some(ProviderFailure::PaymentSettlementFailed)
        );
    }

    #[tokio::test]
    async fn response_body_category_is_logged_without_provider_prose_or_replay_permission() {
        if crate::test_log_capture::run_in_child(
            "payment::diagnostics::tests::response_body_category_is_logged_without_provider_prose_or_replay_permission",
        ) {
            return;
        }
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        scope(async {
            let response = axum::http::Response::builder()
                .status(reqwest::StatusCode::PAYMENT_REQUIRED)
                .body(r#"{"error":{"code":"payment_settlement_failed","message":"PRIVATE_PROVIDER_MESSAGE","requestId":"PRIVATE_REQUEST_ID"}}"#)
                .unwrap();
            let response = reqwest::Response::from(response);
            self::response(&response);
            let result = super::super::PaidClient::unsigned()
                .response_output(response, true, false)
                .await;
            let error = result.as_ref().unwrap_err();
            assert_eq!(
                error.downcast_ref::<super::super::HttpFailure>().unwrap().provider_failure,
                Some(ProviderFailure::PaymentSettlementFailed)
            );
            let progress = SubmissionProgress::default();
            progress.mark_possible();
            finish(&result, &progress, false);
        })
        .with_subscriber(subscriber)
        .await;
        let logs = std::fs::read_to_string(log.path()).unwrap();
        for expected in [
            "WARN",
            "provider_failure=\"payment_settlement_failed\"",
            "The provider reported payment settlement failure; the result remains unverified",
            "authorization_may_charge=true",
            "do not automatically repeat",
        ] {
            assert!(logs.contains(expected), "{logs}");
        }
        assert!(!logs.contains("PRIVATE_"), "{logs}");
    }

    #[test]
    fn cover_admission_preserves_capacity_warnings_and_scopes_optional_failures() {
        for (code, level) in [
            ("cover_owner_limit", tracing::Level::WARN),
            ("cover_active_episode_limit", tracing::Level::WARN),
            ("cover_owner_draining", tracing::Level::DEBUG),
            ("cover_shutdown", tracing::Level::DEBUG),
        ] {
            assert_eq!(cover_admission(&anyhow::anyhow!(code)), (level, code));
        }
        assert_eq!(
            cover_admission(&anyhow::anyhow!("PRIVATE_CONFIGURATION_DETAIL")),
            (tracing::Level::INFO, "cover_configuration_unavailable")
        );
    }

    #[tokio::test]
    async fn transport_failure_reports_safe_cause_and_preserves_charge_caution() {
        if crate::test_log_capture::run_in_child(
            "payment::diagnostics::tests::transport_failure_reports_safe_cause_and_preserves_charge_caution",
        ) {
            return;
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{address}/private-path?token=private-token");
        let network = crate::network::NetworkContext::new(Default::default()).unwrap();
        let error = network
            .discovery(&url, None)
            .unwrap()
            .get(&url)
            .send()
            .await
            .unwrap_err();
        assert_eq!(transport_reason(&error), "connection");
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(log.reopen().unwrap())
            .finish();
        scope(async {
            stage("unsigned_request");
            unsigned_attempt(1, 3);
            let result: Result<()> = Err(error.into());
            finish(&result, &SubmissionProgress::default(), false);
            let progress = SubmissionProgress::default();
            progress.mark_possible();
            finish(&result, &progress, false);
            finish(&result, &SubmissionProgress::default(), true);
            finish(&result, &progress, true);
        })
        .with_subscriber(subscriber)
        .await;
        let logs = std::fs::read_to_string(log.path()).unwrap();
        let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
        assert_eq!(warnings.len(), 3, "{logs}");
        for expected in [
            "http_transport_or_body",
            "transport_reason=\"connection\"",
            "Could not connect to the server or configured proxy",
            "ConnectionRefused",
            "unsigned_attempt=1",
            "max_unsigned_attempts=3",
            "authorization_may_charge=false",
        ] {
            assert!(warnings[0].contains(expected), "{logs}");
        }
        assert!(
            !warnings[0].contains("do not automatically repeat"),
            "{logs}"
        );
        assert!(
            warnings[1].contains("authorization_may_charge=true"),
            "{logs}"
        );
        assert!(
            warnings[1].contains("do not automatically repeat"),
            "{logs}"
        );
        let relay_detail = logs
            .lines()
            .find(|line| line.contains("Relay payment attempt failed"))
            .unwrap();
        assert!(
            relay_detail.contains("DEBUG")
                && relay_detail.contains("authorization_may_charge=false"),
            "{logs}"
        );
        assert!(
            warnings[2].contains("authorization_may_charge=true"),
            "{logs}"
        );
        for private in ["private-path", "private-token", &address.to_string()] {
            assert!(!logs.contains(private), "{logs}");
        }
    }

    #[tokio::test]
    async fn routine_results_are_debug_and_terminal_uncertainty_is_explained_at_warn() {
        if crate::test_log_capture::run_in_child(
            "payment::diagnostics::tests::routine_results_are_debug_and_terminal_uncertainty_is_explained_at_warn",
        ) {
            return;
        }
        for debug in [false, true] {
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(if debug {
                    tracing::Level::DEBUG
                } else {
                    tracing::Level::INFO
                })
                .with_writer(log.reopen().unwrap())
                .finish();
            scope(async {
                // A later output/preparation failure must not blame an earlier 402.
                OBSERVATION.with(|cell| {
                    cell.set(Observation {
                        last_http_status: Some(402),
                        ..cell.get()
                    })
                });
                let progress = SubmissionProgress::default();
                let success: Result<()> = Ok(());
                finish(&success, &progress, false);
                let failure: Result<()> = Err(anyhow::anyhow!("PRIVATE_UPSTREAM_PROSE"));
                finish(&failure, &progress, true);
                progress.mark_possible();
                finish(&failure, &progress, true);
                let admission: Result<()> =
                    Err(crate::rotation::error::AdmissionError::OutcomeUnknown(
                        "PRIVATE_ADMISSION_DETAIL",
                    )
                    .into());
                finish(&admission, &progress, false);
            })
            .with_subscriber(subscriber)
            .await;
            let logs = std::fs::read_to_string(log.path()).unwrap();
            assert_eq!(logs.contains("Payment call completed"), debug, "{logs}");
            assert_eq!(
                logs.contains("Relay payment attempt failed"),
                debug,
                "{logs}"
            );
            assert!(!logs.contains("INFO"), "{logs}");
            let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
            assert_eq!(warnings.len(), 2, "{logs}");
            assert!(
                warnings[0].contains("no further cause information is available"),
                "{logs}"
            );
            assert!(!warnings[0].contains("unsuccessful HTTP status"), "{logs}");
            assert!(
                warnings[1].contains("The payment outcome is unknown; reconciliation is required"),
                "{logs}"
            );
            for warning in warnings {
                assert!(
                    warning.contains("authorization_may_charge=true")
                        && warning.contains("do not automatically repeat"),
                    "{logs}"
                );
            }
            assert!(!logs.contains("PRIVATE_"), "{logs}");
        }
    }
}
