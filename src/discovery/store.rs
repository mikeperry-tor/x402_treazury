//! Single serialized registry snapshot; no wallet material or network activity.
use super::{Record, State};
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
                crate::limits::check(
                    s.len(),
                    256 * 1024 * 1024,
                    "registry size limit",
                    "hardcoded_registry_state_bytes",
                )?;
                serde_json::from_str(&s).context("corrupt source registry")
            })
            .unwrap_or_else(|| Ok(State::default()))?;
        crate::limits::check(
            state.records.len(),
            10000,
            "registry record limit",
            "hardcoded_registry_records",
        )?;
        crate::limits::check(
            state.receipts.len(),
            10000,
            "registry record limit",
            "hardcoded_mutation_receipts",
        )?;
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
        crate::limits::check(
            bytes.len(),
            256 * 1024 * 1024,
            "registry size limit",
            "hardcoded_registry_state_bytes",
        )?;
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
        serde_json::json!({"generation":state.generation,"sources":state.records.values().map(inspect_source).collect::<Vec<_>>()}),
    )
}

fn inspect_source(record: &Record) -> serde_json::Value {
    let mut source = serde_json::json!({"id":record.id,"name":record.candidate.name,"owner":record.owner,"revision":record.revision,"targets":record.targets,"disabled":record.disabled,"removed":record.removed,"spec_hash":record.hash});
    if record.removed {
        source["tools"] = serde_json::json!([]);
        return source;
    }
    let tools = (|| -> Result<_> {
        ensure!(
            record.format == super::FORMAT,
            "unsupported saved source format"
        );
        ensure!(
            super::hash(record.document.as_bytes()) == record.hash,
            "corrupt source document hash"
        );
        super::import::saved_tools(&record.candidate, &record.id, record.document.as_bytes())
    })();
    match tools {
        Ok(tools) => {
            source["tools"] = tools.into_iter().map(|tool| serde_json::json!({"name":tool.name,"method":tool.method,"path":tool.path})).collect();
        }
        Err(_) => {
            // Do not turn an unreadable inventory into a successful empty list
            // or expose untrusted schema/error prose in operator diagnostics.
            source["tools"] = serde_json::Value::Null;
            source["tools_error"] = "saved_catalog_invalid".into();
        }
    }
    source
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn record(number: u128, name: &str, document: &Value) -> Record {
        let document = document.to_string();
        Record {
            id: uuid::Uuid::from_u128(number).to_string(),
            owner: "toolbox".into(),
            candidate: serde_json::from_value(json!({"name":name,"spec_url":"https://offline.invalid/openapi.json","lifetime":"persistent"})).unwrap(),
            targets: vec!["toolbox".into()],
            revision: 1,
            hash: super::super::hash(document.as_bytes()),
            document,
            format: super::super::FORMAT,
            accepted_at: 1,
            updated_at: 1,
            removed: false,
            disabled: None,
        }
    }

    #[test]
    fn saved_inventory_preserves_selection_and_scopes_failures_without_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("registry.sqlite");
        let mut store = Store::open(&path, &[]).unwrap();
        let document = json!({"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],"paths":{
            "/api/read":{"get":{"tags":["company"],"description":"PRIVATE_SCHEMA_SENTINEL"}},
            "/api/write":{"post":{"tags":["company"]}},
            "/api/ignored":{"get":{"tags":["other"]}},
            "/outside":{"get":{"tags":["company"]}}
        }});
        let mut selected = record(1, "selected", &document);
        let prefix = format!("dyn_{}", selected.id.replace('-', ""));
        selected.candidate.selection.include = vec!["/api".into()];
        selected.candidate.selection.tags = vec!["company".into()];
        selected.candidate.selection.include_tools =
            vec![format!("{prefix}_read"), format!("{prefix}_write")];
        selected.candidate.selection.exclude_tools = vec![format!("{prefix}_write")];
        let mut disabled = record(2, "disabled", &document);
        disabled.candidate.selection.include = vec!["/api".into()];
        disabled.candidate.selection.tags = vec!["company".into()];
        disabled.disabled = Some("policy_or_catalog_invalid".into());
        let disabled_prefix = format!("dyn_{}", disabled.id.replace('-', ""));
        let mut removed = record(3, "removed", &document);
        removed.removed = true;
        removed.document.clear();
        let mut invalid = record(4, "invalid_json", &document);
        invalid.document = "{PRIVATE_SCHEMA_SENTINEL".into();
        invalid.hash = super::super::hash(invalid.document.as_bytes());
        let mut unknown = record(5, "unknown_format", &document);
        unknown.format = super::super::FORMAT + 1;
        let mut corrupt = record(6, "hash_mismatch", &document);
        corrupt.hash = "wrong".into();
        let cyclic = record(
            7,
            "cyclic",
            &json!({"openapi":"3.0.3","paths":{},"components":{"schemas":{"loop":{"$ref":"#/components/schemas/loop"}}}}),
        );
        let state = State {
            generation: 7,
            records: [
                selected.clone(),
                disabled,
                removed,
                invalid,
                unknown,
                corrupt,
                cyclic,
            ]
            .into_iter()
            .map(|record| (record.id.clone(), record))
            .collect(),
            receipts: Default::default(),
        };
        store.save(&state).unwrap();
        let contents = || {
            std::fs::read_dir(directory.path())
                .unwrap()
                .map(|entry| {
                    let path = entry.unwrap().path();
                    (
                        path.file_name().unwrap().to_owned(),
                        std::fs::read(path).unwrap(),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let before = contents();
        // Keep the write owner's exclusive lock held throughout inspection.
        assert!(Store::open(&path, &[]).is_err());
        let shown = inspect(&path).unwrap();
        assert_eq!(contents(), before);
        assert_eq!(shown["generation"], 7);
        assert_eq!(shown["sources"].as_array().unwrap().len(), 7);
        assert!(!shown.to_string().contains("PRIVATE_SCHEMA_SENTINEL"));
        let source = |name: &str| {
            shown["sources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|source| source["name"] == name)
                .unwrap()
        };
        assert_eq!(
            source("selected"),
            &json!({"id":selected.id,"name":"selected","owner":"toolbox","revision":1,"targets":["toolbox"],"disabled":null,"removed":false,"spec_hash":selected.hash,"tools":[{"name":format!("{prefix}_read"),"method":"GET","path":"/api/read"}]})
        );
        assert_eq!(
            source("disabled")["tools"],
            json!([
                {"name":format!("{disabled_prefix}_read"),"method":"GET","path":"/api/read"},
                {"name":format!("{disabled_prefix}_write"),"method":"POST","path":"/api/write"}
            ])
        );
        assert_eq!(source("disabled")["disabled"], "policy_or_catalog_invalid");
        assert_eq!(source("removed")["tools"], json!([]));
        assert!(source("removed").get("tools_error").is_none());
        for name in ["invalid_json", "unknown_format", "hash_mismatch", "cyclic"] {
            assert_eq!(source(name)["tools"], Value::Null);
            assert_eq!(source(name)["tools_error"], "saved_catalog_invalid");
        }
    }
}
