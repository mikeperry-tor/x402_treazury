//! Serialized, calculate-only deposit preparation and durable submission control.
mod prover;
#[cfg(all(test, feature = "zcash-testutils"))]
mod proving_tests;
use super::{SyncPhase, Treasury};
use crate::rotation::{
    base::now,
    transaction::{
        BroadcastTransaction, PrepareRequest, PreparedTransaction, SubmissionOutcome,
        TransactionFacts, TransactionPreparer, TransactionPresence, TransactionSubmission,
    },
};
use anyhow::{Context, Result, ensure};
use zcash_keys::address::Address;
use zcash_protocol::value::Zatoshis;
use zingo_netutils::Indexer;

type DepositProposal = zcash_client_backend::proposal::Proposal<
    zcash_primitives::transaction::fees::zip317::FeeRule,
    zingolib::wallet::output::OutputRef,
>;

fn calculate_deposit(
    wallet: &mut zingolib::wallet::LightWallet,
    proposal: &DepositProposal,
) -> Result<Vec<zcash_protocol::TxId>> {
    let chain = wallet.chain_type();
    let usk: zcash_keys::keys::UnifiedSpendingKey = wallet
        .unified_key_store
        .get(&zip32::AccountId::ZERO)
        .context("missing treasury account")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("treasury spend key unavailable"))?;
    let prover = prover::LazySaplingProver::default();
    zcash_client_backend::data_api::wallet::create_proposed_transactions::<
        _,
        _,
        std::convert::Infallible,
        _,
        std::convert::Infallible,
        _,
    >(
        wallet,
        &chain,
        &zcash_client_backend::util::SystemClock,
        &mut zingolib::utils::system_rng(),
        &prover,
        &prover,
        &zcash_client_backend::data_api::wallet::SpendingKeys::new(usk),
        zcash_client_backend::wallet::OvkPolicy::Sender,
        proposal,
        Some(zingolib::wallet::expiry::tx_expiry_height(
            &chain,
            proposal.min_target_height().into(),
        )),
    )
    .map(|ids| ids.into_iter().collect())
    .map_err(|_| anyhow::anyhow!("treasury calculation failed"))
}

