//! Same-owner observation and serving-authorized shielding maintenance.
use super::*;
use crate::rotation::store::shielding::{ServingAuthority, maintenance::AddressCursor};

#[derive(Default)]
pub(crate) struct ShieldingMaintenance {
    address: AddressCursor,
    operation: String,
    #[cfg(test)]
    observation_override: Option<Box<dyn FnMut(&str) -> Result<bool> + Send + Sync>>,
}

impl Treasury {
    /// Observe retained ownership before considering any new work. Historical
    /// IDs are independent of current profiles and serving authority.
    pub(crate) async fn maintain_shielding(
        &mut self,
        maintenance: &mut ShieldingMaintenance,
        authority: Option<&ServingAuthority>,
        sender: &mut impl crate::rotation::transaction::TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<()> {
        let after = maintenance.operation.clone();
        let (operations, pending) = self
            .store
            .call(move |s| {
                Ok((
                    s.shielding_operation_page(&after)?,
                    s.pending_shielding_operation()?,
                ))
            })
            .await?;
        if operations.is_empty() {
            maintenance.operation.clear();
        }
        let mut candidates = Vec::new();
        let mut warning = None;
        // The prioritized slot never advances the historical prefix cursor.
        if let Some(id) = pending.as_ref() {
            let result = self
                .observe_maintenance_operation(maintenance, id.clone(), sender, stop)
                .await;
            if scoped_observation(result, self.healthy, &mut warning)? == Some(true) {
                candidates.push(id.clone());
            }
        }
        for operation in operations {
            ensure!(!stop.is_cancelled(), "treasury stopping");
            if pending.as_ref() != Some(&operation) {
                let result = self
                    .observe_maintenance_operation(maintenance, operation.clone(), sender, stop)
                    .await;
                if scoped_observation(result, self.healthy, &mut warning)? == Some(true) {
                    candidates.push(operation.clone());
                }
            }
            // Only a processed prefix row advances the historical scan. A scoped
            // unavailable observation retains ownership and retries next cycle.
            maintenance.operation = operation;
        }
        if let Some(error) = warning {
            super::super::diagnostics::warn_sync_failure(&error);
        }
        // All independent old observations run before a preflight/submission can
        // fail. Existing attempted bytes never enter this first-intent path.
        if let Some(authority) = authority {
            for operation in candidates {
                let id = operation.clone();
                let policy = authority.clone();
                if self
                    .store
                    .call(move |s| s.automatic_shielding_submission_candidate(&id, &policy))
                    .await?
                {
                    self.submit_automatic_shielding(operation, authority.clone(), sender, stop)
                        .await?;
                }
            }
        }
        let Some(authority) = authority else {
            return Ok(());
        };
        // Existing bytes, including manual bytes, retain the shared send slot.
        if self
            .store
            .call(|s| Ok(s.treasury_sync_state()?.outgoing_pending))
            .await?
        {
            return Ok(());
        }
        if let Some(prepared) = self
            .prepare_automatic_refund(maintenance, authority, stop)
            .await?
        {
            self.submit_automatic_shielding(
                prepared.operation_id().to_owned(),
                authority.clone(),
                sender,
                stop,
            )
            .await?;
        }
        Ok(())
    }

    async fn observe_maintenance_operation(
        &mut self,
        maintenance: &mut ShieldingMaintenance,
        operation: String,
        sender: &mut impl crate::rotation::transaction::TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<bool> {
        #[cfg(test)]
        if let Some(observe) = maintenance.observation_override.as_mut() {
            return observe(&operation);
        }
        #[cfg(not(test))]
        let _ = maintenance;
        self.observe_shielding(operation, sender, stop).await
    }

    async fn observe_shielding(
        &mut self,
        operation: String,
        sender: &mut impl crate::rotation::transaction::TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<bool> {
        use crate::rotation::transaction::TransactionPresence;
        let id = operation.clone();
        let status = self.store.call(move |s| s.operation(&id)).await?;
        if matches!(status.submission.as_str(), "CONFIRMED" | "EXPIRED") {
            let id = operation.clone();
            // The just-completed sync already observed all historical sources.
            // Only retained expiry obligations need an additional proof pass.
            if self
                .store
                .call(move |s| s.source_expiry_recovery_needed(&id))
                .await?
            {
                self.reconcile_source_obligation(operation, sender, stop)
                    .await?;
            }
            return Ok(false);
        }
        let presence = self
            .reconcile_prepared(operation.clone(), sender, stop)
            .await?;
        if matches!(
            presence,
            TransactionPresence::Confirmed { .. } | TransactionPresence::Mempool
        ) {
            return Ok(false);
        }
        let sync = self
            .store
            .call(|s| s.treasury_sync_state())
            .await?
            .sync
            .context("treasury sync missing")?;
        let height = sync.height.context("treasury sync height missing")?;
        if source_expiry_buried(status.facts.expiry_height, height, sync.confirmations) {
            self.recover_expired(operation, sender, stop).await?;
            return Ok(false);
        }
        Ok(status.attempts == 0 && height < u64::from(status.facts.expiry_height))
    }

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

fn scoped_observation(
    result: Result<bool>,
    healthy: bool,
    warning: &mut Option<anyhow::Error>,
) -> Result<Option<bool>> {
    match result {
        Ok(ready) => Ok(Some(ready)),
        Err(error)
            if healthy
                && (error
                    .downcast_ref::<crate::rotation::transaction::RecoveryEvidenceError>()
                    .is_some()
                    || super::super::diagnostics::indexer_observation_failure(&error)) =>
        {
            if warning.is_none() {
                *warning = Some(error);
            }
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
