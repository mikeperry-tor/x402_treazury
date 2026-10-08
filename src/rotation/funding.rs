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
    fn recover(
        &mut self,
        _job: &FundingJob,
    ) -> impl Future<Output = Result<Option<crate::treasury::recovery::RecoveryOutcome>>> + Send
    {
        async { Ok(None) }
    }

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
            if matches!(job.phase, FundingPhase::Complete)
                && status.treasury_operations.iter().any(|o| {
                    o.operation_id == job.operation_id
                        && o.attempts > 0
                        && !matches!(o.submission.as_str(), "CONFIRMED" | "EXPIRED")
                })
                && job.next_poll <= instant
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
        if let Some(job) = status
            .funding_jobs
            .iter()
            .filter(|job| {
                crate::treasury::recovery::needs_recovery(job)
                    && job.phase != FundingPhase::RefundPending
                    && job.next_poll <= instant
            })
            .min_by_key(|job| job.next_poll)
        {
            let recovery = self.backend.recover(job).await;
            let handled = !matches!(
                &recovery,
                Ok(None)
                    | Ok(Some(crate::treasury::recovery::RecoveryOutcome::Waiting(
                        "source_confirmed_waiting_for_swap"
                    )))
            );
            let result = match recovery {
                Ok(Some(crate::treasury::recovery::RecoveryOutcome::Recovered)) => {
                    tracing::info!(job_id = %job.id, pool_id = %job.pool_id, "funding recovery verified; retrying with a new quote under existing funding limits");
                    Ok(())
                }
                Ok(Some(crate::treasury::recovery::RecoveryOutcome::Waiting(reason))) => {
                    tracing::debug!(job_id = %job.id, pool_id = %job.pool_id, reason, "funding recovery waiting; will check again automatically");
                    Ok(())
                }
                Ok(Some(crate::treasury::recovery::RecoveryOutcome::OperatorRequired(reason))) => {
                    Err(anyhow::anyhow!(RecoveryBlocked(reason)))
                }
                Ok(None) => Ok(()),
                Err(error) => Err(error),
            };
            if handled {
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
            tracing::warn!(job_id = %job.id, pool_id = %job.pool_id, phase = ?job.phase,
                "funding exceeded swap timeout; continuing reconciliation without a replacement deposit");
        }
        let result = self.step(&job, instant).await;
        self.finish_step(&job, instant, result).await?;
        // Read committed state: backend credit/preparation can advance phases too.
        let id = job.id.clone();
        let current = self
            .store
            .call(move |s| {
                Ok(s.status()?
                    .funding_jobs
                    .into_iter()
                    .find(|job| job.id == id))
            })
            .await?;
        if let Some(current) = current
            && current.phase != job.phase
        {
            tracing::info!(
                job_id = %job.id,
                pool_id = %job.pool_id,
                from = ?job.phase,
                to = ?current.phase,
                "funding phase changed"
            );
        }
        Ok(())
    }
    async fn finish_step(&self, job: &FundingJob, instant: u64, result: Result<()>) -> Result<()> {
        let wait = result
            .as_ref()
            .err()
            .and_then(|error| waiting_reason(error, &job.phase));
        if let Some(reason) = wait {
            tracing::debug!(job_id = %job.id, pool_id = %job.pool_id, phase = ?job.phase,
                reason, "funding awaiting chain confirmation; polling automatically");
        }
        let error = result
            .as_ref()
            .err()
            .filter(|_| wait.is_none())
            .map(|error| safe_error(error, &job.phase));
        if let Some(category) = &error {
            tracing::warn!(job_id = %job.id, pool_id = %job.pool_id, phase = ?job.phase, category, "funding check failed; backing off and retaining reservations");
        }
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
                s.defer_funding(&id, next, error.as_deref(), quote_attempt)?;
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
                let target = quote.request["amount"]
                    .as_str()
                    .context("funding quote missing output target")?
                    .to_owned();
                let bytes = serde_json::to_vec(&quote)?;
                self.store
                    .call(move |s| s.save_funding_quote_with_target(&id, &bytes, &target))
                    .await?;
            }
            Quoted => self.prepare_quoted(job, instant).await?,
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
                        self.transition(job, RecoveryRequired).await?;
                        anyhow::bail!("prepared_quote_window_exhausted");
                    }
                    self.backend.submit(job).await?;
                }
                // Existing attempts, including UNKNOWN/REQUESTED, only reconcile.
                self.transition(job, DepositPending).await?;
                tracing::info!(job_id = %job.id, pool_id = %job.pool_id,
                    "funding deposit awaiting Zcash confirmations; reconciliation continues automatically");
            }
            DepositPending => {
                if self.backend.reconcile(job).await? {
                    self.transition(job, Swapping).await?;
                } else {
                    tracing::debug!(job_id = %job.id, pool_id = %job.pool_id,
                        "funding awaiting Zcash deposit confirmation; polling automatically");
                }
            }
            Swapping | VerifyingCredit | RefundPending => self.reconcile_swap(job).await?,
            Complete | RecoveryRequired => {}
        }
        Ok(())
    }
    // Readiness and quote refresh precede PREPARING; once that phase commits,
    // only saved-byte recovery may proceed after an interrupted preparation.
    async fn prepare_quoted(&mut self, job: &FundingJob, instant: u64) -> Result<()> {
        use FundingPhase::*;
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
        if let Err(error) = self.backend.prepare(job, &quote).await {
            if error.is::<super::transaction::PreparationDeferred>() {
                let id = job.id.clone();
                self.store
                    .call(move |s| s.defer_unstarted_preparation(&id))
                    .await?;
            }
            return Err(error);
        }
        Ok(())
    }

    // Provider status never releases source exposure. Even SUCCESS needs Base
    // evidence, and a refund-pending job cannot re-enter the success path.
    async fn reconcile_swap(&mut self, job: &FundingJob) -> Result<()> {
        use FundingPhase::*;
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
                tracing::warn!(job_id = %job.id, pool_id = %job.pool_id,
                    "swap failed or was refunded; awaiting refund reconciliation; retaining reservations");
                self.transition(job, RefundPending).await?
            }
            SwapStatus::IncompleteDeposit => {
                tracing::warn!(job_id = %job.id, pool_id = %job.pool_id,
                    "swap reports an incomplete deposit; operator recovery required; retaining reservations");
                self.transition(job, RecoveryRequired).await?;
            }
            _ => {} // Timeouts, unknown states and refunds cannot release funds.
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

