//! Per-address shielding through the backend's public calculate-only API.
use super::*;
use crate::rotation::{
    store::refunds::RefundStatus,
    transaction::{PreparedTransaction, TransactionFacts},
};
use pepper_sync::wallet::OutputInterface;
use zcash_client_backend::{
    data_api::{
        CoinbaseFilter,
        wallet::{
            ConfirmationsPolicy, SpendingKeys, create_proposed_transactions, propose_shielding,
        },
    },
    fees::{DustAction, DustOutputPolicy},
    wallet::OvkPolicy,
};
use zcash_protocol::{ShieldedPool, value::Zatoshis};

impl SyncSession {
    pub(super) async fn reconcile_refunds(&self, height: u64, confirmations: u32) -> Result<()> {
        let (bindings, status) = self
            .store
            .call(|s| Ok((s.refund_bindings()?, s.status()?)))
            .await?;
        let wallet = self.client.wallet().read().await;
        let refunds = collect_refunds(&wallet, &bindings, &status, height, confirmations)?;
        drop(wallet);
        self.store
            .call(move |s| {
                for refund in refunds {
                    s.record_refund(refund)?;
                }
                Ok(())
            })
            .await
    }
}
fn collect_refunds(
    wallet: &zingolib::wallet::LightWallet,
    bindings: &[(String, String)],
    status: &crate::rotation::store::Status,
    height: u64,
    confirmations: u32,
) -> Result<Vec<RefundStatus>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut refunds = vec![];
    for record in wallet.wallet_transactions.values() {
        let Some(h) = record
            .status()
            .get_confirmed_height()
            .map(|h| u64::from(u32::from(h)))
        else {
            continue;
        };
        if h == 0 || height < h || height - h + 1 < u64::from(confirmations) {
            continue;
        }
        for coin in record.transparent_coins() {
            let txid = coin.output_id().txid().to_string();
            let index = coin.output_id().output_index();
            seen.insert((txid.clone(), index));
            if let Some((operation, _)) = bindings
                .iter()
                .find(|(_, address)| address == coin.address())
                && status
                    .treasury_operations
                    .iter()
                    .any(|o| &o.operation_id == operation && o.submission == "CONFIRMED")
            {
                refunds.push(RefundStatus {
                    txid,
                    output_index: index,
                    operation_id: operation.clone(),
                    amount: i64::try_from(coin.value())?,
                    height: i64::try_from(h)?,
                });
            }
        }
    }
    // A refunded input may already be shielded: its originating transaction
    // must still be present and confirmed even when its coin is spent.
    ensure!(
        status
            .refunds
            .iter()
            .all(|r| seen.contains(&(r.txid.clone(), r.output_index))),
        "treasury_refund_reorg: credited refund requires recovery"
    );
    Ok(refunds)
}
impl Treasury {
    /// Operator requested, one job/address per transaction. No broadcast here.
    pub async fn shield_refund(
        &mut self,
        job: String,
        daily_limit: u64,
        max_fee: u64,
        stop: &CancellationToken,
    ) -> Result<PreparedTransaction> {
        self.sync_once(stop).await?;
        let (address, recipient) = self
            .store
            .call(move |s| {
                let funding = s
                    .status()?
                    .funding_jobs
                    .into_iter()
                    .find(|j| j.id == job)
                    .context("network_identity_missing: refund job")?;
                Ok((
                    s.refund_address(&job)?.context("unknown refund address")?,
                    funding.recipient,
                ))
            })
            .await?;
        let status = self.status().await?;
        ensure!(
            status.sync_fresh && !status.outgoing_pending,
            "treasury not ready for shielding"
        );
        let operation = uuid::Uuid::new_v4().to_string();
        self.healthy = false;
        let client = self.client.as_mut().context("treasury unavailable")?;
        let _pause = client
            .pause_sync_scoped()
            .map_err(|_| anyhow::anyhow!("cannot pause sync"))?;
        let chain = self.network.chain();
        let zcash_keys::address::Address::Transparent(receiver) =
            zcash_keys::address::Address::decode(&chain, &address)
                .context("invalid refund address")?
        else {
            anyhow::bail!("refund must be transparent")
        };
        let (raw, facts) = {
            let mut wallet = client.wallet().write().await;
            let selector =
                zcash_client_backend::data_api::wallet::input_selection::GreedyInputSelector::new();
            let strategy = zcash_client_backend::fees::zip317::SingleOutputChangeStrategy::new(
                zcash_primitives::transaction::fees::zip317::FeeRule::standard(),
                None,
                ShieldedPool::Orchard,
                DustOutputPolicy::new(DustAction::AllowDustChange, None),
            );
            let confirmations = wallet.wallet_settings.min_confirmations;
            let proposal = propose_shielding::<_, _, _, _, zingolib::wallet::error::WalletError>(
                &mut *wallet,
                &chain,
                &selector,
                &strategy,
                Zatoshis::const_from_u64(10_000),
                &[receiver],
                zip32::AccountId::ZERO,
                ConfirmationsPolicy::new_symmetrical(confirmations, false),
                CoinbaseFilter::AllTransparentOutputs,
                None,
            )
            .map_err(|_| anyhow::anyhow!("refund shielding proposal failed"))?;
            ensure!(
                proposal.steps().len() == 1,
                "multi-step shielding unsupported"
            );
            let step = proposal.steps().first();
            ensure!(
                step.shielded_inputs().is_none() && !step.transparent_inputs().is_empty(),
                "shielding input mismatch"
            );
            let fee = step.balance().fee_required().into_u64();
            if fee == 0 || fee > max_fee {
                tracing::warn!(
                    fee_zatoshis = fee,
                    limit_zatoshis = max_fee,
                    "refund shielding proposal exceeds max_refund_shielding_fee_zec; standard network fee was not overridden"
                );
                anyhow::bail!("refund shielding fee exceeds max_refund_shielding_fee_zec");
            }
            let input: u64 = step
                .transparent_inputs()
                .iter()
                .map(|i| i.txout().value().into_u64())
                .sum();
            ensure!(
                step.transparent_inputs()
                    .iter()
                    .all(|i| *i.txout().script_pubkey() == receiver.script().into()),
                "shielding would link refund addresses"
            );
            // Reserve only the calculated shielding fee, before creating bytes.
            // Returned principal is internal wallet movement, not another spend.
            let id = operation.clone();
            self.store
                .call(move |s| {
                    s.bind_operation_recipient(&id, &recipient)?;
                    s.reserve(
                        &id,
                        None,
                        u32::try_from(now()? / 86400)?,
                        i64::try_from(fee)?,
                        i64::try_from(daily_limit)?,
                    )
                })
                .await?;
            // Treasury initialization supports mnemonic-backed account zero only.
            let phrase = Zeroizing::new(wallet.mnemonic_phrase().context("missing treasury seed")?);
            let mnemonic = bip0039::Mnemonic::<bip0039::English>::from_phrase(&*phrase)
                .map_err(|_| anyhow::anyhow!("invalid treasury seed"))?;
            let seed = Zeroizing::new(mnemonic.to_seed(""));
            let usk = zcash_keys::keys::UnifiedSpendingKey::from_seed(
                &chain,
                &*seed,
                zip32::AccountId::ZERO,
            )
            .map_err(|_| anyhow::anyhow!("treasury key derivation failed"))?;
            let prover = zcash_proofs::prover::LocalTxProver::with_default_location()
                .context("Sapling parameters unavailable")?;
            let ids = create_proposed_transactions::<
                _,
                _,
                std::convert::Infallible,
                _,
                std::convert::Infallible,
                _,
            >(
                &mut *wallet,
                &chain,
                &prover,
                &prover,
                &SpendingKeys::new(usk),
                OvkPolicy::Sender,
                &proposal,
                None,
            )
            .map_err(|_| anyhow::anyhow!("refund shielding calculation failed"))?;
            ensure!(ids.len() == 1, "multiple shielding transactions");
            let transaction = wallet
                .wallet_transactions
                .get(ids.first())
                .context("shield transaction missing")?
                .transaction();
            let transparent = transaction
                .transparent_bundle()
                .context("shield inputs missing")?;
            ensure!(
                transparent.vout.is_empty()
                    && transparent.vin.len() == step.transparent_inputs().len(),
                "unexpected shielding outputs/inputs"
            );
            ensure!(
                transparent.vin.iter().all(|i| step
                    .transparent_inputs()
                    .iter()
                    .any(|o| o.outpoint() == i.prevout())),
                "shielding input mismatch"
            );
            let value: i64 = [
                transaction
                    .sapling_bundle()
                    .map(|b| i64::from(*b.value_balance())),
                transaction
                    .orchard_bundle()
                    .map(|b| i64::from(*b.value_balance())),
                transaction
                    .ironwood_bundle()
                    .map(|b| i64::from(*b.value_balance())),
            ]
            .into_iter()
            .flatten()
            .sum();
            ensure!(
                i64::try_from(input)?.checked_add(value) == Some(i64::try_from(fee)?),
                "shielding fee mismatch"
            );
            let mut raw = Zeroizing::new(vec![]);
            transaction.write(&mut *raw)?;
            // Shielding moves principal within the treasury; only its fee is cost.
            let facts = TransactionFacts {
                txid: transaction.txid().to_string(),
                expiry_height: transaction.expiry_height().into(),
                amount_zatoshis: 0,
                fee_zatoshis: fee,
                deadline: now()?
                    .checked_add(86400)
                    .context("shielding deadline overflow")?,
            };
            (raw, facts)
        };
        let bytes = snapshot(client).await?;
        let revision = self.revision;
        let (revision, prepared) = self
            .store
            .call(move |s| {
                let revision =
                    s.prepare_with_facts(&operation, revision, &bytes, &raw, Some(facts))?;
                Ok((revision, PreparedTransaction::load(s, &operation)?))
            })
            .await?;
        self.revision = revision;
        self.healthy = true;
        Ok(prepared)
    }
}

#[cfg(all(test, feature = "zcash-testutils"))]
#[path = "../../tests/support/refund_observations.rs"]
mod tests;
