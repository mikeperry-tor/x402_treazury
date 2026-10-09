//! Funding workflow journal. The source operation ID is allocated before any
//! calculation and remains immutable across crashes and ambiguous submissions.
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FundingPhase {
    Allocated,
    Quoted,
    Preparing,
    Prepared,
    DepositPending,
    Swapping,
    VerifyingCredit,
    Complete,
    RefundPending,
    RecoveryRequired,
}
impl FundingPhase {
    fn permits(&self, next: &Self) -> bool {
        use FundingPhase::*;
        matches!(
            (self, next),
            (Allocated, Quoted)
                | (Quoted, Preparing)
                | (Preparing, Prepared)
                | (Prepared, DepositPending)
                | (DepositPending, Swapping)
                | (Swapping, VerifyingCredit)
                | (DepositPending, VerifyingCredit)
                | (DepositPending | Swapping | VerifyingCredit, RefundPending)
                | (
                    Allocated
                        | Quoted
                        | Preparing
                        | Prepared
                        | DepositPending
                        | Swapping
                        | VerifyingCredit
                        | RefundPending,
                    RecoveryRequired
                )
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FundingJob {
    pub id: String,
    pub pool_id: String,
    pub pool_name: String,
    pub wallet_id: String,
    pub recipient: String,
    pub target: String,
    pub phase: FundingPhase,
    pub operation_id: String,
    pub attempts: u32,
    pub started_at: Option<u64>,
    pub error_streak: u32,
    pub timed_out: bool,
    pub next_poll: u64,
    pub last_error: Option<String>,
}
/// Clear obsolete destination-funding errors in the same credit transaction.
pub(super) fn complete_job(tx: &rusqlite::Connection, wallet: &str) -> Result<()> {
    tx.execute(
        "UPDATE funding_jobs SET state='COMPLETE' WHERE wallet_id=?1",
        [wallet],
    )?;
    tx.execute("UPDATE funding_progress SET last_error=CASE WHEN last_error LIKE 'source_reconciliation_failed;%' THEN last_error ELSE NULL END WHERE job_id IN (SELECT id FROM funding_jobs WHERE wallet_id=?1)", [wallet])?;
    tx.execute("UPDATE funding_health SET error_streak=0 WHERE job_id IN (SELECT f.id FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.wallet_id=?1 AND j.last_error IS NULL)", [wallet])?;
    Ok(())
}
fn completed_error(error: Option<&str>) -> Option<&str> {
    // Destination credit does not establish source confirmation.
    error.filter(|e| e.starts_with("source_reconciliation_failed;"))
}
impl Store {
    /// Recovery history survives restarts and operation replacement.
    pub fn funding_recovery_count(&self, job: &str) -> Result<u64> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM funding_recovery r WHERE job_id=?1 AND NOT EXISTS(SELECT 1 FROM funding_quote_refreshes q WHERE q.operation_id=r.operation_id)",
            [job],
            |r| r.get(0),
        )?;
        Ok(u64::try_from(count)?)
    }

    /// Called only after a typed response proving the preparer was never invoked.
    /// Absence of bytes by itself is not authority to repeat preparation.
    pub fn defer_unstarted_preparation(&mut self, job: &str) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (operation, phase): (String, String) = tx.query_row(
            "SELECT operation_id,phase FROM funding_progress WHERE job_id=?1",
            [job],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            serde_json::from_str::<FundingPhase>(&phase)? == FundingPhase::Preparing,
            "preparation is not pending"
        );
        let touched: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) OR EXISTS(SELECT 1 FROM budget_entries WHERE id=?1)", [&operation], |r| r.get(0))?;
        ensure!(
            !touched,
            "preparation has durable effects; recovery required"
        );
        tx.execute(
            "UPDATE funding_progress SET phase=?2 WHERE job_id=?1",
            params![job, serde_json::to_string(&FundingPhase::Quoted)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Operator-only reset. Any signed bytes (even resolved ones) forbid it.
    /// Old encrypted bindings remain archived; the next attempt gets a new ID
    /// and a freshly derived refund address, never another send of old intent.
    pub fn recover_unprepared_funding(&mut self, job: &str) -> Result<()> {
        self.reset_unprepared(job, false)
    }
    pub fn refresh_unprepared_quote(&mut self, job: &str) -> Result<()> {
        self.reset_unprepared(job, true)
    }
    fn reset_unprepared(&mut self, job: &str, refresh: bool) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (operation, phase): (String, String) = tx.query_row("SELECT j.operation_id,j.phase FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id WHERE j.job_id=?1 AND f.state!='COMPLETE' AND w.role='ALLOCATED'", [job], |r| Ok((r.get(0)?,r.get(1)?)))?;
        let parsed: FundingPhase = serde_json::from_str(&phase)?;
        let attempts: u32 = tx.query_row(
            "SELECT attempts FROM funding_progress WHERE job_id=?1",
            [job],
            |r| r.get(0),
        )?;
        ensure!(
            !refresh || parsed == FundingPhase::Quoted,
            "only unprepared quotes can refresh"
        );
        ensure!(
            matches!(
                parsed,
                FundingPhase::Allocated
                    | FundingPhase::Quoted
                    | FundingPhase::Preparing
                    | FundingPhase::RecoveryRequired
            ),
            "funding phase cannot be reset"
        );
        let signed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1)",
            [&operation],
            |r| r.get(0),
        )?;
        ensure!(!signed, "recovery refused: operation has signed bytes");
        let consumed: i64 = tx.query_row(
            "SELECT COALESCE(SUM(consumed),0) FROM budget_entries WHERE id=?1",
            [&operation],
            |r| r.get(0),
        )?;
        ensure!(
            consumed == 0,
            "recovery refused: operation has consumed budget"
        );
        tx.execute("INSERT INTO funding_recovery SELECT j.operation_id,j.job_id,j.phase,j.quote,r.address FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.job_id=?1", [job])?;
        if refresh {
            let reserved: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM budget_entries WHERE id=?1)",
                [&operation],
                |r| r.get(0),
            )?;
            ensure!(
                !reserved,
                "quote refresh refused: preparation has durable effects"
            );
            tx.execute(
                "INSERT INTO funding_quote_refreshes(operation_id) VALUES(?1)",
                [&operation],
            )?;
        }
        tx.execute("DELETE FROM budget_entries WHERE id=?1", [&operation])?;
        tx.execute("DELETE FROM funding_refunds WHERE job_id=?1", [job])?;
        tx.execute("UPDATE funding_progress SET operation_id=?2,phase='\"ALLOCATED\"',quote=NULL,attempts=?3,next_poll=0,last_error=NULL WHERE job_id=?1", params![job,Uuid::new_v4().to_string(),if refresh { attempts } else {0}])?;
        tx.execute("DELETE FROM funding_health WHERE job_id=?1", [job])?;
        tx.commit()?;
        Ok(())
    }

    pub fn refund_address(&self, job: &str) -> Result<Option<String>> {
        let bytes: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT address FROM funding_refunds WHERE job_id=?1",
                [job],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|bytes| {
                let plain = unseal(
                    &self.key,
                    &format!("v1:{}:{}:refund:{job}", self.id, self.network.name()),
                    &bytes,
                )?;
                Ok(std::str::from_utf8(&plain)?.to_owned())
            })
            .transpose()
    }
    pub fn save_refund_address(
        &mut self,
        job: &str,
        address: &str,
        revision: i64,
        snapshot: &[u8],
    ) -> Result<i64> {
        self.save_wallet_snapshot(revision, snapshot, None, Some((job, address)))
    }

    /// Materialize a journal row for every allocation, including databases created
    /// before the worker existed. Never discard jobs for disabled pools.
    pub fn funding_jobs(&mut self) -> Result<Vec<FundingJob>> {
        let tx = self.db.transaction()?;
        let missing: Vec<String> = tx.prepare("SELECT id FROM funding_jobs WHERE id NOT IN (SELECT job_id FROM funding_progress) ORDER BY rowid")?
            .query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        for id in missing {
            tx.execute("INSERT INTO funding_progress(job_id,operation_id,phase) VALUES (?1,?2,'\"ALLOCATED\"')", params![id, Uuid::new_v4().to_string()])?;
        }
        tx.commit()?;
        read_jobs(&self.db)
    }
    /// Persist a quote encrypted; it links treasury refunds to destination keys.
    /// Repeated delivery must match exactly and cannot replace an accepted quote.
    pub fn save_funding_quote(&mut self, id: &str, quote: &[u8]) -> Result<()> {
        self.save_quote(id, quote, None)
    }
    /// A provisional allocation can grow to the validated bridge minimum only
    /// when its first quote commits; wallet and job targets change atomically.
    #[cfg(feature = "zcash")]
    pub fn save_funding_quote_with_target(
        &mut self,
        id: &str,
        quote: &[u8],
        target: &str,
    ) -> Result<()> {
        let parsed: crate::rotation::near::Quote = serde_json::from_slice(quote)?;
        ensure!(parsed.request["amount"] == target, "quote target mismatch");
        self.save_quote(id, quote, Some(target))
    }
    fn save_quote(&mut self, id: &str, quote: &[u8], target: Option<&str>) -> Result<()> {
        ensure!(!quote.is_empty(), "empty funding quote");
        let aad = format!("v1:{}:{}:funding:{id}", self.id, self.network.name());
        let encrypted = seal(&self.key, &aad, quote)?;
        let tx = self.db.transaction()?;
        let (phase, previous): (String, Option<Vec<u8>>) = tx.query_row(
            "SELECT phase,quote FROM funding_progress WHERE job_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if let Some(previous) = previous {
            ensure!(
                unseal(&self.key, &aad, &previous)?.as_slice() == quote,
                "funding_quote_conflict"
            );
        } else {
            ensure!(
                serde_json::from_str::<FundingPhase>(&phase)? == FundingPhase::Allocated,
                "funding phase conflict"
            );
            if let Some(target) = target {
                let (wallet, old, role, pending): (String, String, String, bool) = tx.query_row(
                    "SELECT w.id,w.target,w.role,EXISTS(SELECT 1 FROM outgoing o WHERE o.id=j.operation_id) OR EXISTS(SELECT 1 FROM budget_entries b WHERE b.id=j.operation_id) FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1 AND f.state='QUEUED'", [id],
                    |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
                ensure!(
                    role == "ALLOCATED" && !pending,
                    "funding target already committed to spending"
                );
                ensure!(
                    amount(target)? >= amount(&old)?,
                    "funding target cannot decrease"
                );
                tx.execute(
                    "UPDATE wallets SET target=?2 WHERE id=?1",
                    params![wallet, target],
                )?;
                tx.execute(
                    "UPDATE funding_jobs SET target=?2 WHERE id=?1",
                    params![id, target],
                )?;
            }
            tx.execute(
                "UPDATE funding_progress SET phase='\"QUOTED\"',quote=?2 WHERE job_id=?1",
                params![id, encrypted],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn funding_quote(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let bytes: Vec<u8> = self.db.query_row(
            "SELECT quote FROM funding_progress WHERE job_id=?1",
            [id],
            |r| r.get(0),
        )?;
        unseal(
            &self.key,
            &format!("v1:{}:{}:funding:{id}", self.id, self.network.name()),
            &bytes,
        )
    }
    pub fn advance_funding(
        &mut self,
        id: &str,
        expected: FundingPhase,
        next: FundingPhase,
    ) -> Result<()> {
        ensure!(expected.permits(&next), "invalid funding transition");
        if next == FundingPhase::Preparing {
            self.funding_restriction
                .require_new_funding("funding_preparation_start")?;
            self.check_job_permit(id, 1)?;
        }
        let changed = self.db.execute(
            "UPDATE funding_progress SET phase=?3,last_error=CASE WHEN ?4 THEN last_error ELSE NULL END WHERE job_id=?1 AND phase=?2 AND EXISTS(SELECT 1 FROM funding_jobs f WHERE f.id=job_id AND f.state!='COMPLETE')",
            params![
                id,
                serde_json::to_string(&expected)?,
                serde_json::to_string(&next)?,
                next == FundingPhase::RecoveryRequired
            ],
        )?;
        ensure!(changed == 1, "funding phase conflict");
        Ok(())
    }
    pub fn mark_funding_timeout(&mut self, id: &str) -> Result<()> {
        self.db.execute("INSERT INTO funding_health(job_id,timed_out) VALUES (?1,1) ON CONFLICT(job_id) DO UPDATE SET timed_out=1",[id])?;
        Ok(())
    }
    /// Check before advancing to PREPARING; the owner still rechecks at reserve.
    pub fn check_funding_capacity(&self, instant: u64, input: u64, limit: u64) -> Result<()> {
        self.require_spend_ready(instant, input)?;
        let used: i64 = self.db.query_row("SELECT COALESCE(SUM(reserved),0)+COALESCE(SUM(CASE WHEN day=?1 THEN MAX(0,consumed-COALESCE((SELECT SUM(credited) FROM refund_outputs r WHERE r.operation_id=budget_entries.id),0)) ELSE 0 END),0) FROM budget_entries",[i64::try_from(instant/86400)?],|r|r.get(0))?;
        ensure!(
            u64::try_from(used)?
                .checked_add(input)
                .is_some_and(|n| n <= limit),
            "treasury_budget_exceeded"
        );
        Ok(())
    }
    /// Fair persistent ordering: every selected job moves behind all other jobs,
    /// including siblings in the same pool. Disabled pools may only be recovered.
    pub fn next_funding_job(&mut self, now: u64) -> Result<Option<FundingJob>> {
        self.funding_jobs()?;
        let tx = self.db.transaction()?;
        let id: Option<String> = tx.query_row("SELECT f.id FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id WHERE f.state!='COMPLETE' AND j.phase!='\"RECOVERY_REQUIRED\"' AND j.next_poll<=?1 AND (p.enabled=1 OR j.phase NOT IN ('\"ALLOCATED\"','\"QUOTED\"')) ORDER BY j.turn,f.rowid LIMIT 1", [i64::try_from(now)?], |r| r.get(0)).optional()?;
        if let Some(id) = &id {
            tx.execute("UPDATE funding_progress SET turn=(SELECT COALESCE(MAX(turn),0)+1 FROM funding_progress) WHERE job_id=?1", [id])?;
        }
        tx.commit()?;
        Ok(read_jobs(&self.db)?
            .into_iter()
            .find(|j| Some(&j.id) == id.as_ref()))
    }
    pub fn defer_funding(
        &mut self,
        id: &str,
        next_poll: u64,
        error: Option<&str>,
        quote_attempt: bool,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        let complete: bool = tx.query_row(
            "SELECT state='COMPLETE' FROM funding_jobs WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        let error = if complete {
            completed_error(error)
        } else {
            error
        };
        ensure!(tx.execute("UPDATE funding_progress SET next_poll=?2,last_error=CASE WHEN phase='\"RECOVERY_REQUIRED\"' THEN COALESCE(?3,last_error) ELSE ?3 END,attempts=attempts+?4 WHERE job_id=?1",params![id,i64::try_from(next_poll)?,error,u32::from(quote_attempt)])? == 1, "unknown funding job");
        tx.execute("INSERT INTO funding_health(job_id,error_streak) VALUES (?1,?2) ON CONFLICT(job_id) DO UPDATE SET error_streak=CASE WHEN excluded.error_streak=0 THEN 0 ELSE MIN(error_streak+1,32) END",params![id,u32::from(error.is_some())])?;
        tx.execute("UPDATE funding_health SET timed_out=0 WHERE job_id=?1 AND EXISTS(SELECT 1 FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN treasury_operations o ON o.id=j.operation_id WHERE f.id=?1 AND f.state='COMPLETE' AND o.submission='CONFIRMED')",[id])?;
        tx.commit()?;
        Ok(())
    }
}
pub(super) fn read_jobs(db: &Connection) -> Result<Vec<FundingJob>> {
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let mut stmt = db.prepare("SELECT f.id,w.pool_id,p.name,w.id,w.address,f.target,CASE WHEN f.state='COMPLETE' THEN '\"COMPLETE\"' ELSE j.phase END,j.operation_id,j.attempts,j.next_poll,j.last_error FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id ORDER BY f.rowid")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, u32>(8)?,
            r.get::<_, i64>(9)?,
            r.get::<_, Option<String>>(10)?,
        ))
    })?;
    rows.map(|r| {
        let (
            id,
            pool_id,
            pool_name,
            wallet_id,
            recipient,
            target,
            phase,
            operation_id,
            attempts,
            next_poll,
            last_error,
        ) = r?;
        let (started_at,error_streak,timed_out): (Option<i64>,u32,bool) = if version >= 9 {
            db.query_row("SELECT started_at,error_streak,timed_out FROM funding_health WHERE job_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.unwrap_or((None,0,false))
        } else {(None,0,false)};
        let parsed_phase: FundingPhase = serde_json::from_str(&phase)?;
        // Also normalize historical completed rows for read-only wallet status.
        let last_error = if parsed_phase == FundingPhase::Complete {
            completed_error(last_error.as_deref()).map(str::to_owned)
        } else {
            last_error
        };
        let last_error = last_error.or_else(||timed_out.then(||"swap_timeout; slower reconciliation continues; inspect source and refund status".into()));
        Ok(FundingJob {
            started_at: started_at.map(u64::try_from).transpose()?,
            error_streak: if parsed_phase == FundingPhase::Complete && last_error.is_none() { 0 } else { error_streak },
            timed_out,
            id,
            pool_id,
            pool_name,
            wallet_id,
            recipient,
            target,
            phase: serde_json::from_str(&phase)?,
            operation_id,
            attempts,
            next_poll: u64::try_from(next_poll)?,
            last_error,
        })
    })
    .collect()
}
