//! Bounded canonical source observations, separate from immutable debit accounting.
use super::*;

#[derive(Clone)]
pub(crate) struct Anchor {
    pub(crate) height: u64,
    pub(crate) hash: [u8; 32],
}
#[derive(Clone)]
pub(crate) struct Pass {
    id: String,
    namespace: Uuid,
    base_revision: i64,
    publication_revision: i64,
    pub(crate) anchor: Anchor,
    inventory_revision: i64,
    pub(crate) boundary: i64,
}
#[derive(Clone, Copy)]
pub(crate) enum Disposition {
    Confirmed(u64),
    Unavailable,
    Conflict,
    Expired,
}
impl Disposition {
    fn columns(self) -> Result<(&'static str, Option<i64>)> {
        Ok(match self {
            Self::Confirmed(height) => {
                ensure!(height > 0, "source confirmation height missing");
                ("CONFIRMED", Some(i64::try_from(height)?))
            }
            Self::Unavailable => ("UNAVAILABLE", None),
            Self::Conflict => ("CONFLICT", None),
            Self::Expired => ("EXPIRED", None),
        })
    }
}
pub(crate) enum CleanupCursor {
    Markers(String),
    Passes(String),
}
impl Default for CleanupCursor {
    fn default() -> Self {
        Self::Markers(String::new())
    }
}

