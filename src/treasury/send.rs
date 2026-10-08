//! Serialized, calculate-only deposit preparation and durable submission control.
#[cfg(all(test, feature = "zcash-testutils"))]
mod proving_tests;
use super::{Treasury, snapshot};
use crate::rotation::{
    base::now,
    transaction::{
        BroadcastTransaction, PrepareRequest, PreparedTransaction, SubmissionOutcome,
        TransactionFacts, TransactionPreparer, TransactionPresence, TransactionSubmission,
    },
};
use anyhow::{Context, Result, ensure};
use std::time::Duration;
use zcash_keys::address::Address;
use zcash_protocol::value::Zatoshis;
use zingo_netutils::Indexer;

impl TransactionPreparer for Treasury {
    async fn prepare(&mut self, request: PrepareRequest) -> Result<PreparedTransaction> {
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
        let instant = now()?;
        ensure!(
            request
                .deadline
                .checked_sub(instant)
                .is_some_and(|remaining| remaining
                    >= crate::rotation::transaction::MIN_QUOTE_VALIDITY_SECONDS),
            "funding_deadline_too_close"
        );
        let day = u32::try_from(instant / 86400)?;
        let id = request.operation_id.clone();
        let pool = request.pool_id.clone();
        let limit = i64::try_from(request.daily_limit_zatoshis)?;
        let reserve = i64::try_from(request.max_input_zatoshis)?;
        let observed = self
            .store
            .call(move |s| {
                s.require_spend_ready(instant, reserve as u64)?;
                let observed = s.status()?;
                if let Some(limits) = request.allocation_limits {
                    s.check_funding_allocation(&id, day, limits)?;
                }
                s.reserve(&id, pool.as_deref(), day, reserve, limit)?;
                s.set_sync_phase(crate::rotation::store::SyncPhase::Preparing)?;
                Ok(observed)
            })
            .await?;
        // Any cancellation/error from here poisons the owner. No bytes can be sent
        // before prepare_with_facts commits; reopen to recover durable state.
        self.healthy = false;
        let client = self
            .client
            .as_mut()
            .context("treasury client unavailable")?;
        let identity = crate::network::IsolationId::treasury(&observed.treasury_id);
        let mut indexer = tokio::time::timeout(
            crate::network::global().connection_timeout(Duration::from_secs(30)),
            crate::network::global().grpc(&identity, &settings.endpoint),
        )
        .await
        .map_err(|_| anyhow::anyhow!("indexer connection timed out"))??;
        client.set_indexer(indexer.clone());
        let info = tokio::time::timeout(
            crate::network::global().request_timeout(Duration::from_secs(15)),
            indexer.get_lightd_info(
                crate::network::global()
                    .request_timeout(zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT),
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("indexer network check timed out"))?
        .map_err(|_| anyhow::anyhow!("indexer network check failed"))?;
        ensure!(
            info.chain_name == self.network.rpc_name(),
            "indexer is not on mainnet"
        );

        ensure!(
            observed.sync.as_ref().is_some_and(|o| o
                .fresh(now().unwrap_or(u64::MAX), observed.snapshot_revision)
                && o.height == Some(info.block_height)),
            "treasury requires a fresh sync before calculation"
        );
        let _pause = client
            .pause_sync_scoped()
            .map_err(|_| anyhow::anyhow!("cannot pause treasury sync"))?;
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
            .await
            .map_err(|_| anyhow::anyhow!("treasury proposal failed"))?;
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
            anyhow::bail!("funding_cost_limit_exceeded");
        }
        ensure!(
            zingolib::data::proposal::total_payment_amount(&proposal)? == amount,
            "proposal amount mismatch"
        );
        let ids = client
            .calculate_stored_proposal()
            .await
            .map_err(|_| anyhow::anyhow!("treasury calculation failed"))?;
        ensure!(ids.len() == 1, "calculation produced multiple transactions");
        let (raw, facts) = {
            let wallet = client.wallet().read().await;
            let record = wallet
                .wallet_transactions
                .get(ids.first())
                .context("calculated transaction missing")?;
            let transaction = record.transaction();
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
            let mut raw = zeroize::Zeroizing::new(Vec::new());
            transaction.write(&mut *raw)?;
            let facts = TransactionFacts {
                txid: transaction.txid().to_string(),
                expiry_height: transaction.expiry_height().into(),
                amount_zatoshis: request.amount_zatoshis,
                fee_zatoshis: fee,
                deadline: request.deadline,
            };
            ensure!(facts.expiry_height > 0, "unbounded transaction expiry");
            (raw, facts)
        };
        let bytes = snapshot(client).await?;
        let expected = self.revision;
        let id = request.operation_id;
        let (revision, prepared) = self
            .store
            .call(move |s| {
                let revision = s.prepare_with_facts(&id, expected, &bytes, &raw, Some(facts))?;
                Ok((revision, PreparedTransaction::load(s, &id)?))
            })
            .await?;
        self.revision = revision;
        client.go_offline().await;
        self.healthy = true;
        Ok(prepared)
    }
}
impl Treasury {
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
        // Refresh the chain view even after restart. Sync is permitted while a send
        // is pending; spend admission is not.
        self.sync_once(stop).await?;
        let status = self.status().await?;
        let observation = status.sync.context("treasury_not_synced")?;
        ensure!(
            observation.fresh(now()?, status.snapshot_revision),
            "treasury_sync_stale"
        );
        let height = observation.height.context("missing sync height")?;
        let instant = now()?;
        let operation = id.clone();
        let (prepared, attempt) = self
            .store
            .call(move |s| {
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
    /// Only fresh local sync AND matching lookup evidence at sufficient depth can
    /// release the global send gate. Absence, expiry and rejection never release it.
    pub async fn reconcile_prepared(
        &mut self,
        id: String,
        sender: &mut impl TransactionSubmission,
        stop: &tokio_util::sync::CancellationToken,
    ) -> Result<TransactionPresence> {
        self.sync_once(stop).await?;
        let operation = id.clone();
        let prepared = self
            .store
            .call(move |s| PreparedTransaction::load(s, &operation))
            .await?;
        let presence = tokio::select! {
            biased;
            _ = stop.cancelled() => TransactionPresence::Unknown,
            result = sender.lookup(&prepared) => result.unwrap_or(TransactionPresence::Unknown),
        };
        if let TransactionPresence::Confirmed { height } = presence {
            let client = self
                .client
                .as_ref()
                .context("treasury client unavailable")?;
            let transaction = super::submission::decode(prepared.bytes())?;
            let wallet = client.wallet().read().await;
            let record = wallet
                .wallet_transactions
                .get(&transaction.txid())
                .context("transaction not seen by wallet sync")?;
            ensure!(
                record
                    .status()
                    .get_confirmed_height()
                    .map(|h| u64::from(u32::from(h)))
                    == Some(height),
                "confirmation disagrees with wallet sync"
            );
            let mut bytes = zeroize::Zeroizing::new(Vec::new());
            record.transaction().write(&mut *bytes)?;
            ensure!(
                bytes.as_slice() == prepared.bytes(),
                "confirmed transaction bytes mismatch"
            );
            drop(wallet);
            let instant = now()?;
            self.store
                .call(move |s| {
                    let status = s.status()?;
                    let observation = status.sync.context("treasury_not_synced")?;
                    ensure!(
                        observation.fresh(instant, status.snapshot_revision),
                        "treasury_sync_stale"
                    );
                    ensure!(
                        height > 0
                            && observation.height.is_some_and(|tip| tip >= height
                                && tip - height + 1 >= u64::from(observation.confirmations)),
                        "treasury_confirmation_pending"
                    );
                    let facts = s.operation(&id)?.facts;
                    let cost = facts
                        .amount_zatoshis
                        .checked_add(facts.fee_zatoshis)
                        .context("source cost overflow")?;
                    s.confirm_spend(&id, i64::try_from(cost)?, u32::try_from(instant / 86400)?)
                })
                .await?;
        }
        Ok(presence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::store::{SyncObservation, SyncPhase};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn proposal_failure_poisoning_preserves_snapshot_and_never_submits() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut treasury = Treasury::create(state.clone(), key.clone(), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        let id = treasury.status().await.unwrap().treasury_id;
        let addresses = treasury.addresses().await.unwrap();
        // Deliberately optimistic cached funds: the real empty zingolib wallet
        // must reject the proposal even after journal admission succeeds.
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
                        observed_tip_height: None,
                        height: Some(2_000_000),
                        confirmations: 3,
                        max_age_seconds: 300,
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
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
            let count = count.clone();
            async move {
                assert!(uri.path().ends_with("GetLightdInfo"));
                count.fetch_add(1, Ordering::SeqCst);
                let message = vec![0x22, 4, b'm', b'a', b'i', b'n', 0x38, 0x80, 0x89, 0x7a];
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
                300,
            )
            .unwrap(),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let request = || PrepareRequest {
            allocation_limits: None,
            operation_id: uuid::Uuid::new_v4().to_string(),
            pool_id: None,
            daily_limit_zatoshis: 1_000_000,
            deadline: now().unwrap() + 600,
            recipient: "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into(),
            amount_zatoshis: 50_000,
            max_fee_zatoshis: 20_000,
            max_input_zatoshis: 70_000,
        };
        let attempt = request();
        let operation = attempt.operation_id.clone();
        let error = treasury.prepare(attempt).await.err().unwrap();
        assert!(error.to_string().contains("proposal failed"), "{error}");
        assert!(!treasury.healthy);
        assert_eq!(
            treasury.status().await.unwrap().sync.unwrap().phase,
            SyncPhase::Preparing
        );
        assert_eq!(treasury.status().await.unwrap().snapshot_revision, 2);
        assert!(!treasury.status().await.unwrap().outgoing_pending);
        assert!(
            treasury
                .prepare(request())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("reopen")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        treasury.close().await.unwrap();
        let restored = Treasury::open(state, key, id).await.unwrap();
        assert_eq!(restored.addresses().await.unwrap(), addresses);
        restored
            .store
            .call(move |s| s.abandon_unprepared(&operation))
            .await
            .unwrap();
        restored.close().await.unwrap();
        server.abort();
    }
}
