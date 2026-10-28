//! Signed expiry closes unfinished source work; the SDK owns current capital.
use super::*;
use crate::rotation::transaction::{
    PreparedTransaction, RecoveryEvidenceError, TransactionSubmission,
};

impl Treasury {
    pub async fn recover_expired(
        &mut self,
        id: String,
        _sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<()> {
        if self.source_work_fulfilled(&id).await? {
            return Ok(());
        }
        self.sync_once(stop).await?;
        self.recover_expired_scanned(id, stop).await
    }

    /// Consume the canonical scan already completed by this same source owner.
    /// Callers that only reused a cached observation must request a fresh scan.
    pub(super) async fn recover_expired_scanned(
        &mut self,
        id: String,
        stop: &CancellationToken,
    ) -> Result<()> {
        if self.source_work_fulfilled(&id).await? {
            return Ok(());
        }
        let lookup = id.clone();
        let prepared = self
            .store
            .call(move |s| PreparedTransaction::load(s, &lookup))
            .await?;
        // Authenticates the original signed identity/expiry even when the SDK
        // has no current record. A canonical positive (including shallow
        // inclusion) takes precedence over any absence/expiry inference.
        if self.confirm_sdk_transaction(&prepared).await?.is_some() {
            return Ok(());
        }
        let status = self.store.call(|s| s.treasury_sync_state()).await?;
        let sync = status.sync.context("treasury not synced")?;
        coverage::require_current_tip(&sync)?;
        ensure!(
            sync.phase == SyncPhase::Ready
                && sync.snapshot_revision == status.snapshot_revision
                && status.snapshot_revision == self.revision,
            RecoveryEvidenceError::StaleSync
        );
        let facts = prepared.facts().context("operation facts missing")?;
        let height = sync.height.context("missing sync height")?;
        ensure!(
            facts.expiry_height > 0
                && height >= u64::from(facts.expiry_height) + u64::from(sync.confirmations),
            RecoveryEvidenceError::ExpiryNotBuried
        );
        // Fully scanned SDK history establishes current non-inclusion. Signed
        // expiry prevents later inclusion on that chain. SDK expiry processing
        // already releases pending reservations; missing, relocated or spent
        // current inputs neither grant capital nor keep this source work open.
        ensure!(!stop.is_cancelled(), "expiry recovery cancelled");
        let revision = self.revision;
        self.store
            .call(move |s| s.resolve_expired(&id, revision))
            .await
    }
}
