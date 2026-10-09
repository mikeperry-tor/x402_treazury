//! Ownership of accepted calls is independent of MCP response delivery.
use std::sync::{Arc, Mutex};
use tokio_util::task::TaskTracker;

#[derive(Clone, Default)]
pub struct Work {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    closed: Mutex<bool>,
    tasks: TaskTracker,
}

impl Work {
    pub async fn run<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let handle = {
            let closed = self.inner.closed.lock().expect("work admission lock");
            anyhow::ensure!(!*closed, "listener is draining; new calls are refused");
            // Spawn and close are serialized. Dropping a response waiter does
            // not drop the task's ownership or its durable qualification claim.
            self.inner.tasks.spawn(work)
        };
        Ok(handle.await?)
    }

    pub fn close(&self) {
        *self.inner.closed.lock().expect("work admission lock") = true;
        self.inner.tasks.close();
    }

    pub async fn drain(&self) {
        self.close();
        let started = std::time::Instant::now();
        loop {
            tokio::select! {
                _ = self.inner.tasks.wait() => return,
                _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                    tracing::info!(stage = "accepted_mcp_calls", calls = self.inner.tasks.len(), elapsed_seconds = started.elapsed().as_secs(), "Waiting for accepted work to drain");
                }
            }
        }
    }
}
