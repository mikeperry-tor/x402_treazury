//! Per-address shielding through the backend's public calculate-only API.
use super::*;
use crate::rotation::transaction::{PreparedTransaction, TransactionFacts};
use pepper_sync::wallet::{KeyIdInterface, OutputInterface};
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

impl Treasury {
    /// Operator requested, one job/address per transaction. No broadcast here.
    pub async fn shield_refund(
        &mut self,
        job: String,
        max_fee: u64,
        stop: &CancellationToken,
    ) -> Result<PreparedTransaction> {
        self.sync_once(stop).await?;
        let (address, recipient) = self
            .store
            .call(move |s| {
                let funding = s
                    .funding_job(&job)?
                    .context("network_identity_missing: refund job")?;
                let address = s.refund_address(&job)?.context("unknown refund address")?;
                Ok((address, funding.recipient))
            })
            .await?;
        self.shield_address(address, Some(recipient), max_fee, stop)
            .await
    }

    pub(super) async fn shield_address(
        &mut self,
        address: String,
        recipient: Option<String>,
        max_fee: u64,
        stop: &CancellationToken,
    ) -> Result<PreparedTransaction> {
        ensure!(self.healthy, "treasury SDK unavailable");
        ensure!(!stop.is_cancelled(), "treasury stopping");
        self.store
            .call(|s| s.require_preparation_available())
            .await?;
        let operation = uuid::Uuid::new_v4().to_string();
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
            ensure!(
                wallet
                    .is_transparent_wallet_address(&receiver)
                    .is_some_and(|id| id.account_id() == zip32::AccountId::ZERO),
                "shielding address is not owned by treasury account"
            );
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
                Zatoshis::ZERO,
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
            let input = step
                .transparent_inputs()
                .iter()
                .try_fold(0u64, |sum, i| sum.checked_add(i.txout().value().into_u64()))
                .context("shielding input overflow")?;
            // The SDK also permits fee-only proposals. They cannot recover any
            // capital; reject them before accepting an input owner.
            let shielded_value = input
                .checked_sub(fee)
                .filter(|value| *value > 0)
                .context("shielding proposal has no shielded output")?;
            ensure!(
                step.transparent_inputs()
                    .iter()
                    .all(|i| *i.txout().script_pubkey() == receiver.script().into()),
                "shielding would link refund addresses"
            );
            // This captures actual selected input ownership, not returned-principal credit.
            let selected = step
                .transparent_inputs()
                .iter()
                .map(|input| crate::rotation::store::preparation::OwnedInput {
                    pool: crate::rotation::store::preparation::InputPool::Transparent,
                    txid: zcash_primitives::transaction::TxId::from_bytes(*input.outpoint().hash())
                        .to_string(),
                    output_index: input.outpoint().n(),
                })
                .collect();
            let calculation = crate::rotation::store::preparation::CalculationInputs {
                destination: address.clone(),
                inputs: selected,
            };
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
            let id = operation.clone();
            self.healthy = false;
            let stop_for_acceptance = stop.clone();
            let acceptance = self
                .store
                .call(move |s| {
                    ensure!(!stop_for_acceptance.is_cancelled(), "treasury stopping");
                    if let Some(recipient) = recipient {
                        s.bind_operation_recipient(&id, &recipient)?;
                    }
                    s.accept_preparation(&id, None, i64::try_from(fee)?, Some(calculation))
                })
                .await;
            if let Err(error) = acceptance {
                let id = operation.clone();
                if self
                    .store
                    .call(move |s| s.require_unstarted_preparation(&id))
                    .await
                    .is_ok()
                {
                    self.healthy = true;
                    return Err(error.context(crate::rotation::transaction::PreparationDeferred));
                }
                return Err(error);
            }
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
                &zcash_client_backend::util::SystemClock,
                &mut zingolib::utils::system_rng(),
                &prover,
                &prover,
                &SpendingKeys::new(usk),
                OvkPolicy::Sender,
                &proposal,
                None,
            )
            .map_err(|_| anyhow::anyhow!("refund shielding calculation failed"))?;
            ensure!(ids.len() == 1, "multiple shielding transactions");
            let record = wallet
                .wallet_transactions
                .get(ids.first())
                .context("shield transaction missing")?;
            let transaction = record.transaction();
            let mut raw = Zeroizing::new(vec![]);
            transaction.write(&mut *raw)?;
            let facts: Result<TransactionFacts> = (|| {
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
                // The pinned backend scans calculated bytes with this wallet's
                // UFVKs before returning. Require its decrypted account-zero
                // shielded notes to contain the entire principal (the SDK may
                // promote the change pool after a network upgrade).
                let owned = record
                    .orchard_notes()
                    .iter()
                    .map(|note| (note.key_id().account_id(), note.value()))
                    .chain(
                        record
                            .sapling_notes()
                            .iter()
                            .map(|note| (note.key_id().account_id(), note.value())),
                    )
                    .chain(
                        record
                            .ironwood_notes()
                            .iter()
                            .map(|note| (note.key_id().account_id(), note.value())),
                    )
                    .filter(|(account, _)| *account == zip32::AccountId::ZERO)
                    .try_fold(0u64, |sum, (_, value)| sum.checked_add(value))
                    .context("shielding output overflow")?;
                ensure!(
                    owned == shielded_value,
                    "shielding output is not owned by treasury"
                );
                // Shielding moves principal within the treasury; only its fee is cost.
                let facts = TransactionFacts {
                    txid: transaction.txid().to_string(),
                    expiry_height: transaction.expiry_height().into(),
                    amount_zatoshis: 0,
                    fee_zatoshis: fee,
                    deadline: None,
                };
                Ok(facts)
            })();
            (raw, facts)
        };
        self.retain_calculation(operation, raw, facts).await
    }
}
