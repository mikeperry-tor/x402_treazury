//! Explicit resource rejection; never return a partial document as successful data.
use anyhow::{Result, bail};
/// Public inventory page selection; not a total-result or acquisition bound.
pub const DEFAULT_PAGE_SIZE: usize = 25;
pub const MCP_REQUEST_BYTES: usize = 4 * 1024 * 1024;
pub const RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const HELP_BYTES: usize = 4 * 1024 * 1024;
pub const SPEC_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const API_CACHE_TTL_SECONDS: u64 = 24 * 60 * 60;
pub fn response_default() -> usize {
    RESPONSE_BYTES
}
pub fn help_default() -> usize {
    HELP_BYTES
}

/// Reject a resource bound with safe diagnostics, never caller-supplied content.
pub(crate) fn check(
    actual: usize,
    limit: usize,
    reason: &'static str,
    setting: &'static str,
) -> Result<()> {
    if actual > limit {
        tracing::warn!(
            reason,
            actual,
            limit,
            setting,
            "resource limit exceeded; operation rejected"
        );
        bail!("{reason}: actual={actual}, limit={limit}, setting={setting}");
    }
    Ok(())
}

pub(crate) fn busy(reason: &'static str, limit: usize) -> &'static str {
    tracing::warn!(
        reason,
        limit,
        "resource capacity exhausted; operation rejected"
    );
    reason
}

/// Reject before retaining work; callers keep their existing public error context.
pub(crate) fn try_capacity<'a>(
    gate: &'a tokio::sync::Semaphore,
    reason: &'static str,
    limit: usize,
) -> Result<tokio::sync::SemaphorePermit<'a>, tokio::sync::TryAcquireError> {
    gate.try_acquire().inspect_err(|error| {
        if matches!(error, tokio::sync::TryAcquireError::NoPermits) {
            busy(reason, limit);
        }
    })
}

fn waiting(reason: &'static str, limit: Option<usize>, setting: &'static str) {
    tracing::warn!(
        reason,
        limit,
        setting,
        action = "waiting",
        "resource capacity exhausted; waiting without rejecting accepted work"
    );
}

pub(crate) async fn acquire_capacity<'a>(
    gate: &'a tokio::sync::Semaphore,
    reason: &'static str,
    limit: Option<usize>,
    setting: &'static str,
) -> Result<tokio::sync::SemaphorePermit<'a>, tokio::sync::AcquireError> {
    match gate.try_acquire() {
        Ok(permit) => Ok(permit),
        Err(error) => {
            if matches!(error, tokio::sync::TryAcquireError::NoPermits) {
                waiting(reason, limit, setting);
            }
            gate.acquire().await
        }
    }
}

pub(crate) async fn acquire_owned_capacity(
    gate: std::sync::Arc<tokio::sync::Semaphore>,
    reason: &'static str,
    limit: Option<usize>,
    setting: &'static str,
) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::AcquireError> {
    match gate.clone().try_acquire_owned() {
        Ok(permit) => Ok(permit),
        Err(error) => {
            if matches!(error, tokio::sync::TryAcquireError::NoPermits) {
                waiting(reason, limit, setting);
            }
            gate.acquire_owned().await
        }
    }
}

/// A full queue retains the exact original value and the existing send contract.
pub(crate) async fn send_capacity<T>(
    sender: &tokio::sync::mpsc::Sender<T>,
    value: T,
    reason: &'static str,
    limit: usize,
) -> Result<(), tokio::sync::mpsc::error::SendError<T>> {
    match sender.try_send(value) {
        Ok(()) => Ok(()),
        Err(tokio::sync::mpsc::error::TrySendError::Full(value)) => {
            waiting(reason, Some(limit), "hardcoded");
            sender.send(value).await
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(value)) => {
            Err(tokio::sync::mpsc::error::SendError(value))
        }
    }
}

#[derive(Debug)]
pub(crate) struct DownloadLimit {
    pub kind: &'static str,
    pub setting: &'static str,
    pub limit: usize,
}
impl std::fmt::Display for DownloadLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} exceeds {}={} bytes; content rejected, no partial content returned. Ask the operator to raise {} if appropriate.",
            self.kind, self.setting, self.limit, self.setting
        )
    }
}
impl std::error::Error for DownloadLimit {}

