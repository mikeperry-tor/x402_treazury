//! Source-scoped observations, independent of optional-work admission and accounting.
use super::{Capability, Session, range};
use crate::cover::Limits;
use tracing::Level;

pub(super) fn capability(capability: &Capability) -> (&'static str, Option<&'static str>) {
    match capability {
        Capability::Unknown => ("unknown", None),
        Capability::Checking => ("checking", None),
        Capability::Available { .. } => ("available", None),
        Capability::Unavailable(code) => ("unavailable", Some(code)),
    }
}

pub(super) fn status(session: &Session, limits: &Limits, code: &'static str, level: Level) {
    for scope in session.scopes.lock().unwrap().iter() {
        macro_rules! event {
            ($level:expr) => {
                tracing::event!($level,
                    listener = %scope.listener, source = %scope.source,
                    episode_id = %session.id, code,
                    episode_request_limit = session.config.max_requests_per_episode,
                    episode_body_byte_limit = session.config.max_cover_body_bytes_per_episode,
                    process_request_limit = limits.max_requests_per_window,
                    process_stream_limit = limits.max_active_streams,
                    process_history_limit = limits.max_requests_per_window.saturating_mul(2),
                    process_body_byte_limit = limits.max_cover_body_bytes_per_window,
                    process_padding_byte_limit = limits.max_padding_value_bytes_per_window,
                    episode_padding_byte_limit = session.config.padding.as_ref().map(|p| p.max_value_bytes_per_episode),
                    header_list_byte_limit = session.config.padding.as_ref().map(|p| p.max_total_header_list_bytes),
                    "optional cover status; ordinary API execution continues")
            };
        }
        match level {
            Level::DEBUG => event!(Level::DEBUG),
            Level::INFO => event!(Level::INFO),
            _ => event!(Level::WARN),
        }
    }
}

pub(super) fn attempt(
    session: &Session,
    resource: &'static str,
    qualifying: bool,
    outcome: &range::Outcome,
    capability_after: Option<&Capability>,
    fallback_selected: bool,
) {
    let (capability, retained_failure) = match capability_after {
        Some(state) => {
            let (name, failure) = capability(state);
            (Some(name), failure)
        }
        None => (None, None),
    };
    let code = outcome
        .result
        .as_ref()
        .map(|_| "cover_range_completed")
        .unwrap_or_else(|e| e.0);
    let observation = &outcome.observation;
    for scope in session.scopes.lock().unwrap().iter() {
        macro_rules! event {
            ($level:expr, $($message:tt)+) => {
                tracing::event!($level,
                    listener = %scope.listener, source = %scope.source,
                    episode_id = %session.id, code, resource, qualifying,
                    fresh_attempt = true, fallback_selected, capability_applied = capability_after.is_some(), capability, retained_failure,
                    automatic_requalification = false,
                    stage = observation.stage,
                    range_start = observation.start, requested_bytes = observation.requested_bytes,
                    http_status = observation.http_status, http_version = ?observation.http_version,
                    declared_body_bytes = observation.declared_body_bytes,
                    received_bytes = observation.received_bytes, elapsed_ms = observation.elapsed_ms,
                    $($message)+)
            };
        }
        if matches!(capability_after, Some(Capability::Unavailable(_))) {
            event!(
                Level::WARN,
                "HTTP/2 range cover traffic unavailable for provider [{}]; further range attempts disabled for this identity/origin until restart or cache eviction",
                scope.source
            );
        } else if fallback_selected {
            event!(
                Level::INFO,
                "optional cover range outcome; no full-download fallback"
            );
        } else {
            event!(
                Level::DEBUG,
                "optional cover range outcome; no full-download fallback"
            );
        }
    }
}

pub(super) fn episode(
    session: &Session,
    limits: &Limits,
    code: &'static str,
    capability_after: &Capability,
    resource: &'static str,
    newly_unavailable: bool,
) {
    let (capability, retained_failure) = capability(capability_after);
    let e = session.episode.lock().unwrap();
    let deadline_kind = if e.tail_deadline.is_some_and(|tail| tail < e.deadline) {
        "tail"
    } else {
        "episode"
    };
    let deadline_expired =
        tokio::time::Instant::now() >= e.tail_deadline.unwrap_or(e.deadline).min(e.deadline);
    for scope in session.scopes.lock().unwrap().iter() {
        macro_rules! event {
            ($level:expr, $($message:tt)+) => {
                tracing::event!($level,
                    listener = %scope.listener, source = %scope.source,
                    episode_id = %session.id, code, resource, capability, retained_failure,
                    automatic_requalification = false, deadline_kind, deadline_expired, stop_reason = e.stopped,
                    sampled_body_bytes = e.target, cover_body_bytes = e.received,
                    range_requests = e.requests, padding_value_bytes = e.padding,
                    episode_request_limit = session.config.max_requests_per_episode,
                    episode_body_byte_limit = session.config.max_cover_body_bytes_per_episode,
                    process_request_limit = limits.max_requests_per_window,
                    process_stream_limit = limits.max_active_streams,
                    process_history_limit = limits.max_requests_per_window.saturating_mul(2),
                    process_body_byte_limit = limits.max_cover_body_bytes_per_window,
                    process_padding_byte_limit = limits.max_padding_value_bytes_per_window,
                    episode_padding_byte_limit = session.config.padding.as_ref().map(|p| p.max_value_bytes_per_episode),
                    header_list_byte_limit = session.config.padding.as_ref().map(|p| p.max_total_header_list_bytes),
                    elapsed_ms = e.started.elapsed().as_millis() as u64,
                    $($message)+)
            };
        }
        if newly_unavailable {
            event!(
                Level::WARN,
                "HTTP/2 range cover qualification did not finish for provider [{}]; further range attempts disabled for this identity/origin until restart or cache eviction",
                scope.source
            );
        } else if matches!(
            code,
            "cover_task_failed"
                | "cover_sampling_exhausted"
                | "cover_stream_limit"
                | "cover_request_limit"
                | "cover_body_limit"
                | "cover_padding_limit"
                | "cover_history_limit"
        ) {
            event!(
                Level::WARN,
                "HTTP/2 cover traffic failed for provider [{}]; see diagnostic fields",
                scope.source
            );
        } else if matches!(capability_after, Capability::Unavailable(_)) {
            event!(
                Level::DEBUG,
                "optional cover episode finished; pooled best-effort, not a privacy guarantee"
            );
        } else {
            event!(
                Level::INFO,
                "optional cover episode finished; pooled best-effort, not a privacy guarantee"
            );
        }
    }
}

pub(super) fn cached(session: &Session, code: &'static str, resource: &'static str) {
    for scope in session.scopes.lock().unwrap().iter() {
        tracing::debug!(listener = %scope.listener, source = %scope.source,
            episode_id = %session.id, code, resource, fresh_attempt = false,
            capability = "unavailable", retained_failure = code, automatic_requalification = false,
            "cached cover capability; no new range attempted");
    }
}
