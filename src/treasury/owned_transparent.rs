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
        if let Some(id) = self.store.call(|s| s.jobless_pending_operation()).await? {
            let presence = self.reconcile_prepared(id.clone(), sender, stop).await?;
            if matches!(
                presence,
                TransactionPresence::Absent | TransactionPresence::Unknown
            ) {
                let operation = id.clone();
                let (status, sync) = self
                    .store
                    .call(move |s| Ok((s.operation(&operation)?, s.treasury_sync_state()?.sync)))
                    .await?;
                if status.facts.expiry_height > 0
                    && sync
                        .and_then(|s| s.height.map(|h| (h, s.confirmations)))
                        .is_some_and(|(height, depth)| {
                            height >= u64::from(status.facts.expiry_height) + u64::from(depth)
                        })
                {
                    // Two current zero-lag scans and original unspent inputs,
                    // never absence alone, authorize releasing this owner.
                    self.recover_expired(id, sender, stop).await?;
                } else if let Some(max_fee) = self.automatic_shielding_fee {
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
}
