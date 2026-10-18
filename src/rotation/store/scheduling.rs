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

/// Created only by specific payload decoders, never by SQL execution/stepping.
#[derive(Debug)]
pub(super) struct FundingDecodeFailure;
impl std::fmt::Display for FundingDecodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("funding_payload_decode_failed")
    }
}
impl std::error::Error for FundingDecodeFailure {}
#[derive(Debug, Clone)]
pub(crate) struct MalformedFunding {
    pub id: String,
    pub operation: String,
}
impl std::fmt::Display for MalformedFunding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("malformed_funding_inventory")
    }
}
impl std::error::Error for MalformedFunding {}
pub(crate) type FundingEntry = (String, std::result::Result<FundingWork, MalformedFunding>);

#[cfg(test)]
#[path = "scheduling_tests.rs"]
mod inventory_tests;

impl Store {
    /// Rows are never deleted. Capture committed insertion order independently
    /// from the indexed ID cursor so later UUIDs cannot join this pass.
    pub(crate) fn funding_work_boundary(&self) -> Result<i64> {
        #[cfg(test)]
        self.inventory_read_fault(super::inventory::Read::FundingBoundary)?;
        Ok(self
            .db
            .query_row("SELECT COALESCE(MAX(rowid),0) FROM funding_jobs", [], |r| {
                r.get(0)
            })?)
    }

    pub(crate) fn funding_work_page(&self, after: &str, through: i64) -> Result<Vec<FundingEntry>> {
        #[cfg(test)]
        self.inventory_read_fault(super::inventory::Read::FundingPage)?;
        let ids = self.db.prepare("SELECT id FROM funding_jobs INDEXED BY funding_unfinished_jobs WHERE state!='COMPLETE' AND id>?1 AND rowid<=?2 UNION SELECT f.id FROM outgoing x JOIN funding_progress j ON j.operation_id=x.id JOIN funding_jobs f ON f.id=j.job_id WHERE x.state='PREPARED' AND f.id>?1 AND f.rowid<=?2 ORDER BY id LIMIT ?3")?
            .query_map(params![after,through,PAGE_SIZE as i64], |r| r.get::<_,String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut jobs = Vec::with_capacity(ids.len());
        for id in ids {
            let result = (|| {
                let job = self
                    .funding_job(&id)?
                    .context("funding inventory changed")?;
                let facts = self.funding_facts(&id, &job.operation_id)?;
                Ok(FundingWork { job, facts })
            })();
            let payload = match result {
                Ok(work) => Ok(work),
                Err(error) => Err(self.scope_funding_decode_failure(&id, error)?),
            };
            jobs.push((id, payload));
        }
        Ok(jobs)
    }

    /// A completion belongs to its captured operation even if inventory has
    /// rolled over since publication failed. Never relabel its diagnostic.
    pub(crate) fn scope_funding_completion_failure(
        &self,
        id: &str,
        expected: &str,
        error: anyhow::Error,
    ) -> Result<Option<MalformedFunding>> {
        if error.downcast_ref::<FundingDecodeFailure>().is_none() {
            return Err(error);
        }
        let current: String = self.db.query_row(
            "SELECT operation_id FROM funding_progress WHERE job_id=?1",
            [id],
            |r| r.get(0),
        )?;
        if current != expected {
            return Ok(None);
        }
        self.scope_funding_decode_failure(id, error).map(Some)
    }

    /// Decode faults are local only after ownership and source correspondence
    /// are established. This read never repairs or releases financial records.
    pub(crate) fn scope_funding_decode_failure(
        &self,
        id: &str,
        error: anyhow::Error,
    ) -> Result<MalformedFunding> {
        if error.downcast_ref::<FundingDecodeFailure>().is_none() {
            return Err(error);
        }
        let (operation, pool, ownership): (String, String, bool) = self.db.query_row(
            "SELECT j.operation_id,f.pool_id,EXISTS(SELECT 1 FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE w.id=f.wallet_id AND w.pool_id=f.pool_id) FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1",
            [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        ensure!(
            !id.is_empty() && !operation.is_empty() && ownership,
            "unknown_funding_ownership"
        );
        let (outgoing, source, budget_pool): (bool, bool, Option<String>) = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1),EXISTS(SELECT 1 FROM treasury_operations WHERE id=?1),(SELECT pool_id FROM budget_entries WHERE id=?1)",
            [&operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        ensure!(outgoing == source, "unknown_funding_source_correspondence");
        let budget_exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM budget_entries WHERE id=?1)",
            [&operation],
            |r| r.get(0),
        )?;
        ensure!(
            !budget_exists || budget_pool.as_deref() == Some(&pool),
            "funding_source_pool_mismatch"
        );
        ensure!(
            !source || budget_exists,
            "unknown_funding_source_correspondence"
        );
        if source {
            self.operation(&operation)?;
        }
        Ok(MalformedFunding {
            id: id.to_owned(),
            operation,
        })
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
        let changed=self.db.execute("UPDATE funding_progress SET phase='\"RECOVERY_REQUIRED\"',last_error=?3 WHERE job_id=?1 AND operation_id=?2 AND phase='\"PREPARED\"'",params![job,operation,diagnostic])?;
        ensure!(changed == 1, "funding phase conflict");
        Ok(())
    }
    /// Publish the serialized recovery owner's diagnostic. Canonical obligations
    /// and current policy, never this text, determine subsequent disposition.
    pub(crate) fn record_source_operator_requirement(
        &mut self,
        job: &str,
        operation: &str,
        diagnostic: &str,
    ) -> Result<()> {
        self.db.execute(
            "UPDATE funding_progress SET last_error=?3 WHERE job_id=?1 AND operation_id=?2 AND phase!='\"COMPLETE\"'",
            params![job, operation, diagnostic],
        )?;
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
        let changed=tx.execute("UPDATE funding_progress SET next_poll=CASE WHEN phase='\"COMPLETE\"' THEN 0 ELSE ?3 END,last_error=CASE WHEN provider_incomplete=1 OR (provider_success=1 AND (provider_failed=1 OR provider_refunded=1)) THEN 'funding_integrity_conflict; contradictory provider outcome' WHEN phase='\"COMPLETE\"' THEN NULL ELSE ?4 END WHERE job_id=?1 AND operation_id=?2",params![job,operation,i64::try_from(next)?,error])?;
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
            crate::rotation::test_capacity::two_reserves("5"),
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
            assert_eq!(saved.last_error.as_deref(), error);
            assert!(!crate::rotation::store::funding::can_recover_facts(
                &saved,
                None,
                &store.funding_facts(&job.id, &job.operation_id).unwrap()
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
        assert_eq!(
            unknown.last_error.as_deref(),
            Some("funding_quote_failed; no preparation started")
        );
        assert!(!crate::rotation::store::funding::can_recover_facts(
            &unknown,
            None,
            &store.funding_facts(&job.id, &job.operation_id).unwrap()
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
            assert_eq!(blocked.last_error.as_deref(), error);
            assert!(!crate::rotation::store::funding::can_recover_facts(
                &blocked,
                None,
                &store.funding_facts(&job.id, &job.operation_id).unwrap()
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
            crate::rotation::test_capacity::two_reserves("5"),
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
            assert!(current.last_error.is_none());
            assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 1);
            assert!(crate::rotation::store::funding::can_recover_facts(
                &current,
                None,
                &store.funding_facts(&job.id, &job.operation_id).unwrap()
            ));
        }
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "pool",
            crate::rotation::test_capacity::two_reserves("5"),
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
