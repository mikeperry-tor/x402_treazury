//! SDK retirement is separate from the lifetime of the shared serialized store.
use super::*;

type ScanResult = Result<(SyncSession, SyncObservation, Result<()>)>;

#[derive(Debug)]
struct SnapshotChanged;
impl std::fmt::Display for SnapshotChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("treasury snapshot changed during reconstruction")
    }
}
impl std::error::Error for SnapshotChanged {}

pub(super) fn retryable_reconstruction(error: &anyhow::Error) -> bool {
    error.is::<SnapshotUnavailable>() || error.is::<SnapshotChanged>()
}

pub(super) struct SyncOwner {
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<ScanResult>,
}
impl SyncOwner {
    pub(super) fn start(
        session: SyncSession,
        settings: SyncSettings,
        observation: SyncObservation,
        cancel: CancellationToken,
    ) -> Self {
        let task_cancel = cancel.clone();
        Self {
            cancel,
            join: tokio::task::spawn_blocking(move || {
                session.run(settings, observation, task_cancel)
            }),
        }
    }
}
impl Drop for SyncOwner {
    fn drop(&mut self) {
        // Dropping the whole Treasury never grants replacement authority. The
        // session retains its StoreHandle until its runtime has drained.
        self.cancel.cancel();
    }
}

/// Constructed only after the old SDK and every retained scan owner have drained.
/// Store cleanup uses this capability rather than interpreting missing SDK state.
#[derive(Clone)]
pub(crate) struct RetiredSdk {
    namespace: uuid::Uuid,
}
impl RetiredSdk {
    pub(crate) fn namespace(&self) -> uuid::Uuid {
        self.namespace
    }
}

impl Treasury {
    pub(super) async fn finish_sync_owner(&mut self) -> ScanResult {
        // Poll in place: cancelling this waiter must not detach the active scan.
        let result = (&mut self
            .sync_owner
            .as_mut()
            .context("treasury sync owner missing")?
            .join)
            .await;
        self.sync_owner.take();
        result.context("treasury sync worker failed")?
    }

    pub(super) async fn retire_sdk(&mut self) -> Result<RetiredSdk> {
        self.healthy = false;
        self.reusable_sync_revision = None;
        self.last_completed_sync = None;
        if let Some(owner) = &self.sync_owner {
            owner.cancel.cancel();
        }
        if self.sync_owner.is_some() {
            // Even a panic has completed runtime teardown when the join returns.
            // Never save or publish the returned client's observation here.
            drop(self.finish_sync_owner().await);
        }
        if let Some(client) = &mut self.client {
            client.go_offline().await;
        }
        drop(self.client.take());
        // Accepted store closures outlive their reply waiters. This job is the
        // fence after the retired SDK's final possible queue admission.
        self.store
            .call(|s| s.set_sync_phase(SyncPhase::Offline))
            .await?;
        Ok(RetiredSdk {
            namespace: self.store.runtime_namespace(),
        })
    }

    async fn settle_retired_preparations(
        &self,
        retired: RetiredSdk,
        stop: &CancellationToken,
    ) -> Result<()> {
        let mut after = String::new();
        loop {
            ensure!(!stop.is_cancelled(), "treasury stopping");
            let proof = retired.clone();
            let next = self
                .store
                .call(move |s| s.close_retired_preparations_page(&proof, after))
                .await?;
            let Some(next) = next else {
                return Ok(());
            };
            after = next;
            tokio::task::yield_now().await;
        }
    }

    /// Called only by initial reconstruction while the original exclusive Store
    /// has just been moved into its first serialized worker. No old SDK survives
    /// that exclusive open, and this newly decoded SDK has not mutated anything.
    pub(super) async fn finish_open_handoff(&self) -> Result<()> {
        self.settle_retired_preparations(
            RetiredSdk {
                namespace: self.store.runtime_namespace(),
            },
            &CancellationToken::new(),
        )
        .await
    }

    pub(super) async fn reconstruct_live(&mut self, stop: &CancellationToken) -> Result<()> {
        let retired = self.retire_sdk().await?;
        ensure!(!stop.is_cancelled(), "treasury stopping");
        self.settle_retired_preparations(retired, stop).await?;
        let (revision, bytes) = self.store.call(|s| s.snapshot()).await?;
        let scratch = tempfile::tempdir()?;
        let client = LightClient::from_reader(
            bytes.as_slice(),
            config(scratch.path(), WalletConfig::Read, self.network)?,
        )
        .await
        .map_err(|_| SnapshotUnavailable)?;
        ensure!(
            client.indexer_uri().is_none(),
            "reconstructed treasury must be offline"
        );
        ensure!(!stop.is_cancelled(), "treasury stopping");
        self.store
            .call(move |s| {
                ensure!(s.snapshot()?.0 == revision, SnapshotChanged);
                Ok(())
            })
            .await?;
        ensure!(!stop.is_cancelled(), "treasury stopping");
        self._scratch = scratch;
        self.client = Some(client);
        self.revision = revision;
        self.healthy = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
