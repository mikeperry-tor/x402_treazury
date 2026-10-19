//! Safe observations only; these fields never grant payment or retry authority.
use super::SubmissionProgress;
use anyhow::Result;
use std::{cell::Cell, future::Future};

#[derive(Clone, Copy)]
struct Observation {
    stage: &'static str,
    last_http_status: Option<u16>,
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
    emit(result.is_ok(), kind, progress);
}

/// Direct callers own body consumption; observe only the returned headers here.
pub(crate) fn finish_response(response: &reqwest::Response, progress: &SubmissionProgress) {
    self::response(response);
    emit(response.status().is_success(), "http_status", progress);
}

fn emit(success: bool, failure_kind: &'static str, progress: &SubmissionProgress) {
    let observation = OBSERVATION.with(Cell::get);
    let stage = observation.stage;
    let last_http_status = observation.last_http_status;
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
            authorization_may_charge,
            "Payment call failed; do not automatically repeat a call whose authorization may charge"
        );
    }
}
