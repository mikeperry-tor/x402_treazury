//! Retained ownership after concrete loss of canonical source support.
use super::*;

#[cfg(test)]
#[path = "source_obligations_tests.rs"]
mod tests;

pub(super) fn retain(db: &Connection, operation: &str) -> Result<()> {
    db.execute(
        "INSERT INTO source_obligations(operation_id) VALUES(?1) ON CONFLICT(operation_id) DO UPDATE SET clear_pass_id=NULL",
        [operation],
    )?;
    Ok(())
}

pub(super) fn disputed(db: &Connection, operation: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM active_source_obligations WHERE operation_id=?1)",
        [operation],
        |row| row.get(0),
    )?)
}

/// Stage a possible release without changing the active ownership guard.
pub(super) fn stage_clear(db: &Connection, operation: &str, pass: &str) -> Result<()> {
    db.execute("UPDATE source_obligations SET clear_pass_id=?2 WHERE operation_id=?1 AND NOT EXISTS(SELECT 1 FROM expired_operations WHERE id=?1)", params![operation,pass])?;
    Ok(())
}

/// One header publication activates all exact staged releases with the snapshot.
pub(super) fn activate(db: &Connection, pass: &str) -> Result<()> {
    db.execute(
        "UPDATE source_support_passes SET activated=1 WHERE id=?1",
        [pass],
    )?;
    Ok(())
}

impl Store {
    pub(crate) fn source_inputs_disputed(&self, operation: &str) -> Result<bool> {
        disputed(&self.db, operation)
    }

    pub(crate) fn source_expiry_recovery_needed(&self, operation: &str) -> Result<bool> {
        if !disputed(&self.db, operation)? {
            return Ok(false);
        }
        let facts = self.operation(operation)?.facts;
        let sync = self.funding_sync_observation()?;
        Ok(sync.height.is_some_and(|height| {
            crate::rotation::transaction::expiry_buried(
                facts.expiry_height,
                height,
                sync.confirmations,
            )
        }))
    }

    pub(crate) fn require_submission_inputs_available(
        &self,
        operation: &str,
        facts: &TransactionFacts,
    ) -> Result<()> {
        if !self.validated_internal_shielding(operation, facts)? {
            self.require_source_inputs_available()?;
        }
        Ok(())
    }

    /// Generic sends select shielded inputs through the SDK. Until that selector
    /// can exclude the exact orphaned inputs, preserve their original owner.
    /// Transparent shielding uses its separate immutable outpoint claims.
    pub(crate) fn require_source_inputs_available(&self) -> Result<()> {
        let disputed: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM active_source_obligations o WHERE NOT EXISTS(SELECT 1 FROM shielding_operations s WHERE s.operation_id=o.operation_id))",
            [],
            |row| row.get(0),
        )?;
        ensure!(!disputed, "treasury_source_inputs_disputed");
        Ok(())
    }
}
