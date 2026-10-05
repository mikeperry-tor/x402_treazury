//! Inspection-only catalog observations. No files, authority, or additional I/O.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap, future::Future};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    NotStarted,
    Cancelled,
    Failed,
    Completed,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub source: String,
    pub state: State,
    pub stage: String,
    pub http_status: Option<u16>,
}
tokio::task_local! {
    static OBSERVATIONS: RefCell<BTreeMap<String, Observation>>;
}
pub(super) fn collecting() -> bool {
    OBSERVATIONS.try_with(|_| ()).is_ok()
}
pub(super) async fn capture<T>(future: impl Future<Output = T>) -> (T, Vec<Observation>) {
    OBSERVATIONS
        .scope(RefCell::default(), async {
            let result = future.await;
            let rows = OBSERVATIONS.with(|v| v.borrow().values().cloned().collect());
            (result, rows)
        })
        .await
}
pub(super) fn register<'a>(sources: impl Iterator<Item = &'a String>) -> Result<()> {
    OBSERVATIONS.try_with(|v| {
        let mut rows = v.borrow_mut();
        for source in sources {
            if rows.len() == 10000 {
                tracing::warn!(limit_sources=10000, "Catalog evidence source limit exceeded; inspection rejected");
                anyhow::bail!("catalog evidence exceeds 10000 sources; no complete inspection can be published");
            }
            ensure!(!rows.contains_key(source), "duplicate catalog evidence source");
            rows.insert(source.clone(), Observation { source: source.clone(), state: State::NotStarted, stage: "configuration".into(), http_status: None });
        }
        Ok(())
    }).unwrap_or(Ok(()))
}
pub(super) fn record(source: &str, state: State, stage: &str, http_status: Option<u16>) {
    let _ = OBSERVATIONS.try_with(|v| {
        if let Some(row) = v.borrow_mut().get_mut(source) {
            row.state = state;
            row.stage = stage.into();
            row.http_status = http_status;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn source_bound_has_both_consumer_error_and_content_free_warning() {
        let writer = Writer(Arc::default());
        let sink = writer.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish();
        let sources: Vec<_> = (0..10001).map(|i| format!("private-source-{i}")).collect();
        let error = tracing::subscriber::with_default(subscriber, || {
            OBSERVATIONS.sync_scope(RefCell::default(), || register(sources.iter()).unwrap_err())
        });
        assert!(error.to_string().contains("10000 sources"));
        let log = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("limit_sources=10000"));
        assert!(log.contains("inspection rejected"));
        assert!(!log.contains("private-source"));
        // Ordinary unobserved serving does not acquire an extra source cap.
        register(sources.iter()).unwrap();
    }
}
