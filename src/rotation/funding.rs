//! Restartable funding coordinator. Remote observations never release source
//! reservations or mark a wallet ready; those actions require chain evidence.
use super::{
    base::now,
    near::{Quote, SwapStatus},
    store::{
        StoreHandle,
        funding::{FundingJob, FundingPhase},
    },
};
use anyhow::{Context, Result, ensure};
use std::future::Future;
use tokio_util::sync::CancellationToken;

pub trait FundingBackend {
    fn swap_timeout_seconds(&self) -> u64 {
        1800
    }
    fn ready(
        &mut self,
        _job: &FundingJob,
        _quote: &Quote,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
    fn quote(&mut self, job: &FundingJob) -> impl Future<Output = Result<Quote>> + Send;
    fn prepare(
        &mut self,
        job: &FundingJob,
        quote: &Quote,
    ) -> impl Future<Output = Result<()>> + Send;
    fn submit(&mut self, job: &FundingJob) -> impl Future<Output = Result<()>> + Send;
    fn reconcile(&mut self, job: &FundingJob) -> impl Future<Output = Result<bool>> + Send;
    fn status(&mut self, quote: &Quote) -> impl Future<Output = Result<SwapStatus>> + Send;
    fn credit(&mut self, job: &FundingJob) -> impl Future<Output = Result<()>> + Send;
    fn max_attempts(&self, job: &FundingJob) -> u32;
}
pub struct FundingWorker<B> {
    pub store: StoreHandle,
    pub backend: B,
    pub poll_seconds: u64,
}
impl<B: FundingBackend> FundingWorker<B> {
    pub async fn run(mut self, stop: CancellationToken) -> Result<()> {
        while !stop.is_cancelled() {
            // Do not cancel an accepted mutation. Backend requests are bounded;
            // the treasury owner independently observes shutdown during sync/send.
            self.tick(now()?).await?;
            tokio::select! { _=stop.cancelled()=>break, _=tokio::time::sleep(std::time::Duration::from_secs(self.poll_seconds))=>{} }
        }
        Ok(())
    }
    pub async fn tick(&mut self, instant: u64) -> Result<()> {
        // Base credit can arrive before Zcash reaches its configured depth.
        // Completed/quarantined jobs must still settle their source outbox, even
        // though the ordinary funding scheduler no longer selects them.
        let status = self.store.call(|s| s.status()).await?;
        for job in &status.funding_jobs {
            if matches!(
                job.phase,
                FundingPhase::Complete | FundingPhase::RecoveryRequired
            ) && status.treasury_operations.iter().any(|o| {
                o.operation_id == job.operation_id
                    && o.attempts > 0
                    && !matches!(o.submission.as_str(), "CONFIRMED" | "EXPIRED")
            }) && job.next_poll <= instant
            {
                let result = self
                    .backend
                    .reconcile(job)
                    .await
                    .map(|_| ())
                    .context("source_reconciliation_failed");
                self.finish_step(job, instant, result).await?;
            }
        }
        let Some(mut job) = self
            .store
            .call(move |s| s.next_funding_job(instant))
            .await?
        else {
            return Ok(());
        };
        if job.started_at.is_some_and(|started| {
            instant.saturating_sub(started) >= self.backend.swap_timeout_seconds()
        }) && !job.timed_out
        {
            let id = job.id.clone();
            self.store
                .call(move |s| s.mark_funding_timeout(&id))
                .await?;
            job.timed_out = true;
        }
        let result = self.step(&job, instant).await;
        self.finish_step(&job, instant, result).await
    }
    async fn finish_step(&self, job: &FundingJob, instant: u64, result: Result<()>) -> Result<()> {
        let error = result
            .as_ref()
            .err()
            .map(|error| safe_error(error, &job.phase));
        let streak = if error.is_some() {
            job.error_streak.saturating_add(1)
        } else {
            0
        };
        let delay = self
            .poll_seconds
            .saturating_mul(1u64 << streak.min(6))
            .min(300)
            .max(if job.timed_out { 60 } else { 1 });
        let jitter =
            job.id.bytes().map(u64::from).sum::<u64>() % self.poll_seconds.saturating_add(1).max(1);
        let next = instant.saturating_add(delay).saturating_add(jitter);
        let quote_attempt = error.is_some() && job.phase == FundingPhase::Allocated;
        let exhausted =
            quote_attempt && job.attempts.saturating_add(1) >= self.backend.max_attempts(job);
        let id = job.id.clone();
        self.store
            .call(move |s| {
                if exhausted {
                    s.advance_funding(
                        &id,
                        FundingPhase::Allocated,
                        FundingPhase::RecoveryRequired,
                    )?;
                }
                s.defer_funding(&id, next, error, quote_attempt)?;
                Ok(())
            })
            .await
    }
    async fn transition(&self, job: &FundingJob, next: FundingPhase) -> Result<()> {
        let id = job.id.clone();
        let expected = job.phase.clone();
        self.store
            .call(move |s| s.advance_funding(&id, expected, next))
            .await
    }
    async fn step(&mut self, job: &FundingJob, instant: u64) -> Result<()> {
        use FundingPhase::*;
        match job.phase {
            Allocated => {
                let quote = self.backend.quote(job).await?;
                ensure!(quote.deposit.is_some(), "cannot fund from dry quote");
                let id = job.id.clone();
                let bytes = serde_json::to_vec(&quote)?;
                self.store
                    .call(move |s| s.save_funding_quote(&id, &bytes))
                    .await?;
            }
            Quoted => {
                let status = self.store.call(|s| s.status()).await?;
                if status.outgoing_pending || !status.sync_fresh {
                    return Ok(());
                }
                let quote = self.quote(job).await?;
                self.backend.ready(job, &quote).await?;
                if quote.deadline.saturating_sub(instant) < 300 {
                    if job.attempts.saturating_add(1) >= self.backend.max_attempts(job) {
                        self.transition(job, RecoveryRequired).await?;
                        anyhow::bail!("quote_refresh_exhausted");
                    }
                    let id = job.id.clone();
                    return self
                        .store
                        .call(move |s| s.refresh_unprepared_quote(&id))
                        .await;
                }
                self.transition(job, Preparing).await?;
                // The adapter commits PREPARED with bytes+snapshot. A crash/error
                // with only PREPARING requires guarded recovery, never another send.
                self.backend.prepare(job, &quote).await?;
            }
            Preparing => {
                let id = job.operation_id.clone();
                let prepared = self.store.call(move |s| s.operation_pending(&id)).await?;
                self.transition(job, if prepared { Prepared } else { RecoveryRequired })
                    .await?;
            }
            Prepared => {
                let id = job.operation_id.clone();
                let operation = self.store.call(move |s| s.operation(&id)).await?;
                if operation.attempts == 0 {
                    if operation.facts.deadline.saturating_sub(instant) < 300 {
                        return self.transition(job, RecoveryRequired).await;
                    }
                    self.backend.submit(job).await?;
                }
                // Existing attempts, including UNKNOWN/REQUESTED, only reconcile.
                self.transition(job, DepositPending).await?;
            }
            DepositPending => {
                if self.backend.reconcile(job).await? {
                    self.transition(job, Swapping).await?;
                }
            }
            Swapping | VerifyingCredit | RefundPending => {
                let quote = self.quote(job).await?;
                match self
                    .backend
                    .status(&quote)
                    .await
                    .context("funding_status_unavailable")?
                {
                    SwapStatus::Success if job.phase != RefundPending => {
                        if job.phase == Swapping {
                            self.transition(job, VerifyingCredit).await?;
                        }
                        self.backend
                            .credit(job)
                            .await
                            .context("base_credit_unverified")?;
                    }
                    SwapStatus::Refunded | SwapStatus::Failed if job.phase != RefundPending => {
                        self.transition(job, RefundPending).await?
                    }
                    SwapStatus::IncompleteDeposit => self.transition(job, RecoveryRequired).await?,
                    _ => {} // Timeouts, unknown states and refunds cannot release funds.
                }
            }
            Complete | RecoveryRequired => {}
        }
        Ok(())
    }
    async fn quote(&self, job: &FundingJob) -> Result<Quote> {
        let id = job.id.clone();
        self.store
            .call(move |s| Ok(serde_json::from_slice(&s.funding_quote(&id)?)?))
            .await
    }
}

/// Production adapters are assembled by the owner, never by MCP arguments.
pub struct Backend {
    pub treasury: crate::treasury::actor::TreasuryHandle,
    pub store: StoreHandle,
    pub near: super::near::NearClient,
    pub base: super::base::BaseRpc,
    pub wallets: std::collections::BTreeMap<String, super::config::WalletConfig>,
    pub funding: super::config::FundingConfig,
    pub daily_limit: u64,
}
impl Backend {
    fn policy(&self, job: &FundingJob) -> Result<(super::near::Limits, u32)> {
        let super::config::WalletConfig::ZcashRotation {
            max_input_zec,
            max_fee_bps,
            max_attempts,
            ..
        } = self
            .wallets
            .get(&job.pool_name)
            .context("funding pool disabled")?
        else {
            anyhow::bail!("funding pool disabled")
        };
        let max_input = u64::try_from(super::config::zatoshis(max_input_zec)?)?;
        // The hard input cap includes source fees. The exact remainder is reserved
        // after quoting, and preparation tightens it to the actual ZIP-317 fee.
        Ok((
            super::near::Limits {
                max_input,
                max_fee: 0,
                max_fee_bps: *max_fee_bps,
            },
            *max_attempts,
        ))
    }
}
impl FundingBackend for Backend {
    fn swap_timeout_seconds(&self) -> u64 {
        self.funding.swap_timeout_seconds
    }
    async fn ready(&mut self, job: &FundingJob, _: &Quote) -> Result<()> {
        let (limits, _) = self.policy(job)?;
        let daily = self.daily_limit;
        self.store
            .call(move |s| s.check_funding_capacity(now()?, limits.max_input, daily))
            .await
    }
    async fn quote(&mut self, job: &FundingJob) -> Result<Quote> {
        let (limits, _) = self.policy(job)?;
        let refund = self.treasury.refund_address(job.id.clone()).await?;
        let assets = self.near.assets().await?;
        let instant = now()?;
        let request = super::near::request(
            &assets,
            &job.recipient,
            &refund,
            &job.target,
            &self.funding.confidentiality,
            self.funding.slippage_bps,
            instant
                .checked_add(self.funding.quote_deadline_seconds)
                .context("deadline overflow")?,
            false,
        )?;
        self.near.quote(request, &limits, instant).await
    }
    async fn prepare(&mut self, job: &FundingJob, quote: &Quote) -> Result<()> {
        let (limits, _) = self.policy(job)?;
        let checked = super::near::validate_quote(
            quote.request.clone(),
            quote.response.clone(),
            &limits,
            now()?,
        )?;
        ensure!(
            checked.request["recipient"] == job.recipient
                && checked.request["amount"] == job.target
                && checked.request["confidentiality"] == self.funding.confidentiality,
            "persisted funding binding mismatch"
        );
        self.treasury
            .prepare(super::transaction::PrepareRequest {
                operation_id: job.operation_id.clone(),
                pool_id: Some(job.pool_id.clone()),
                daily_limit_zatoshis: self.daily_limit,
                deadline: checked.deadline,
                recipient: checked.deposit.context("missing deposit")?,
                amount_zatoshis: checked.input,
                max_fee_zatoshis: limits
                    .max_input
                    .checked_sub(checked.input)
                    .context("input cap exceeded")?,
                max_input_zatoshis: limits.max_input,
            })
            .await?;
        Ok(())
    }
    async fn submit(&mut self, job: &FundingJob) -> Result<()> {
        let (limits, _) = self.policy(job)?;
        let job_id = job.id.clone();
        let quote: Quote = self
            .store
            .call(move |s| Ok(serde_json::from_slice(&s.funding_quote(&job_id)?)?))
            .await?;
        ensure!(
            quote.request["confidentiality"] == self.funding.confidentiality,
            "funding mode changed; explicit recovery required"
        );
        let id = job.operation_id.clone();
        let facts = self
            .store
            .call(move |s| Ok(s.operation(&id)?.facts))
            .await?;
        ensure!(
            facts
                .amount_zatoshis
                .checked_add(facts.fee_zatoshis)
                .is_some_and(|n| n <= limits.max_input),
            "prepared source exceeds current input cap"
        );
        self.treasury
            .submit(job.operation_id.clone(), false)
            .await?;
        Ok(())
    }
    async fn reconcile(&mut self, job: &FundingJob) -> Result<bool> {
        let id = job.operation_id.clone();
        if self
            .store
            .call(move |s| Ok(s.operation(&id)?.submission == "CONFIRMED"))
            .await?
        {
            return Ok(true);
        }
        Ok(matches!(
            self.treasury.reconcile(job.operation_id.clone()).await?,
            super::transaction::TransactionPresence::Confirmed { .. }
        ))
    }
    async fn status(&mut self, quote: &Quote) -> Result<SwapStatus> {
        self.near.status(quote).await
    }
    async fn credit(&mut self, job: &FundingJob) -> Result<()> {
        let pool = job.pool_id.clone();
        let query = self.store.call(move |s| s.chain_query(&pool)).await?;
        let view = self.base.view(query).await?;
        let balance = view
            .balances
            .get(&job.wallet_id)
            .context("missing candidate balance")?
            .to_string();
        let wallet = job.wallet_id.clone();
        self.store
            .call(move |s| {
                s.record_credit(
                    &wallet,
                    &balance,
                    &view.anchor.hash,
                    i64::try_from(view.anchor.height)?,
                )
            })
            .await
    }
    fn max_attempts(&self, job: &FundingJob) -> u32 {
        self.policy(job).map(|(_, n)| n).unwrap_or(1)
    }
}

/// Only fixed categories reach status. Never copy upstream bodies, URLs, keys,
/// quote addresses or arbitrary error prose into the public journal.
fn safe_error(error: &anyhow::Error, phase: &FundingPhase) -> &'static str {
    for cause in error.chain() {
        match cause.to_string().as_str() {
            "near_http_401" | "near_http_403" => {
                return "near_authentication_required; check selected mode and configured credentials";
            }
            "near_http_429" => return "near_rate_limited; backing off",
            "treasury_budget_exceeded" => {
                return "treasury_budget_exceeded; wait for budget or review daily_input_zec";
            }
            "treasury_insufficient_spendable_funds" => {
                return "treasury_insufficient_spendable_funds; fund and sync the shielded treasury";
            }
            "quote_refresh_exhausted" => {
                return "quote_refresh_exhausted; review deadlines and recover-unprepared explicitly";
            }
            "source_reconciliation_failed" => {
                return "source_reconciliation_failed; retain reservation and inspect wallet status";
            }
            "base_credit_unverified" => {
                return "base_credit_unverified; waiting for independent confirmed Base credit";
            }
            _ => {}
        }
    }
    match phase {
        FundingPhase::Allocated => {
            "funding_quote_failed; check route, cost caps and NEAR availability"
        }
        FundingPhase::Quoted => {
            "funding_prepare_waiting; check treasury sync, source limits and operation status"
        }
        FundingPhase::Preparing | FundingPhase::Prepared => {
            "funding_submission_unresolved; inspect operation; never issue a replacement deposit"
        }
        FundingPhase::DepositPending => {
            "source_confirmation_pending; inspect sync and reconcile the existing operation"
        }
        _ => "funding_status_unavailable; continuing bounded reconciliation of the existing swap",
    }
}

#[cfg(test)]
#[path = "../../tests/support/funding_backend.rs"]
mod tests;
