//! Indexed funding inventory and reporting; runtime due times own scheduling.
use super::funding::{FundingFacts, FundingJob};
use super::*;

pub(crate) const PAGE_SIZE: usize = 64;

/// One scheduling snapshot, captured entirely by the serialized store owner.
/// It is not authority for later work: operation-bound commits still revalidate.
pub(crate) struct FundingWork {
    pub job: FundingJob,
    pub facts: FundingFacts,
}

#[cfg(test)]
#[path = "scheduling_tests.rs"]
mod inventory_tests;

impl Store {
    /// Rows are never deleted. Capture committed insertion order independently
    /// from the indexed ID cursor so later UUIDs cannot join this pass.
    pub(crate) fn funding_work_boundary(&self) -> Result<i64> {
        Ok(self
            .db
            .query_row("SELECT COALESCE(MAX(rowid),0) FROM funding_jobs", [], |r| {
                r.get(0)
            })?)
    }

    pub(crate) fn funding_work_page(&self, after: &str, through: i64) -> Result<Vec<FundingWork>> {
        let ids = self.db.prepare("SELECT id FROM funding_jobs INDEXED BY funding_actionable_inventory WHERE (state!='COMPLETE' OR readiness_disputed IS NOT NULL) AND id>?1 AND rowid<=?2 ORDER BY id LIMIT ?3")?
            .query_map(params![after,through,PAGE_SIZE as i64], |r| r.get::<_,String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut jobs = Vec::with_capacity(ids.len());
        for id in ids {
            // Current allocations create their exact operation journal atomically.
            // Missing current-schema provenance is an error, never a new attempt.
            let job = self
                .funding_job(&id)?
                .context("funding inventory changed")?;
            // Recovery cannot replace the operation between these reads. Keep
            // phase and facts together rather than looking up facts after delivery.
            let facts = self.funding_facts(&id, &job.operation_id)?;
            jobs.push(FundingWork { job, facts });
        }
        Ok(jobs)
    }

    /// This is a source-owner decision about retained signed bytes, not an
    /// observation-health callback. Commit its phase and diagnostic together.
    pub(crate) fn mark_prepared_quote_window_exhausted(
        &mut self,
        job: &str,
        operation: &str,
        instant: u64,
        diagnostic: &str,
    ) -> Result<()> {
        ensure!(
            diagnostic.starts_with("prepared_quote_window_exhausted;"),
            "invalid source disposition"
        );
        let facts = self.funding_facts(job, operation)?;
        ensure!(
            facts.source_attempts == 0,
            "source submission already attempted"
        );
        ensure!(
            self.operation(operation)?
                .facts
                .deadline
                .saturating_sub(instant)
                < super::super::transaction::MIN_QUOTE_VALIDITY_SECONDS,
            "prepared quote window remains usable"
        );
        let changed=self.db.execute("UPDATE funding_progress SET phase='\"RECOVERY_REQUIRED\"',last_error=CASE WHEN last_error LIKE 'funding_integrity_conflict;%' THEN last_error ELSE ?3 END WHERE job_id=?1 AND operation_id=?2 AND phase='\"PREPARED\"'",params![job,operation,diagnostic])?;
        ensure!(changed == 1, "funding phase conflict");
        Ok(())
    }
    /// An actual serialized recovery owner returned an operator disposition.
    /// Successful unrelated reads cannot erase that disposition.
    pub(crate) fn record_source_operator_requirement(
        &mut self,
        job: &str,
        operation: &str,
        diagnostic: &str,
    ) -> Result<()> {
        ensure!(
            diagnostic.starts_with("funding_recovery_requires_operator;"),
            "invalid source disposition"
        );
        let tx = self.db.transaction()?;
        let previous:Option<Option<String>>=tx.query_row("SELECT last_error FROM funding_progress WHERE job_id=?1 AND operation_id=?2 AND phase!='\"COMPLETE\"'",params![job,operation],|row|row.get(0)).optional()?;
        let Some(previous) = previous else {
            return Ok(());
        };
        let conflict = diagnostic
            == "funding_recovery_requires_operator; conflicting_transaction_or_input_evidence";
        let retained = previous.as_deref().is_some_and(|value| {
            [
                "funding_attempt_closed;",
                "funding_quote_failed;",
                "quote_refresh_exhausted;",
                "prepared_quote_window_exhausted;",
            ]
            .iter()
            .any(|prefix| value.starts_with(prefix))
        });
        let diagnostic = if previous
            .as_deref()
            .is_some_and(|value| value.starts_with("funding_integrity_conflict;"))
        {
            previous.unwrap()
        } else if retained && !conflict {
            let base = previous
                .as_deref()
                .unwrap()
                .split("; source_operator: ")
                .next()
                .unwrap();
            let reason = diagnostic
                .strip_prefix("funding_recovery_requires_operator; ")
                .context("invalid source disposition")?;
            format!("{base}; source_operator: {reason}")
        } else {
            diagnostic.to_owned()
        };
        tx.execute(
            "UPDATE funding_progress SET last_error=?3 WHERE job_id=?1 AND operation_id=?2",
            params![job, operation, diagnostic],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Report the owner's combined outcomes without granting phase/financial
    /// authority. Exact operation binding discards late archived callbacks.
    pub(crate) fn record_funding_observation_health(
        &mut self,
        job: &str,
        operation: &str,
        next: u64,
        streak: u32,
        error: Option<&str>,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        let changed=tx.execute("UPDATE funding_progress SET next_poll=CASE WHEN phase='\"COMPLETE\"' THEN 0 ELSE ?3 END,last_error=CASE WHEN last_error LIKE 'funding_integrity_conflict;%' THEN last_error WHEN ?4 LIKE 'funding_integrity_conflict;%' THEN ?4 WHEN phase='\"COMPLETE\"' THEN NULL WHEN last_error LIKE 'funding_recovery_requires_operator;%' THEN last_error WHEN phase IN ('\"RECOVERY_REQUIRED\"','\"PREPARING\"') THEN last_error ELSE ?4 END WHERE job_id=?1 AND operation_id=?2",params![job,operation,i64::try_from(next)?,error])?;
        if changed == 1 {
            tx.execute("INSERT INTO funding_health(job_id,error_streak) SELECT ?1,CASE WHEN phase='\"COMPLETE\"' THEN 0 ELSE ?2 END FROM funding_progress WHERE job_id=?1 ON CONFLICT(job_id) DO UPDATE SET error_streak=excluded.error_streak",params![job,streak])?;
        }
        tx.execute("UPDATE funding_health SET timed_out=0 WHERE job_id=?1 AND EXISTS(SELECT 1 FROM funding_jobs WHERE id=?1 AND state='COMPLETE')",[job])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_observation_callbacks_preserve_recovery_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "pool",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        store
            .advance_funding(
                &job.id,
                funding::FundingPhase::Allocated,
                funding::FundingPhase::RecoveryRequired,
            )
            .unwrap();
        store
            .defer_funding(
                &job.id,
                0,
                Some("quote_refresh_exhausted; no preparation started"),
                false,
            )
            .unwrap();
        for error in [
            Some("base_credit_unverified; retry original observation"),
            None,
            Some("funding_status_unavailable; retry original deposit"),
        ] {
            store
                .record_funding_observation_health(&job.id, &job.operation_id, 100, 1, error)
                .unwrap();
            let saved = store.funding_job(&job.id).unwrap().unwrap();
            assert_eq!(
                saved.last_error.as_deref(),
                Some("quote_refresh_exhausted; no preparation started")
            );
            assert!(crate::rotation::store::funding::can_recover_facts(
                &saved, None
            ));
        }
        store
            .db
            .execute(
                "UPDATE funding_progress SET last_error=NULL WHERE job_id=?1",
                [&job.id],
            )
            .unwrap();
        store
            .record_funding_observation_health(
                &job.id,
                &job.operation_id,
                100,
                1,
                Some("funding_quote_failed; no preparation started"),
            )
            .unwrap();
        let unknown = store.funding_job(&job.id).unwrap().unwrap();
        assert!(unknown.last_error.is_none());
        assert!(!crate::rotation::store::funding::can_recover_facts(
            &unknown, None
        ));
        store
            .record_source_operator_requirement(
                &job.id,
                &job.operation_id,
                "funding_recovery_requires_operator; unresolved preparation",
            )
            .unwrap();
        for error in [
            None,
            Some("base_credit_unverified; retry original observation"),
        ] {
            store
                .record_funding_observation_health(&job.id, &job.operation_id, 200, 1, error)
                .unwrap();
            let blocked = store.funding_job(&job.id).unwrap().unwrap();
            assert_eq!(
                blocked.last_error.as_deref(),
                Some("funding_recovery_requires_operator; unresolved preparation")
            );
            assert!(!crate::rotation::store::funding::can_recover_facts(
                &blocked, None
            ));
        }

        assert_eq!(
            store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .source_attempts,
            0
        );
    }
}

#[cfg(test)]
mod source_disposition_tests {
    use super::*;
    #[test]
    fn policy_disposition_preserves_closed_attempt_proof_and_reauthorization() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "pool",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        store
            .close_unprepared_funding_for_operation(&job.id, &job.operation_id)
            .unwrap();
        store.disable_pool("pool").unwrap();
        for reason in [
            "funding_disabled",
            "recovery_attempt_limit_reached",
            "funding_disabled",
        ] {
            store
                .record_source_operator_requirement(
                    &job.id,
                    &job.operation_id,
                    &format!("funding_recovery_requires_operator; {reason}"),
                )
                .unwrap();
            store
                .record_funding_observation_health(&job.id, &job.operation_id, 100, 0, None)
                .unwrap();
            let current = store.funding_job(&job.id).unwrap().unwrap();
            assert_eq!(current.operation_id, job.operation_id);
            let diagnostic = current.last_error.as_deref().unwrap();
            assert!(diagnostic.starts_with("funding_attempt_closed;"));
            assert_eq!(diagnostic.matches("; source_operator: ").count(), 1);
            assert!(crate::rotation::store::funding::can_recover_facts(
                &current, None
            ));
        }
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "pool",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        store
            .reauthorize_closed_funding_for_operation(&job.id, &job.operation_id, 3)
            .unwrap();
        let current = store.funding_job(&job.id).unwrap().unwrap();
        assert_ne!(current.operation_id, job.operation_id);
        assert_eq!(current.phase, funding::FundingPhase::Allocated);
        assert!(current.last_error.is_none());
        let archived: String = store
            .db
            .query_row(
                "SELECT operation_id FROM funding_recovery WHERE job_id=?1",
                [&job.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(archived, job.operation_id);
        let source_effects: i64 = store
            .db
            .query_row(
                "SELECT (SELECT COUNT(*) FROM outgoing)+(SELECT COUNT(*) FROM budget_entries)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_effects, 0);
    }
}
