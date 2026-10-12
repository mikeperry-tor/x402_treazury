//! Funding recovery never submits transactions or relaxes canonical expiry evidence.
use super::*;
use crate::rotation::{
    store::funding::{FundingJob, FundingPhase, safely_unprepared},
    transaction::TransactionSubmission,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum RecoveryOutcome {
    Recovered,
    Waiting(&'static str),
    OperatorRequired(&'static str),
}

/// Recovery disposition plus whether this invocation attempted the ordinary
/// source observation. Early operator dispositions do not abandon that work.
#[derive(Debug, Clone)]
pub struct RecoveryObservation {
    pub outcome: RecoveryOutcome,
    pub source_observed: bool,
}

pub(crate) fn needs_recovery(job: &FundingJob) -> bool {
    job.phase == FundingPhase::RecoveryRequired
        || job.phase == FundingPhase::RefundPending
        || (job.timed_out
            && matches!(
                job.phase,
                FundingPhase::Preparing | FundingPhase::Prepared | FundingPhase::DepositPending
            ))
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
        allow_new_attempt: bool,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<RecoveryOutcome> {
        Ok(self
            .recover_funding_observed(job_id, max_attempts, allow_new_attempt, sender, stop)
            .await?
            .outcome)
    }

    pub(crate) async fn recover_funding_observed(
        &mut self,
        job_id: String,
        max_attempts: u32,
        allow_new_attempt: bool,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<RecoveryObservation> {
        let mut source_observed = false;
        let result = self
            .recover_funding_job_inner(
                job_id,
                max_attempts,
                allow_new_attempt,
                sender,
                stop,
                &mut source_observed,
            )
            .await;
        let outcome = match result {
            Err(error) => match evidence_outcome(&error) {
                Some(outcome) => outcome,
                None => return Err(error),
            },
            Ok(outcome) => outcome,
        };
        Ok(RecoveryObservation {
            outcome,
            source_observed,
        })
    }
    async fn recover_funding_job_inner(
        &mut self,
        job_id: String,
        max_attempts: u32,
        allow_new_attempt: bool,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
        source_observed: &mut bool,
    ) -> Result<RecoveryOutcome> {
        use RecoveryOutcome::*;
        ensure!(self.healthy, "treasury requires reopen");
        if crate::qualification::active() {
            return Ok(OperatorRequired("qualification_recovery_requires_review"));
        }
        let (job, operation) = self
            .store
            .call(move |s| s.funding_recovery_context(&job_id))
            .await?;
        let completed_job = job.id.clone();
        let completed_operation = job.operation_id.clone();
        if self
            .store
            .call(move |s| s.finish_funding_if_proven(&completed_job, &completed_operation))
            .await?
        {
            return Ok(Recovered);
        }
        if !needs_recovery(&job) {
            if job.last_error.as_deref().is_some_and(|reason| {
                [
                    "treasury_insufficient_spendable_funds",
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
        let Some(operation) = operation.as_ref() else {
            if !safely_unprepared(&job) {
                return Ok(OperatorRequired("preparation_outcome_unknown"));
            }
            let id = job.id.clone();
            let expected_operation = job.operation_id.clone();
            let (closed_count, completed) = self
                .store
                .call(move |s| {
                    s.close_unprepared_funding_for_operation(&id, &expected_operation)?;
                    Ok((
                        s.funding_recovery_count(&id)?,
                        s.finish_funding_if_proven(&id, &expected_operation)?,
                    ))
                })
                .await?;
            if completed {
                return Ok(Recovered);
            }
            if !allow_new_attempt {
                return Ok(OperatorRequired("funding_disabled"));
            }
            if closed_count > u64::from(max_attempts) {
                return Ok(OperatorRequired("recovery_attempt_limit_reached"));
            }
            let id = job.id.clone();
            let expected_operation = job.operation_id.clone();
            self.store
                .call(move |s| {
                    s.reauthorize_closed_funding_for_operation(
                        &id,
                        &expected_operation,
                        max_attempts,
                    )
                })
                .await?;
            return Ok(Recovered);
        };
        if operation.submission == "CONFIRMED" && job.phase != FundingPhase::RecoveryRequired {
            return Ok(Waiting("source_confirmed_waiting_for_swap"));
        }
        if operation.submission == "EXPIRED" {
            if !allow_new_attempt {
                return Ok(OperatorRequired("funding_disabled"));
            }
            if count > u64::from(max_attempts) {
                return Ok(OperatorRequired("recovery_attempt_limit_reached"));
            }
            let id = job.id.clone();
            let expected_operation = job.operation_id.clone();
            self.store
                .call(move |s| {
                    s.reauthorize_closed_funding_for_operation(
                        &id,
                        &expected_operation,
                        max_attempts,
                    )
                })
                .await?;
            return Ok(Recovered);
        }
        if operation.submission == "CONFIRMED" {
            return Ok(OperatorRequired("swap_outcome_requires_review"));
        }
        let terminal = job.phase == FundingPhase::RecoveryRequired;
        let operation_id = operation.operation_id.clone();
        let expiry = u64::from(operation.facts.expiry_height);
        // Submitted/ambiguous work is observed, never resubmitted. Even expired
        // operations must pass the independent two-sync input proof below.
        if operation.attempts > 0 {
            *source_observed = true;
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
            let id = operation_id.clone();
            if self
                .store
                .call(move |s| Ok(s.operation(&id)?.submission == "CONFIRMED"))
                .await?
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
        let sync = self.store.call(|s| s.funding_sync_observation()).await?;
        if expiry == 0 {
            return Ok(OperatorRequired("transaction_expiry_missing"));
        }
        if sync
            .height
            .is_none_or(|height| height < expiry.saturating_add(u64::from(sync.confirmations)))
        {
            return Ok(Waiting("waiting_for_transaction_expiry"));
        }
        self.recover_expired(operation_id, sender, stop).await?;
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        if self
            .store
            .call(move |s| s.finish_funding_if_proven(&id, &operation))
            .await?
        {
            return Ok(Recovered);
        }
        if !allow_new_attempt {
            return Ok(OperatorRequired("funding_disabled"));
        }
        if exhausted {
            return Ok(OperatorRequired("recovery_attempt_limit_reached"));
        }
        let id = job.id.clone();
        let expected_operation = job.operation_id.clone();
        self.store
            .call(move |s| {
                s.reauthorize_closed_funding_for_operation(&id, &expected_operation, max_attempts)
            })
            .await?;
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
        crate::rotation::test_capacity::treasury_pair(
            &owner,
            "test",
            "2",
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0,
        )
        .await
        .unwrap();
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
        let observation = owner
            .recover_funding_observed(job.id.clone(), 1, true, &mut NoNetwork, &stop)
            .await
            .unwrap();
        assert!(
            !observation.source_observed,
            "early disposition must not claim a source observation"
        );
        assert_eq!(
            observation.outcome,
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
                .recover_funding_job(job.id.clone(), 1, false, &mut NoNetwork, &stop)
                .await
                .unwrap(),
            RecoveryOutcome::OperatorRequired("funding_disabled")
        );
        let closed = owner
            .status()
            .await
            .unwrap()
            .funding_jobs
            .into_iter()
            .find(|j| j.id == job.id)
            .unwrap();
        assert_eq!(closed.operation_id, job.operation_id);
        assert_eq!(
            owner
                .recover_funding_job(job.id.clone(), 1, true, &mut NoNetwork, &stop)
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
                .recover_funding_job(job.id.clone(), 1, true, &mut NoNetwork, &stop)
                .await
                .unwrap(),
            RecoveryOutcome::OperatorRequired("recovery_attempt_limit_reached")
        );
        let id = job.id.clone();
        assert_eq!(
            owner
                .store
                .call(move |s| s.funding_recovery_count(&id))
                .await
                .unwrap(),
            2
        );
        let closed = owner
            .status()
            .await
            .unwrap()
            .funding_jobs
            .into_iter()
            .find(|j| j.id == job.id)
            .unwrap();
        assert_eq!(closed.phase, FundingPhase::RecoveryRequired);
        assert_eq!(
            owner
                .recover_funding_job(job.id.clone(), 1, true, &mut NoNetwork, &stop)
                .await
                .unwrap(),
            RecoveryOutcome::OperatorRequired("recovery_attempt_limit_reached")
        );
        let id = job.id.clone();
        assert_eq!(
            owner
                .store
                .call(move |s| s.funding_recovery_count(&id))
                .await
                .unwrap(),
            2
        );
        assert!(owner.status().await.unwrap().treasury_operations.is_empty());
        owner.close().await.unwrap();
    }
}
