//! Replace completed, uncredited reserve inventory without reopening old work.
use super::*;

// Root the lookup in the exact current slot, not historical completions.
const BLOCKED_JOB: &str = "SELECT f.id FROM reserve_claims c JOIN pools p ON p.id=c.pool_id JOIN wallets w ON w.id=c.wallet_id AND w.pool_id=c.pool_id JOIN funding_jobs f ON f.wallet_id=w.id AND f.pool_id=c.pool_id AND f.allocation_kind=c.slot AND f.claim_generation=c.generation WHERE c.pool_id=?1 AND c.slot=?2 AND p.enabled=1 AND c.enabled=1 AND w.role='ALLOCATED' AND f.state='COMPLETE' AND f.readiness_requirement IS NULL";

pub(super) fn blocked_job(db: &Connection, pool: &str, slot: &str) -> Result<Option<String>> {
    Ok(db
        .query_row(BLOCKED_JOB, params![pool, slot], |r| r.get(0))
        .optional()?)
}

impl Store {
    /// One startup notice per blocked current slot; polling does not repeat it.
    pub(crate) fn report_blocked_reserves(&self, name: &str) -> Result<()> {
        let pool: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM pools WHERE name=?1 AND enabled=1",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(pool) = pool {
            for kind in [AllocationKind::Small, AllocationKind::Large] {
                if let Some(job) = blocked_job(&self.db, &pool, kind.as_str())? {
                    tracing::info!(
                        wallet = name,
                        slot = kind.as_str(),
                        job_id = job,
                        "Completed funding still occupies an uncredited reserve; replacement is waiting for original Zcash source ownership to resolve; existing wallets remain usable and automatic checks continue"
                    );
                }
            }
        }
        Ok(())
    }
    /// Automatic funding may replace inventory, never replay a completed job.
    /// An interrupted pass can leave a vacancy; ordinary allocation fills it.
    pub(super) fn replace_completed_reserve(
        &mut self,
        pool: &str,
        kind: AllocationKind,
    ) -> Result<()> {
        self.funding_restriction
            .require_new_funding("capacity_allocation")?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(job) = blocked_job(&tx, pool, kind.as_str())? else {
            return Ok(());
        };
        // Include prior attempts of this job, without blocking unrelated pools.
        // Confirmation closes source ownership even when bridge delivery remains
        // unknown. Unlike reattempting an old job, replacement uses a new payer.
        let unresolved: bool = tx.query_row(
            "WITH operations AS (SELECT operation_id FROM funding_progress WHERE job_id=?1 UNION SELECT operation_id FROM funding_recovery WHERE job_id=?1) SELECT EXISTS(SELECT 1 FROM operation_network n JOIN operations o ON o.operation_id=n.operation_id WHERE n.preparation IS NOT NULL) OR EXISTS(SELECT 1 FROM outgoing x JOIN operations o ON o.operation_id=x.id WHERE x.state!='RESOLVED' OR NOT (EXISTS(SELECT 1 FROM treasury_operations t WHERE t.id=x.id AND t.submission='CONFIRMED') OR EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=x.id)))",
            [&job], |r| r.get(0),
        )?;
        if unresolved {
            tracing::debug!(
                pool_id = pool,
                slot = kind.as_str(),
                job_id = job,
                "Reserve replacement is waiting for original Zcash source ownership to resolve"
            );
            return Ok(());
        }
        let (wallet, generation): (String, i64) = tx.query_row(
            "SELECT wallet_id,claim_generation FROM funding_jobs WHERE id=?1",
            [&job],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let next = generation
            .checked_add(1)
            .context("reserve_generation_overflow")?;
        tx.execute("UPDATE wallets SET role='RETAINED' WHERE id=?1", [&wallet])?;
        tx.execute(
            "UPDATE reserve_claims SET wallet_id=NULL,generation=?3 WHERE pool_id=?1 AND slot=?2",
            params![pool, kind.as_str(), next],
        )?;
        tx.commit()?;
        self.notify_capacity_changed(pool);
        tracing::info!(
            pool_id = pool,
            slot = kind.as_str(),
            job_id = job,
            "Completed funding left an uncredited reserve; old wallet and financial history retained; a fresh reserve will be allocated automatically"
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "reserve_replacement_tests.rs"]
mod tests;