#[derive(Debug)]
struct RecoveryBlocked(&'static str);
impl std::fmt::Display for RecoveryBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for RecoveryBlocked {}

/// Production adapters are assembled by the owner, never by MCP arguments.
pub struct Backend {
    pub treasury: crate::treasury::actor::TreasuryHandle,
    pub store: StoreHandle,
    pub near: super::near::NearClient,
    pub base: super::base::BaseRpc,
    pub wallets: std::collections::BTreeMap<String, super::config::WalletConfig>,
    pub funding: super::config::FundingConfig,
    pub daily_limit: u64,
    pub max_network_fee: u64,
}
impl Backend {
    fn policy(&self, job: &FundingJob) -> Result<(super::near::Limits, u32)> {
        let super::config::WalletConfig::ZcashRotation {
            max_funding_spend_zec,
            funding_amount_usdc,
            max_funding_amount_usdc,
            max_conversion_overhead_percent,
            max_attempts,
            ..
        } = self
            .wallets
            .get(&job.pool_name)
            .context("funding pool disabled")?
        else {
            anyhow::bail!("funding pool disabled")
        };
        let max_input = max_funding_spend_zec
            .as_deref()
            .map(super::config::zatoshis)
            .transpose()?
            .unwrap_or(i64::MAX) as u64;
        // The hard input cap includes source fees. The exact remainder is reserved
        // after quoting, and preparation tightens it to the actual ZIP-317 fee.
        Ok((
            super::near::Limits {
                max_output: super::config::usdc_amount(
                    max_funding_amount_usdc
                        .as_deref()
                        .unwrap_or(funding_amount_usdc),
                )?,
                max_input,
                max_fee: 0,
                max_fee_bps: max_conversion_overhead_percent
                    .checked_mul(100)
                    .context("invalid conversion overhead percent")?,
            },
            *max_attempts,
        ))
    }
}
impl FundingBackend for Backend {
    async fn recover(
        &mut self,
        job: &FundingJob,
    ) -> Result<Option<crate::treasury::recovery::RecoveryOutcome>> {
        self.treasury
            .recover_funding(job.id.clone(), self.max_attempts(job))
            .await
            .map(Some)
    }

    fn swap_timeout_seconds(&self) -> u64 {
        self.funding.swap_timeout_seconds
    }
    async fn ready(&mut self, job: &FundingJob, quote: &Quote) -> Result<()> {
        let (limits, _) = self.policy(job)?;
        let daily = self.daily_limit;
        let job_id = job.id.clone();
        let operation = job.operation_id.clone();
        let allocation_limits = self.funding.allocation_limits()?;
        let reserve = quote
            .input
            .checked_add(self.max_network_fee)
            .context("funding cost overflow")?
            .min(limits.max_input);
        self.store
            .call(move |s| {
                s.check_job_permit(&job_id, reserve)?;
                let instant = now()?;
                s.check_funding_allocation(
                    &operation,
                    u32::try_from(instant / 86400)?,
                    allocation_limits,
                )?;
                s.check_funding_capacity(instant, reserve, daily)
            })
            .await
    }
    async fn quote(&mut self, job: &FundingJob) -> Result<Quote> {
        let (limits, _) = self.policy(job)?;
        let refund = self.treasury.refund_address(job.id.clone()).await?;
        let assets = self
            .near
            .assets()
            .await
            .context(NearStage("asset catalog"))?;
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
        self.near
            .quote_with_minimum(request, &limits, instant)
            .await
            .context(NearStage("quote"))
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
                allocation_limits: Some(self.funding.allocation_limits()?),
                operation_id: job.operation_id.clone(),
                pool_id: Some(job.pool_id.clone()),
                daily_limit_zatoshis: self.daily_limit,
                deadline: checked.deadline,
                recipient: checked.deposit.context("missing deposit")?,
                amount_zatoshis: checked.input,
                max_fee_zatoshis: limits
                    .max_input
                    .checked_sub(checked.input)
                    .context("input cap exceeded")?
                    .min(self.max_network_fee),
                max_input_zatoshis: checked
                    .input
                    .checked_add(self.max_network_fee)
                    .context("funding cost overflow")?
                    .min(limits.max_input),
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
        let query = self
            .store
            .call(move |s| s.chain_query(&pool))
            .await
            .context(super::base::VerificationStage("funding credit query"))?;
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
            .context(super::base::VerificationStage("funding credit persistence"))
    }
    fn max_attempts(&self, job: &FundingJob) -> u32 {
        self.policy(job).map(|(_, n)| n).unwrap_or(1)
    }
}

#[derive(Debug)]
struct NearStage(&'static str);
impl std::fmt::Display for NearStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NEAR {}", self.0)
    }
}
impl std::error::Error for NearStage {}

