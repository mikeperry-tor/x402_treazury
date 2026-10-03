//! Expiry alone or an indexer's NotFound response never releases exposure.
use super::*;
use crate::rotation::transaction::{
    PreparedTransaction, TransactionPresence, TransactionSubmission,
};
use pepper_sync::wallet::{
    IronwoodNote, OrchardNote, OutputInterface, SaplingNote, TransparentCoin,
};
use zingolib::wallet::LightWallet;

/// Match every owned input from the immutable calculation snapshot against the
/// fully synced wallet. Failed transaction markers alone are insufficient: its
/// originating outputs must still exist, be confirmed and be unspent.
pub(super) fn check_inputs<T: OutputInterface>(
    original: &LightWallet,
    current: &LightWallet,
    txid: &zcash_primitives::transaction::TxId,
    height: u64,
    depth: u32,
) -> Result<u64> {
    let record = original
        .wallet_transactions
        .get(txid)
        .context("prepared transaction missing from snapshot")?;
    let inputs = T::transaction_inputs(record);
    check_owned_outputs(
        original
            .wallet_outputs::<T>()
            .into_iter()
            .filter(|o| o.spend_link().is_some_and(|link| inputs.contains(&&link))),
        current,
        height,
        depth,
    )
}
fn check_owned_outputs<'a, T: OutputInterface + 'a>(
    outputs: impl IntoIterator<Item = &'a T>,
    current: &LightWallet,
    height: u64,
    depth: u32,
) -> Result<u64> {
    let mut value = 0u64;
    for old in outputs {
        let output = current
            .wallet_outputs::<T>()
            .into_iter()
            .find(|o| o.output_id() == old.output_id())
            .context("expiry recovery input missing")?;
        ensure!(
            output.spend_link() == old.spend_link() && output.value() == old.value(),
            "expiry recovery input mismatch"
        );
        let confirmed = current
            .output_transaction(output)
            .status()
            .get_confirmed_height()
            .map(|h| u64::from(u32::from(h)));
        ensure!(
            confirmed.is_some_and(|h| h > 0 && height >= h && height - h + 1 >= u64::from(depth)),
            "expiry recovery input not confirmed"
        );
        ensure!(
            current.output_spend_status(output).is_unspent(),
            "expiry recovery input remains spent or ambiguous"
        );
        value = value
            .checked_add(output.value())
            .context("input value overflow")?;
    }
    Ok(value)
}
impl Treasury {
    pub async fn recover_expired(
        &mut self,
        id: String,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<()> {
        self.sync_once(stop).await?;
        let lookup = id.clone();
        let (prepared, original) = self
            .store
            .call(move |s| {
                Ok((
                    PreparedTransaction::load(s, &lookup)?,
                    s.prepared_snapshot(&lookup)?,
                ))
            })
            .await?;
        let status = self.status().await?;
        let sync = status.sync.context("treasury not synced")?;
        let height = sync.height.context("missing sync height")?;
        ensure!(
            sync.fresh(now()?, status.snapshot_revision),
            "expiry recovery sync stale"
        );
        let facts = prepared.facts().context("operation facts missing")?;
        ensure!(
            height >= u64::from(facts.expiry_height) + u64::from(sync.confirmations),
            "transaction expiry not buried"
        );
        ensure!(!stop.is_cancelled(), "expiry recovery cancelled");
        let presence = sender.lookup(&prepared).await?;
        ensure!(
            matches!(
                presence,
                TransactionPresence::Absent | TransactionPresence::Unknown
            ),
            "expiry recovery contradicts indexer inclusion"
        );
        // Some indexers return generic Internal for missing transactions. Keep
        // that result Unknown; establish non-inclusion from fresh chain-scanned
        // unspent nullifiers/outpoints instead of interpreting server prose.
        self.sync_once(stop).await?;
        let status = self.status().await?;
        let sync = status.sync.context("treasury not synced")?;
        let height = sync.height.context("missing recovery height")?;
        ensure!(
            sync.fresh(now()?, status.snapshot_revision)
                && height >= u64::from(facts.expiry_height) + u64::from(sync.confirmations),
            "expiry recovery chain changed"
        );
        let original = LightClient::from_reader(
            original.as_slice(),
            config(self._scratch.path(), WalletConfig::Read, self.network)?,
        )
        .await
        .map_err(|_| anyhow::anyhow!("invalid preparation snapshot"))?;
        let original = original.wallet().read().await;
        let current = self
            .client
            .as_ref()
            .context("treasury unavailable")?
            .wallet()
            .read()
            .await;
        let txid = zcash_primitives::transaction::TxId::from_hex(&facts.txid)
            .context("invalid transaction identity")?;
        ensure!(
            current
                .wallet_transactions
                .get(&txid)
                .is_some_and(|r| r.status().is_failed()),
            "sync has not invalidated expired transaction"
        );
        let input = [
            check_inputs::<TransparentCoin>(
                &original,
                &current,
                &txid,
                height,
                sync.confirmations,
            )?,
            check_inputs::<SaplingNote>(&original, &current, &txid, height, sync.confirmations)?,
            check_inputs::<OrchardNote>(&original, &current, &txid, height, sync.confirmations)?,
            check_inputs::<IronwoodNote>(&original, &current, &txid, height, sync.confirmations)?,
        ]
        .into_iter()
        .try_fold(0u64, |a, b| a.checked_add(b))
        .context("input value overflow")?;
        validate_owned_value(input, facts.amount_zatoshis, facts.fee_zatoshis)?;
        drop(current);
        drop(original);
        ensure!(!stop.is_cancelled(), "expiry recovery cancelled");
        let revision = self.revision;
        self.store
            .call(move |s| s.resolve_expired(&id, revision, now()?))
            .await
    }
}

#[cfg(all(test, feature = "zcash-testutils"))]
#[path = "../../tests/support/expiry_inputs.rs"]
mod tests;

fn validate_owned_value(input: u64, amount: u64, fee: u64) -> Result<()> {
    ensure!(
        input >= amount.checked_add(fee).context("cost overflow")? && input > 0,
        "expiry recovery missing owned inputs"
    );
    Ok(())
}
#[cfg(test)]
mod value_tests {
    #[test]
    fn expiry_requires_enough_owned_value_without_overflow() {
        for (input, amount, fee, allowed) in [
            (0, 0, 0, false),
            (99, 80, 20, false),
            (100, 80, 20, true),
            (101, 80, 20, true),
            (u64::MAX, u64::MAX, 1, false),
        ] {
            assert_eq!(
                super::validate_owned_value(input, amount, fee).is_ok(),
                allowed
            );
        }
    }
}
