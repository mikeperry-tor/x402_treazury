//! Safe observations only; these fields never grant payment or retry authority.
use super::SubmissionProgress;
use anyhow::Result;
use std::{cell::Cell, future::Future};

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
fn transport_reason(error: &reqwest::Error) -> &'static str {
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

pub(crate) fn finish<T>(result: &Result<T>, progress: &SubmissionProgress) {
    let kind = match result {
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
    emit(result.is_ok(), kind, transport, progress);
}

/// Direct callers own body consumption; observe only the returned headers here.
pub(crate) fn finish_response(response: &reqwest::Response, progress: &SubmissionProgress) {
    self::response(response);
    emit(
        response.status().is_success(),
        "http_status",
        None,
        progress,
    );
}

fn emit(
    success: bool,
    failure_kind: &'static str,
    transport: Option<&reqwest::Error>,
    progress: &SubmissionProgress,
) {
    let observation = OBSERVATION.with(Cell::get);
    let stage = observation.stage;
    let last_http_status = observation.last_http_status;
    let unsigned_attempt = observation.unsigned_attempt;
    let max_unsigned_attempts = observation.max_unsigned_attempts;
    let transport_reason = transport.map(transport_reason);
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
        tracing::info!(
            stage,
            last_http_status,
            authorization_may_charge,
            "Payment call completed; completion does not establish settlement"
        );
    } else {
        tracing::warn!(
            stage,
            last_http_status,
            failure_kind,
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

    #[tokio::test]
    async fn transport_failure_reports_safe_cause_and_preserves_charge_caution() {
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
            .with_writer(log.reopen().unwrap())
            .finish();
        scope(async {
            stage("unsigned_request");
            unsigned_attempt(1, 3);
            let result: Result<()> = Err(error.into());
            finish(&result, &SubmissionProgress::default());
            let progress = SubmissionProgress::default();
            progress.mark_possible();
            finish(&result, &progress);
        })
        .with_subscriber(subscriber)
        .await;
        let logs = std::fs::read_to_string(log.path()).unwrap();
        let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
        assert_eq!(warnings.len(), 2, "{logs}");
        for expected in [
            "http_transport_or_body",
            "transport_reason=\"connection\"",
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
        for private in ["private-path", "private-token", &address.to_string()] {
            assert!(!logs.contains(private), "{logs}");
        }
    }
}
