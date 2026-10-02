//! Single serialized registry snapshot; no wallet material or network activity.
use super::State;
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use rusqlite::Connection;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};
pub struct Store {
    db: Connection,
    _lock: File,
}
fn normalized(path: &Path) -> Result<PathBuf> {
    if path.is_relative() {
        return normalized(&std::env::current_dir()?.join(path));
    }
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path.parent().context("registry parent missing")?;
    if path.file_name().is_none() {
        anyhow::bail!("invalid registry path")
    }
    Ok(normalized(parent)?.join(path.file_name().unwrap()))
}
pub fn protect(path: &Path, protected: &[PathBuf]) -> Result<()> {
    for suffix in ["", ".owner.lock", "-journal", "-wal", "-shm"] {
        protect_one(
            &PathBuf::from(format!("{}{suffix}", path.display())),
            protected,
        )?;
    }
    Ok(())
}
fn protect_one(path: &Path, protected: &[PathBuf]) -> Result<()> {
    let path = normalized(path)?;
    for p in protected {
        let protected = normalized(p)?;
        ensure!(
            !path.starts_with(&protected),
            "registry aliases a protected config/treasury path"
        );
        #[cfg(unix)]
        if path.exists() && protected.exists() {
            use std::os::unix::fs::MetadataExt;
            let a = std::fs::metadata(&path)?;
            let b = std::fs::metadata(&protected)?;
            ensure!(
                a.dev() != b.dev() || a.ino() != b.ino(),
                "registry aliases a protected file"
            );
        }
    }
    Ok(())
}
impl Store {
    pub fn open(path: &Path, protected: &[PathBuf]) -> Result<Self> {
        let path = normalized(path)?;
        protect(&path, protected)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        for suffix in [".owner.lock", "-journal", "-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
            if let Ok(meta) = std::fs::symlink_metadata(sidecar) {
                ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "unsafe registry sidecar"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    ensure!(meta.nlink() == 1, "hard-linked registry sidecar");
                }
            }
        }
        let file = options.open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let meta = file.metadata()?;
            ensure!(
                meta.is_file() && meta.nlink() == 1 && meta.permissions().mode() & 0o077 == 0,
                "registry must be a private file without hard links"
            );
        }
        drop(file);
        let lock = options.open(format!("{}.owner.lock", path.display()))?;
        lock.try_lock_exclusive()
            .context("source registry already owned")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                lock.metadata()?.permissions().mode() & 0o077 == 0,
                "registry must have owner-only permissions"
            );
        }
        let db = Connection::open(&path)?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version <= 1, "unsupported source registry version");
        if version == 0 {
            let count: u32 = db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            ensure!(count == 0, "unrecognized source registry");
            db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE snapshot(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL); PRAGMA user_version=1; COMMIT;")?;
        }
        Ok(Self { db, _lock: lock })
    }
    #[cfg(test)]
    pub fn reject_writes(&self) {
        self.db.execute_batch("PRAGMA query_only=ON").unwrap();
    }
    pub fn load(&self) -> Result<State> {
        use rusqlite::OptionalExtension;
        let data: Option<String> = self
            .db
            .query_row("SELECT json FROM snapshot WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let state: State = data
            .map(|s| {
                ensure!(s.len() <= 256 * 1024 * 1024, "registry size limit");
                serde_json::from_str(&s).context("corrupt source registry")
            })
            .unwrap_or_else(|| Ok(State::default()))?;
        ensure!(
            state.records.len() <= 10000 && state.receipts.len() <= 10000,
            "registry record limit"
        );
        for (id, r) in &state.records {
            ensure!(
                id == &r.id && uuid::Uuid::parse_str(id).is_ok() && r.revision > 0,
                "corrupt source identity"
            );
            ensure!(
                r.removed || super::hash(r.document.as_bytes()) == r.hash,
                "corrupt source document hash"
            );
        }
        Ok(state)
    }
    pub fn save(&mut self, state: &State) -> Result<()> {
        let bytes = serde_json::to_string(state)?;
        ensure!(bytes.len() <= 256 * 1024 * 1024, "registry size limit");
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO snapshot(id,json) VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[bytes])?;
        tx.commit()?;
        Ok(())
    }
}
pub fn inspect(path: &Path) -> Result<serde_json::Value> {
    ensure!(path.is_file(), "source registry does not exist");
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(version == 1, "unsupported source registry version");
    use rusqlite::OptionalExtension;
    let data: Option<String> = db
        .query_row("SELECT json FROM snapshot WHERE id=1", [], |r| r.get(0))
        .optional()?;
    let state: State = data
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    Ok(
        serde_json::json!({"generation":state.generation,"sources":state.records.values().map(|r|serde_json::json!({"id":r.id,"name":r.candidate.name,"owner":r.owner,"revision":r.revision,"targets":r.targets,"disabled":r.disabled,"removed":r.removed,"spec_hash":r.hash})).collect::<Vec<_>>()}),
    )
}
