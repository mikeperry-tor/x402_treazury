//! Inspect a stopped, exclusively owned database without touching its journals.
//! The caller must retain the application owner lock throughout this snapshot.
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags};
use std::{fs::OpenOptions, path::Path};

pub(super) struct Snapshot {
    pub db: Connection,
    // SQLite closes before the private temporary directory is removed.
    _directory: tempfile::TempDir,
}

pub(super) fn snapshot(dir: &Path) -> Result<Snapshot> {
    let mut temporary = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let temporary = temporary.tempdir()?;
    // Immutable SQLite handles cannot recover a database whose current schema
    // lives in WAL. Copy both files under exclusive ownership, then let SQLite
    // recover only the disposable copy. Wallet payloads remain encrypted;
    // database metadata is plaintext. No source SHM is needed.
    for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-journal"] {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut source = match options.open(dir.join(name)) {
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
        _directory: temporary,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn disposable_snapshot_keeps_database_and_directory_private() {
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("state.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE fixture(value); INSERT INTO fixture VALUES(7);")
            .unwrap();
        drop(db);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let copy = snapshot(source.path()).unwrap();
        for path in [
            copy._directory.path().to_owned(),
            copy._directory.path().join("state.sqlite"),
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
}
