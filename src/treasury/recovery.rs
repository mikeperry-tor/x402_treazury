//! Funding recovery never submits transactions or relaxes canonical expiry evidence.
use super::*;
use crate::rotation::{
    store::funding::{FundingJob, FundingPhase},
    transaction::TransactionSubmission,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum RecoveryOutcome {
    Recovered,
    Waiting(&'static str),
    OperatorRequired(&'static str),
}

pub(crate) fn needs_recovery(job: &FundingJob) -> bool {
    job.phase == FundingPhase::RecoveryRequired
        || job.phase == FundingPhase::RefundPending
        || (job.timed_out
            && matches!(
                job.phase,
                FundingPhase::Allocated
                    | FundingPhase::Quoted
                    | FundingPhase::Preparing
                    | FundingPhase::Prepared
                    | FundingPhase::DepositPending
            ))
}

// Missing bytes are not proof that an interrupted preparer did no work.
fn safely_unprepared(job: &FundingJob) -> bool {
    matches!(job.phase, FundingPhase::Allocated | FundingPhase::Quoted)
        || (job.phase == FundingPhase::RecoveryRequired
            && job.last_error.as_deref().is_some_and(|e| {
                e.starts_with("quote_refresh_exhausted;") || e.starts_with("funding_quote_failed;")
            }))
}

pub(crate) fn can_recover(status: &Status, job: &FundingJob) -> bool {
    if job
        .last_error
        .as_deref()
        .is_some_and(|e| e.starts_with("funding_recovery_requires_operator;"))
    {
        return false;
    }
    if job.phase == FundingPhase::RefundPending {
        return false;
    }
    if job.phase != FundingPhase::RecoveryRequired {
        return true;
    }
    status
        .treasury_operations
        .iter()
        .find(|op| op.operation_id == job.operation_id)
        .map(|op| !matches!(op.submission.as_str(), "CONFIRMED" | "EXPIRED"))
        .unwrap_or_else(|| safely_unprepared(job))
}

fn evidence_outcome(error: &anyhow::Error) -> Option<RecoveryOutcome> {
    for cause in error.chain() {
        match cause.to_string().as_str() {
            "transaction expiry not buried" | "sync has not invalidated expired transaction" => {
                return Some(RecoveryOutcome::Waiting("waiting_for_transaction_expiry"));
            }
            "expiry recovery requires zero observed tip lag; synchronize again before releasing reservations"
            | "expiry recovery sync stale"
            | "expiry recovery chain changed" => {
                return Some(RecoveryOutcome::Waiting("waiting_for_fresh_canonical_sync"));
            }
            "expiry recovery input missing"
            | "expiry recovery input mismatch"
            | "expiry recovery input remains spent or ambiguous"
            | "expiry recovery contradicts indexer inclusion"
            | "confirmed transaction bytes mismatch"
            | "confirmation disagrees with wallet sync" => {
                return Some(RecoveryOutcome::OperatorRequired(
                    "conflicting_transaction_or_input_evidence",
                ));
            }
            _ => {}
        }
    }
    None
}

impl Treasury {
    pub async fn recover_funding_job(
        &mut self,
        job_id: String,
        max_attempts: u32,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<RecoveryOutcome> {
        match self
            .recover_funding_job_inner(job_id, max_attempts, sender, stop)
            .await
        {
            Err(error) => match evidence_outcome(&error) {
                Some(outcome) => Ok(outcome),
                None => Err(error),
            },
            result => result,
        }
    }
    async fn recover_funding_job_inner(
        &mut self,
        job_id: String,
        max_attempts: u32,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<RecoveryOutcome> {
        use RecoveryOutcome::*;
        ensure!(self.healthy, "treasury requires reopen");
        if crate::qualification::active() {
            return Ok(OperatorRequired("qualification_recovery_requires_review"));
        }
        let status = self.status().await?;
        let job = status
            .funding_jobs
            .iter()
            .find(|j| j.id == job_id)
            .context("recovery job missing")?;
        if !needs_recovery(job) {
            if job.last_error.as_deref().is_some_and(|reason| {
                [
                    "treasury_insufficient_spendable_funds",
                    "treasury_budget_exceeded",
                    "daily_funding_limit_exceeded",
                    "total_funding_limit_exceeded",
                    "funding_cost_limit_exceeded",
                    "funding_amount_limit_exceeded",
                    "qualification_funding_denied",
                    "near_authentication_required",
                ]
                .iter()
                .any(|prefix| reason.starts_with(prefix))
            }) {
                return Ok(OperatorRequired(
                    "funding_balance_budget_or_configuration_required",
                ));
            }
            return Ok(Waiting("funding_in_progress"));
        }
        if job.last_error.as_deref().is_some_and(|reason| {
            reason.starts_with(
                "funding_recovery_requires_operator; conflicting_transaction_or_input_evidence",
            )
        }) {
            return Ok(OperatorRequired(
                "conflicting_transaction_or_input_evidence",
            ));
        }
        if job.phase == FundingPhase::RefundPending {
            return Ok(OperatorRequired("refund_requires_review"));
        }
        let id = job.id.clone();
        let count = self
            .store
            .call(move |s| s.funding_recovery_count(&id))
            .await?;
        let exhausted = count >= u64::from(max_attempts);
        let operation = status
            .treasury_operations
            .iter()
            .find(|op| op.operation_id == job.operation_id);
        let Some(operation) = operation else {
            if !safely_unprepared(job) {
                return Ok(OperatorRequired("preparation_outcome_unknown"));
            }
            if exhausted {
                return Ok(OperatorRequired("recovery_attempt_limit_reached"));
            }
            let id = job.id.clone();
            self.store
                .call(move |s| s.recover_unprepared_funding(&id))
                .await?;
            return Ok(Recovered);
        };
        if operation.submission == "CONFIRMED" && job.phase != FundingPhase::RecoveryRequired {
            return Ok(Waiting("source_confirmed_waiting_for_swap"));
        }
        if matches!(operation.submission.as_str(), "CONFIRMED" | "EXPIRED") {
            return Ok(OperatorRequired("swap_outcome_requires_review"));
        }
        let terminal = job.phase == FundingPhase::RecoveryRequired;
        let operation_id = operation.operation_id.clone();
        let expiry = u64::from(operation.facts.expiry_height);
        // Submitted/ambiguous work is observed, never resubmitted. Even expired
        // operations must pass the independent two-sync input proof below.
        if operation.attempts > 0 {
            if let Err(error) = self
                .reconcile_prepared(operation_id.clone(), sender, stop)
                .await
            {
                if error
                    .chain()
                    .any(|cause| cause.to_string() == "treasury_confirmation_pending")
                {
                    return Ok(Waiting("zcash_confirmations_pending"));
                }
                return Err(error);
            }
            let status = self.status().await?;
            if status
                .treasury_operations
                .iter()
                .any(|op| op.operation_id == operation_id && op.submission == "CONFIRMED")
            {
                return Ok(if terminal {
                    OperatorRequired("source_confirmed_check_swap_outcome")
                } else {
                    Waiting("source_confirmed_waiting_for_swap")
                });
            }
        } else {
            self.sync_once(stop).await?;
        }
        let status = self.status().await?;
        let sync = status.sync.context("recovery sync unavailable")?;
        if expiry == 0 {
            return Ok(OperatorRequired("transaction_expiry_missing"));
        }
        if sync
            .height
            .is_none_or(|height| height < expiry.saturating_add(u64::from(sync.confirmations)))
        {
            return Ok(Waiting("waiting_for_transaction_expiry"));
        }
        if exhausted {
            return Ok(OperatorRequired("recovery_attempt_limit_reached"));
        }
        self.recover_expired(operation_id, sender, stop).await?;
        Ok(Recovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoNetwork;
    impl TransactionSubmission for NoNetwork {
        async fn submit(
            &mut self,
            _: crate::rotation::transaction::BroadcastTransaction,
        ) -> Result<crate::rotation::transaction::SubmissionOutcome> {
            panic!("recovery must never submit")
        }
        async fn lookup(
            &mut self,
            _: &crate::rotation::transaction::PreparedTransaction,
        ) -> Result<crate::rotation::transaction::TransactionPresence> {
            panic!("unprepared recovery must not look up a transaction")
        }
    }

    #[test]
    fn conflicting_evidence_never_becomes_a_retryable_reset() {
        assert_eq!(
            evidence_outcome(&anyhow::anyhow!("expiry recovery input mismatch")),
            Some(RecoveryOutcome::OperatorRequired(
                "conflicting_transaction_or_input_evidence"
            ))
        );
        assert_eq!(
            evidence_outcome(&anyhow::anyhow!("transaction expiry not buried")),
            Some(RecoveryOutcome::Waiting("waiting_for_transaction_expiry"))
        );
        assert_eq!(
            evidence_outcome(&anyhow::anyhow!("private upstream error")),
            None
        );
    }
    #[tokio::test]
    async fn recovery_requires_preparation_evidence_and_bound_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut owner = Treasury::create(state.clone(), key.clone(), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        owner.ensure_pool("test".into(), "2".into()).await.unwrap();
        let job = owner.status().await.unwrap().funding_jobs.remove(0);
        let id = job.id.clone();
        owner
            .store
            .call(move |s| {
                s.advance_funding(&id, FundingPhase::Allocated, FundingPhase::RecoveryRequired)
            })
            .await
            .unwrap();
        let stop = CancellationToken::new();
        assert_eq!(
            owner
                .recover_funding_job(job.id.clone(), 1, &mut NoNetwork, &stop)
                .await
                .unwrap(),
            RecoveryOutcome::OperatorRequired("preparation_outcome_unknown")
        );
        let id = job.id.clone();
        owner
            .store
            .call(move |s| {
                s.defer_funding(
                    &id,
                    0,
                    Some("quote_refresh_exhausted; no preparation started"),
                    false,
                )
            })
            .await
            .unwrap();
        assert_eq!(
            owner
                .recover_funding_job(job.id.clone(), 1, &mut NoNetwork, &stop)
                .await
                .unwrap(),
            RecoveryOutcome::Recovered
        );
        let id = job.id.clone();
        let count = owner
            .store
            .call(move |s| s.funding_recovery_count(&id))
            .await
            .unwrap();
        assert_eq!(count, 1);
        let id = job.id.clone();
        owner
            .store
            .call(move |s| {
                s.advance_funding(&id, FundingPhase::Allocated, FundingPhase::RecoveryRequired)?;
                s.defer_funding(
                    &id,
                    0,
                    Some("quote_refresh_exhausted; no preparation started"),
                    false,
                )
            })
            .await
            .unwrap();
        let treasury_id = owner.status().await.unwrap().treasury_id;
        owner.close().await.unwrap();
        let mut owner = Treasury::open(state, key, treasury_id).await.unwrap();
        assert_eq!(
            owner
                .recover_funding_job(job.id, 1, &mut NoNetwork, &stop)
                .await
                .unwrap(),
            RecoveryOutcome::OperatorRequired("recovery_attempt_limit_reached")
        );
        assert!(owner.status().await.unwrap().treasury_operations.is_empty());
        owner.close().await.unwrap();
    }
}