impl TransactionPreparer for Treasury {
    async fn prepare(&mut self, request: PrepareRequest) -> Result<PreparedTransaction> {
        self.prepare_with_observation(request, None, None).await
    }
}
impl Treasury {
    pub(super) async fn source_work_fulfilled(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.store
            .call(move |s| s.funding_operation_fulfilled(&id))
            .await
    }
    pub(crate) async fn prepare_with_observation(
        &mut self,
        request: PrepareRequest,
        base: Option<crate::rotation::base::BaseRpc>,
        stop: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<PreparedTransaction> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed preparation"
        );
        uuid::Uuid::parse_str(&request.operation_id).context("operation ID must be a UUID")?;
        let recipient = Address::decode(&self.network.chain(), &request.recipient)
            .context("invalid mainnet recipient")?;
        let Address::Transparent(receiver) = recipient else {
            anyhow::bail!(
                "deposit requires a transparent mainnet address; TEX/unified/shielded recipients are unsupported"
            );
        };
        ensure!(
            request.amount_zatoshis > 0 && request.max_input_zatoshis > request.amount_zatoshis,
            "invalid deposit limits"
        );
        let amount = Zatoshis::from_u64(request.amount_zatoshis)?;
        let settings = self
            .sync_settings
            .clone()
            .context("sync endpoint not configured")?;
        let id = request.operation_id.clone();
        let authority = request.managed_funding.clone();
        let principal = request.amount_zatoshis;
        let deposit = request.recipient.clone();
        let binding_pool = request.pool_id.clone();
        let deadline = request.deadline;
        let observe_destination = base.is_some();
        let (treasury_id, observation_scope) = self
            .store
            .call(move |s| {
                // Existing accepted ownership must be recognized before any
                // binding/authority error can claim this request is unstarted.
                s.require_unstarted_preparation(&id)?;
                s.require_preparation_binding(
                    &id,
                    binding_pool.as_deref(),
                    &deposit,
                    principal,
                    deadline,
                )
                .map_err(|e| e.context(crate::rotation::transaction::PreparationDeferred))?;
                s.require_preparation_authority(&id, authority.as_ref())
                    .map_err(|e| e.context(crate::rotation::transaction::PreparationDeferred))?;
                let (treasury_id, _, _) = s.funding_preparation_context()?;
                let scope = if observe_destination {
                    let authority = authority
                        .as_ref()
                        .context("destination observation requires managed funding")?;
                    let job = s
                        .funding_job(&authority.job_id)?
                        .context("funding job missing")?;
                    ensure!(
                        job.operation_id == id && Some(&job.pool_id) == binding_pool.as_ref(),
                        "funding observation binding changed"
                    );
                    Some((job.pool_id, job.wallet_id))
                } else {
                    None
                };
                Ok((treasury_id, scope))
            })
            .await?;
        let observation = base.zip(observation_scope);
        let identity = crate::network::IsolationId::treasury(&treasury_id);
        // SDK selection is local. Capital waiting must not probe tips between
        // shared scan ticks merely to learn that spendable funds are unchanged.
        let mut selected = self.select_deposit(&request, amount).await?;
        if let Some(stop) = stop {
            let validation = self.can_reuse_sync(stop).await;
            if !matches!(validation, Ok(true)) {
                self.client
                    .as_mut()
                    .context("treasury client unavailable")?
                    .clear_proposal()
                    .await;
                validation.map_err(|error| {
                    error.context(crate::rotation::transaction::PreparationDeferred)
                })?;
                self.sync_once(stop).await.map_err(|error| {
                    error.context(crate::rotation::transaction::PreparationDeferred)
                })?;
                // The proposal pause is released before scanning. Select exact
                // inputs again against the changed canonical SDK snapshot.
                selected = self.select_deposit(&request, amount).await?;
            }
        }
        // Read-only checks precede any reservation, library mutation or poisoning.
        let preflight: Result<_> = async {
            // The actor already validated the canonical network/anchor above.
            // Only direct preparation needs its own network connection and check.
            if stop.is_none() {
                let mut indexer = crate::network::global()
                    .grpc(&identity, &settings.endpoint)
                    .await?;
                let info = indexer
                    .get_lightd_info(zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT)
                    .await
                    .map_err(|status| super::diagnostics::tip_failure(status, "preparation_tip"))?;
                ensure!(
                    info.chain_name == self.network.rpc_name(),
                    "indexer is not on mainnet"
                );
            }
            let instant = now()?;
            ensure!(request.deadline > instant, "funding_deadline_expired");
            Ok(())
        }
        .await;
        if let Err(error) = preflight {
            self.client
                .as_mut()
                .context("treasury client unavailable")?
                .clear_proposal()
                .await;
            return Err(error.context(crate::rotation::transaction::PreparationDeferred));
        }
        let client = self
            .client
            .as_mut()
            .context("treasury client unavailable")?;
        let (calculation, authorized_cost, fee, proposal) = selected;
        // Spendability and actual fees are known before destination discovery.
        // This advisory one-pass read cannot gate valid preparation on RPC health;
        // its positive result consumes the claim checked again at acceptance.
        if let Some((base, (pool, wallet))) = observation
            && let Err(error) = self.store.observe_balance(pool, wallet, base).await
        {
            if error.is::<crate::rotation::store::observation::BalanceStoreUnavailable>() {
                client.clear_proposal().await;
                return Err(error.context(crate::rotation::transaction::PreparationDeferred));
            }
            tracing::debug!(
                category = "funding_destination_observation_unavailable",
                "Initial funding destination observation unavailable; continuing with existing claim information"
            );
        }
        let id = request.operation_id.clone();
        let pool = request.pool_id.clone();
        let deadline = request.deadline;
        let authority = request.managed_funding;
        // A commit may succeed without an acknowledgment. Poison before dispatch;
        // only a serialized proof of no accepted intent permits reuse on failure.
        self.healthy = false;
        let acceptance = self
            .store
            .call(move |s| {
                ensure!(now()? < deadline, "funding_deadline_expired");
                s.require_preparation_authority(&id, authority.as_ref())?;
                s.accept_preparation(&id, pool.as_deref(), authorized_cost, Some(calculation))?;
                s.set_sync_phase(crate::rotation::store::SyncPhase::Preparing)?;
                Ok(())
            })
            .await;
        if let Err(error) = acceptance {
            let id = request.operation_id.clone();
            if self
                .store
                .call(move |s| s.require_unstarted_preparation(&id))
                .await
                .is_ok()
            {
                client.clear_proposal().await;
                self.healthy = true;
                return Err(error.context(crate::rotation::transaction::PreparationDeferred));
            }
            return Err(error);
        }
        let calculation_result = {
            let mut wallet = client.wallet().write().await;
            calculate_deposit(&mut wallet, &proposal)
        };
        // Public cleanup also restores the SDK proposal pause. Drop the wallet
        // guard before acquiring it through the client on either result path.
        client.clear_proposal().await;
        let ids = calculation_result?;
        ensure!(ids.len() == 1, "calculation produced multiple transactions");
        let (raw, facts) = {
            let wallet = client.wallet().read().await;
            let record = wallet
                .wallet_transactions
                .get(
                    ids.first()
                        .context("calculated transaction identity missing")?,
                )
                .context("calculated transaction missing")?;
            let transaction = record.transaction();
            let mut raw = zeroize::Zeroizing::new(Vec::new());
            transaction.write(&mut *raw)?;
            let facts: Result<TransactionFacts> = (|| {
                let transparent = transaction
                    .transparent_bundle()
                    .context("deposit output missing")?;
                ensure!(
                    transparent.vin.is_empty() && transparent.vout.len() == 1,
                    "unexpected transparent inputs or outputs"
                );
                let output = &transparent.vout[0];
                ensure!(
                    output.value() == amount && *output.script_pubkey() == receiver.script().into(),
                    "calculated recipient or amount mismatch"
                );
                let shielded_value = [
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
                .try_fold(0i64, |total, value| total.checked_add(value))
                .context("transaction value overflow")?;
                ensure!(
                    shielded_value.checked_sub(i64::try_from(request.amount_zatoshis)?)
                        == Some(i64::try_from(fee)?),
                    "calculated fee mismatch"
                );
                let facts = TransactionFacts {
                    txid: transaction.txid().to_string(),
                    expiry_height: transaction.expiry_height().into(),
                    amount_zatoshis: request.amount_zatoshis,
                    fee_zatoshis: fee,
                    deadline: Some(request.deadline),
                };
                Ok(facts)
            })();
            (raw, facts)
        };
        self.retain_calculation(request.operation_id, raw, facts)
            .await
    }
}
impl Treasury {
    async fn select_deposit(
        &mut self,
        request: &PrepareRequest,
        amount: Zatoshis,
    ) -> Result<(
        crate::rotation::store::preparation::CalculationInputs,
        i64,
        u64,
        DepositProposal,
    )> {
        // SyncSession returns an offline client after stopping its isolated
        // runtime; create/open/reconstruction also restore offline clients.
        // Only that session launches SDK sync, so no scan can race selection.
        let client = self
            .client
            .as_mut()
            .context("treasury client unavailable")?;
        let payment = zcash_client_backend::zip321::Payment::new(
            request.recipient.parse()?,
            Some(amount),
            None,
            None,
            None,
            vec![],
        )?;
        let proposal = client
            .propose_send(
                zcash_client_backend::zip321::TransactionRequest::new(vec![payment])?,
                zip32::AccountId::ZERO,
            )
            .await;
        let proposal = match proposal {
            Ok(proposal) => proposal,
            Err(zingolib::wallet::error::ProposeSendError::Proposal(
                zcash_client_backend::data_api::error::Error::InsufficientFunds { .. },
            )) => {
                let mut error =
                    anyhow::Error::new(crate::rotation::error::FundingCondition::InsufficientFunds);
                if let Ok(addresses) = super::address_info(client).await
                    && let Some(address) = addresses
                        .get(0)
                        .and_then(|address| address["encoded_address"].as_str())
                {
                    error = error.context(crate::rotation::error::TreasuryDepositAddress(
                        address.to_owned(),
                    ));
                }
                return Err(error.context(crate::rotation::transaction::PreparationDeferred));
            }
            Err(_) => {
                return Err(anyhow::anyhow!("treasury proposal failed")
                    .context(crate::rotation::transaction::PreparationDeferred));
            }
        };
        let selected_proposal: Result<_> = async {
        ensure!(
            proposal.steps().len() == 1,
            "multi-step deposits are unsupported"
        );
        let step = proposal.steps().first();
        ensure!(
            step.transparent_inputs().is_empty() && step.shielded_inputs().is_some(),
            "deposit must use shielded inputs"
        );
        let fee = zingolib::data::proposal::total_fee(&proposal)?.into_u64();
        let total = request
            .amount_zatoshis
            .checked_add(fee)
            .context("source cost overflow")?;
        if fee > request.max_fee_zatoshis || total > request.max_input_zatoshis {
            tracing::warn!(
                fee_zatoshis = fee,
                fee_limit_zatoshis = request.max_fee_zatoshis,
                total_zatoshis = total,
                transfer_limit_zatoshis = request.max_input_zatoshis,
                "funding proposal exceeds max_funding_transaction_fee_zec or max_funding_spend_zec; standard network fee was not overridden"
            );
            anyhow::bail!(crate::rotation::error::FundingCondition::CostLimit);
        }
        ensure!(
            zingolib::data::proposal::total_payment_amount(&proposal)? == amount,
            "proposal amount mismatch"
        );
        let selected = super::inputs::selected_inputs(&proposal)?;
        let calculation = crate::rotation::store::preparation::CalculationInputs {
            destination: request.recipient.clone(),
            inputs: selected,
        };
        Ok((calculation, i64::try_from(total)?, fee, proposal))
        }.await;
        match selected_proposal {
            Ok(value) => Ok(value),
            Err(error) => {
                client.clear_proposal().await;
                Err(error.context(crate::rotation::transaction::PreparationDeferred))
            }
        }
    }
    /// Explicit submission/rebroadcast only; no caller constructs a new transfer
    /// because a response was lost. The adapter must be a trusted configured sender.
    pub async fn submit_prepared(
        &mut self,
        id: String,
        sender: &mut impl TransactionSubmission,
        rebroadcast: bool,
        stop: &tokio_util::sync::CancellationToken,
    ) -> Result<SubmissionOutcome> {
        ensure!(self.healthy, "treasury requires reopen");
        // Reuse only an unchanged in-process scan after rechecking the chain.
        // Restart/mutation still requires scanning; a pending send stays owned.
        self.sync_for_command(stop).await?;
        let (_, revision, observation) =
            self.store.call(|s| s.funding_preparation_context()).await?;
        ensure!(
            (observation.phase == SyncPhase::Ready && observation.snapshot_revision == revision),
            "treasury_snapshot_changed"
        );
        let height = observation.height.context("missing sync height")?;
        let operation = id.clone();
        let durable = self
            .store
            .call(move |s| PreparedTransaction::load(s, &operation))
            .await?;
        tokio::select! {
            biased;
            _ = stop.cancelled() => anyhow::bail!("treasury stopping before submission intent"),
            result = sender.preflight(&durable, rebroadcast) => result?,
        }
        let deadline = durable.facts().context("operation facts missing")?.deadline;
        // Re-read required time after awaited preflight. Only the store mints authority;
        // cancellation after that commit remains an unknown outcome.
        let operation = id.clone();
        let (prepared, attempt) = self
            .store
            .call(move |s| {
                // Shielding has no wall deadline. Its timestamp is diagnostic only;
                // source deposits still require a readable current clock.
                let instant = now();
                let instant = if deadline.is_some() {
                    instant?
                } else {
                    instant.unwrap_or_default()
                };
                let prepared =
                    BroadcastTransaction::request(s, &operation, instant, height, rebroadcast)?;
                let attempt = prepared.attempt();
                Ok((prepared, attempt))
            })
            .await?;
        let outcome = tokio::select! {
            biased;
            _ = stop.cancelled() => SubmissionOutcome::Unknown,
            result = sender.submit(prepared) => result.unwrap_or(SubmissionOutcome::Unknown),
        };
        let accepted = outcome == SubmissionOutcome::Accepted;
        self.store
            .call(move |s| s.broadcast_result(&id, attempt, accepted))
            .await?;
        Ok(outcome)
    }
    /// The completed SDK observation proves exact-byte confirmation at configured
    /// depth. Lookup can request further observation but cannot veto that proof.
    pub async fn reconcile_prepared(
        &mut self,
        id: String,
        sender: &mut impl TransactionSubmission,
        stop: &tokio_util::sync::CancellationToken,
    ) -> Result<TransactionPresence> {
        if self.source_work_fulfilled(&id).await? {
            return Ok(TransactionPresence::Unknown);
        }
        self.sync_for_command(stop).await?;
        self.reconcile_scanned(id, sender, stop).await
    }
    pub(crate) async fn reconcile_for_observation(
        &mut self,
        id: String,
        sender: &mut impl TransactionSubmission,
        stop: &tokio_util::sync::CancellationToken,
    ) -> Result<TransactionPresence> {
        if self.source_work_fulfilled(&id).await? {
            return Ok(TransactionPresence::Unknown);
        }
        self.sync_for_observation(stop).await?;
        self.reconcile_scanned(id, sender, stop).await
    }
    async fn reconcile_scanned(
        &mut self,
        id: String,
        sender: &mut impl TransactionSubmission,
        stop: &tokio_util::sync::CancellationToken,
    ) -> Result<TransactionPresence> {
        if self.source_work_fulfilled(&id).await? {
            return Ok(TransactionPresence::Unknown);
        }
        let operation = id.clone();
        let prepared = self
            .store
            .call(move |s| PreparedTransaction::load(s, &operation))
            .await?;
        if let Some(height) = self.confirm_sdk_transaction(&prepared).await? {
            return Ok(TransactionPresence::Confirmed { height });
        }
        let expiry = prepared
            .facts()
            .context("operation facts missing")?
            .expiry_height;
        let expected_revision = self.revision;
        let expiry_buried = self
            .store
            .call(move |s| {
                let (_, revision, observation) = s.funding_preparation_context()?;
                Ok(revision == expected_revision
                    && observation.phase == SyncPhase::Ready
                    && observation.snapshot_revision == revision
                    && expiry > 0
                    && observation.height.is_some_and(|height| {
                        height >= u64::from(expiry) + u64::from(observation.confirmations)
                    }))
            })
            .await?;
        if expiry_buried {
            // A lookup cannot establish more than the next canonical input
            // proof. Unknown grants no absence, release or submission authority.
            return Ok(TransactionPresence::Unknown);
        }
        let presence = tokio::select! {
            biased;
            _ = stop.cancelled() => TransactionPresence::Unknown,
            result = sender.lookup(&prepared) => result.unwrap_or(TransactionPresence::Unknown),
        };
        if let TransactionPresence::Confirmed { height } = presence {
            let wallet = self
                .client
                .as_ref()
                .context("treasury client unavailable")?
                .wallet()
                .read()
                .await;
            let confirmation = crate::rotation::transaction::ConfirmationObservation {
                lookup_height: height,
                wallet_height: None,
                scanned_height: wallet
                    .wallet_blocks
                    .keys()
                    .next_back()
                    .map(|height| u64::from(u32::from(*height))),
            };
            if confirmation.awaiting_scan() {
                return Err(
                    crate::rotation::transaction::RecoveryEvidenceError::ConfirmationPending.into(),
                );
            }
            return Err(anyhow::Error::new(
                crate::rotation::transaction::RecoveryEvidenceError::ConfirmationMismatch,
            )
            .context(confirmation));
        }
        Ok(presence)
    }

