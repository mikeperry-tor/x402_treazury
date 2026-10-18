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

pub(crate) mod scheduler;

pub trait FundingObservations: Clone + Send + Sync + 'static {
    type QuoteRequest: Clone + Send + Sync + 'static;
    fn quote(&self, request: Self::QuoteRequest) -> impl Future<Output = Result<Quote>> + Send;
    fn status(&self, quote: Quote) -> impl Future<Output = Result<SwapStatus>> + Send;
    fn credit(&self, job: FundingJob) -> impl Future<Output = Result<()>> + Send;
}

pub trait FundingBackend: Send + 'static {
    type Observations: FundingObservations;
    fn observations(&self) -> Self::Observations;
    /// Explicit configured groups whose vacancies may be filled ahead of demand.
    fn allocation_groups(&self) -> Vec<String> {
        Vec::new()
    }
    /// Reports whether recovery attempted the ordinary source observation.
    /// The worker reconciles remaining pending work only when it did not.
    fn recover(
        &mut self,
        _job: &FundingJob,
    ) -> impl Future<Output = Result<Option<crate::treasury::recovery::RecoveryObservation>>> + Send
    {
        async { Ok(None) }
    }

    fn swap_timeout_seconds(&self) -> u64 {
        1800
    }
    /// Read-only source refresh before preparation admission. Implementations
    /// without an owned treasury can leave refresh to their existing observer.
    fn refresh_source(&mut self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    fn ready(
        &mut self,
        _job: &FundingJob,
        _quote: &Quote,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
    fn quote_prerequisite(
        &mut self,
        job: &FundingJob,
    ) -> impl Future<Output = Result<<Self::Observations as FundingObservations>::QuoteRequest>> + Send;
    fn quote(&mut self, job: &FundingJob) -> impl Future<Output = Result<Quote>> + Send {
        async move {
            let request = self.quote_prerequisite(job).await?;
            self.observations().quote(request).await
        }
    }
    fn prepare(
        &mut self,
        job: &FundingJob,
        quote: &Quote,
    ) -> impl Future<Output = Result<()>> + Send;
    fn submit(&mut self, job: &FundingJob) -> impl Future<Output = Result<()>> + Send;
    fn reconcile(&mut self, job: &FundingJob) -> impl Future<Output = Result<bool>> + Send;
    fn status(&mut self, quote: &Quote) -> impl Future<Output = Result<SwapStatus>> + Send {
        let reader = self.observations();
        let quote = quote.clone();
        async move { reader.status(quote).await }
    }
    fn credit(&mut self, job: &FundingJob) -> impl Future<Output = Result<()>> + Send {
        let reader = self.observations();
        let job = job.clone();
        async move { reader.credit(job).await }
    }
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
    pub backend: std::sync::Arc<tokio::sync::Mutex<B>>,
    pub poll_seconds: u64,
    #[cfg(test)]
    inventory_test_hook: Option<std::sync::Arc<scheduler::InventoryTestHook>>,
}
impl<B> Clone for FundingWorker<B> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            backend: self.backend.clone(),
            poll_seconds: self.poll_seconds,
            #[cfg(test)]
            inventory_test_hook: self.inventory_test_hook.clone(),
        }
    }
}
impl<B: FundingBackend> FundingWorker<B> {
    pub fn new(store: StoreHandle, backend: B, poll_seconds: u64) -> Self {
        Self {
            store,
            backend: std::sync::Arc::new(tokio::sync::Mutex::new(backend)),
            poll_seconds,
            #[cfg(test)]
            inventory_test_hook: None,
        }
    }
    pub async fn run(self, stop: CancellationToken) -> Result<()> {
        self.schedule(now()?, stop, false).await
    }
    pub async fn tick(&self, instant: u64) -> Result<()> {
        self.schedule(instant, CancellationToken::new(), true).await
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
    async fn source_step(
        &self,
        job: &FundingJob,
        clock: &TickClock,
    ) -> Result<Option<<B::Observations as FundingObservations>::QuoteRequest>> {
        use crate::treasury::recovery::RecoveryOutcome;
        use FundingPhase::*;
        // Recovery owns the ordinary submitted-source observation when eligible,
        // including its separate two-sync expiry proof. Do not sync/lookup twice.
        let recovery =
            if crate::treasury::recovery::needs_recovery(job) && job.phase != RefundPending {
                self.backend.lock().await.recover(job).await?
            } else {
                None
            };
        let source_observed = recovery
            .as_ref()
            .is_some_and(|result| result.source_observed);
        let recovery = recovery.map(|result| result.outcome);
        if matches!(recovery, Some(RecoveryOutcome::Recovered)) {
            return Ok(None);
        }
        let facts = self.facts(job).await?;
        let mut observation_error = None;
        // An operator disposition can precede source observation (for example,
        // conflicting evidence or qualification). Retain that liability's normal
        // reconciliation; a source already confirmed by recovery needs no repeat.
        if facts.source_reconciliation_needed() && !source_observed {
            match self.backend.lock().await.reconcile(job).await {
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
        if source_observed {
            let (id, operation) = (job.id.clone(), job.operation_id.clone());
            self.store
                .call(move |s| s.reconcile_funding_phase(&id, &operation))
                .await?;
        }
        if self.finish_if_proven(job).await? {
            return Ok(None);
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
        if facts.initial_credit_recorded
            && !facts.source_pending
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
            return Ok(None);
        }
        match recovery {
            Some(RecoveryOutcome::OperatorRequired(reason)) => {
                let error = anyhow::anyhow!(RecoveryBlocked(reason));
                let diagnostic = safe_error(&error, &current.phase);
                let (id, operation) = (current.id.clone(), current.operation_id.clone());
                self.store
                    .call(move |s| {
                        s.record_source_operator_requirement(&id, &operation, &diagnostic)
                    })
                    .await?;
                return Err(error);
            }
            Some(RecoveryOutcome::Recovered | RecoveryOutcome::Waiting(_)) => {
                return observation_error.map_or(Ok(None), Err);
            }
            None => {}
        }
        if facts.source_pending {
            if facts.source_attempts == 0 && !facts.initial_credit_recorded && facts.enabled {
                let id = current.operation_id.clone();
                let operation = self.store.call(move |s| s.operation(&id)).await?;
                if operation.facts.deadline.saturating_sub(clock.now())
                    < super::transaction::MIN_QUOTE_VALIDITY_SECONDS
                {
                    let error = anyhow::anyhow!("prepared_quote_window_exhausted");
                    let diagnostic = safe_error(&error, &current.phase);
                    let (id, operation) = (current.id.clone(), current.operation_id.clone());
                    let instant = clock.now();
                    self.store
                        .call(move |s| {
                            s.mark_prepared_quote_window_exhausted(
                                &id,
                                &operation,
                                instant,
                                &diagnostic,
                            )
                        })
                        .await?;
                    return Err(error);
                }
                let result = self.backend.lock().await.submit(&current).await;
                let id = current.id.clone();
                let operation = current.operation_id.clone();
                self.store
                    .call(move |s| s.reconcile_funding_phase(&id, &operation))
                    .await?;
                result?;
            }
            return observation_error.map_or(Ok(None), Err);
        }
        match current.phase {
            Allocated if !facts.initial_credit_recorded && facts.enabled => {
                return self
                    .backend
                    .lock()
                    .await
                    .quote_prerequisite(&current)
                    .await
                    .map(Some);
            }
            Quoted if !facts.initial_credit_recorded && facts.enabled => {
                self.prepare_quoted(&current, clock).await?
            }
            Preparing => {
                let id = current.operation_id.clone();
                let prepared = self.store.call(move |s| s.operation_pending(&id)).await?;
                self.transition(&current, if prepared { Prepared } else { RecoveryRequired })
                    .await?;
            }
            _ => {}
        }
        if let Some(error) = observation_error {
            return Err(error);
        }
        Ok(None)
    }
    // Readiness and quote refresh precede PREPARING; once that phase commits,
    // only saved-byte recovery may proceed after an interrupted preparation.
    async fn prepare_quoted(&self, job: &FundingJob, clock: &TickClock) -> Result<()> {
        let quote = self.quote(job).await?;
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        let max_attempts = self.backend.lock().await.max_attempts(job);
        let suitable = self
            .store
            .call(move |s| {
                s.require_funding_attempt(&id, max_attempts)?;
                s.accepted_quote_suits_current_claim(&id, &operation)
            })
            .await?;
        if !suitable {
            let id = job.id.clone();
            let operation = job.operation_id.clone();
            return self
                .store
                .call(move |s| s.refresh_unprepared_quote_for_operation(&id, &operation))
                .await;
        }
        let instant = now()?;
        if !self
            .store
            .call(move |s| s.funding_source_ready(instant))
            .await?
        {
            // Refund-address derivation advances the wallet revision. Waiting for
            // periodic maintenance here would strand otherwise-ready quotes.
            self.backend.lock().await.refresh_source().await?;
            let instant = now()?;
            if !self
                .store
                .call(move |s| s.funding_source_ready(instant))
                .await?
            {
                return Ok(());
            }
        }
        self.backend.lock().await.ready(job, &quote).await?;
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
        self.backend.lock().await.prepare(job, &quote).await?;
        Ok(())
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
    pub max_network_fee: u64,
}
impl Backend {
    fn require_auto_funding(&self) -> Result<()> {
        ensure!(
            self.funding.auto_fund,
            super::error::FundingCondition::Disabled
        );
        Ok(())
    }
    fn policy(&self, job: &FundingJob) -> Result<(super::near::Limits, u32)> {
        let super::config::WalletConfig::ZcashRotation {
            max_funding_spend_zec,
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
                // Only quote/preparation use this field; they supply the exact
                // current claim's output ceiling before validating any quote.
                max_output: u64::MAX,
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
    type Observations = Observations;
    fn allocation_groups(&self) -> Vec<String> {
        if !self.funding.auto_fund {
            return Vec::new();
        }
        self.wallets
            .iter()
            .filter_map(|(name, config)| {
                matches!(config, super::config::WalletConfig::ZcashRotation { .. })
                    .then_some(name.clone())
            })
            .collect()
    }
    fn observations(&self) -> Observations {
        Observations {
            store: self.store.clone(),
            near: self.near.clone(),
            base: self.base.clone(),
        }
    }
    async fn recover(
        &mut self,
        job: &FundingJob,
    ) -> Result<Option<crate::treasury::recovery::RecoveryObservation>> {
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
    async fn refresh_source(&mut self) -> Result<()> {
        self.require_auto_funding()?;
        let state = self.store.call(|s| s.treasury_sync_state()).await?;
        // Pending source bytes retain ownership. Their normal reconciliation
        // drives confirmation; another waiting quote must not duplicate scans.
        if !state.outgoing_pending
            && state.sync.is_none_or(|sync| {
                sync.phase != crate::rotation::store::SyncPhase::Ready
                    || sync.snapshot_revision != state.snapshot_revision
            })
        {
            self.treasury.sync().await?;
        }
        Ok(())
    }
    async fn ready(&mut self, job: &FundingJob, quote: &Quote) -> Result<()> {
        self.require_auto_funding()?;
        let (limits, _) = self.policy(job)?;
        let job_id = job.id.clone();
        // The exact fee is checked and reserved by the serialized owner before signing.
        let reserve = quote.input;
        ensure!(
            reserve < limits.max_input,
            super::error::FundingCondition::CostLimit
        );
        let state = self.store.call(|s| s.treasury_sync_state()).await?;
        // A still-fresh low balance can predate an operator's deposit. Scan
        // before reporting insufficient capital again; pending bytes instead
        // retain their ordinary reconciliation owner.
        if !state.outgoing_pending
            && state
                .sync
                .is_some_and(|sync| sync.spendable_shielded_zatoshis < reserve)
        {
            self.treasury.sync().await?;
        }
        self.store
            .call(move |s| {
                s.check_job_permit(&job_id, reserve)?;
                let instant = now()?;
                s.check_funding_capacity(instant, reserve)
            })
            .await
    }
    async fn quote_prerequisite(&mut self, job: &FundingJob) -> Result<QuoteRequest> {
        self.require_auto_funding()?;
        let (mut limits, max_attempts) = self.policy(job)?;
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        let claim = self
            .store
            .call(move |s| {
                s.require_funding_attempt(&id, max_attempts)?;
                let claim = s.current_funding_claim(&id)?;
                s.select_quote_terms(
                    &id,
                    &operation,
                    &claim.target.to_string(),
                    &claim.maximum_output.to_string(),
                )?;
                Ok(claim)
            })
            .await?;
        let target = claim.target.to_string();
        limits.max_output = u64::try_from(claim.maximum_output)
            .context("funding output exceeds route amount range")?;
        let refund = self.treasury.refund_address(job.id.clone()).await?;
        Ok(QuoteRequest {
            job: job.clone(),
            refund,
            target,
            limits,
            max_attempts,
            confidentiality: self.funding.confidentiality.clone(),
            slippage_bps: self.funding.slippage_bps,
            deadline_seconds: self.funding.quote_deadline_seconds,
        })
    }
    async fn prepare(&mut self, job: &FundingJob, quote: &Quote) -> Result<()> {
        self.require_auto_funding()
            .map_err(|error| error.context(super::transaction::PreparationDeferred))?;
        let (mut limits, max_attempts) = self.policy(job)?;
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        let supplied = serde_json::to_value(quote)?;
        let claim = self
            .store
            .call(move |s| {
                let stored: serde_json::Value =
                    serde_json::from_slice(&s.funding_quote_for_operation(&id, &operation)?)?;
                ensure!(stored == supplied, "persisted funding quote mismatch");
                ensure!(
                    s.accepted_quote_suits_current_claim(&id, &operation)?,
                    "funding quote unsuitable for current claim"
                );
                s.current_funding_claim(&id)
            })
            .await?;
        limits.max_output = u64::try_from(claim.maximum_output)
            .context("funding output exceeds route amount range")?;
        let checked = self
            .near
            .validate_quote(
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

                operation_id: job.operation_id.clone(),
                pool_id: Some(job.pool_id.clone()),

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
            .call(move |s| {
                Ok(!s.operation_pending(&id)? && s.operation(&id)?.submission == "CONFIRMED")
            })
            .await?
        {
            return Ok(true);
        }
        Ok(matches!(
            self.treasury.reconcile(job.operation_id.clone()).await?,
            super::transaction::TransactionPresence::Confirmed { .. }
        ))
    }
    fn max_attempts(&self, job: &FundingJob) -> u32 {
        self.policy(job).map(|(_, n)| n).unwrap_or(1)
    }
}

/// Immutable unsigned input captured after the treasury-owned refund-address
/// prerequisite. It is not a funding permit and cannot authorize preparation.
#[derive(Clone)]
pub struct QuoteRequest {
    job: FundingJob,
    refund: String,
    target: String,
    limits: super::near::Limits,
    max_attempts: u32,
    confidentiality: String,
    slippage_bps: u32,
    deadline_seconds: u64,
}

#[derive(Clone)]
pub struct Observations {
    store: StoreHandle,
    near: super::near::NearClient,
    base: super::base::BaseRpc,
}
impl FundingObservations for Observations {
    type QuoteRequest = QuoteRequest;
    async fn quote(&self, request: QuoteRequest) -> Result<Quote> {
        let (job, operation, max_attempts) = (
            request.job.id.clone(),
            request.job.operation_id.clone(),
            request.max_attempts,
        );
        // Recheck before outbound acquisition: a completed prerequisite cannot
        // revive a closed operation or bypass a newly installed funding pause.
        self.store
            .call(move |s| {
                s.require_funding_attempt(&job, max_attempts)?;
                let current = s.funding_job(&job)?.context("funding job missing")?;
                let facts = s.funding_facts(&job, &operation)?;
                ensure!(
                    current.operation_id == operation
                        && current.phase == FundingPhase::Allocated
                        && facts.enabled
                        && !facts.initial_credit_recorded
                        && !facts.accepted_quote
                        && !facts.archived
                        && facts.source_submission.is_none(),
                    "funding quote boundary changed"
                );
                Ok(())
            })
            .await?;
        let assets = self
            .near
            .assets()
            .await
            .context(NearStage("asset catalog"))?;
        let instant = now()?;
        let quote_request = self.near.request(
            &assets,
            &request.job.recipient,
            &request.refund,
            &request.target,
            &request.confidentiality,
            request.slippage_bps,
            instant
                .checked_add(request.deadline_seconds)
                .context("deadline overflow")?,
            false,
        )?;
        self.near
            .quote_with_minimum(quote_request, &request.limits, instant)
            .await
            .context(NearStage("quote"))
    }
    async fn status(&self, quote: Quote) -> Result<SwapStatus> {
        self.near.status(&quote).await
    }
    async fn credit(&self, job: FundingJob) -> Result<()> {
        let balance = self.base.current_balance(&job.recipient).await?;
        self.store
            .call(move |s| {
                s.record_funding_balance(
                    &job.id,
                    &job.operation_id,
                    &job.wallet_id,
                    &job.recipient,
                    balance,
                )
            })
            .await
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
fn funding_condition(error: &anyhow::Error) -> Option<super::error::FundingCondition> {
    use super::error::{AdmissionError, FundingCondition as Condition};
    if let Some(condition) = error.downcast_ref::<Condition>() {
        return Some(*condition);
    }
    if matches!(
        error.downcast_ref::<AdmissionError>(),
        Some(AdmissionError::FundingRestricted | AdmissionError::FundingPermitDenied)
    ) {
        return Some(Condition::QualificationDenied);
    }
    if matches!(
        error.downcast_ref::<super::near::HttpStatus>(),
        Some(super::near::HttpStatus(401 | 403))
    ) {
        return Some(Condition::AuthenticationRequired);
    }
    if error.downcast_ref::<super::base::evidence::EvidenceFailure>()
        == Some(&super::base::evidence::EvidenceFailure::MalformedSupport)
    {
        return Some(Condition::EvidenceInvalid);
    }
    None
}

fn waiting_reason(error: &anyhow::Error, phase: &FundingPhase) -> Option<&'static str> {
    if matches!(
        phase,
        FundingPhase::DepositPending | FundingPhase::Complete | FundingPhase::RecoveryRequired
    ) && error.downcast_ref::<super::transaction::RecoveryEvidenceError>()
        == Some(&super::transaction::RecoveryEvidenceError::ConfirmationPending)
    {
        return Some("zcash_confirmations_pending");
    }
    None
}

/// Only fixed categories reach status. Never copy upstream bodies, URLs, keys,
/// quote addresses or arbitrary error prose into the public journal.
fn safe_error(error: &anyhow::Error, phase: &FundingPhase) -> String {
    if error.downcast_ref::<super::base::evidence::EvidenceFailure>()
        == Some(&super::base::evidence::EvidenceFailure::MalformedSupport)
    {
        return "funding_integrity_conflict; malformed_durable_evidence_support; inspect this wallet's readiness evidence; original work remains owned".into();
    }
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
    if let Some(detail) = crate::treasury::diagnostics::sync_detail(error) {
        return format!("{category}; treasury: {detail}");
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

#[cfg(test)]
#[path = "funding/credit_tests.rs"]
mod credit_tests;
