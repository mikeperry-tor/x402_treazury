//! Preserve existing source evidence and own only unfinished accepted work.
use super::*;

pub(crate) fn initialize(db: &Connection) -> Result<()> {
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(matches!(version, 21..=23), "unsupported state schema");
    let tx = db.unchecked_transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS source_disposition_history(id TEXT NOT NULL REFERENCES outgoing(id), submission TEXT NOT NULL, height INTEGER, revision INTEGER REFERENCES snapshots(revision), PRIMARY KEY(id,submission,revision)); DROP INDEX IF EXISTS one_outgoing; DROP INDEX IF EXISTS one_calculation; PRAGMA user_version=23;")?;
    tx.execute_batch("CREATE INDEX IF NOT EXISTS funding_progress_unsettled_requests ON funding_progress(job_id,operation_id) WHERE quote IS NOT NULL AND NOT(typeof(provider_success)='integer' AND provider_success!=0) AND NOT(typeof(provider_failed)='integer' AND provider_failed!=0) AND NOT(typeof(provider_refunded)='integer' AND provider_refunded!=0);CREATE INDEX IF NOT EXISTS funding_recovery_unsettled_requests ON funding_recovery(job_id,operation_id) WHERE quote IS NOT NULL AND NOT(typeof(provider_success)='integer' AND provider_success!=0) AND NOT(typeof(provider_failed)='integer' AND provider_failed!=0) AND NOT(typeof(provider_refunded)='integer' AND provider_refunded!=0);")?;
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS outgoing_pending ON outgoing(id) WHERE state='PREPARED';",
    )?;
    tx.commit()?;
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
            b"original signed bytes",
            Some(TransactionFacts {
                txid: "fixture".into(),
                expiry_height: 20,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: Some(10000),
            }),
        )?;
        Ok(())
    }
    #[test]
    fn legacy_history_blocks_automatic_disclosure_without_reopening_terminal_work() -> Result<()> {
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
        // Older branch versions could restore multiple accepted originals. They
        // remain owned, but must not gain first-disclosure authority on reopen.
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
        assert!(!s.automatic_shielding_submission("legacy", 100)?);
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
    fn owned_legacy_upgrade_excludes_old_readers_without_rewriting_financial_records() -> Result<()>
    {
        for old_version in [21, 22] {
            let dir = tempfile::tempdir()?;
            let state = dir.path().join("state");
            let key = dir.path().join("key");
            let mut store = Store::create(&state, &key, 1, b"initial")?;
            prepare(&mut store, "original")?;
            // A retained legacy intent and pending result are independently
            // authenticated evidence; the upgrade must preserve both verbatim.
            store.db.execute(
                "UPDATE outgoing SET state='RESOLVED' WHERE id='original'",
                [],
            )?;
            store.accept_preparation("retained-intent", None, 1, None)?;
            store.db.execute(
                "UPDATE outgoing SET state='PREPARED' WHERE id='original'",
                [],
            )?;
            let intent: Vec<u8> = store.db.query_row(
                "SELECT preparation FROM operation_network WHERE operation_id='retained-intent'",
                [],
                |r| r.get(0),
            )?;
            let identity = store.id().to_owned();
            let outgoing: Vec<u8> =
                store
                    .db
                    .query_row("SELECT raw FROM outgoing WHERE id='original'", [], |r| {
                        r.get(0)
                    })?;
            let snapshots: Vec<(i64, Vec<u8>)> = store
                .db
                .prepare("SELECT revision,bytes FROM snapshots ORDER BY revision")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            if old_version == 21 {
                store
                    .db
                    .execute_batch("DROP TABLE source_disposition_history;")?;
            }
            store.db.execute_batch(&format!("DROP INDEX outgoing_pending; DROP INDEX funding_progress_unsettled_requests; DROP INDEX funding_recovery_unsettled_requests; CREATE UNIQUE INDEX one_outgoing ON outgoing((1)) WHERE state='PREPARED'; CREATE UNIQUE INDEX one_calculation ON operation_network((1)) WHERE preparation IS NOT NULL; PRAGMA user_version={old_version};"))?;
            drop(store);
            // Read-only discovery/status never perform the owned upgrade.
            assert_eq!(super::super::treasury_id(&state)?, identity);
            assert_eq!(
                super::super::status(&state)?.state_schema_version,
                old_version
            );
            let readonly = Connection::open_with_flags(
                state.join("state.sqlite"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            for name in ["one_outgoing", "one_calculation"] {
                assert!(
                    readonly.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
                        [name],
                        |r| r.get::<_, bool>(0)
                    )?,
                    "readonly inspection must preserve {name}"
                );
            }
            for index in [
                "funding_progress_unsettled_requests",
                "funding_recovery_unsettled_requests",
                "outgoing_pending",
            ] {
                assert!(!readonly.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
                    [index],
                    |r| r.get::<_, bool>(0)
                )?);
            }
            drop(readonly);
            let store = Store::open(&state, &key, &identity)?;
            assert_eq!(
                store
                    .db
                    .pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))?,
                23
            );
            for name in ["one_outgoing", "one_calculation"] {
                assert!(
                    !store.db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
                        [name],
                        |r| r.get::<_, bool>(0)
                    )?,
                    "owned upgrade must remove redundant {name}"
                );
            }
            assert_eq!(
                store
                    .db
                    .query_row("SELECT raw FROM outgoing WHERE id='original'", [], |r| r
                        .get::<_, Vec<u8>>(0))?,
                outgoing
            );
            let after: Vec<(i64, Vec<u8>)> = store
                .db
                .prepare("SELECT revision,bytes FROM snapshots ORDER BY revision")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            assert_eq!(after, snapshots);
            assert_eq!(
            store.db.query_row(
                "SELECT preparation FROM operation_network WHERE operation_id='retained-intent'",
                [],
                |r| r.get::<_, Vec<u8>>(0)
            )?,
            intent
        );
            assert_eq!(
                store.preparation_intent("retained-intent")?.cost_zatoshis,
                1
            );
            assert_eq!(store.operation("original")?.attempts, 0);
            assert!(store.operation_pending("original")?);
            // The prior schema22 reader's preflight rejects this capability boundary
            // before opening a writable database or touching source evidence.
            let old_reader_version: u32 =
                store
                    .db
                    .pragma_query_value(None, "user_version", |r| r.get(0))?;
            assert!(!matches!(old_reader_version, 21 | 22));
        }
        Ok(())
    }
}
