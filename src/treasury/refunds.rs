//! Per-address shielding through the backend's public calculate-only API.
use super::*;
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

/// Operation-scoped unresolved accounting from the actual sync stage. Original
/// bindings/credits stay durable in their existing journals; this is a read result.
#[derive(Debug)]
pub(crate) struct ReturnReconciliation {
    pub observed_outputs: usize,
    pub issues: Vec<ReturnAccountingIssue>,
}
#[derive(Debug)]
struct ReturnObservation {
    observed_outputs: usize,
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
            issues: observation.issues,
        };
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
                ensure!(
                    accounting
                        .bindings
                        .values()
                        .filter(|owners| owners.contains(&operation_id))
                        .count()
                        == 1
                        && accounting
                            .bindings
                            .values()
                            .any(|owners| owners.len() == 1 && owners.contains(&operation_id))
                        && !accounting.ambiguous_operations.contains(&operation_id),
                    "credited return identity and ownership unidentifiable"
                );
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
            if affected_operations.contains(operation)
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
                .is_some_and(|s| now().is_ok_and(|at| s.fresh(at, status.snapshot_revision)))
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
