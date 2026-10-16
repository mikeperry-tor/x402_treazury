//! Provenance for the calculate-only source owner, not a second recovery workflow.
//!
//! Contract 1 cannot submit or expose signed bytes before the atomic outgoing /
//! snapshot commit. A failed caller may close only after retiring its SDK owner;
//! accepted StoreHandle jobs are serialized ahead of that closure. Unknown legacy
//! PREPARING rows carry no such proof and remain unresolved.
use super::*;

const CALCULATE_ONLY_V1: i64 = 1;

#[derive(Clone, Debug)]
pub(crate) struct Preparation {
    operation: String,
    job: Option<String>,
    revision: i64,
    token: String,
}
impl Preparation {
    pub(crate) fn operation(&self) -> &str {
        &self.operation
    }
    pub(crate) fn revision(&self) -> i64 {
        self.revision
    }
}

pub(super) fn check_commit(
    db: &Connection,
    operation: &str,
    revision: i64,
    owner: Option<&Preparation>,
) -> Result<()> {
    let saved: Option<(Option<String>, i64, i64, String, bool)> = db.query_row(
        "SELECT job_id,base_revision,contract_version,token,closed FROM treasury_preparations WHERE operation_id=?1",
        [operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional()?;
    match (saved, owner) {
        (None, None) => Ok(()),
        (Some((job, base, contract, token, closed)), Some(owner)) => {
            ensure!(
                owner.operation == operation
                    && owner.revision == revision
                    && owner.job == job
                    && base == revision
                    && contract == CALCULATE_ONLY_V1
                    && token == owner.token
                    && !closed,
                "preparation provenance changed"
            );
            let current_job: Option<String> = db
                .query_row(
                    "SELECT job_id FROM funding_progress WHERE operation_id=?1",
                    [operation],
                    |r| r.get(0),
                )
                .optional()?;
            ensure!(
                current_job == owner.job,
                "preparation operation binding changed"
            );
            Ok(())
        }
        _ => anyhow::bail!("preparation owner proof missing"),
    }
}

/// Read-only historical reports predate this contract. Runtime admission only
/// opens the current schema; an unknown marker always retains ownership.
pub(super) fn source_state(db: &Connection, operation: &str) -> Result<(bool, bool)> {
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let archive: Option<(String, String)> = db
        .query_row(
            "SELECT job_id,phase FROM funding_recovery WHERE operation_id=?1",
            [operation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(scoped_decode)?;
    let archive_unknown = archive
        .as_ref()
        .is_some_and(|(_, phase)| !matches!(phase.as_str(), "\"ALLOCATED\"" | "\"QUOTED\""));
    if version < 20 {
        return Ok((false, archive_unknown));
    }
    type Marker = (Option<String>, i64, i64, String, i64);
    let marker: Option<Marker> = db.query_row(
        "SELECT job_id,base_revision,contract_version,token,closed FROM treasury_preparations WHERE operation_id=?1", [operation],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional().map_err(scoped_decode)?;
    let Some((job, revision, contract, token, closed)) = marker else {
        return Ok((false, archive_unknown));
    };
    ensure!(
        revision > 0 && matches!(closed, 0 | 1) && Uuid::parse_str(&token).is_ok(),
        super::scheduling::FundingDecodeFailure
    );
    let proven_closed = closed == 1
        && contract == CALCULATE_ONLY_V1
        && archive
            .as_ref()
            .is_some_and(|(archived_job, _)| job.as_deref() == Some(archived_job.as_str()));
    Ok((true, !proven_closed))
}

fn scoped_decode(error: rusqlite::Error) -> anyhow::Error {
    if matches!(
        error,
        rusqlite::Error::InvalidColumnType(..)
            | rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
    ) {
        anyhow::Error::new(error).context(super::scheduling::FundingDecodeFailure)
    } else {
        error.into()
    }
}

impl Store {
    /// Reconstruct contract-1 ownership only at the drained SDK handoff. Unknown
    /// contracts, changed snapshots and contradictory effects retain liability.
    #[cfg(feature = "zcash")]
    pub(crate) fn close_retired_preparations_page(
        &mut self,
        retired: &crate::treasury::ownership::RetiredSdk,
        after: String,
    ) -> Result<Option<String>> {
        ensure!(
            retired.namespace() == self.evidence_namespace,
            "retired SDK store binding changed"
        );
        let mut after = after;
        let rows = self.db.prepare(
            r#"SELECT t.operation_id,
                t.closed=0 AND t.contract_version=1
                AND typeof(t.base_revision)='integer'
                AND t.base_revision=(SELECT MAX(revision) FROM snapshots)
                AND typeof(t.token)='text'
                AND (t.job_id IS NULL OR typeof(t.job_id)='text')
                AND NOT EXISTS(SELECT 1 FROM outgoing WHERE id=t.operation_id)
                AND NOT EXISTS(SELECT 1 FROM treasury_operations WHERE id=t.operation_id)
                AND NOT EXISTS(SELECT 1 FROM expired_operations WHERE id=t.operation_id)
                AND NOT EXISTS(SELECT 1 FROM budget_entries WHERE id=t.operation_id AND consumed!=0)
                AND ((t.job_id IS NULL AND NOT EXISTS(SELECT 1 FROM funding_progress WHERE operation_id=t.operation_id))
                    OR EXISTS(SELECT 1 FROM funding_progress WHERE job_id=t.job_id AND operation_id=t.operation_id AND phase='"PREPARING"')),
                t.job_id,t.base_revision,t.token
                FROM treasury_preparations t WHERE t.operation_id>?1
                ORDER BY t.operation_id LIMIT 64"#,
        )?.query_map([&after], |r| {
            let operation: String = r.get(0)?;
            let owner = if r.get::<_,Option<bool>>(1)?.unwrap_or(false) {
                Some(Preparation { operation:operation.clone(),job:r.get(2)?,revision:r.get(3)?,token:r.get(4)? })
            } else { None };
            Ok((operation,owner))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            return Ok(None);
        }
        for (operation, owner) in rows {
            after = operation;
            if let Some(owner) = owner
                && Uuid::parse_str(&owner.token).is_ok()
            {
                self.close_retired_preparation(&owner)?;
            }
        }
        Ok(Some(after))
    }

    /// Accept the reviewed SDK contract and exact starting committed snapshot in
    /// the same transaction that enters managed PREPARING.
    pub(crate) fn begin_owned_preparation(
        &mut self,
        operation: &str,
        revision: i64,
        authority: Option<&super::super::transaction::ManagedFundingAuthority>,
    ) -> Result<Preparation> {
        self.require_preparation_authority(operation, authority)?;
        self.require_unstarted_preparation(operation)?;
        let owner = Preparation {
            operation: operation.to_owned(),
            job: authority.map(|a| a.job_id.clone()),
            revision,
            token: Uuid::new_v4().to_string(),
        };
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: i64 = tx.query_row("SELECT MAX(revision) FROM snapshots", [], |r| r.get(0))?;
        ensure!(current == revision, "snapshot_revision_conflict");
        if let Some(job) = &owner.job {
            let changed = tx.execute("UPDATE funding_progress SET phase='\"PREPARING\"',last_error=NULL WHERE job_id=?1 AND operation_id=?2 AND phase='\"QUOTED\"' AND EXISTS(SELECT 1 FROM funding_jobs f WHERE f.id=job_id AND f.readiness_requirement IS NULL AND f.state!='COMPLETE')", params![job,operation])?;
            ensure!(changed == 1, "funding preparation boundary changed");
        }
        tx.execute("INSERT INTO treasury_preparations(operation_id,job_id,base_revision,contract_version,token) VALUES(?1,?2,?3,?4,?5)",params![operation,owner.job,revision,CALCULATE_ONLY_V1,owner.token])?;
        tx.commit()?;
        Ok(owner)
    }

    pub(crate) fn require_preparation_owner(&self, owner: &Preparation) -> Result<()> {
        check_commit(&self.db, &owner.operation, owner.revision, Some(owner))
    }

    pub(crate) fn require_owned_reservation(&self, owner: &Preparation) -> Result<()> {
        self.require_preparation_owner(owner)?;
        let touched: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) OR EXISTS(SELECT 1 FROM budget_entries WHERE id=?1)",
            [owner.operation()], |r| r.get(0),
        )?;
        ensure!(
            !touched,
            "preparation has durable effects; recovery required"
        );
        Ok(())
    }

    /// Only the serialized Treasury owner calls this after go_offline + dropping
    /// its poisoned SDK, without saving it. No result/absence inference at callers.
    /// Returns false if an accepted atomic commit already retained the capability.
    pub(crate) fn close_retired_preparation(&mut self, owner: &Preparation) -> Result<bool> {
        self.require_preparation_owner(owner)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check_commit(&tx, &owner.operation, owner.revision, Some(owner))?;
        let outgoing: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1)",
            [&owner.operation],
            |r| r.get(0),
        )?;
        if outgoing {
            return Ok(false);
        }
        let current: i64 = tx.query_row("SELECT MAX(revision) FROM snapshots", [], |r| r.get(0))?;
        ensure!(
            current == owner.revision,
            "preparation snapshot changed; cleanup unresolved"
        );
        let contradictory: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM treasury_operations WHERE id=?1) OR EXISTS(SELECT 1 FROM budget_entries WHERE id=?1 AND consumed!=0) OR EXISTS(SELECT 1 FROM expired_operations WHERE id=?1)", [&owner.operation], |r| r.get(0))?;
        ensure!(
            !contradictory,
            "preparation has unexplained durable effects"
        );
        if let Some(job) = &owner.job {
            let matches: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM funding_progress WHERE job_id=?1 AND operation_id=?2 AND phase='\"PREPARING\"')",params![job,owner.operation],|r|r.get(0))?;
            ensure!(matches, "funding preparation owner changed");
            super::funding::archive_unprepared(&tx, job, &owner.operation)?;
        }
        tx.execute(
            "DELETE FROM budget_entries WHERE id=?1 AND consumed=0",
            [&owner.operation],
        )?;
        // Needed for unmanaged operations too: no funding phase exists to fence
        // a delayed calculate result from publishing after this closure.
        let changed = tx.execute("UPDATE treasury_preparations SET closed=1 WHERE operation_id=?1 AND token=?2 AND closed=0",params![owner.operation,owner.token])?;
        ensure!(changed == 1, "preparation closure changed");
        tx.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
