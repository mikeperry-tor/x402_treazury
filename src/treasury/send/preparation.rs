//! Calculate-only SDK mutation, fenced by its durable preparation owner.
use super::*;
use crate::rotation::store::preparation::Preparation;

impl Treasury {
    pub(super) async fn calculate_owned_preparation(
        &mut self,
        request: PrepareRequest,
        indexer: zingo_netutils::GrpcIndexer,
        amount: Zatoshis,
        recipient: Address,
        owner: &Preparation,
    ) -> Result<PreparedTransaction> {
        let Address::Transparent(receiver) = recipient else {
            anyhow::bail!("deposit requires a transparent recipient");
        };
        let client = self
            .client
            .as_mut()
            .context("treasury client unavailable")?;
        client.set_indexer(indexer);
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
            anyhow::bail!(crate::rotation::error::FundingCondition::CostLimit);
        }
        ensure!(
            zingolib::data::proposal::total_payment_amount(&proposal)? == amount,
            "proposal amount mismatch"
        );
        let id = request.operation_id.clone();
        let pool = request.pool_id.clone();
        let reserve = i64::try_from(total)?;
        let reservation_owner = owner.clone();
        self.store
            .call(move |s| {
                s.require_owned_reservation(&reservation_owner)?;
                let instant = now()?;
                s.require_spend_ready(instant, reserve as u64)?;
                ensure!(
                    request
                        .deadline
                        .checked_sub(instant)
                        .is_some_and(|remaining| remaining
                            >= crate::rotation::transaction::MIN_QUOTE_VALIDITY_SECONDS),
                    "funding_deadline_too_close"
                );
                let day = u32::try_from(instant / 86400)?;
                s.reserve(&id, pool.as_deref(), day, reserve)?;
                s.set_sync_phase(crate::rotation::store::SyncPhase::Preparing)?;
                Ok(())
            })
            .await?;
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
                deadline: Some(request.deadline),
            };
            ensure!(facts.expiry_height > 0, "unbounded transaction expiry");
            (raw, facts)
        };
        let bytes = snapshot(client).await?;
        let commit_owner = owner.clone();
        let id = request.operation_id;
        let (revision, prepared) = self
            .store
            .call(move |s| {
                let revision = s.prepare_owned(&commit_owner, &bytes, &raw, facts)?;
                Ok((revision, PreparedTransaction::load(s, &id)?))
            })
            .await?;
        self.revision = revision;
        client.go_offline().await;
        self.healthy = true;
        Ok(prepared)
    }
}