    /// Publish an already completed SDK proof. No indexer lookup or submission
    /// is needed, including when the saved submission-attempt label is stale.
    pub(super) async fn confirm_sdk_transaction(
        &self,
        prepared: &PreparedTransaction,
    ) -> Result<Option<u64>> {
        let signed = super::submission::decode(prepared.bytes())?;
        let wallet = self
            .client
            .as_ref()
            .context("treasury client unavailable")?
            .wallet()
            .read()
            .await;
        let Some(record) = wallet.wallet_transactions.get(&signed.txid()) else {
            return Ok(None);
        };
        let Some(height) = record
            .status()
            .get_confirmed_height()
            .map(|height| u64::from(u32::from(height)))
        else {
            return Ok(None);
        };
        // ZIP 244 permits different authorizing bytes with the same committed
        // effects. Match the canonical transaction's computed identity, while
        // retaining the exact original signed journal for any disclosure.
        ensure!(
            record.transaction().txid() == signed.txid(),
            crate::rotation::transaction::RecoveryEvidenceError::TransactionMismatch
        );
        drop(wallet);
        let id = prepared.operation_id().to_owned();
        let expected_revision = self.revision;
        self.store
            .call(move |s| {
                let (_, revision, observation) = s.funding_preparation_context()?;
                ensure!(
                    revision == expected_revision
                        && observation.phase == SyncPhase::Ready
                        && observation.snapshot_revision == revision,
                    "treasury_snapshot_changed"
                );
                ensure!(
                    height > 0
                        && observation.height.is_some_and(|tip| tip >= height
                            && tip - height + 1 >= u64::from(observation.confirmations)),
                    crate::rotation::transaction::RecoveryEvidenceError::ConfirmationPending
                );
                s.confirm_spend(&id)
            })
            .await?;
        Ok(Some(height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::store::{SyncObservation, SyncPhase};
    use crate::treasury::snapshot;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn known_sdk_proposal_rejection_defers_without_poisoning_or_accepting_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut treasury = Treasury::create(state.clone(), key.clone(), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        let id = treasury.status().await.unwrap().treasury_id;
        let addresses = treasury.addresses().await.unwrap();
        // Deliberately optimistic cached funds: the real empty zingolib wallet
        // must reject the proposal without producing bytes or a budget reservation.
        let bytes = snapshot(treasury.client.as_ref().unwrap()).await.unwrap();
        treasury.revision = treasury
            .store
            .call(move |s| {
                s.save_sync_snapshot(
                    1,
                    &bytes,
                    Some(SyncObservation {
                        phase: SyncPhase::Ready,
                        last_error: None,
                        snapshot_revision: 1,
                        checked_at: Some(now()?),
                        checkpoint_at: 0,
                        scanned_blocks: 1,
                        target_height: Some(2_000_000),
                        observed_tip_height: Some(2_000_000),
                        height: Some(2_000_000),
                        confirmations: 3,
                        confirmed_pool_balances_zatoshis: None,
                        confirmed_shielded_zatoshis: 1_000_000,
                        spendable_shielded_zatoshis: 1_000_000,
                    }),
                )
            })
            .await
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let advancement = Arc::new(AtomicUsize::new(4));
        let tip = advancement.clone();
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
            let count = count.clone();
            let tip = tip.clone();
            async move {
                assert!(uri.path().ends_with("GetLightdInfo"));
                count.fetch_add(1, Ordering::SeqCst);
                let message = vec![
                    0x22,
                    4,
                    b'm',
                    b'a',
                    b'i',
                    b'n',
                    0x38,
                    0x80 + tip.load(Ordering::SeqCst) as u8,
                    0x89,
                    0x7a,
                ];
                let body = [
                    vec![0],
                    (message.len() as u32).to_be_bytes().to_vec(),
                    message,
                ]
                .concat();
                (
                    [("content-type", "application/grpc"), ("grpc-status", "0")],
                    body,
                )
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        treasury.configure_sync(
            super::super::SyncSettings::new(
                format!("http://{}", listener.local_addr().unwrap()),
                3,
            )
            .unwrap(),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let request = || PrepareRequest {
            managed_funding: None,

            operation_id: uuid::Uuid::new_v4().to_string(),
            pool_id: None,

            deadline: now().unwrap() + 600,
            recipient: "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into(),
            amount_zatoshis: 50_000,
            max_fee_zatoshis: 20_000,
            max_input_zatoshis: 70_000,
        };
        for _ in 0..2 {
            let attempt = request();
            let operation = attempt.operation_id.clone();
            let error = treasury.prepare(attempt).await.err().unwrap();
            assert!(
                error.is::<crate::rotation::transaction::PreparationDeferred>(),
                "{error:#}"
            );
            assert!(treasury.healthy);
            treasury
                .store
                .call(move |s| s.require_unstarted_preparation(&operation))
                .await
                .unwrap();
            assert!(!treasury.status().await.unwrap().outgoing_pending);
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "rejected local SDK proposals must not probe unchanged chain tips"
        );
        treasury.close().await.unwrap();
        let restored = Treasury::open(state, key, id).await.unwrap();
        assert_eq!(restored.addresses().await.unwrap(), addresses);
        restored
            .store
            .call(move |s| s.abandon_unprepared("never-accepted"))
            .await
            .unwrap();
        restored.close().await.unwrap();
        server.abort();
    }
}
