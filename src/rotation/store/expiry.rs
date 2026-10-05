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
    pub(crate) fn resolve_expired(&mut self, id: &str, revision: i64, instant: u64) -> Result<()> {
        let status = self.status()?;
        let sync = status.sync.context("missing recovery sync")?;
        ensure!(
            status.snapshot_revision == revision && sync.fresh(instant, revision),
            "expiry recovery requires fresh unchanged snapshot"
        );
        ensure!(
            sync.at_observed_tip(),
            "expiry recovery requires zero observed tip lag"
        );
        let op = self.operation(id)?;
        let height = sync.height.context("missing recovery height")?;
        ensure!(
            height >= u64::from(op.facts.expiry_height) + u64::from(sync.confirmations),
            "transaction expiry not buried"
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
        // A replacement is a new operation, quote and refund derivation. Preserve
        // the old signed bytes forever; they can never mint another broadcast.
        let job: Option<String> = tx.query_row("SELECT job_id FROM funding_progress WHERE operation_id=?1 AND EXISTS(SELECT 1 FROM funding_jobs f WHERE f.id=job_id AND f.state!='COMPLETE')",[id],|r|r.get(0)).optional()?;
        if let Some(job) = job {
            tx.execute("INSERT INTO funding_recovery SELECT j.operation_id,j.job_id,j.phase,j.quote,r.address FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.job_id=?1",[&job])?;
            tx.execute("DELETE FROM funding_refunds WHERE job_id=?1", [&job])?;
            tx.execute("DELETE FROM funding_health WHERE job_id=?1", [&job])?;
            tx.execute("UPDATE funding_progress SET operation_id=?2,phase='\"ALLOCATED\"',quote=NULL,attempts=0,next_poll=0,last_error=NULL WHERE job_id=?1",params![job,Uuid::new_v4().to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expiry_release_is_revision_guarded_and_replaces_job_identity() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut s = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"initial",
        )?;
        let pool = s.ensure_pool("a", "5")?;
        let job = s.funding_jobs()?.remove(0);
        s.save_funding_quote(&job.id, b"quote")?;
        s.advance_funding(
            &job.id,
            funding::FundingPhase::Quoted,
            funding::FundingPhase::Preparing,
        )?;
        s.reserve(&job.operation_id, Some(&pool), 1, 100, 1000)?;
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
            max_age_seconds: 300,
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
            assert!(error.to_string().contains("zero observed tip lag"));
            assert!(s.operation_pending(&job.operation_id)?);
        }
        revision = s.save_sync_snapshot(revision, b"synced", Some(observation))?;
        assert!(s.resolve_expired(&job.operation_id, 2, 100).is_err());
        assert!(s.resolve_expired(&job.operation_id, revision, 401).is_err());
        s.resolve_expired(&job.operation_id, revision, 100)?;
        assert_eq!(s.operation(&job.operation_id)?.submission, "EXPIRED");
        assert!(!s.operation_pending(&job.operation_id)?);
        let next = s.funding_jobs()?.remove(0);
        assert_ne!(next.operation_id, job.operation_id);
        assert_eq!(next.phase, funding::FundingPhase::Allocated);
        assert!(s.resolve_expired(&job.operation_id, revision, 100).is_err());
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