impl Store {
    pub(crate) fn revoke_return_support(&self, operation: &str) -> Result<()> {
        self.db.execute("UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=?1",[operation])?;
        Ok(())
    }
    /// Reclaim a physical page, including ineligible rows, before yielding.
    pub(crate) fn cleanup_source_support_page(
        &mut self,
        cursor: CleanupCursor,
    ) -> Result<Option<CleanupCursor>> {
        match cursor {
            CleanupCursor::Markers(after) => {
                let rows = self.db.prepare("SELECT o.operation_id,COALESCE(p.activated,0) FROM source_obligations o LEFT JOIN source_support_passes p ON p.id=o.clear_pass_id WHERE o.operation_id>?1 ORDER BY o.operation_id LIMIT 64")?
                    .query_map([after], |row| Ok((row.get::<_,String>(0)?,row.get::<_,bool>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let Some((last, _)) = rows.last() else {
                    return Ok(Some(CleanupCursor::Passes(String::new())));
                };
                let next = CleanupCursor::Markers(last.clone());
                let tx = self.db.transaction()?;
                for (operation, resolved) in rows {
                    if resolved {
                        tx.execute(
                            "DELETE FROM source_obligations WHERE operation_id=?1",
                            [operation],
                        )?;
                    }
                }
                tx.commit()?;
                Ok(Some(next))
            }
            CleanupCursor::Passes(after) => {
                let id: Option<String> = self.db.query_row(
                    "SELECT id FROM source_support_passes WHERE id>?1 AND id IS NOT (SELECT pass_id FROM source_support_current WHERE singleton=1) ORDER BY id LIMIT 1",
                    [&after], |row| row.get(0)).optional()?;
                let Some(id) = id else { return Ok(None) };
                let retained: bool = self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM source_obligations WHERE clear_pass_id=?1)",
                    [&id],
                    |row| row.get(0),
                )?;
                let tx = self.db.transaction()?;
                // Referenced headers retain staged-clear identity only. Removed
                // inventory can never be continued or activated as a full pass.
                tx.execute(
                    "UPDATE source_support_passes SET complete=0 WHERE id=?1",
                    [&id],
                )?;
                let changed = tx.execute("DELETE FROM source_support_rows WHERE pass_id=?1 AND operation_id IN (SELECT operation_id FROM source_support_rows WHERE pass_id=?1 ORDER BY operation_id LIMIT 64)",[&id])?;
                if changed == 0 && !retained {
                    tx.execute("DELETE FROM source_support_passes WHERE id=?1", [&id])?;
                }
                tx.commit()?;
                Ok(Some(CleanupCursor::Passes(if changed == 0 {
                    id
                } else {
                    after
                })))
            }
        }
    }
    pub(crate) fn begin_source_support(&mut self, revision: i64, anchor: Anchor) -> Result<Pass> {
        ensure!(anchor.height > 0, "source anchor height missing");
        let current: i64 = self
            .db
            .query_row("SELECT MAX(revision) FROM snapshots", [], |r| r.get(0))?;
        ensure!(revision == current, "source snapshot changed");
        let pass = Pass {
            id: Uuid::new_v4().to_string(),
            namespace: self.evidence_namespace,
            base_revision: revision,
            publication_revision: revision,
            anchor,
            inventory_revision: self.db.query_row(
                "SELECT inventory_revision FROM source_support_current WHERE singleton=1",
                [],
                |r| r.get(0),
            )?,
            boundary: self.source_recheck_boundary()?,
        };
        self.db.execute("INSERT INTO source_support_passes(id,base_revision,inventory_revision,boundary,anchor_height,anchor_hash) VALUES(?1,?2,?3,?4,?5,?6)",params![pass.id,revision,pass.inventory_revision,pass.boundary,i64::try_from(pass.anchor.height)?,pass.anchor.hash.as_slice()])?;
        Ok(pass)
    }
    /// The treasury owner has rechecked the original historical block hash
    /// after catch-up. Only an unchanged sealed inventory can cross snapshots.
    pub(crate) fn continue_source_support(
        &self,
        pass: &Pass,
        revision: i64,
    ) -> Result<Option<Pass>> {
        ensure!(
            pass.namespace == self.evidence_namespace,
            "source pass owner changed"
        );
        let exact: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_support_passes p,source_support_current c WHERE p.id=?1 AND p.base_revision=?2 AND p.inventory_revision=?3 AND c.inventory_revision=?3 AND p.complete=1 AND p.anchor_height=?4 AND p.anchor_hash=?5 AND ?6=(SELECT MAX(revision) FROM snapshots))",
            params![pass.id,pass.base_revision,pass.inventory_revision,i64::try_from(pass.anchor.height)?,pass.anchor.hash.as_slice(),revision], |r| r.get(0))?;
        Ok(exact.then(|| {
            let mut continued = pass.clone();
            continued.publication_revision = revision;
            continued
        }))
    }
    fn check_source_pass(&self, pass: &Pass) -> Result<()> {
        ensure!(
            pass.namespace == self.evidence_namespace,
            "source pass owner changed"
        );
        let exact:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM source_support_passes p,source_support_current c WHERE p.id=?1 AND p.base_revision=?2 AND p.inventory_revision=?3 AND c.inventory_revision=?3)",params![pass.id,pass.base_revision,pass.inventory_revision],|r|r.get(0))?;
        ensure!(exact, "source inventory changed");
        Ok(())
    }
    /// Called after the treasury owner compares exact serialized transaction bytes
    /// against its synchronized wallet record. One operation per Store job bounds
    /// both raw transaction decoding and publication work.
    pub(crate) fn stage_source_support(
        &mut self,
        pass: &Pass,
        operation: &OperationStatus,
        raw: &[u8],
        disposition: Disposition,
        live_obligation: bool,
    ) -> Result<()> {
        self.check_source_pass(pass)?;
        let after: String = self.db.query_row(
            "SELECT after_id FROM source_support_passes WHERE id=?1 AND complete=0",
            [&pass.id],
            |r| r.get(0),
        )?;
        let expected = self
            .source_recheck_page(&after, pass.boundary)?
            .into_iter()
            .next()
            .context("source inventory exhausted")?;
        ensure!(
            expected.operation_id == operation.operation_id
                && expected.facts == operation.facts
                && expected.submission == operation.submission
                && expected.attempts == operation.attempts,
            "source operation changed"
        );
        ensure!(
            self.prepared_bytes(&operation.operation_id)?.as_slice() == raw,
            "source bytes changed"
        );
        let (facts,revision):(String,i64)=self.db.query_row("SELECT o.facts,x.revision FROM treasury_operations o JOIN outgoing x ON x.id=o.id WHERE o.id=?1",[&operation.operation_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let (kind, height) = disposition.columns()?;
        let tx = self.db.transaction()?;
        // A demonstrated contradiction revokes current support and retains
        // input ownership before independently useful sync work can complete.
        if !matches!(disposition, Disposition::Confirmed(_)) {
            revoke(&tx, &operation.operation_id)?;
            if live_obligation || matches!(disposition, Disposition::Conflict) {
                super::source_obligations::retain(&tx, &operation.operation_id)?;
            }
        } else {
            super::source_obligations::stage_clear(&tx, &operation.operation_id, &pass.id)?;
        }
        tx.execute("INSERT INTO source_support_rows(pass_id,operation_id,facts,outgoing_revision,disposition,confirmed_height) VALUES(?1,?2,?3,?4,?5,?6)",params![pass.id,operation.operation_id,facts,revision,kind,height])?;
        tx.execute(
            "UPDATE source_support_passes SET after_id=?2 WHERE id=?1",
            params![pass.id, operation.operation_id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn seal_source_support(&mut self, pass: &Pass) -> Result<()> {
        self.check_source_pass(pass)?;
        let after: String = self.db.query_row(
            "SELECT after_id FROM source_support_passes WHERE id=?1",
            [&pass.id],
            |r| r.get(0),
        )?;
        ensure!(
            self.source_recheck_page(&after, pass.boundary)?.is_empty(),
            "source inventory incomplete"
        );
        self.db.execute(
            "UPDATE source_support_passes SET complete=1 WHERE id=?1",
            [&pass.id],
        )?;
        Ok(())
    }
    pub(crate) fn stage_return_support(
        &mut self,
        pass: &Pass,
        operation: &str,
        returned: U256,
        issue: bool,
    ) -> Result<()> {
        self.check_source_pass(pass)?;
        let tx = self.db.transaction()?;
        if issue {
            tx.execute("UPDATE source_support_rows SET return_issue=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=?1",[operation])?;
        }
        ensure!(tx.execute("UPDATE source_support_rows SET returned_amount=?3,return_issue=?4 WHERE pass_id=?1 AND operation_id=?2 AND EXISTS(SELECT 1 FROM source_support_passes WHERE id=?1 AND complete=0)",params![pass.id,operation,returned.to_string(),issue])?==1,"return source row missing");
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn source_pass_return_inventory(&self, pass: &Pass, revision: i64) -> Result<()> {
        self.check_source_pass(pass)?;
        ensure!(
            pass.inventory_revision == revision,
            "return inventory changed"
        );
        Ok(())
    }
}
fn revoke(db: &Connection, operation: &str) -> Result<()> {
    db.execute("UPDATE source_support_rows SET revoked=1 WHERE pass_id=(SELECT pass_id FROM source_support_current WHERE singleton=1) AND operation_id=?1",[operation])?;
    Ok(())
}
/// Final pointer activation shares the encrypted snapshot's revision transaction.
pub(super) fn activate(
    db: &Connection,
    namespace: Uuid,
    pass: &Pass,
    expected: i64,
    revision: i64,
) -> Result<()> {
    ensure!(
        pass.namespace == namespace && pass.publication_revision == expected,
        "source snapshot binding changed"
    );
    let changed=db.execute("UPDATE source_support_current SET pass_id=?1,snapshot_revision=?2 WHERE singleton=1 AND inventory_revision=?3 AND EXISTS(SELECT 1 FROM source_support_passes WHERE id=?1 AND complete=1 AND base_revision=?4 AND inventory_revision=?3)",params![pass.id,revision,pass.inventory_revision,pass.base_revision])?;
    ensure!(changed == 1, "source support pass incomplete or changed");
    super::source_obligations::activate(db, &pass.id)?;
    Ok(())
}
impl Store {
    pub(crate) fn source_confirmation_supported(&self, operation: &str) -> Result<bool> {
        confirmed(&self.db, operation)
    }
    pub(crate) fn source_observation_current(&self, operation: &str) -> Result<bool> {
        current(&self.db, operation, false)
    }
}
/// No lookup of an older pass can restore support lost in a newer canonical scan.
pub(super) fn confirmed(db: &Connection, operation: &str) -> Result<bool> {
    current(db, operation, true)
}
pub(super) fn returned(db: &Connection, operation: &str) -> Result<Option<U256>> {
    if !confirmed(db, operation)? {
        return Ok(None);
    }
    let amount:Option<String>=db.query_row("SELECT r.returned_amount FROM source_support_rows r JOIN source_support_current c ON c.pass_id=r.pass_id WHERE r.operation_id=?1 AND r.return_issue=0",[operation],|row|row.get(0)).optional()?.flatten();
    amount
        .map(|value| U256::from_str_radix(&value, 10).context("invalid canonical return amount"))
        .transpose()
}
fn current(db: &Connection, operation: &str, confirmed_only: bool) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM source_support_current c JOIN treasury_sync t ON t.singleton=1 AND c.snapshot_revision=CAST(json_extract(t.observation,'$.snapshot_revision') AS INTEGER) JOIN source_support_rows r ON r.pass_id=c.pass_id JOIN treasury_operations o ON o.id=r.operation_id AND o.facts=r.facts JOIN outgoing x ON x.id=o.id AND x.revision=r.outgoing_revision WHERE r.operation_id=?1 AND (?2=0 OR r.disposition='CONFIRMED') AND r.revoked=0 AND (?2=0 OR (o.submission='CONFIRMED' AND NOT EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=o.id))))",params![operation,confirmed_only],|r|r.get(0))?)
}
#[cfg(test)]
pub(crate) mod tests;
