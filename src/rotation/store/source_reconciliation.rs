//! Preserve existing source evidence and own only unfinished accepted work.
use super::*;

pub(crate) fn initialize(db: &Connection) -> Result<()> {
    // A current store can be interrupted after its schema commit and before
    // runtime history initialization. Complete that additive initialization only.
    db.execute_batch("CREATE TABLE IF NOT EXISTS source_disposition_history(id TEXT NOT NULL REFERENCES outgoing(id), submission TEXT NOT NULL, height INTEGER, revision INTEGER REFERENCES snapshots(revision), PRIMARY KEY(id,submission,revision));")?;
    Ok(())
}

impl Store {
    pub(crate) fn source_previously_closed(&self, id: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_disposition_history WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn unowned_pending_sources(&self, after: Option<&str>) -> Result<Vec<String>> {
        let mut query = self.db.prepare("SELECT x.id FROM outgoing x WHERE x.state='PREPARED' AND NOT EXISTS(SELECT 1 FROM funding_progress j WHERE j.operation_id=x.id) AND (?1 IS NULL OR x.id>?1) ORDER BY x.id LIMIT ?2")?;
        Ok(query
            .query_map(params![after, scheduling::PAGE_SIZE as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Preserve an earlier expiry proof before recording canonical confirmation.
pub(super) fn archive_expiry(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<()> {
    tx.execute("INSERT INTO source_disposition_history(id,submission,height,revision) SELECT e.id,'EXPIRED',e.height,e.revision FROM expired_operations e WHERE e.id=?1 AND NOT EXISTS(SELECT 1 FROM source_disposition_history h WHERE h.id=e.id AND h.submission='EXPIRED' AND h.revision=e.revision)", [id])?;
    tx.execute("DELETE FROM expired_operations WHERE id=?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepare(store: &mut Store, id: &str) -> Result<()> {
        store.accept_preparation(id, None, 60, None)?;
        store.bind_operation_recipient(id, &format!("0x{:040x}", 1))?;
        store.prepare_with_facts(
            id,
            store.snapshot()?.0,
            b"original SDK",
            crate::rotation::store::test_support::transaction_raw(20).as_slice(),
            Some(TransactionFacts {
                txid: crate::rotation::store::test_support::transaction_txid(20),
                expiry_height: 20,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: Some(10000),
            }),
        )?;
        Ok(())
    }
    #[test]
    fn current_history_blocks_automatic_disclosure_without_reopening_terminal_work() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut s = Store::create(&state, &key, 1, b"initial")?;
        assert!(!s.db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='one_calculation')", [], |r|r.get::<_,bool>(0))?);
        prepare(&mut s, "terminal")?;
        s.confirm_spend("terminal")?;
        prepare(&mut s, "legacy")?;
        let revision = s.snapshot()?.0;
        s.db.execute(
            "INSERT INTO source_disposition_history VALUES ('legacy','EXPIRED',22,?1)",
            [revision],
        )?;
        prepare(&mut s, "sibling").unwrap_err();
        // Restored accepted originals remain owned, but must not gain
        // first-disclosure authority on reopen.
        s.db.execute("UPDATE outgoing SET state='RESOLVED' WHERE id='legacy'", [])?;
        prepare(&mut s, "sibling")?;
        s.db.execute("UPDATE outgoing SET state='PREPARED' WHERE id='legacy'", [])?;
        let identity = s.id().to_owned();
        drop(s);
        let mut s = Store::open(&state, &key, &identity)?;
        assert_eq!(s.operation("terminal")?.submission, "CONFIRMED");
        assert!(!s.operation_pending("terminal")?);
        assert_eq!(
            s.unowned_pending_sources(None)?,
            vec!["legacy".to_owned(), "sibling".to_owned()]
        );
        let indexes: Vec<(String, bool)> =
            s.db.prepare("PRAGMA index_list('outgoing')")?
                .query_map([], |r| Ok((r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
        assert!(
            indexes
                .iter()
                .any(|(name, unique)| name == "outgoing_pending" && !unique)
        );
        let indexed:Vec<String>=s.db.prepare("SELECT id FROM outgoing INDEXED BY outgoing_pending WHERE state='PREPARED' ORDER BY id")?.query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        assert_eq!(indexed, vec!["legacy".to_owned(), "sibling".to_owned()]);
        assert!(s.source_previously_closed("legacy")?);
        assert!(s.request_broadcast("legacy", 1, 1, false).is_err());
        assert!(s.require_preparation_available().is_err());
        for _ in 0..3 {
            let revision = s.snapshot()?.0;
            s.save_snapshot(revision, b"new SDK")?;
        }
        let retained: bool = s.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM snapshots WHERE revision=?1)",
            [revision],
            |r| r.get(0),
        )?;
        assert!(
            retained,
            "historical expiry evidence keeps its immutable snapshot"
        );
        Ok(())
    }
    #[test]
    fn interrupted_current_history_initialization_preserves_accepted_work() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"initial")?;
        prepare(&mut store, "original")?;
        let identity = store.id().to_owned();
        let raw: Vec<u8> =
            store
                .db
                .query_row("SELECT raw FROM outgoing WHERE id='original'", [], |r| {
                    r.get(0)
                })?;
        let snapshot = store.snapshot()?;
        // Old current-schema creation committed before these idempotent steps.
        store.db.execute_batch(
            "DROP TABLE source_disposition_history; DROP TABLE payment_resolution_history;",
        )?;
        drop(store);
        for _ in 0..2 {
            let store = Store::open(&state, &key, &identity)?;
            assert_eq!(store.snapshot()?, snapshot);
            assert_eq!(
                store
                    .db
                    .query_row("SELECT raw FROM outgoing WHERE id='original'", [], |r| r
                        .get::<_, Vec<u8>>(0))?,
                raw
            );
            assert!(store.operation_pending("original")?);
            assert_eq!(store.operation("original")?.attempts, 0);
            for name in [
                "source_disposition_history",
                "payment_resolution_history",
                "resolution_history_attempt",
            ] {
                assert!(store.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
                    [name],
                    |r| r.get::<_, bool>(0)
                )?);
            }
            assert_eq!(store.status()?.state_schema_version, 23);
        }
        // The canonical schema itself includes persistent histories and indexes.
        let db = Connection::open_in_memory()?;
        db.execute_batch(super::super::SCHEMA)?;
        for table in ["source_disposition_history", "payment_resolution_history"] {
            assert_eq!(
                db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
        }
        Ok(())
    }
}
