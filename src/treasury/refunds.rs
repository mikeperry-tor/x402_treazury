//! Per-address shielding through the backend's public calculate-only API.
use super::*;

pub(crate) mod maintenance;

// Existing backend proposal threshold, shared by read-only automatic selection.
const SHIELDING_THRESHOLD: u64 = 10_000;
use crate::rotation::{
    store::refunds::{RefundStatus, ReturnAccountingIssue},
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

/// Scope of refund accounting only; never treasury spending authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReturnAttribution {
    Scoped,
    Unidentifiable,
}

/// Unresolved accounting from the actual sync stage, with explicit attribution scope.
/// Original bindings/credits stay durable; this transient result never grants closure.
#[derive(Debug)]
pub(crate) struct ReturnReconciliation {
    pub observed_outputs: usize,
    pub attribution: ReturnAttribution,
    pub issues: Vec<ReturnAccountingIssue>,
}
#[derive(Debug)]
struct ReturnObservation {
    observed_outputs: usize,
    attribution: ReturnAttribution,
    credits: Vec<RefundStatus>,
    issues: Vec<ReturnAccountingIssue>,
}
impl SyncSession {
    pub(super) async fn reconcile_refunds(
        &self,
        height: u64,
        confirmations: u32,
    ) -> Result<ReturnReconciliation> {
        let accounting = self.store.call(|s| s.return_accounting()).await?;
        let wallet = self.client.wallet().read().await;
        let observation = collect_refunds(&wallet, &accounting, height, confirmations)?;
        drop(wallet);
        let report = ReturnReconciliation {
            observed_outputs: observation.observed_outputs,
            attribution: observation.attribution,
            issues: observation.issues,
        };
        if report.attribution == ReturnAttribution::Unidentifiable {
            tracing::warn!(
                reason = "unidentifiable_return_attribution",
                observed_outputs = report.observed_outputs,
                "Refund attribution incomplete; new credits and refund shielding remain blocked"
            );
        }
        if !observation.credits.is_empty() {
            self.store
                .call(move |s| {
                    for refund in observation.credits {
                        s.record_refund(refund)?;
                    }
                    Ok(())
                })
                .await?;
        }
        // Only fixed categories/counts go to stderr. Exact operation identities
        // remain in the typed result for dependent accounting/status consumers.
        for (reason, count) in [
            (
                "missing_return_binding",
                report
                    .issues
                    .iter()
                    .filter(|issue| {
                        matches!(issue, ReturnAccountingIssue::MissingOriginalBinding { .. })
                    })
                    .count(),
            ),
            (
                "ambiguous_return_binding",
                report
                    .issues
                    .iter()
                    .filter(|issue| {
                        matches!(
                            issue,
                            ReturnAccountingIssue::AmbiguousOriginalBinding { .. }
                        )
                    })
                    .count(),
            ),
            (
                "credited_return_evidence",
                report
                    .issues
                    .iter()
                    .filter(|issue| {
                        !matches!(
                            issue,
                            ReturnAccountingIssue::MissingOriginalBinding { .. }
                                | ReturnAccountingIssue::AmbiguousOriginalBinding { .. }
                        )
                    })
                    .count(),
            ),
        ] {
            if count > 0 {
                tracing::warn!(
                    reason,
                    affected_operations = count,
                    observed_outputs = report.observed_outputs,
                    "treasury return accounting requires operation-scoped review"
                );
            }
        }
        Ok(report)
    }
}
fn collect_refunds(
    wallet: &zingolib::wallet::LightWallet,
    accounting: &crate::rotation::store::refunds::ReturnAccounting,
    height: u64,
    confirmations: u32,
) -> Result<ReturnObservation> {
    let mut issues = accounting
        .issues
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let canonical_height = |record: &pepper_sync::wallet::WalletTransaction| {
        record
            .status()
            .get_confirmed_height()
            .map(|h| u64::from(u32::from(h)))
            .filter(|h| *h > 0 && height >= *h && height - *h + 1 >= u64::from(confirmations))
    };
    // Deduplication precedes revalidation: disputed historical credits never
    // become discoveries or receive another credit when they reappear.
    let mut known = std::collections::HashSet::with_capacity(accounting.credited.len());
    let mut attribution = ReturnAttribution::Scoped;
    for credited in &accounting.credited {
        let operation_id = credited.operation_id.clone();
        ensure!(
            !operation_id.is_empty(),
            "credited return ownership missing"
        );
        let txid = match zcash_primitives::transaction::TxId::from_hex(&credited.txid) {
            Some(txid) => txid,
            None => {
                // Without either an outpoint or one unambiguous original address,
                // we cannot identify which other discovery might duplicate this credit.
                let scoped = accounting
                    .bindings
                    .values()
                    .filter(|owners| owners.contains(&operation_id))
                    .count()
                    == 1
                    && accounting
                        .bindings
                        .values()
                        .any(|owners| owners.len() == 1 && owners.contains(&operation_id))
                    && !accounting.ambiguous_operations.contains(&operation_id);
                if !scoped {
                    attribution = ReturnAttribution::Unidentifiable;
                }
                issues.insert(ReturnAccountingIssue::InvalidCreditedIdentity { operation_id });
                continue;
            }
        };
        known.insert((txid, credited.output_index));
        if !accounting.confirmed.contains(&operation_id) {
            issues.insert(ReturnAccountingIssue::UnconfirmedCreditedSource {
                operation_id: operation_id.clone(),
            });
        }
        let Some(record) = wallet.wallet_transactions.get(&txid) else {
            issues.insert(ReturnAccountingIssue::MissingCreditedTransaction { operation_id });
            continue;
        };
        let Some(coin) = record
            .transparent_coins()
            .iter()
            .find(|coin| coin.output_id().output_index() == credited.output_index)
        else {
            issues.insert(ReturnAccountingIssue::MissingCreditedOutput { operation_id });
            continue;
        };
        if canonical_height(record).is_none() {
            issues.insert(ReturnAccountingIssue::NoncanonicalCreditedOutput {
                operation_id: operation_id.clone(),
            });
        }
        if i64::try_from(coin.value()).ok() != Some(credited.amount) {
            issues.insert(ReturnAccountingIssue::CreditedAmountConflict {
                operation_id: operation_id.clone(),
            });
        }
        if let Some(owners) = accounting.bindings.get(coin.address()) {
            if !owners.contains(&operation_id) {
                // Both original and asserted new owners must reject shielding.
                issues.insert(ReturnAccountingIssue::CreditedOwnershipConflict { operation_id });
                issues.extend(owners.iter().cloned().map(|operation_id| {
                    ReturnAccountingIssue::CreditedOwnershipConflict { operation_id }
                }));
            }
        } else {
            issues.insert(ReturnAccountingIssue::MissingOriginalBinding { operation_id });
        }
    }
    let affected_operations: std::collections::HashSet<_> = issues
        .iter()
        .map(|issue| issue.operation_id().to_owned())
        .collect();
    let mut refunds = vec![];
    let mut observed_outputs = 0;
    // The pinned dependency has no incremental unknown-output feed. Inspect its
    // resident history once without copying it; matching is keyed by address.
    for record in wallet.wallet_transactions.values() {
        let Some(h) = canonical_height(record) else {
            continue;
        };
        for coin in record.transparent_coins() {
            let Some(owners) = accounting.bindings.get(coin.address()) else {
                continue;
            };
            if owners.len() != 1 {
                issues.extend(owners.iter().cloned().map(|operation_id| {
                    ReturnAccountingIssue::AmbiguousOriginalBinding { operation_id }
                }));
                continue;
            }
            let operation = owners.first().expect("one owner");
            if accounting.ambiguous_operations.contains(operation)
                || !accounting.confirmed.contains(operation)
            {
                continue;
            }
            observed_outputs += 1;
            // Known outputs retain their original accounting even when disputed.
            // Only previously uncredited discoveries may be published.
            if attribution == ReturnAttribution::Unidentifiable
                || affected_operations.contains(operation)
                || known.contains(&(coin.output_id().txid(), coin.output_id().output_index()))
            {
                continue;
            }
            refunds.push(RefundStatus {
                txid: coin.output_id().txid().to_string(),
                output_index: coin.output_id().output_index(),
                operation_id: operation.clone(),
                amount: i64::try_from(coin.value())?,
                height: i64::try_from(h)?,
            });
        }
    }
    Ok(ReturnObservation {
        observed_outputs,
        attribution,
        credits: refunds,
        issues: issues.into_iter().collect(),
    })
}
fn require_refund_shielding_evidence(
    wallet: &zingolib::wallet::LightWallet,
    accounting: &crate::rotation::store::refunds::ReturnAccounting,
    height: u64,
    confirmations: u32,
    operation: &str,
) -> Result<()> {
    let observation = collect_refunds(wallet, accounting, height, confirmations)?;
    ensure!(
        observation.attribution == ReturnAttribution::Scoped,
        "unidentifiable_return_attribution: shielding requires accounting review"
    );
    ensure!(
        !observation
            .issues
            .iter()
            .any(|issue| issue.operation_id() == operation),
        "refund_accounting_issue: shielding requires operation-scoped review"
    );
    Ok(())
}
impl Treasury {
    /// Operator requested, one job/address per transaction. No broadcast here.
    pub async fn shield_refund(
        &mut self,
        job: String,
        max_fee: u64,
        stop: &CancellationToken,
    ) -> Result<PreparedTransaction> {
        self.sync_once(stop).await?;
        let policy_job = job.clone();
        let (address, recipient, funding_operation, accounting) = self
            .store
            .call(move |s| {
                let funding = s
                    .funding_job(&job)?
                    .context("network_identity_missing: refund job")?;
                let address = s.refund_address(&job)?.context("unknown refund address")?;
                let accounting = s.return_accounting()?;
                ensure!(
                    accounting
                        .bindings
                        .get(&address)
                        .is_some_and(|owners| owners.len() == 1
                            && owners.contains(&funding.operation_id)
                            && !accounting
                                .ambiguous_operations
                                .contains(&funding.operation_id)),
                    "ambiguous_return_binding: shielding requires operation-scoped review"
                );
                Ok((address, funding.recipient, funding.operation_id, accounting))
            })
            .await?;
        let status = self.store.call(|s| s.treasury_sync_state()).await?;
        ensure!(
            status
                .sync
                .as_ref()
                .map(|s| s.fresh(now()?, status.snapshot_revision))
                .transpose()?
                .unwrap_or(false)
                && !status.outgoing_pending,
            "treasury not ready for shielding"
        );
        let sync = status.sync.as_ref().context("treasury sync missing")?;
        let height = sync.height.context("treasury sync height missing")?;
        {
            let wallet = self
                .client
                .as_ref()
                .context("treasury unavailable")?
                .wallet()
                .read()
                .await;
            require_refund_shielding_evidence(
                &wallet,
                &accounting,
                height,
                sync.confirmations,
                &funding_operation,
            )?;
        }
        let operation = uuid::Uuid::new_v4().to_string();
        let revision = self.revision;
        let selected_address = address.clone();
        let (owner, policy) = self
            .store
            .call(move |s| {
                s.begin_manual_shielding(
                    &operation,
                    revision,
                    &policy_job,
                    &selected_address,
                    max_fee,
                )
            })
            .await?;
        self.finish_owned_refund(address, recipient, max_fee, owner, policy)
            .await
    }

