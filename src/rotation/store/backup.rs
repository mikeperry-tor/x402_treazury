//! Exclusive-owner, consistent encrypted database backups. The encryption key
//! accompanies the database: protect this directory as a full spending backup.
use super::*;
use std::io::Write;
// One implementation for production and deterministic write/sync failure tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    DatabaseWrite,
    DatabaseSync,
    KeyWrite,
    KeySync,
    ManifestWrite,
    ManifestPartial,
    ManifestSync,
    Publish,
    DirectorySync,
    ParentSync,
}
impl Store {
    pub fn backup(&self, destination: &Path) -> Result<()> {
        self.backup_at_boundaries(destination, |_| Ok(()))
    }
    fn backup_at_boundaries(
        &self,
        destination: &Path,
        mut boundary: impl FnMut(Boundary) -> Result<()>,
    ) -> Result<()> {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(destination)
            .context("backup destination must not exist")?;
        let database = destination.join("state.sqlite");
        // Pre-create owner-only: SQLite VACUUM INTO accepts an empty file.
        let database_file = private_file(&database, true)?;
        boundary(Boundary::DatabaseWrite)?;
        self.db.execute(
            "VACUUM INTO ?1",
            [database.to_str().context("backup path must be UTF-8")?],
        )?;
        boundary(Boundary::DatabaseSync)?;
        database_file.sync_all()?;
        let mut key = private_file(&destination.join("key"), true)?;
        boundary(Boundary::KeyWrite)?;
        key.write_all(self.key.as_slice())?;
        boundary(Boundary::KeySync)?;
        key.sync_all()?;
        // Only a complete, synced manifest is ever visible under the final name.
        // A failed destination remains reserved; retry must use a new directory.
        let temporary = destination.join("backup.json.tmp");
        let mut manifest = private_file(&temporary, true)?;
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "format":1,"treasury_id":self.id,"snapshot_revision":self.snapshot()?.0,
            "network":self.network.name(),"database":"state.sqlite","key":"key"
        }))?;
        boundary(Boundary::ManifestWrite)?;
        let middle = bytes.len() / 2;
        manifest.write_all(&bytes[..middle])?;
        boundary(Boundary::ManifestPartial)?;
        manifest.write_all(&bytes[middle..])?;
        boundary(Boundary::ManifestSync)?;
        manifest.sync_all()?;
        boundary(Boundary::Publish)?;
        fs::rename(&temporary, destination.join("backup.json"))?;
        boundary(Boundary::DirectorySync)?;
        File::open(destination)?.sync_all()?;
        if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
            boundary(Boundary::ParentSync)?;
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_backups_never_publish_partial_manifests_or_overwrite() {
        for failure in [
            Boundary::DatabaseWrite,
            Boundary::DatabaseSync,
            Boundary::KeyWrite,
            Boundary::KeySync,
            Boundary::ManifestWrite,
            Boundary::ManifestPartial,
            Boundary::ManifestSync,
            Boundary::Publish,
            Boundary::DirectorySync,
            Boundary::ParentSync,
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let mut source = Store::create(
                &tmp.path().join("state"),
                &tmp.path().join("key"),
                1,
                b"initial",
            )
            .unwrap();
            source.reserve("pending", None, 1, 100, 1000).unwrap();
            source
                .prepare("pending", 1, b"prepared", b"signed bytes")
                .unwrap();
            let original = serde_json::to_value(source.status().unwrap()).unwrap();
            let destination = tmp.path().join("backup");
            let error = source
                .backup_at_boundaries(&destination, |stage| {
                    if stage == failure {
                        Err(std::io::Error::other("injected I/O failure").into())
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert!(error.to_string().contains("injected I/O"));
            assert_eq!(
                serde_json::to_value(source.status().unwrap()).unwrap(),
                original
            );
            assert_eq!(
                source.prepared_bytes("pending").unwrap().as_slice(),
                b"signed bytes"
            );
            let published = matches!(failure, Boundary::DirectorySync | Boundary::ParentSync);
            assert_eq!(
                destination.join("backup.json").exists(),
                published,
                "{failure:?}"
            );
            if published {
                let manifest: serde_json::Value =
                    serde_json::from_slice(&fs::read(destination.join("backup.json")).unwrap())
                        .unwrap();
                assert_eq!(manifest["format"], 1);
                assert_eq!(manifest["treasury_id"], source.id);
                assert_eq!(manifest["snapshot_revision"], 2);
                let restored =
                    Store::open(&destination, &destination.join("key"), &source.id).unwrap();
                assert_eq!(
                    serde_json::to_value(restored.status().unwrap()).unwrap(),
                    original
                );
                assert_eq!(
                    restored.prepared_bytes("pending").unwrap().as_slice(),
                    b"signed bytes"
                );
            }
            if failure == Boundary::ManifestPartial {
                assert!(
                    !fs::read(destination.join("backup.json.tmp"))
                        .unwrap()
                        .is_empty()
                );
                assert!(
                    serde_json::from_slice::<serde_json::Value>(
                        &fs::read(destination.join("backup.json.tmp")).unwrap()
                    )
                    .is_err()
                );
            }
            let files_before: Vec<_> = fs::read_dir(&destination)
                .unwrap()
                .map(|e| {
                    let p = e.unwrap().path();
                    (p.clone(), fs::read(p).unwrap())
                })
                .collect();
            assert!(source.backup(&destination).is_err());
            for (path, bytes) in files_before {
                assert_eq!(fs::read(path).unwrap(), bytes);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&destination).unwrap().permissions().mode() & 0o077,
                    0
                );
                for entry in fs::read_dir(&destination).unwrap() {
                    assert_eq!(
                        entry.unwrap().metadata().unwrap().permissions().mode() & 0o077,
                        0
                    );
                }
            }
            source.save_snapshot(2, b"still usable").unwrap();
        }
    }
}