fn near_diagnostic(error: &anyhow::Error) -> String {
    for cause in error.chain() {
        let text = cause.to_string();
        if let Some(code) = text.strip_prefix("near_http_")
            && code.len() == 3
            && code.bytes().all(|b| b.is_ascii_digit())
            && let Ok(code @ 100..=599) = code.parse::<u16>()
        {
            return format!("HTTP {code}");
        }
        let category = match text.as_str() {
            "near_unavailable: timeout" => "request timeout",
            "near_unavailable: connect" => "connection failure",
            "near_unavailable: transport" => "request transport failure",
            "near_response_failed: timeout" => "response body timeout",
            "near_response_failed: body transport" => "response body transport failure",
            "invalid NEAR JSON" => "invalid JSON response",
            "quote input limit" => "input cap exceeded",
            "quote overhead limit" | "platform fee exceeds overhead cap" => {
                "fee/overhead cap exceeded"
            }
            "quote deadline too close" => "quote deadline too close",
            "near_bridge_minimum_not_increasing" => "invalid bridge minimum",
            "required asset missing or ambiguous" => "required asset missing or ambiguous",
            "invalid token catalog" => "invalid asset catalog",
            "quote output mismatch" => "output target mismatch",
            "missing deposit" => "missing deposit address",
            "invalid transparent address" => "invalid deposit address",
            _ if text.starts_with("near_response_too_large:") => {
                "response rejected: fixed 2000000-byte limit exceeded"
            }
            _ if text.starts_with("quote binding mismatch:") => "quote binding mismatch",
            _ => continue,
        };
        return category.into();
    }
    "validation or internal error; response not accepted".into()
}

