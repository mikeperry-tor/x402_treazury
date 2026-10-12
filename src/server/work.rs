//! Ownership of accepted calls is independent of MCP response delivery.
use std::sync::{Arc, Mutex};
use tokio_util::task::TaskTracker;
use tracing::{Instrument, instrument::WithSubscriber};

#[derive(Clone, Default)]
pub struct Work {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    closed: Mutex<bool>,
    tasks: TaskTracker,
}

tokio::task_local! {
    static OWNERS: Vec<Arc<Inner>>;
}

pub(crate) fn retained() -> bool {
    OWNERS.try_with(|_| ()).is_ok()
}

impl Work {
    pub fn run<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = T> + Send + 'static,
    ) -> impl std::future::Future<Output = anyhow::Result<T>> + Send + '_ {
        let work = Box::pin(work);
        async move {
            let mut owners = OWNERS.try_with(Clone::clone).unwrap_or_default();
            if owners.iter().any(|owner| Arc::ptr_eq(owner, &self.inner)) {
                // This workflow already holds admission and tracking for this
                // owner. Closing admission cannot reject its remaining steps.
                return Ok(work.await);
            }
            let nested = !owners.is_empty();
            owners.push(self.inner.clone());
            if nested {
                let _tracked = {
                    let closed = self.inner.closed.lock().expect("work admission lock");
                    anyhow::ensure!(!*closed, "work owner is draining; new calls are refused");
                    self.inner.tasks.token()
                };
                return Ok(OWNERS.scope(owners, work).await);
            }
            let handle = {
                let closed = self.inner.closed.lock().expect("work admission lock");
                anyhow::ensure!(!*closed, "listener is draining; new calls are refused");
                // Spawn and close are serialized. Dropping a response waiter does
                // not drop the task's ownership or its durable qualification claim.
                self.inner.tasks.spawn(
                    OWNERS
                        .scope(owners, work)
                        .in_current_span()
                        .with_current_subscriber(),
                )
            };
            Ok(handle.await?)
        }
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
