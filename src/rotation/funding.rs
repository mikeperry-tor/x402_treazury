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
// Scheduling tests supply an epoch; elapsed work advances that same clock.
// Every authority decision reads it again after awaited work.
struct TickClock {
    epoch: u64,
    started: std::time::Instant,
}
impl TickClock {
    fn now(&self) -> u64 {
        self.epoch.saturating_add(self.started.elapsed().as_secs())
    }
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
        let clock = TickClock {
            epoch: instant,
            started: std::time::Instant::now(),
        };
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
                "funding exceeded swap delay warning threshold; reconciliation continues with at least 60 seconds between polls and no replacement deposit");
        }
        let result = self.step(&job, &clock).await;
        self.finish_step(&job, clock.now(), result).await?;
        // Read committed state: backend credit/preparation can advance phases too.
        let id = job.id.clone();
        let current = self.store.call(move |s| s.funding_job(&id)).await?;
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
        // Allocated failures cannot represent transaction preparation or a send.
        // One quote read per pass plus capped exponential backoff bounds outages
        // without consuming durable preparation/recovery authority.
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        self.store
            .call(move |s| {
                s.defer_funding_result(&id, Some(&operation), next, error.as_deref(), false)
            })
            .await
    }

    async fn transition(&self, job: &FundingJob, next: FundingPhase) -> Result<()> {
        let id = job.id.clone();
        let expected = job.phase.clone();
        let operation = job.operation_id.clone();
        self.store
            .call(move |s| s.advance_funding_operation(&id, &operation, expected, next))
            .await
    }
    async fn facts(&self, job: &FundingJob) -> Result<super::store::funding::FundingFacts> {
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        self.store
            .call(move |s| s.funding_facts(&id, &operation))
            .await
    }
    async fn finish_if_proven(&self, job: &FundingJob) -> Result<bool> {
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        self.store
            .call(move |s| s.finish_funding_if_proven(&id, &operation))
            .await
    }
    async fn step(&mut self, job: &FundingJob, clock: &TickClock) -> Result<()> {
        use FundingPhase::*;
        let facts = self.facts(job).await?;
        let mut observation_error = None;
        if (!facts.readiness_recorded || facts.readiness_disputed)
            && let Err(error) = self.backend.credit(job).await
        {
            observation_error = Some(error.context("base_credit_unverified"));
        }
        let facts = self.facts(job).await?;
        if facts.status_needed()
            && let Err(error) = self.reconcile_swap(job).await
        {
            observation_error.get_or_insert(error);
        }
        if facts.source_reconciliation_needed() {
            match self.backend.reconcile(job).await {
                Ok(_) => {
                    let id = job.id.clone();
                    let operation = job.operation_id.clone();
                    self.store
                        .call(move |s| s.reconcile_funding_phase(&id, &operation))
                        .await?;
                }
                Err(error) => {
                    observation_error.get_or_insert(error.context("source_reconciliation_failed"));
                }
            }
        }
        if self.finish_if_proven(job).await? {
            return Ok(());
        }
        let id = job.id.clone();
        let current = self
            .store
            .call(move |s| s.funding_job(&id))
            .await?
            .context("funding job missing")?;
        ensure!(
            current.operation_id == job.operation_id,
            "funding operation binding changed"
        );
        let facts = self.facts(&current).await?;
        if facts.readiness_recorded
            && !facts.readiness_disputed
            && matches!(current.phase, Allocated | Quoted)
        {
            let id = current.id.clone();
            let operation = current.operation_id.clone();
            self.store
                .call(move |s| s.close_unprepared_funding_for_operation(&id, &operation))
                .await?;
            ensure!(
                self.finish_if_proven(&current).await?,
                "closed destination lacks completion proof"
            );
            return Ok(());
        }
        if crate::treasury::recovery::needs_recovery(&current) && current.phase != RefundPending {
            match self.backend.recover(&current).await? {
                Some(crate::treasury::recovery::RecoveryOutcome::OperatorRequired(reason)) => {
                    return Err(anyhow::anyhow!(RecoveryBlocked(reason)));
                }
                Some(crate::treasury::recovery::RecoveryOutcome::Recovered) => return Ok(()),
                Some(crate::treasury::recovery::RecoveryOutcome::Waiting(_)) => {
                    return observation_error.map_or(Ok(()), Err);
                }
                None => {}
            }
        }
        match current.phase {
            Allocated if !facts.readiness_recorded && facts.enabled => {
                let quote = self.backend.quote(&current).await?;
                ensure!(quote.deposit.is_some(), "cannot fund from dry quote");
                let id = current.id.clone();
                let target = quote.request["amount"]
                    .as_str()
                    .context("funding quote missing output target")?
                    .to_owned();
                let bytes = serde_json::to_vec(&quote)?;
                let operation = current.operation_id.clone();
                self.store
                    .call(move |s| {
                        s.save_funding_quote_for_operation(&id, &operation, &bytes, &target)
                    })
                    .await?;
            }
            Quoted if !facts.readiness_recorded && facts.enabled => {
                self.prepare_quoted(&current, clock).await?
            }
            Preparing => {
                let id = current.operation_id.clone();
                let prepared = self.store.call(move |s| s.operation_pending(&id)).await?;
                self.transition(&current, if prepared { Prepared } else { RecoveryRequired })
                    .await?;
            }
            Prepared if facts.source_attempts == 0 => {
                let id = current.operation_id.clone();
                let operation = self.store.call(move |s| s.operation(&id)).await?;
                if operation.facts.deadline.saturating_sub(clock.now())
                    < super::transaction::MIN_QUOTE_VALIDITY_SECONDS
                {
                    self.transition(&current, RecoveryRequired).await?;
                    anyhow::bail!("prepared_quote_window_exhausted");
                }
                let result = self.backend.submit(&current).await;
                let id = current.id.clone();
                let operation = current.operation_id.clone();
                self.store
                    .call(move |s| s.reconcile_funding_phase(&id, &operation))
                    .await?;
                result?;
            }
            _ => {}
        }
        if let Some(error) = observation_error {
            return Err(error);
        }
        Ok(())
    }
    // Readiness and quote refresh precede PREPARING; once that phase commits,
    // only saved-byte recovery may proceed after an interrupted preparation.
    async fn prepare_quoted(&mut self, job: &FundingJob, clock: &TickClock) -> Result<()> {
        let instant = now()?;
        if !self
            .store
            .call(move |s| s.funding_source_ready(instant))
            .await?
        {
            return Ok(());
        }
        let quote = self.quote(job).await?;
        self.backend.ready(job, &quote).await?;
        if quote.deadline.saturating_sub(clock.now())
            < super::transaction::MIN_QUOTE_VALIDITY_SECONDS
        {
            let id = job.id.clone();
            let operation = job.operation_id.clone();
            return self
                .store
                .call(move |s| s.refresh_unprepared_quote_for_operation(&id, &operation))
                .await;
        }
        // The treasury owner accepts PREPARING after its read-only preflight.
        // A queued command cannot outrun a destination-readiness commit.
        self.backend.prepare(job, &quote).await?;
        Ok(())
    }

    // The adapter validates the original deposit and complete request echo.
    // Persisted facts accumulate; source confirmation never overwrites them.
    async fn reconcile_swap(&mut self, job: &FundingJob) -> Result<()> {
        let quote = self.quote(job).await?;
        let status = self
            .backend
            .status(&quote)
            .await
            .context("funding_status_unavailable")?;
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        let observed = now()?;
        self.store
            .call(move |s| s.record_provider_outcome(&id, &operation, &status, observed))
            .await
    }

    async fn quote(&self, job: &FundingJob) -> Result<Quote> {
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        self.store
            .call(move |s| {
                Ok(serde_json::from_slice(
                    &s.funding_quote_for_operation(&id, &operation)?,
                )?)
            })
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
    fn require_auto_funding(&self) -> Result<()> {
        ensure!(self.funding.auto_fund, "funding_disabled");
        Ok(())
    }
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
            .recover_funding(
                job.id.clone(),
                self.max_attempts(job),
                self.funding.auto_fund && self.wallets.contains_key(&job.pool_name),
            )
            .await
            .map(Some)
    }

    fn swap_timeout_seconds(&self) -> u64 {
        self.funding.swap_timeout_seconds
    }
    async fn ready(&mut self, job: &FundingJob, quote: &Quote) -> Result<()> {
        self.require_auto_funding()?;
        let (limits, _) = self.policy(job)?;
        let daily = self.daily_limit;
        let job_id = job.id.clone();
        let operation = job.operation_id.clone();
        let allocation_limits = self.funding.allocation_limits()?;
        // The exact fee is checked and reserved by the serialized owner before signing.
        let reserve = quote.input;
        ensure!(reserve < limits.max_input, "funding_cost_limit_exceeded");
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
        self.require_auto_funding()?;
        let (limits, max_attempts) = self.policy(job)?;
        let super::config::WalletConfig::ZcashRotation {
            funding_amount_usdc,
            ..
        } = self
            .wallets
            .get(&job.pool_name)
            .context("funding pool disabled")?
        else {
            anyhow::bail!("funding pool disabled");
        };
        let target = super::config::usdc_amount(funding_amount_usdc)?.to_string();
        let selected = target.clone();
        let ceiling = limits.max_output.to_string();
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        self.store
            .call(move |s| {
                s.require_funding_attempt(&id, max_attempts)?;
                s.select_quote_terms(&id, &operation, &selected, &ceiling)
            })
            .await?;
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
            &target,
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
        self.require_auto_funding()
            .map_err(|error| error.context(super::transaction::PreparationDeferred))?;
        let (limits, max_attempts) = self.policy(job)?;
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        let supplied = serde_json::to_value(quote)?;
        self.store
            .call(move |s| {
                let stored: serde_json::Value =
                    serde_json::from_slice(&s.funding_quote_for_operation(&id, &operation)?)?;
                ensure!(stored == supplied, "persisted funding quote mismatch");
                Ok(())
            })
            .await?;
        let checked = super::near::validate_quote(
            quote.request.clone(),
            quote.response.clone(),
            &limits,
            now()?,
        )
        .map_err(|error| {
            // The scheduler may have committed PREPARING while this quote crossed
            // its delivery boundary. No treasury command has been sent here.
            if error.is::<super::near::QuoteWindowExpired>() {
                error.context(super::transaction::PreparationDeferred)
            } else {
                error
            }
        })?;
        ensure!(
            checked.request["recipient"] == job.recipient
                && checked.request["confidentiality"] == self.funding.confidentiality,
            "persisted funding binding mismatch"
        );
        self.treasury
            .prepare(super::transaction::PrepareRequest {
                managed_funding: Some(super::transaction::ManagedFundingAuthority {
                    job_id: job.id.clone(),
                    auto_fund: self.funding.auto_fund,
                    max_attempts,
                }),
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
        self.require_auto_funding()?;
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
        let candidate = job.wallet_id.clone();
        let gate = self.store.group_gate(&job.pool_id)?;
        let capture_guard = std::sync::Arc::new(gate.clone().lock_owned().await);
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        let historical = self
            .store
            .call_with_guard(capture_guard, move |s| {
                s.funding_readiness_reverification(&id, &operation)
            })
            .await?;
        if let Some(request) = historical {
            let certificate = match self.base.verify_historical_balance(request).await {
                Ok(certificate) => certificate,
                Err(error) => {
                    let guard = std::sync::Arc::new(gate.clone().lock_owned().await);
                    return self
                        .store
                        .call_with_guard(guard, move |s| {
                            s.record_base_failure(&error)?;
                            Err(error)
                        })
                        .await;
                }
            };
            let id = job.id.clone();
            let operation = job.operation_id.clone();
            let guard = std::sync::Arc::new(gate.lock_owned().await);
            return self
                .store
                .call_with_guard(guard, move |s| {
                    s.restore_funding_readiness_support(&id, &operation, certificate)
                })
                .await;
        }
        let capture_guard = std::sync::Arc::new(gate.clone().lock_owned().await);
        let query = self
            .store
            .call_with_guard(capture_guard, move |s| {
                s.scoped_chain_query(&pool, vec![candidate], vec![])
            })
            .await
            .context(super::base::VerificationStage("funding credit query"))?;
        let view = match self.base.view(query).await {
            Ok(view) => view,
            Err(error) => {
                let guard = std::sync::Arc::new(gate.clone().lock_owned().await);
                return self
                    .store
                    .call_with_guard(guard, move |s| {
                        s.record_base_failure(&error)?;
                        Err(error)
                    })
                    .await;
            }
        };
        ensure!(
            view.balances.contains_key(&job.wallet_id),
            "missing candidate balance"
        );
        let wallet = job.wallet_id.clone();
        let job_id = job.id.clone();
        let operation = job.operation_id.clone();
        // Acquisition remains unsigned and independent; publication shares the
        // payment owner's gate through the accepted store job's completion.
        let guard = std::sync::Arc::new(gate.lock_owned().await);
        self.store
            .call_with_guard(guard, move |s| {
                s.record_verified_funding_credit(&wallet, Some((&job_id, &operation)), view)
                    .map(|_| ())
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
    if error
        .chain()
        .any(|cause| cause.to_string() == "funding_disabled")
    {
        return "funding_disabled; new funding is paused by current configuration; original work remains owned";
    }
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
