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
    use crate::rotation::transaction::RecoveryEvidenceError as Evidence;
    Some(match error.downcast_ref::<Evidence>()? {
        Evidence::ConfirmationPending => RecoveryOutcome::Waiting("zcash_confirmations_pending"),
        Evidence::ExpiryNotBuried | Evidence::ExpiryNotInvalidated => {
            RecoveryOutcome::Waiting("waiting_for_transaction_expiry")
        }
        Evidence::TipLag | Evidence::StaleSync | Evidence::ChainChanged => {
            RecoveryOutcome::Waiting("waiting_for_fresh_canonical_sync")
        }
        Evidence::InputMissing
        | Evidence::InputMismatch
        | Evidence::InputSpent
        | Evidence::IndexerInclusion
        | Evidence::TransactionMismatch
        | Evidence::ConfirmationMismatch => {
            RecoveryOutcome::OperatorRequired("conflicting_transaction_or_input_evidence")
        }
    })
}

impl Treasury {
    pub(crate) async fn recovery_needed(&self, job: &FundingJob) -> Result<bool> {
        let (id, operation) = (job.id.clone(), job.operation_id.clone());
        let facts = self
            .store
            .call(move |s| s.funding_facts(&id, &operation))
            .await?;
        Ok(facts.preparation_accepted || facts.source_pending || needs_recovery(job))
    }

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
        let id = job.id.clone();
        let count = self
            .store
            .call(move |s| s.funding_recovery_count(&id))
            .await?;
        let exhausted = count >= u64::from(max_attempts);
        let Some(operation) = operation.as_ref() else {
            return self
                .recover_unprepared_job(&job, max_attempts, allow_new_attempt)
                .await;
        };
        if operation.submission == "CONFIRMED" {
            return Ok(Waiting("source_confirmed_waiting_for_swap"));
        }
        if operation.submission == "EXPIRED" {
            return self
                .reauthorize_recovery(
                    &job,
                    max_attempts,
                    allow_new_attempt,
                    count > u64::from(max_attempts),
                )
                .await;
        }
        let terminal = job.phase == FundingPhase::RecoveryRequired;
        let operation_id = operation.operation_id.clone();
        let expiry = u64::from(operation.facts.expiry_height);
        if let Some(outcome) = self
            .observe_recovery_source(operation, terminal, sender, stop, source_observed)
            .await?
        {
            return Ok(outcome);
        }
        let sync = self.store.call(|s| s.funding_sync_observation()).await?;
        if expiry == 0 {
            return Ok(OperatorRequired("transaction_has_no_consensus_expiry"));
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
        self.reauthorize_recovery(&job, max_attempts, allow_new_attempt, exhausted)
            .await
    }

