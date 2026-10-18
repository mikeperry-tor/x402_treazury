//! Finalization of operator-requested expiry recovery after wallet/chain proof.
use super::*;
impl Store {
    pub(crate) fn prepared_snapshot(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let (revision, bytes): (i64,Vec<u8>) = self.db.query_row("SELECT s.revision,s.bytes FROM outgoing o JOIN snapshots s ON s.revision=o.revision WHERE o.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        unseal(
            &self.key,
            &format!("v1:{}:{}:snapshot:{revision}", self.id, self.network.name()),
            &bytes,
        )
    }
    /// Evidence is checked by Treasury, never supplied by a CLI/MCP caller.
    pub(crate) fn resolve_expired(&mut self, id: &str, revision: i64, _instant: u64) -> Result<()> {
        let closed: Option<bool> = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1 AND state='RESOLVED') AND EXISTS(SELECT 1 FROM budget_entries WHERE id=?1 AND reserved=0 AND consumed=0) FROM expired_operations WHERE id=?1",
            [id], |row| row.get(0)).optional()?;
        if let Some(closed) = closed {
            ensure!(closed, "expired operation accounting conflict");
            return Ok(());
        }
        let observation: String = self.db.query_row(
            "SELECT observation FROM treasury_sync WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let sync: SyncObservation = serde_json::from_str(&observation)?;
        let current_revision: i64 = self.db.query_row(
            "SELECT revision FROM snapshots ORDER BY revision DESC LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        ensure!(
            current_revision == revision
                && (sync.phase == SyncPhase::Ready && sync.snapshot_revision == revision),
            "expiry recovery requires fresh unchanged snapshot"
        );
        ensure!(
            sync.at_observed_tip(),
            crate::rotation::transaction::RecoveryEvidenceError::TipLag
        );
        let op = self.operation(id)?;
        let height = sync.height.context("missing recovery height")?;
        ensure!(
            height >= u64::from(op.facts.expiry_height) + u64::from(sync.confirmations),
            crate::rotation::transaction::RecoveryEvidenceError::ExpiryNotBuried
        );
        let tx = self.db.transaction()?;
        ensure!(
            tx.execute(
                "UPDATE outgoing SET state='RESOLVED' WHERE id=?1 AND state='PREPARED'",
                [id]
            )? == 1,
            "operation is not pending"
        );
        ensure!(
            tx.execute(
                "UPDATE budget_entries SET reserved=0 WHERE id=?1 AND consumed=0",
                [id]
            )? == 1,
            "operation already consumed"
        );
        tx.execute(
            "INSERT INTO expired_operations VALUES (?1,?2,?3)",
            params![id, i64::try_from(height)?, revision],
        )?;
        // Canonical closure does not grant another attempt. Preserve the original
        // operation, quote, refund binding and exact bytes until a separate
        // current-authority check permits reauthorization on the same job.
        let job: Option<String> = tx.query_row("SELECT job_id FROM funding_progress WHERE operation_id=?1 AND EXISTS(SELECT 1 FROM funding_jobs f WHERE f.id=job_id AND f.state!='COMPLETE')",[id],|r|r.get(0)).optional()?;
        if let Some(job) = job {
            tx.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote,refund,provider_success,provider_failed,provider_refunded,provider_incomplete,provider_observed_at,quote_target,quote_ceiling,accepted_output,accepted_recipient) SELECT j.operation_id,j.job_id,j.phase,j.quote,r.address,j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,j.provider_observed_at,j.quote_target,j.quote_ceiling,j.accepted_output,j.accepted_recipient FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.job_id=?1",[&job])?;
            tx.execute("UPDATE funding_progress SET phase='\"RECOVERY_REQUIRED\"',last_error='funding_attempt_closed; current authority required for another attempt' WHERE job_id=?1",[&job])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expiry_release_is_revision_guarded_and_preserves_job_identity() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut s = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"initial",
        )?;
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            s,
            "a",
            crate::rotation::test_capacity::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )?;
        let job = s.funding_jobs()?.remove(0);
        s.save_funding_quote(
            &job.id,
            &serde_json::to_vec(
                &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
            )?,
        )?;
        s.advance_funding(
            &job.id,
            funding::FundingPhase::Quoted,
            funding::FundingPhase::Preparing,
        )?;
        s.reserve(&job.operation_id, Some(&pool), 1, 100)?;
        s.prepare_with_facts(
            &job.operation_id,
            1,
            b"prepared",
            b"signed",
            Some(TransactionFacts {
                txid: "test".into(),
                expiry_height: 20,
                amount_zatoshis: 80,
                fee_zatoshis: 20,
                deadline: 1000,
            }),
        )?;
        assert!(!s.funding_facts(&job.id, &job.operation_id)?.status_needed());
        s.request_broadcast(&job.operation_id, 1, 1, false)?;
        assert!(s.funding_facts(&job.id, &job.operation_id)?.status_needed());
        let observation = SyncObservation {
            phase: SyncPhase::Ready,
            last_error: None,
            snapshot_revision: 2,
            checked_at: Some(100),
            checkpoint_at: 100,
            scanned_blocks: 22,
            target_height: Some(22),
            observed_tip_height: Some(22),
            height: Some(22),
            confirmations: 2,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 1000,
            spendable_shielded_zatoshis: 1000,
        };
        let mut shallow = observation.clone();
        shallow.height = Some(21);
        s.save_sync_snapshot(2, b"shallow", Some(shallow))?;
        assert!(s.resolve_expired(&job.operation_id, 3, 100).is_err());
        assert!(s.operation_pending(&job.operation_id)?);
        let mut revision = 3;
        for tip in [None, Some(23)] {
            let mut lagged = observation.clone();
            lagged.observed_tip_height = tip;
            revision = s.save_sync_snapshot(revision, b"lagged", Some(lagged))?;
            let error = s
                .resolve_expired(&job.operation_id, revision, 100)
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<crate::rotation::transaction::RecoveryEvidenceError>(),
                Some(&crate::rotation::transaction::RecoveryEvidenceError::TipLag)
            );
            assert!(s.operation_pending(&job.operation_id)?);
        }
        revision = s.save_sync_snapshot(revision, b"synced", Some(observation))?;
        assert!(s.resolve_expired(&job.operation_id, 2, 100).is_err());
        s.resolve_expired(&job.operation_id, revision, 401)?;
        assert_eq!(s.operation(&job.operation_id)?.submission, "EXPIRED");
        let facts = s.funding_facts(&job.id, &job.operation_id)?;
        assert!(facts.archived && facts.accepted_quote && facts.source_attempts == 1);
        assert!(
            !facts.status_needed(),
            "archived operations cannot trigger status I/O"
        );
        assert!(!s.operation_pending(&job.operation_id)?);
        let next = s.funding_jobs()?.remove(0);
        assert_eq!(next.operation_id, job.operation_id);
        assert_eq!(next.phase, funding::FundingPhase::RecoveryRequired);
        assert_eq!(s.funding_recovery_count(&job.id)?, 1);
        assert!(s.reauthorize_closed_funding(&job.id, 0).is_err());
        s.funding_restriction =
            super::super::super::restriction::FundingRestriction::DenyNewFunding;
        assert!(s.reauthorize_closed_funding(&job.id, 1).is_err());
        assert_eq!(
            s.funding_job(&job.id)?.unwrap().operation_id,
            job.operation_id
        );
        s.funding_restriction = super::super::super::restriction::FundingRestriction::Ordinary;
        s.reauthorize_closed_funding(&job.id, 1)?;
        assert_ne!(
            s.funding_job(&job.id)?.unwrap().operation_id,
            job.operation_id
        );
        assert_eq!(s.funding_recovery_count(&job.id)?, 1);
        s.resolve_expired(&job.operation_id, revision, 100)?;
        assert_eq!(&*s.prepared_snapshot(&job.operation_id)?, b"prepared");
        assert!(s.confirm_spend(&job.operation_id, 100, 1).is_err());
        let treasury = s.id().to_owned();
        let recovery_revision = revision;
        let mut revision = s.snapshot()?.0;
        for _ in 0..10 {
            revision = s.save_snapshot(revision, b"new checkpoint")?;
        }
        let retained: Vec<i64> =
            s.db.prepare("SELECT revision FROM snapshots ORDER BY revision")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
        assert_eq!(retained, vec![2, recovery_revision, revision]);
        drop(s);
        let s = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &treasury,
        )?;
        assert_eq!(&*s.prepared_snapshot(&job.operation_id)?, b"prepared");
        assert_eq!(s.operation(&job.operation_id)?.submission, "EXPIRED");
        Ok(())
    }
}
