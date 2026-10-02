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
    pub next_poll: u64,
    pub last_error: Option<String>,
}
impl Store {
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
        let changed = self.db.execute(
            "UPDATE funding_progress SET phase=?3,last_error=NULL WHERE job_id=?1 AND phase=?2",
            params![
                id,
                serde_json::to_string(&expected)?,
                serde_json::to_string(&next)?
            ],
        )?;
        ensure!(changed == 1, "funding phase conflict");
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
        ensure!(self.db.execute("UPDATE funding_progress SET next_poll=?2,last_error=?3,attempts=attempts+?4 WHERE job_id=?1",params![id,i64::try_from(next_poll)?,error,u32::from(quote_attempt)])? == 1, "unknown funding job");
        Ok(())
    }
}
pub(super) fn read_jobs(db: &Connection) -> Result<Vec<FundingJob>> {
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
        Ok(FundingJob {
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