    async fn recover_unprepared_job(
        &mut self,
        job: &FundingJob,
        max_attempts: u32,
        allow_new_attempt: bool,
    ) -> Result<RecoveryOutcome> {
        use RecoveryOutcome::*;
        let (id, operation) = (job.id.clone(), job.operation_id.clone());
        let facts = self
            .store
            .call(move |s| s.funding_facts(&id, &operation))
            .await?;
        if !safely_unprepared(job, &facts) {
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
        self.reauthorize_recovery(
            job,
            max_attempts,
            allow_new_attempt,
            closed_count > u64::from(max_attempts),
        )
        .await
    }

    /// Closure already consumed the durable attempt; each caller supplies its
    /// existing before/after-closure comparison rather than recomputing it here.
    async fn reauthorize_recovery(
        &mut self,
        job: &FundingJob,
        max_attempts: u32,
        allow_new_attempt: bool,
        attempt_limit_reached: bool,
    ) -> Result<RecoveryOutcome> {
        use RecoveryOutcome::*;
        if !allow_new_attempt {
            return Ok(OperatorRequired("funding_disabled"));
        }
        if attempt_limit_reached {
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

    async fn observe_recovery_source(
        &mut self,
        operation: &crate::rotation::transaction::OperationStatus,
        terminal: bool,
        sender: &mut impl TransactionSubmission,
        stop: &CancellationToken,
        source_observed: &mut bool,
    ) -> Result<Option<RecoveryOutcome>> {
        use RecoveryOutcome::*;
        let operation_id = operation.operation_id.clone();
        // Submitted/ambiguous work is observed, never resubmitted. Even expired
        // operations still require the caller's independent two-sync input proof.
        if operation.attempts > 0 {
            *source_observed = true;
            if let Err(error) = self
                .reconcile_prepared(operation_id.clone(), sender, stop)
                .await
            {
                if error.downcast_ref::<crate::rotation::transaction::RecoveryEvidenceError>()
                    == Some(
                        &crate::rotation::transaction::RecoveryEvidenceError::ConfirmationPending,
                    )
                {
                    return Ok(Some(Waiting("zcash_confirmations_pending")));
                }
                return Err(error);
            }
            let id = operation_id.clone();
            if self
                .store
                .call(move |s| Ok(s.operation(&id)?.submission == "CONFIRMED"))
                .await?
            {
                return Ok(Some(if terminal {
                    OperatorRequired("source_confirmed_check_swap_outcome")
                } else {
                    Waiting("source_confirmed_waiting_for_swap")
                }));
            }
        } else {
            self.sync_once(stop).await?;
        }
        Ok(None)
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
            evidence_outcome(
                &anyhow::Error::new(
                    crate::rotation::transaction::RecoveryEvidenceError::InputMismatch
                )
                .context("outer diagnostic")
            ),
            Some(RecoveryOutcome::OperatorRequired(
                "conflicting_transaction_or_input_evidence"
            ))
        );
        assert_eq!(
            evidence_outcome(&anyhow::Error::new(
                crate::rotation::transaction::RecoveryEvidenceError::ExpiryNotBuried
            )),
            Some(RecoveryOutcome::Waiting("waiting_for_transaction_expiry"))
        );
        assert_eq!(
            evidence_outcome(&anyhow::anyhow!("private upstream error")),
            None
        );
    }
    #[test]
    fn diagnostic_text_cannot_select_a_recovery_disposition() {
        use crate::rotation::transaction::RecoveryEvidenceError as Evidence;
        for evidence in [
            Evidence::ExpiryNotBuried,
            Evidence::ExpiryNotInvalidated,
            Evidence::TipLag,
            Evidence::StaleSync,
            Evidence::ChainChanged,
            Evidence::InputMissing,
            Evidence::InputMismatch,
            Evidence::InputSpent,
            Evidence::IndexerInclusion,
            Evidence::TransactionMismatch,
            Evidence::ConfirmationMismatch,
            Evidence::ConfirmationPending,
        ] {
            let prose = evidence.to_string();
            assert!(evidence_outcome(&anyhow::anyhow!(prose)).is_none());
            let typed = anyhow::Error::new(evidence).context("arbitrary diagnostic context");
            assert!(evidence_outcome(&typed).is_some());
        }
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
        let (operation, pool) = (job.operation_id.clone(), job.pool_id.clone());
        owner
            .store
            .call(move |s| s.accept_preparation(&operation, Some(&pool), 10, None))
            .await
            .unwrap();
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
            RecoveryOutcome::OperatorRequired("preparation_outcome_unknown")
        );
        let job = owner
            .status()
            .await
            .unwrap()
            .funding_jobs
            .into_iter()
            .find(|other| other.id != job.id)
            .unwrap();
        let id = job.id.clone();
        owner
            .store
            .call(move |s| s.close_unprepared_funding(&id))
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
            .call(move |s| s.close_unprepared_funding(&id))
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

    async fn unfunded_recovery_fixture() -> (tempfile::TempDir, Treasury, Vec<FundingJob>) {
        let dir = tempfile::tempdir().unwrap();
        let owner = Treasury::create(dir.path().join("state"), dir.path().join("key"), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        crate::rotation::test_capacity::treasury_pair(
            &owner,
            "disposition",
            "2",
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0,
        )
        .await
        .unwrap();
        let jobs = owner.status().await.unwrap().funding_jobs;
        (dir, owner, jobs)
    }

    fn recovery_rows(
        dir: &std::path::Path,
        financial_only: bool,
    ) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        let db = rusqlite::Connection::open_with_flags(
            dir.join("state/state.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut queries = vec![
            "SELECT * FROM operation_network ORDER BY operation_id",
            "SELECT * FROM outgoing ORDER BY id",
            "SELECT * FROM treasury_operations ORDER BY id",
            "SELECT * FROM payment_attempts ORDER BY id",
            "SELECT * FROM reserve_claims ORDER BY pool_id,slot",
            "SELECT id,role,balance,unresolved_exposure FROM wallets ORDER BY id",
            "SELECT * FROM funding_jobs ORDER BY id",
        ];
        if !financial_only {
            queries.extend([
                "SELECT * FROM funding_progress ORDER BY job_id",
                "SELECT * FROM funding_recovery ORDER BY operation_id",
                "SELECT * FROM funding_quote_refreshes ORDER BY operation_id",
                "SELECT * FROM funding_health ORDER BY job_id",
                "SELECT * FROM funding_refunds ORDER BY job_id",
            ]);
        }
        queries
            .into_iter()
            .map(|sql| {
                let mut statement = db.prepare(sql).unwrap();
                let columns = statement.column_count();
                statement
                    .query_map([], |row| {
                        (0..columns).map(|column| row.get(column)).collect()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap()
            })
            .collect()
    }

    #[tokio::test]
    async fn accepted_calculation_ownership_overrules_phase_and_diagnostic_text() {
        let (dir, mut owner, jobs) = unfunded_recovery_fixture().await;
        let job = &jobs[0];
        let (id, pool) = (job.operation_id.clone(), job.pool_id.clone());
        owner
            .store
            .call(move |s| s.accept_preparation(&id, Some(&pool), 10, None))
            .await
            .unwrap();
        let stop = CancellationToken::new();
        for reason in [
            None,
            Some("quote_refresh_exhausted; no preparation started"),
            Some("arbitrary annotation"),
        ] {
            let id = job.id.clone();
            owner
                .store
                .call(move |s| s.defer_funding(&id, 0, reason, false))
                .await
                .unwrap();
            let before = recovery_rows(dir.path(), false);
            let result = owner
                .recover_funding_observed(job.id.clone(), 0, true, &mut NoNetwork, &stop)
                .await
                .unwrap();
            assert_eq!(
                result.outcome,
                RecoveryOutcome::OperatorRequired("preparation_outcome_unknown")
            );
            assert!(!result.source_observed);
            assert_eq!(recovery_rows(dir.path(), false), before);
        }
        owner.close().await.unwrap();
    }

    #[tokio::test]
    async fn zero_recovery_allowance_closes_once_and_disabled_funding_precedes_attempt_limit() {
        let (dir, mut owner, jobs) = unfunded_recovery_fixture().await;
        let job = &jobs[0];
        let id = job.id.clone();
        owner
            .store
            .call(move |store| store.close_unprepared_funding(&id))
            .await
            .unwrap();
        let financial_before = recovery_rows(dir.path(), true);
        let stop = CancellationToken::new();
        for (allowed, reason) in [
            (false, "funding_disabled"),
            (true, "recovery_attempt_limit_reached"),
            (false, "funding_disabled"),
            (true, "recovery_attempt_limit_reached"),
        ] {
            let observation = owner
                .recover_funding_observed(job.id.clone(), 0, allowed, &mut NoNetwork, &stop)
                .await
                .unwrap();
            assert_eq!(
                observation.outcome,
                RecoveryOutcome::OperatorRequired(reason)
            );
            assert!(!observation.source_observed);
            let id = job.id.clone();
            let (current, count) = owner
                .store
                .call(move |store| {
                    Ok((
                        store.funding_job(&id)?.unwrap(),
                        store.funding_recovery_count(&id)?,
                    ))
                })
                .await
                .unwrap();
            assert_eq!(current.operation_id, job.operation_id);
            assert_eq!(current.phase, FundingPhase::RecoveryRequired);
            assert_eq!(count, 1, "closure must not archive the same attempt twice");
            assert_eq!(recovery_rows(dir.path(), true), financial_before);
        }
        let observation = owner
            .recover_funding_observed(job.id.clone(), 1, true, &mut NoNetwork, &stop)
            .await
            .unwrap();
        assert_eq!(observation.outcome, RecoveryOutcome::Recovered);
        assert!(!observation.source_observed);
        let id = job.id.clone();
        let (current, count) = owner
            .store
            .call(move |store| {
                Ok((
                    store.funding_job(&id)?.unwrap(),
                    store.funding_recovery_count(&id)?,
                ))
            })
            .await
            .unwrap();
        assert_ne!(current.operation_id, job.operation_id);
        assert_eq!(current.phase, FundingPhase::Allocated);
        assert_eq!(count, 1);
        assert_eq!(recovery_rows(dir.path(), true), financial_before);
        owner.close().await.unwrap();
    }
}
