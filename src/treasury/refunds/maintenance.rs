//! Staged serving-only preparation. The actor does not invoke this until the
//! canonical live-orphan/expiry gate is complete. No submission occurs here.
#![allow(dead_code)]
use super::*;
use crate::rotation::store::shielding::{ServingAuthority, maintenance::AddressCursor};

#[derive(Default)]
pub(crate) struct ShieldingMaintenance {
    address: AddressCursor,
}

impl Treasury {
    /// One bounded journal page per maintenance turn; only actual confirmed,
    /// unspent, unclaimed wallet inputs can start calculate-only preparation.
    /// Attribution controls accounting, not ownership of these exact inputs.
    pub(crate) async fn prepare_automatic_refund(
        &mut self,
        maintenance: &mut ShieldingMaintenance,
        authority: &ServingAuthority,
        stop: &CancellationToken,
    ) -> Result<Option<PreparedTransaction>> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed preparation"
        );
        ensure!(!stop.is_cancelled(), "treasury stopping");
        let status = self.store.call(|s| s.treasury_sync_state()).await?;
        let sync = status.sync.context("treasury sync missing")?;
        ensure!(
            sync.fresh(now()?, status.snapshot_revision)? && !status.outgoing_pending,
            "treasury not ready for shielding"
        );
        ensure!(
            status.snapshot_revision == self.revision,
            "snapshot_revision_conflict"
        );
        let height = sync.height.context("treasury sync height missing")?;
        let cursor = maintenance.address.clone();
        let policy = authority.clone();
        let page = self
            .store
            .call(move |s| s.shielding_address_page(&policy, &cursor))
            .await?;
        for candidate in page.candidates {
            ensure!(!stop.is_cancelled(), "treasury stopping");
            maintenance.address = candidate.cursor();
            let inputs = {
                let wallet = self
                    .client
                    .as_ref()
                    .context("treasury unavailable")?
                    .wallet()
                    .read()
                    .await;
                wallet
                    .wallet_outputs::<pepper_sync::wallet::TransparentCoin>()
                    .into_iter()
                    .filter(|coin| {
                        coin.address() == candidate.address
                            && wallet.output_spend_status(*coin).is_unspent()
                            && wallet
                                .output_transaction(*coin)
                                .status()
                                .get_confirmed_height()
                                .map(|h| u64::from(u32::from(h)))
                                .is_some_and(|h| {
                                    h > 0
                                        && height >= h
                                        && height - h + 1 >= u64::from(sync.confirmations)
                                })
                    })
                    .map(|coin| {
                        (
                            coin.output_id().txid().to_string(),
                            coin.output_id().output_index(),
                            coin.value(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let value = inputs.iter().try_fold(0u64, |total, (_, _, amount)| {
                total
                    .checked_add(*amount)
                    .context("shielding input value overflow")
            })?;
            if value < SHIELDING_THRESHOLD {
                continue;
            }
            let inputs = inputs
                .into_iter()
                .map(|(txid, index, _)| (txid, index))
                .collect::<Vec<_>>();
            if !self
                .store
                .call(move |s| s.shielding_inputs_available(&inputs))
                .await?
            {
                continue;
            }
            let operation = uuid::Uuid::new_v4().to_string();
            let revision = self.revision;
            let selected = candidate.clone();
            let authority = authority.clone();
            let (owner, policy) = self
                .store
                .call(move |s| {
                    s.begin_shielding_candidate(&operation, revision, &selected, &authority)
                })
                .await?;
            return self
                .finish_owned_refund(
                    candidate.address,
                    candidate.recipient,
                    candidate.max_fee,
                    owner,
                    policy,
                )
                .await
                .map(Some);
        }
        maintenance.address = page.next.unwrap_or_default();
        Ok(None)
    }
}
