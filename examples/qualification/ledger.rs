use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub struct Ledger {
    db: Connection,
    _lock: File,
}
fn regular(path: &Path) -> Result<()> {
    if path.exists() || path.is_symlink() {
        let m = std::fs::symlink_metadata(path)?;
        ensure!(
            m.is_file() && !m.file_type().is_symlink(),
            "ledger path must be a regular file"
        );
        #[cfg(unix)]
        ensure!(
            m.nlink() == 1 && m.mode() & 0o077 == 0,
            "ledger file must be private and singly linked"
        );
    }
    Ok(())
}
pub fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    Ok(options.open(path)?)
}
impl Ledger {
    pub fn create(dir: &Path, prepared: &Value, budget: u64) -> Result<Self> {
        ensure!(budget <= i64::MAX as u64, "budget overflow");
        std::fs::create_dir(dir)
            .context("run already exists; use execute/report with its pinned manifest")?;
        #[cfg(unix)]
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let mut lock = private_file(&dir.join("owner.lock"))?;
        use std::io::Write;
        lock.write_all(b"qualification ledger\n")?;
        lock.sync_all()?;
        let _db = private_file(&dir.join("run.sqlite"))?;
        let mut ledger = Self::open(dir)?;
        let tx = ledger.db.transaction()?;
        tx.execute_batch("CREATE TABLE run (id INTEGER PRIMARY KEY CHECK(id=1), prepared TEXT NOT NULL, budget INTEGER NOT NULL); CREATE TABLE attempts (id TEXT PRIMARY KEY, reservation INTEGER NOT NULL CHECK(reservation>=0), outcome TEXT NOT NULL, detail TEXT NOT NULL); CREATE TABLE events (sequence INTEGER PRIMARY KEY, at INTEGER NOT NULL DEFAULT(unixepoch()), kind TEXT NOT NULL, detail TEXT NOT NULL);")?;
        tx.execute(
            "INSERT INTO run VALUES(1,?1,?2)",
            params![prepared.to_string(), budget as i64],
        )?;
        tx.commit()?;
        File::open(dir)?.sync_all()?;
        File::open(dir.parent().context("missing ledger parent")?)?.sync_all()?;
        Ok(ledger)
    }
    pub fn open(dir: &Path) -> Result<Self> {
        let m = std::fs::symlink_metadata(dir)?;
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "ledger directory must be ordinary"
        );
        #[cfg(unix)]
        ensure!(m.mode() & 0o077 == 0, "ledger directory must be owner-only");
        for name in [
            "owner.lock",
            "run.sqlite",
            "run.sqlite-journal",
            "run.sqlite-wal",
            "run.sqlite-shm",
        ] {
            regular(&dir.join(name))?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.join("owner.lock"))?;
        lock.try_lock_exclusive()
            .context("another driver owns this run")?;
        let db = Connection::open_with_flags(
            dir.join("run.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
        Ok(Self { db, _lock: lock })
    }
    pub fn prepared(&self) -> Result<Value> {
        Ok(serde_json::from_str(&self.db.query_row::<String, _, _>(
            "SELECT prepared FROM run WHERE id=1",
            [],
            |r| r.get(0),
        )?)?)
    }
    pub fn reserve(&mut self, id: &str, amount: u64) -> Result<()> {
        ensure!(amount <= i64::MAX as u64, "reservation overflow");
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (budget, used): (i64, i64) = tx.query_row(
            "SELECT budget,(SELECT coalesce(sum(reservation),0) FROM attempts) FROM run WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            amount <= u64::try_from(budget.checked_sub(used).context("budget overflow")?)?,
            "experiment API budget exhausted; nothing sent"
        );
        tx.execute(
            "INSERT INTO attempts VALUES(?1,?2,'possibly_executed','{}')",
            params![id, amount as i64],
        )
        .context("case already reserved; never replay it, even after failure or restart")?;
        tx.execute(
            "INSERT INTO events(kind,detail) VALUES('reserve',?1)",
            [json!({"case":id,"atomic":amount}).to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish(&mut self, id: &str, outcome: &str, detail: &Value) -> Result<()> {
        let tx = self.db.transaction()?;
        ensure!(tx.execute("UPDATE attempts SET outcome=?2,detail=?3 WHERE id=?1 AND outcome='possibly_executed'",params![id,outcome,detail.to_string()])?==1,"missing or finalized attempt");
        tx.execute(
            "INSERT INTO events(kind,detail) VALUES('result',?1)",
            [json!({"case":id,"outcome":outcome,"detail":detail}).to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn event(&self, kind: &str, detail: &Value) -> Result<()> {
        self.db.execute(
            "INSERT INTO events(kind,detail) VALUES(?1,?2)",
            params![kind, detail.to_string()],
        )?;
        Ok(())
    }
    pub fn report(&self) -> Result<Value> {
        let mut q = self
            .db
            .prepare("SELECT id,reservation,outcome,detail FROM attempts ORDER BY rowid")?;
        let attempts=q.query_map([],|r|Ok(json!({"case":r.get::<_,String>(0)?,"reserved_atomic":r.get::<_,i64>(1)?,"outcome":r.get::<_,String>(2)?,"detail":r.get::<_,String>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let budget: i64 = self
            .db
            .query_row("SELECT budget FROM run WHERE id=1", [], |r| r.get(0))?;
        Ok(
            json!({"budget_atomic":budget,"reserved_atomic":attempts.iter().map(|a|a["reserved_atomic"].as_i64().unwrap()).sum::<i64>(),"attempts":attempts,"note":"Reservations are conservative lifetime bounds, not confirmed spending; none are automatically released"}),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crash_resume_failure_and_changed_folder_cannot_replay_or_release_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("run");
        {
            let mut l = Ledger::create(&p, &json!({"pinned":true}), 100).unwrap();
            l.reserve("a", 60).unwrap();
            assert!(Ledger::open(&p).is_err());
        }
        let mut l = Ledger::open(&p).unwrap();
        assert_eq!(l.prepared().unwrap()["pinned"], true);
        assert!(l.reserve("a", 1).is_err());
        assert!(l.reserve("b", 41).is_err());
        l.finish("a", "tool_error", &json!({})).unwrap();
        assert!(l.reserve("b", 41).is_err());
        l.reserve("b", 40).unwrap();
        assert_eq!(l.report().unwrap()["reserved_atomic"], 100);
        assert!(Ledger::create(&p, &json!({}), 1000).is_err());
    }
    #[test]
    fn overflow_and_links_fail_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("run");
        let mut l = Ledger::create(&p, &json!({}), 1).unwrap();
        assert!(l.reserve("huge", u64::MAX).is_err());
        drop(l);
        std::fs::hard_link(p.join("run.sqlite"), tmp.path().join("alias")).unwrap();
        assert!(Ledger::open(&p).is_err());
    }
}
