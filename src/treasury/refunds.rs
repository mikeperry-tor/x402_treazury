//! Explicit refusal of unsupported transparent-input shielding.
use super::*;
use crate::rotation::transaction::PreparedTransaction;

impl Treasury {
    /// Rebuild visibility from the existing persisted SDK snapshot. Network
    /// discovery remains owned by the ordinary consensus-paced scan scheduler.
    pub(crate) async fn observe_funding_refunds(&mut self) -> Result<()> {
        let mut after = String::new();
        loop {
            let cursor = after.clone();
            let page = self
                .store
                .call(move |s| s.funding_refund_page(&cursor))
                .await?;
            if page.is_empty() {
                return Ok(());
            }
            for (job, operation, wallet) in page {
                after = operation.clone();
                if let Err(error) = self
                    .observe_funding_refund(job, operation.clone(), wallet)
                    .await
                {
                    // A damaged original cannot suppress sibling observation.
                    diagnostics::report_pending_source_failure(&error, Some(&operation));
                }
            }
        }
    }

    async fn observe_funding_refund(
        &self,
        job: String,
        operation: String,
        wallet_name: String,
    ) -> Result<()> {
        let (id, original, revision) = (job.clone(), operation.clone(), self.revision);
        let Some(address) = self
            .store
            .call(move |s| s.funding_refund_receiver(&id, &original, revision))
            .await?
        else {
            return Ok(());
        };
        let client = self.client.as_ref().context("treasury unavailable")?;
        let confirmed_zatoshis = {
            let wallet = client.wallet().read().await;
            notes::native_receipt(&wallet, &address, revision)?.confirmed_zatoshis
        };
        let received = confirmed_zatoshis > 0;
        let original = operation.clone();
        let changed = self
            .store
            .call(move |s| s.observe_funding_refund(&job, &original, revision, received))
            .await?;
        if changed {
            if received {
                tracing::info!(wallet=%wallet_name, operation, snapshot_revision=revision, confirmed_zatoshis,
                    "Confirmed Ironwood funds observed at the saved refund receiver; replacement funding will recheck the provider outcome, original source closure and current unused claim");
            } else {
                tracing::info!(wallet=%wallet_name, operation, snapshot_revision=revision,
                    "No confirmed Ironwood receipt is visible at the saved refund receiver; NEAR status checks and Zcash scans continue");
            }
        }
        Ok(())
    }

    /// Unsupported admission performs no wallet mutation, Store lookup or network I/O.
    pub async fn shield_refund(
        &mut self,
        _job: String,
        _stop: &CancellationToken,
    ) -> Result<PreparedTransaction> {
        anyhow::bail!("Transparent-input shielding is unsupported; treasury payments use Ironwood")
    }
}
