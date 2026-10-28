//! Best-effort per-address recovery of SDK-owned transparent capital.
use super::*;
use crate::rotation::transaction::{TransactionPresence, TransactionSubmission};
use pepper_sync::wallet::KeyIdInterface;

impl Treasury {
    /// Called only after a new successfully persisted sync, by the existing
    /// mutable owner. Pending work always precedes new proposal selection.
    pub(super) async fn maintain_owned_transparent(
        &mut self,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<()> {
        let mut after = None;
        let mut had_pending = false;
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
                had_pending = true;
                if stop.is_cancelled() {
                    return Ok(());
                }
                if let Err(error) = self.maintain_pending_source(id.clone(), sender, stop).await {
                    if !self.healthy {
                        return Err(error);
                    }
                    diagnostics::report_shielding_failure(&error, Some(&id));
                }
            }
        }
        if had_pending {
            return Ok(());
        }
        let Some(max_fee) = self.automatic_shielding_fee else {
            return Ok(());
        };
        if stop.is_cancelled()
            || !self
                .store
                .call(|s| s.automatic_shielding_available())
                .await?
        {
            return Ok(());
        }
        let addresses: Vec<String> = self
            .client
            .as_ref()
            .context("treasury unavailable")?
            .wallet()
            .read()
            .await
            .transparent_addresses()
            .iter()
            .filter(|(id, _)| id.account_id() == zip32::AccountId::ZERO)
            .map(|(_, address)| address.clone())
            .collect();
        for address in addresses {
            if stop.is_cancelled() {
                break;
            }
            match self.shield_address(address, None, max_fee, stop).await {
                Ok(prepared) => {
                    self.submit_prepared(prepared.operation_id().to_owned(), sender, false, stop)
                        .await?;
                    break;
                }
                // No calculation has been accepted while the SDK is healthy.
                // A bad/empty address must not starve another independent one.
                Err(_) if self.healthy => continue,
                Err(error) => return Err(error),
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
            } else if let Some(max_fee) = self
                .automatic_shielding_fee
                .filter(|_| status.attempts == 0 && status.submission == "PREPARED")
            {
                let operation = id.clone();
                let allowed = self
                    .store
                    .call(move |s| s.automatic_shielding_submission(&operation, max_fee))
                    .await?;
                if allowed {
                    self.submit_prepared(id, sender, false, stop).await?;
                }
            }
        }
        Ok(())
    }
}