pub fn exceeded(kind: &'static str, setting: &'static str, limit: usize) -> anyhow::Error {
    // Do not include URLs, headers, document excerpts or credentials in logs.
    tracing::warn!(
        resource = kind,
        setting,
        limit_bytes = limit,
        "download limit exceeded; content rejected, no partial content returned"
    );
    DownloadLimit {
        kind,
        setting,
        limit,
    }
    .into()
}
pub async fn read(
    mut response: reqwest::Response,
    limit: usize,
    kind: &'static str,
    setting: &'static str,
) -> Result<Vec<u8>> {
    if limit == 0 {
        bail!("{setting} must be positive");
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(exceeded(kind, setting, limit));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(exceeded(kind, setting, limit));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        future::Future,
        sync::{Arc, Mutex},
        task::{Context, Poll, Waker},
    };
    use tracing::instrument::WithSubscriber;

    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    pub(crate) async fn capture<T>(future: impl Future<Output = T>) -> (T, String) {
        let logs = Logs::default();
        let sink = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish();
        let result = future.with_subscriber(subscriber).await;
        let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        (result, text)
    }

    fn pending<T>(future: std::pin::Pin<&mut impl Future<Output = T>>) {
        assert!(matches!(
            future.poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
    }

    #[tokio::test]
    async fn capacity_rejection_warns_with_safe_threshold_and_preserves_error() {
        let (_, logs) = capture(async {
            let gate = tokio::sync::Semaphore::new(1);
            let held = try_capacity(&gate, "test_capacity_busy", 1).unwrap();
            let error = try_capacity(&gate, "test_capacity_busy", 1)
                .map_err(anyhow::Error::from)
                .map_err(|error| error.context("existing consumer error"))
                .unwrap_err();
            assert_eq!(error.to_string(), "existing consumer error");
            drop(held);
            assert!(try_capacity(&gate, "test_capacity_busy", 1).is_ok());
        })
        .await;
        assert_eq!(logs.matches("WARN").count(), 1, "{logs}");
        assert!(
            logs.contains("test_capacity_busy") && logs.contains("limit=1"),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn capacity_waits_warn_once_and_resume_without_new_admission() {
        let (_, logs) = capture(async {
            let gate = Arc::new(tokio::sync::Semaphore::new(1));
            let held = gate.acquire().await.unwrap();
            let borrowed = acquire_capacity(&gate, "borrowed_busy", Some(1), "hardcoded");
            tokio::pin!(borrowed);
            pending(borrowed.as_mut());
            pending(borrowed.as_mut());
            drop(held);
            drop(borrowed.await.unwrap());
            let held = gate.clone().acquire_owned().await.unwrap();
            let owned = acquire_owned_capacity(
                gate.clone(),
                "owned_busy",
                None,
                "startup.catalog_concurrency",
            );
            tokio::pin!(owned);
            pending(owned.as_mut());
            pending(owned.as_mut());
            drop(held);
            drop(owned.await.unwrap());
            drop(
                acquire_capacity(&gate, "ready", Some(1), "hardcoded")
                    .await
                    .unwrap(),
            );
            drop(
                acquire_owned_capacity(gate.clone(), "ready", None, "startup.catalog_concurrency")
                    .await
                    .unwrap(),
            );
            gate.close();
            assert!(try_capacity(&gate, "closed", 1).is_err());
            assert!(
                acquire_capacity(&gate, "closed", Some(1), "hardcoded")
                    .await
                    .is_err()
            );
            assert!(
                acquire_owned_capacity(gate.clone(), "closed", None, "startup.catalog_concurrency")
                    .await
                    .is_err()
            );
        })
        .await;
        assert_eq!(logs.matches("WARN").count(), 2, "{logs}");
        assert!(
            logs.contains("borrowed_busy") && logs.contains("owned_busy"),
            "{logs}"
        );
        assert!(
            logs.contains("startup.catalog_concurrency") && logs.contains("action=\"waiting\""),
            "{logs}"
        );
        assert!(
            !logs.contains("ready") && !logs.contains("closed"),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn full_queue_warns_once_and_preserves_exact_payload_and_closed_error() {
        let (_, logs) = capture(async {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            send_capacity(&sender, String::from("PRIVATE_FIRST"), "test_queue_full", 1)
                .await
                .unwrap();
            let waiting = send_capacity(
                &sender,
                String::from("PRIVATE_SECOND"),
                "test_queue_full",
                1,
            );
            tokio::pin!(waiting);
            pending(waiting.as_mut());
            pending(waiting.as_mut());
            assert_eq!(receiver.recv().await.unwrap(), "PRIVATE_FIRST");
            waiting.await.unwrap();
            assert_eq!(receiver.recv().await.unwrap(), "PRIVATE_SECOND");
            assert!(receiver.try_recv().is_err());
            drop(receiver);
            let error = send_capacity(
                &sender,
                String::from("PRIVATE_CLOSED"),
                "test_queue_full",
                1,
            )
            .await
            .unwrap_err();
            assert_eq!(error.0, "PRIVATE_CLOSED");
        })
        .await;
        assert_eq!(logs.matches("WARN").count(), 1, "{logs}");
        assert!(
            logs.contains("test_queue_full") && logs.contains("limit=1"),
            "{logs}"
        );
        assert!(!logs.contains("PRIVATE"), "{logs}");
    }
}