    async fn finish_owned_refund(
        &mut self,
        address: String,
        recipient: String,
        max_fee: u64,
        owner: crate::rotation::store::preparation::Preparation,
        policy: crate::rotation::store::shielding::Policy,
    ) -> Result<PreparedTransaction> {
        self.healthy = false;
        let result = self
            .calculate_owned_refund(address, recipient, max_fee, &owner, policy)
            .await;
        if result.is_err() {
            // No mutated SDK snapshot can escape a failed calculate-only owner.
            // An accepted atomic commit remains authoritative after lost acknowledgement.
            self.retire_sdk()
                .await
                .context("shielding SDK retirement unresolved")?;
            self.store
                .call(move |s| s.close_retired_preparation(&owner))
                .await
                .context("shielding preparation cleanup unresolved")?;
        }
        result
    }

    async fn calculate_owned_refund(
        &mut self,
        address: String,
        recipient: String,
        max_fee: u64,
        owner: &crate::rotation::store::preparation::Preparation,
        policy: crate::rotation::store::shielding::Policy,
    ) -> Result<PreparedTransaction> {
        let operation = owner.operation().to_owned();
        let readiness = self.store.call(|s| s.treasury_sync_state()).await?;
        let synced = readiness.sync.context("treasury sync missing")?;
        ensure!(
            synced.fresh(now()?, readiness.snapshot_revision)?,
            "treasury sync stale"
        );
        let scanned_height = synced.height.context("treasury sync height missing")?;
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
        let (raw, facts, binding) = {
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
                Zatoshis::const_from_u64(SHIELDING_THRESHOLD),
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
                .try_fold(0u64, |sum, i| sum.checked_add(i.txout().value().into_u64()))
                .context("shielding input value overflow")?;
            ensure!(
                step.transparent_inputs()
                    .iter()
                    .all(|i| *i.txout().script_pubkey() == receiver.script().into()),
                "shielding would link refund addresses"
            );
            let owned = wallet.wallet_outputs::<pepper_sync::wallet::TransparentCoin>();
            for selected in step.transparent_inputs() {
                let txid =
                    zcash_primitives::transaction::TxId::from_bytes(*selected.outpoint().hash());
                let coin = owned
                    .iter()
                    .find(|coin| {
                        coin.output_id().txid() == txid
                            && coin.output_id().output_index() == selected.outpoint().n()
                    })
                    .context("shielding input ownership missing")?;
                let confirmed = wallet
                    .output_transaction(*coin)
                    .status()
                    .get_confirmed_height()
                    .map(|h| u64::from(u32::from(h)));
                ensure!(
                    coin.address() == address
                        && coin.value() == selected.txout().value().into_u64()
                        && wallet.output_spend_status(*coin).is_unspent()
                        && confirmed.is_some_and(|height| height > 0
                            && scanned_height >= height
                            && scanned_height - height + 1 >= u64::from(synced.confirmations)),
                    "shielding input is not confirmed and unspent"
                );
            }
            // Reserve only the calculated shielding fee, before creating bytes.
            // Returned principal is internal wallet movement, not another spend.
            let selected = step
                .transparent_inputs()
                .iter()
                .map(|input| crate::rotation::store::shielding::Input {
                    txid: zcash_primitives::transaction::TxId::from_bytes(*input.outpoint().hash())
                        .to_string(),
                    output_index: input.outpoint().n(),
                    amount: input.txout().value().into_u64(),
                    script: input.txout().script_pubkey().0.0.clone(),
                })
                .collect();
            let binding = crate::rotation::store::shielding::Binding::new(policy, selected, fee)?;
            let id = operation.clone();
            let reservation_owner = owner.clone();
            let reservation_inputs = binding.clone();
            self.store
                .call(move |s| {
                    s.check_shielding_inputs(&reservation_owner, &reservation_inputs)?;
                    let state = s.treasury_sync_state()?;
                    ensure!(
                        state
                            .sync
                            .as_ref()
                            .map(|sync| sync.fresh(now()?, state.snapshot_revision))
                            .transpose()?
                            .unwrap_or(false)
                            && !state.outgoing_pending,
                        "treasury not ready for shielding"
                    );
                    s.bind_operation_recipient(&id, &recipient)?;
                    s.reserve(
                        &id,
                        None,
                        u32::try_from(now()? / 86400)?,
                        i64::try_from(fee)?,
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
            let record = wallet
                .wallet_transactions
                .get(ids.first())
                .context("shield transaction missing")?;
            let owned_output = record
                .orchard_notes()
                .iter()
                .try_fold(0u64, |sum, note| sum.checked_add(note.value()))
                .context("shielding output value overflow")?;
            ensure!(
                owned_output
                    == input
                        .checked_sub(fee)
                        .context("shielding output underflow")?,
                "shielding output is not owned by treasury"
            );
            let transaction = record.transaction();
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
            .try_fold(0i64, |sum, value| sum.checked_add(value))
            .context("shielding transaction value overflow")?;
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
                deadline: None,
            };
            binding.check_transaction(&raw, &facts)?;
            (raw, facts, binding)
        };
        let bytes = snapshot(client).await?;
        let commit_owner = owner.clone();
        let (revision, prepared) = self
            .store
            .call(move |s| {
                let revision =
                    s.prepare_owned_shielding(&commit_owner, &bytes, &raw, facts, &binding)?;
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

#[cfg(all(test, feature = "zcash-testutils"))]
#[path = "refunds/proving_tests.rs"]
mod proving_tests;
