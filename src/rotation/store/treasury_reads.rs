//! Scoped source-owner reads; operational sync never expands payment history.
use super::*;

pub(crate) struct TreasurySyncState {
    pub snapshot_revision: i64,
    pub sync: Option<RuntimeSyncObservation>,
    pub outgoing_pending: bool,
}
impl Store {
    pub(crate) fn treasury_sync_state(&self) -> Result<TreasurySyncState> {
        let snapshot_revision = self.db.query_row(
            "SELECT revision FROM snapshots ORDER BY revision DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        let sync = self
            .db
            .query_row(
                "SELECT observation FROM treasury_sync WHERE singleton=1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|s| serde_json::from_str(&s))
            .transpose()?;
        let outgoing_pending = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE state='PREPARED')",
            [],
            |r| r.get(0),
        )?;
        Ok(TreasurySyncState {
            snapshot_revision,
            sync: sync.map(|observation| self.runtime_sync_observation(observation)),
            outgoing_pending,
        })
    }
    pub(crate) fn source_recheck_boundary(&self) -> Result<i64> {
        Ok(self.db.query_row(
            "SELECT COALESCE(MAX(rowid),0) FROM treasury_operations",
            [],
            |r| r.get(0),
        )?)
    }
    /// The source actor owns mutations throughout this pass. A captured insertion
    /// boundary also prevents later rows from extending a scan indefinitely.
    pub(crate) fn source_recheck_page(
        &self,
        after: &str,
        boundary: i64,
    ) -> Result<Vec<OperationStatus>> {
        let mut query = self.db.prepare("SELECT o.id,o.facts,CASE WHEN e.id IS NOT NULL THEN 'EXPIRED' ELSE o.submission END,o.attempts FROM treasury_operations o LEFT JOIN expired_operations e ON e.id=o.id WHERE o.id>?1 AND o.rowid<=?2 AND (o.submission='CONFIRMED' OR e.id IS NOT NULL) ORDER BY o.id LIMIT 64")?;
        query
            .query_map(params![after, boundary], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .map(|row| {
                let (operation_id, facts, submission, attempts) = row?;
                Ok(OperationStatus {
                    operation_id,
                    facts: serde_json::from_str(&facts)?,
                    submission,
                    attempts,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_reads_preserve_expiry_precedence_and_captured_pages_without_pool_materialization() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let facts = serde_json::to_string(&TransactionFacts {
            txid: "fixture".into(),
            expiry_height: 10,
            amount_zatoshis: 1,
            fee_zatoshis: 1,
            deadline: Some(100),
        })
        .unwrap();
        let insert = |id: &str, submission: &str, facts: &str| {
            store
                .db
                .execute(
                    "INSERT INTO budget_entries VALUES (?1,NULL,0,0,1,0,0)",
                    [id],
                )
                .unwrap();
            store
                .db
                .execute("INSERT INTO outgoing VALUES (?1,'RESOLVED',X'00',1)", [id])
                .unwrap();
            store
                .db
                .execute(
                    "INSERT INTO treasury_operations VALUES (?1,?2,?3,0)",
                    params![id, facts, submission],
                )
                .unwrap();
        };
        for n in 0..130 {
            insert(&format!("op-{n:03}"), "CONFIRMED", &facts);
        }
        store
            .db
            .execute(
                "INSERT INTO expired_operations(id,height,revision) VALUES ('op-000',20,1)",
                [],
            )
            .unwrap();
        // Unrelated unfinished source data is not decoded by the historical pass.
        insert("unrelated", "PREPARED", "not JSON");
        let boundary = store.source_recheck_boundary().unwrap();
        insert("op-999", "CONFIRMED", &facts);
        let mut cursor = String::new();
        let mut all = Vec::new();
        loop {
            let page = store.source_recheck_page(&cursor, boundary).unwrap();
            assert!(page.len() <= 64);
            let Some(last) = page.last() else { break };
            cursor = last.operation_id.clone();
            all.extend(page);
        }
        assert_eq!(all.len(), 130);
        assert_eq!(all[0].submission, "EXPIRED");
        assert!(all[1..].iter().all(|op| op.submission == "CONFIRMED"));
        assert!(store.treasury_sync_state().unwrap().sync.is_none());
        assert!(
            store.status().is_err(),
            "full reporting still validates unrelated corrupt facts"
        );
    }
}
