//! Inspect a stopped, exclusively owned database without touching its journals.
//! The caller must retain the application owner lock throughout this snapshot.
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags};
use std::{fs::OpenOptions, path::Path};

pub(super) struct Snapshot {
    pub db: Connection,
    // SQLite closes before the private temporary directory is removed.
    _directory: Option<tempfile::TempDir>,
}

pub(super) fn snapshot(dir: &Path) -> Result<Snapshot> {
    let mut sources = Vec::with_capacity(3);
    let mut needs_recovery = false;
    for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-journal"] {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let source = match options.open(dir.join(name)) {
            Ok(file) => file,
            Err(error)
                if name != "state.sqlite" && error.kind() == std::io::ErrorKind::NotFound =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let metadata = source.metadata()?;
        ensure!(metadata.is_file(), "invalid preflight database file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "preflight database permissions are not private"
            );
        }
        needs_recovery |= name != "state.sqlite" && metadata.len() != 0;
        sources.push((name, source));
    }
    if !needs_recovery {
        // Exclusive application ownership makes the source immutable for this
        // handle's lifetime. SQLite reads pages on demand and creates no journals
        // or SHM, even when the database header records WAL mode. Do not use this
        // path with journal data: immutable SQLite deliberately ignores it.
        let mut uri = reqwest::Url::from_file_path(std::path::absolute(dir.join("state.sqlite"))?)
            .map_err(|()| anyhow::anyhow!("invalid preflight database path"))?;
        uri.set_query(Some("immutable=1"));
        let db = Connection::open_with_flags(
            uri.as_str(),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )?;
        return Ok(Snapshot {
            db,
            _directory: None,
        });
    }

    let mut temporary = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let temporary = temporary.tempdir()?;
    // Nonempty journals may contain the authoritative schema. Let SQLite recover
    // only a disposable copy, without parsing WAL/rollback formats ourselves.
    // Wallet payloads remain encrypted; database metadata is plaintext.
    // No source SHM is needed.
    for (name, mut source) in sources {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut destination = options.open(temporary.path().join(name))?;
        std::io::copy(&mut source, &mut destination)?;
    }
    let db = Connection::open_with_flags(
        temporary.path().join("state.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.execute_batch("PRAGMA query_only=ON;")?;
    Ok(Snapshot {
        db,
        _directory: Some(temporary),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const FILES: [&str; 3] = ["state.sqlite", "state.sqlite-wal", "state.sqlite-journal"];

    fn private_files(dir: &Path) {
        for name in FILES {
            let path = dir.join(name);
            if path.exists() {
                fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
    }

    fn contents(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_str().unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    }

    #[test]
    fn disposable_snapshot_keeps_database_and_directory_private() {
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("state.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=PERSIST; CREATE TABLE fixture(value); INSERT INTO fixture VALUES(7);")
            .unwrap();
        drop(db);
        private_files(source.path());
        assert!(
            fs::metadata(source.path().join("state.sqlite-journal"))
                .unwrap()
                .len()
                > 0
        );
        let copy = snapshot(source.path()).unwrap();
        let directory = copy
            ._directory
            .as_ref()
            .expect("nonempty journal requires a copy");
        for path in [
            directory.path().to_owned(),
            directory.path().join("state.sqlite"),
        ] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        assert_eq!(
            copy.db
                .query_row("SELECT value FROM fixture", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            7
        );
        assert!(copy.db.execute("DELETE FROM fixture", []).is_err());
    }

    #[test]
    fn clean_preflight_reads_in_place_without_creating_or_changing_files() {
        for mode in ["DELETE", "WAL"] {
            for version in [12, 16] {
                // URI punctuation must remain part of the filename.
                let source = tempfile::Builder::new()
                    .prefix("preflight ?#% ")
                    .tempdir()
                    .unwrap();
                let db = Connection::open(source.path().join("state.sqlite")).unwrap();
                db.execute_batch(&format!("PRAGMA journal_mode={mode}; PRAGMA user_version={version}; CREATE TABLE payload(bytes); INSERT INTO payload VALUES(zeroblob(4194304));")).unwrap();
                drop(db);
                // Empty leftovers do not carry a newer schema or recovery data.
                for name in &FILES[1..] {
                    fs::write(source.path().join(name), []).unwrap();
                }
                private_files(source.path());
                let before = contents(source.path());
                for _ in 0..2 {
                    let view = snapshot(source.path()).unwrap();
                    assert!(
                        view._directory.is_none(),
                        "clean reopen must not copy the database"
                    );
                    assert!(view.db.is_readonly(rusqlite::MAIN_DB).unwrap());
                    assert_eq!(
                        view.db
                            .pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
                            .unwrap(),
                        version
                    );
                    assert!(view.db.execute_batch("PRAGMA user_version=17").is_err());
                    assert!(view.db.execute("DELETE FROM payload", []).is_err());
                }
                assert_eq!(before, contents(source.path()));
            }
        }
    }

    #[test]
    fn committed_wal_and_hot_rollback_journal_use_private_recovery() {
        for wal in [true, false] {
            let source = tempfile::tempdir().unwrap();
            let db = Connection::open(source.path().join("state.sqlite")).unwrap();
            db.execute_batch("PRAGMA user_version=12; CREATE TABLE fixture(value); INSERT INTO fixture VALUES(7);").unwrap();
            if wal {
                db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA user_version=16; UPDATE fixture SET value=8;").unwrap();
            } else {
                db.execute_batch(
                    "BEGIN IMMEDIATE; PRAGMA user_version=16; UPDATE fixture SET value=8;",
                )
                .unwrap();
                db.cache_flush().unwrap();
            }
            // A quiescent copy simulates stopped state with no surviving writer or
            // SHM. WAL carries a committed version; rollback carries undo data.
            let frozen = tempfile::tempdir().unwrap();
            for name in FILES {
                if source.path().join(name).exists() {
                    fs::copy(source.path().join(name), frozen.path().join(name)).unwrap();
                }
            }
            private_files(frozen.path());
            let before = contents(frozen.path());
            for _ in 0..2 {
                let view = snapshot(frozen.path()).unwrap();
                assert!(view._directory.is_some());
                assert_eq!(
                    view.db
                        .pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
                        .unwrap(),
                    if wal { 16 } else { 12 }
                );
                assert_eq!(
                    view.db
                        .query_row::<u32, _, _>("SELECT value FROM fixture", [], |r| r.get(0))
                        .unwrap(),
                    if wal { 8 } else { 7 }
                );
                assert!(view.db.execute("DELETE FROM fixture", []).is_err());
            }
            assert_eq!(before, contents(frozen.path()));
            assert!(!frozen.path().join("state.sqlite-shm").exists());
        }
    }

    #[test]
    fn fast_path_still_rejects_nonprivate_and_symlinked_files() {
        for name in FILES {
            let source = tempfile::tempdir().unwrap();
            let db = Connection::open(source.path().join("state.sqlite")).unwrap();
            db.execute_batch("PRAGMA user_version=16;").unwrap();
            drop(db);
            for sidecar in &FILES[1..] {
                fs::write(source.path().join(sidecar), []).unwrap();
            }
            private_files(source.path());
            let path = source.path().join(name);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(snapshot(source.path()).is_err());
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let target = source.path().join("target");
            fs::rename(&path, &target).unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            assert!(snapshot(source.path()).is_err());
        }
    }
}