// Recognize only explicit local wait conditions, never an entire funding phase:
// sync, RPC, validation and storage failures must remain warnings.
fn waiting_reason(error: &anyhow::Error, phase: &FundingPhase) -> Option<&'static str> {
    if matches!(
        phase,
        FundingPhase::DepositPending | FundingPhase::Complete | FundingPhase::RecoveryRequired
    ) && error
        .chain()
        .any(|cause| cause.to_string() == "treasury_confirmation_pending")
    {
        return Some("zcash_confirmations_pending");
    }
    if matches!(
        phase,
        FundingPhase::Swapping | FundingPhase::VerifyingCredit
    ) && error
        .downcast_ref::<super::base::VerificationStage>()
        .is_some_and(|stage| stage.0 == "funding credit persistence")
        && error
            .chain()
            .any(|cause| cause.to_string() == "insufficient confirmed credit")
    {
        return Some("base_credit_pending");
    }
    None
}

/// Only fixed categories reach status. Never copy upstream bodies, URLs, keys,
/// quote addresses or arbitrary error prose into the public journal.
fn safe_error(error: &anyhow::Error, phase: &FundingPhase) -> String {
    if let Some(blocked) = error.downcast_ref::<RecoveryBlocked>() {
        return format!("funding_recovery_requires_operator; {}", blocked.0);
    }

    if error
        .downcast_ref::<super::base::VerificationStage>()
        .is_some_and(|stage| stage.0 == "funding credit persistence")
    {
        let reason = error.chain().find_map(|cause| match cause.to_string().as_str() {
            "insufficient confirmed credit" => Some("confirmed balance is below the funding target; waiting for independent Base credit"),
            "credit verification requires an allocated candidate" => Some("candidate role changed during verification; inspect wallet status for concurrent completion"),
            _ => None,
        }).unwrap_or("local credit state could not be committed; inspect wallet status and local storage");
        return format!(
            "base_credit_unverified; funding credit persistence: {reason}; retaining reservations"
        );
    }
    if error.is::<super::base::RpcFailure>() || error.is::<super::base::VerificationStage>() {
        return format!(
            "base_credit_unverified; {}; check configured Base RPC access and rate limits; retaining reservations",
            super::base::safe_diagnostic(error)
        );
    }
    let category = safe_error_category(error, phase);
    if let Some(stage) = error.downcast_ref::<NearStage>() {
        return format!("{category}; NEAR {}: {}", stage.0, near_diagnostic(error));
    }
    let sync_category = crate::treasury::diagnostics::sync_failure(error);
    if sync_category != "sync_internal_or_storage_failure" {
        return format!("{category}; treasury: {sync_category}");
    }
    category.into()
}
fn safe_error_category(error: &anyhow::Error, phase: &FundingPhase) -> &'static str {
    if error.downcast_ref::<super::error::AdmissionError>()
        == Some(&super::error::AdmissionError::FundingRestricted)
    {
        return "qualification_funding_denied; new funding limit is zero; preparation paused before calculation; existing active-wallet calls remain available";
    }
    if error.downcast_ref::<super::error::AdmissionError>()
        == Some(&super::error::AdmissionError::FundingPermitDenied)
    {
        return "qualification_funding_denied; funding permit missing, exhausted, changed, or unavailable; inspect qualification registry; existing active-wallet calls remain available";
    }
    if error.is::<super::transaction::PreparationDeferred>() {
        return "treasury_preparation_deferred; pre-preparation sync unavailable; no calculation started; retrying";
    }
    for cause in error.chain() {
        match cause.to_string().as_str() {
            "funding_cost_limit_exceeded" => {
                return "funding_cost_limit_exceeded; review max_funding_transaction_fee_zec and max_funding_spend_zec; standard network fee was not overridden; inspect unprepared recovery before trying again";
            }
            "funding_amount_limit_exceeded" => {
                return "funding_amount_limit_exceeded; bridge minimum exceeds max_funding_amount_usdc (defaults to funding_amount_usdc); no funds submitted";
            }
            "daily_funding_limit_exceeded" => {
                return "daily_funding_limit_exceeded; new funding paused; review daily_funding_limit_usdc and unresolved funding; existing funded wallets remain usable";
            }
            "total_funding_limit_exceeded" => {
                return "total_funding_limit_exceeded; new funding paused; review total_funding_limit_usdc; allowance does not reset on restart or refund";
            }
            "funding_allocation_evidence_missing" | "funding_allocation_evidence_invalid" => {
                return "funding_allocation_evidence_invalid; new funding paused; retained allocation history must be complete";
            }
            "near_http_401" | "near_http_403" => {
                return "near_authentication_required; check selected mode and configured credentials";
            }
            "near_bridge_minimum_unstable: three quote attempts exhausted; no funds submitted" => {
                return "near_bridge_minimum_unstable; three unsigned quote attempts exhausted; no funds submitted";
            }
            "quote input limit" => {
                return "funding_input_cap_exceeded; bridge quote exceeds max_funding_spend_zec; no funds submitted";
            }
            "near_http_429" => return "near_rate_limited; backing off",
            "treasury_budget_exceeded" => {
                return "treasury_budget_exceeded; wait for budget or review daily_treasury_spend_limit_zec";
            }
            "treasury_insufficient_spendable_funds" => {
                return "treasury_insufficient_spendable_funds; refill paused before transaction preparation; fund and sync the shielded treasury; existing funded wallets remain usable";
            }
            "prepared_quote_window_exhausted" => {
                return "prepared_quote_window_exhausted; fewer than 300 seconds remained before the swap quote deadline; no submission attempted; signed bytes retained; automatic recovery will wait for transaction expiry before retrying funding; wallet recover can inspect recovery";
            }
            "quote_refresh_exhausted" => {
                return "quote_refresh_exhausted; automatic recovery can retry within the durable recovery limit; use wallet recover for an assessment";
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
            "funding_prepare_waiting; check treasury sync, funding limits and operation status"
        }
        FundingPhase::Preparing | FundingPhase::Prepared => {
            "funding_submission_unresolved; inspect operation; never issue a replacement deposit"
        }
        FundingPhase::DepositPending => {
            "source_reconciliation_failed; automatic reconciliation will retry the existing operation"
        }
        _ => "funding_status_unavailable; continuing bounded reconciliation of the existing swap",
    }
}

#[cfg(test)]
#[path = "../../tests/support/funding_backend.rs"]
mod tests;
