//! Generic recovery of already accepted source bytes, with no new allocations.
use super::*;
use crate::rotation::transaction::{TransactionPresence, TransactionSubmission};

impl Treasury {
    /// Called only after a new successfully persisted sync, by the existing
    /// mutable owner. Pending work always precedes new proposal selection.
    pub(super) async fn maintain_pending_sources(
        &mut self,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<()> {
        let mut after = None;
        loop {
            let cursor = after.clone();
            let page = self
                .store
                .call(move |s| s.unowned_pending_sources(cursor.as_deref()))
                .await?;
            if page.is_empty() {
                break;
            }
            for id in page {
                after = Some(id.clone());
                if stop.is_cancelled() {
                    return Ok(());
                }
                if let Err(error) = self.maintain_pending_source(id.clone(), sender, stop).await {
                    if !self.healthy {
                        return Err(error);
                    }
                    diagnostics::report_pending_source_failure(&error, Some(&id));
                }
            }
        }
        Ok(())
    }
    async fn maintain_pending_source(
        &mut self,
        id: String,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<()> {
        if self.source_work_fulfilled(&id).await? {
            return Ok(());
        }
        let presence = self
            .reconcile_for_observation(id.clone(), sender, stop)
            .await?;
        if matches!(
            presence,
            TransactionPresence::Absent | TransactionPresence::Unknown
        ) {
            let operation = id.clone();
            let (status, sync) = self
                .store
                .call(move |s| {
                    Ok((
                        s.observation_operation(&operation)?,
                        s.treasury_sync_state()?.sync,
                    ))
                })
                .await?;
            if status.facts.expiry_height > 0
                && sync
                    .and_then(|s| s.height.map(|h| (h, s.confirmations)))
                    .is_some_and(|(height, depth)| {
                        height >= u64::from(status.facts.expiry_height) + u64::from(depth)
                    })
            {
                // A completed canonical SDK tip and authenticated buried
                // signed expiry close this work; lookup absence alone does not.
                self.recover_expired_scanned(id, stop).await?;
            }
        }
        Ok(())
    }
}
