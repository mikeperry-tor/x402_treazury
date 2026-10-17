//! Canonical accounting observations reuse the serialized sync's existing source pass.
use super::*;
use crate::rotation::store::source_support::{Anchor, Disposition, Pass};

impl SyncSession {
    pub(super) async fn retain_source_support(
        &self,
        pass: Pass,
        indexer: &mut impl zingo_netutils::Indexer,
        height: u64,
    ) -> Result<Option<Pass>> {
        if height < pass.anchor.height {
            return Ok(None);
        }
        let block = indexer
            .get_block(
                zingo_netutils::lightwallet_protocol::BlockId {
                    height: pass.anchor.height,
                    hash: vec![],
                },
                zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT,
            )
            .await
            .map_err(|_| anyhow::anyhow!("source historical anchor unavailable"))?;
        ensure!(
            block.height == pass.anchor.height && block.hash.len() == 32,
            "source historical anchor malformed"
        );
        if block.hash.as_slice() != pass.anchor.hash.as_slice() {
            return Ok(None);
        }
        let revision = self.revision;
        self.store
            .call(move |s| s.continue_source_support(&pass, revision))
            .await
    }
    pub(super) async fn recheck_source_support(
        &self,
        height: u64,
        confirmations: u32,
        returns: refunds::ReturnReconciliation,
    ) -> Result<Pass> {
        let anchor = self
            .client
            .wallet()
            .read()
            .await
            .wallet_blocks
            .get(&u32::try_from(height)?.into())
            .map(|block| Anchor {
                height,
                hash: block.block_hash().0,
            })
            .context("source historical anchor missing")?;
        let mut cleanup = crate::rotation::store::source_support::CleanupCursor::default();
        loop {
            let Some(next) = self
                .store
                .call(move |s| s.cleanup_source_support_page(cleanup))
                .await?
            else {
                break;
            };
            cleanup = next;
            tokio::task::yield_now().await;
        }
        let revision = self.revision;
        let pass = self
            .store
            .call(move |s| s.begin_source_support(revision, anchor))
            .await?;
        let checked = pass.clone();
        let return_revision = returns.inventory_revision;
        self.store
            .call(move |store| store.source_pass_return_inventory(&checked, return_revision))
            .await?;
        let mut cursor = String::new();
        loop {
            let after = cursor.clone();
            let boundary = pass.boundary;
            let operations = self
                .store
                .call(move |s| s.source_recheck_page(&after, boundary))
                .await?;
            let Some(last) = operations.last() else { break };
            cursor = last.operation_id.clone();
            for operation in operations {
                let id = operation.operation_id.clone();
                let raw = self.store.call(move |s| s.prepared_bytes(&id)).await?;
                let transaction = submission::decode(&raw)?;
                ensure!(
                    transaction.txid().to_string() == operation.facts.txid
                        && u32::from(transaction.expiry_height()) == operation.facts.expiry_height,
                    "durable transaction identity mismatch"
                );
                let buried =
                    source_expiry_buried(operation.facts.expiry_height, height, confirmations);
                let wallet = self.client.wallet().read().await;
                let record = wallet.wallet_transactions.get(&transaction.txid());
                let confirmed = record
                    .and_then(|r| r.status().get_confirmed_height())
                    .map(|h| u64::from(u32::from(h)))
                    .filter(|h| {
                        *h > 0 && height >= *h && height - *h + 1 >= u64::from(confirmations)
                    });
                let exact = if let Some(record) = record {
                    let mut bytes = Zeroizing::new(Vec::new());
                    record.transaction().write(&mut *bytes)?;
                    bytes.as_slice() == raw.as_slice()
                } else {
                    false
                };
                let disposition = if operation.submission == "EXPIRED" {
                    if record.is_some_and(|r| r.status().is_confirmed()) {
                        Disposition::Conflict
                    } else {
                        Disposition::Expired
                    }
                } else if let Some(confirmed) = confirmed.filter(|_| exact) {
                    Disposition::Confirmed(confirmed)
                } else if record.is_some() {
                    Disposition::Conflict
                } else {
                    Disposition::Unavailable
                };
                drop(wallet);
                let captured = pass.clone();
                let staged = operation.clone();
                self.store
                    .call(move |s| {
                        s.stage_source_support(&captured, &staged, &raw, disposition, !buried)
                    })
                    .await?;
                let captured = pass.clone();
                let id = operation.operation_id.clone();
                let returned = returns.canonical.get(&id).copied();
                self.store
                    .call(move |store| {
                        store.stage_return_support(
                            &captured,
                            &id,
                            returned.unwrap_or_default(),
                            returned.is_none(),
                        )
                    })
                    .await?;
            }
            tokio::task::yield_now().await;
        }
        let sealed = pass.clone();
        self.store
            .call(move |s| s.seal_source_support(&sealed))
            .await?;
        Ok(pass)
    }
}

impl Treasury {
    /// Observation-only closure of retained original inputs. The caller's sender
    /// is used solely for lookup; this path cannot prepare or submit a replacement.
    pub(super) async fn reconcile_source_obligation(
        &mut self,
        operation: String,
        sender: &mut impl crate::rotation::transaction::TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<bool> {
        if self
            .reconcile_source_confirmation(operation.clone(), stop)
            .await?
        {
            return Ok(true);
        }
        let id = operation.clone();
        if self
            .store
            .call(move |s| s.source_expiry_recovery_needed(&id))
            .await?
        {
            self.recover_expired(operation, sender, stop).await?;
        }
        // This result means canonical confirmation, not generic progress.
        Ok(false)
    }

    /// Existing freshness and canonical tip checks coalesce source accounting
    /// reads. A completed pass includes unavailable observations: repeating the
    /// same whole-history scan for every unavailable operation adds no evidence.
    pub(super) async fn reconcile_source_confirmation(
        &mut self,
        operation: String,
        stop: &CancellationToken,
    ) -> Result<bool> {
        self.sync_for_command(stop).await?;
        let id = operation.clone();
        let current = self
            .store
            .call(move |s| s.source_observation_current(&id))
            .await?;
        if !current {
            // Confirmation or revocation after the reusable scan requires a new
            // complete pass; no old row may stand in for that observation.
            self.sync_once(stop).await?;
        }
        self.store
            .call(move |s| s.source_confirmation_supported(&operation))
            .await
    }
}
