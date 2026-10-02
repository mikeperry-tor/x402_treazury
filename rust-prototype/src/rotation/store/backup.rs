//! Exclusive-owner, consistent encrypted database backups. The encryption key
//! accompanies the database: protect this directory as a full spending backup.
use super::*;
use std::io::Write;
impl Store {
    pub fn backup(&self, destination: &Path) -> Result<()> {
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
        self.db.execute(
            "VACUUM INTO ?1",
            [database.to_str().context("backup path must be UTF-8")?],
        )?;
        database_file.sync_all()?;
        let mut key = private_file(&destination.join("key"), true)?;
        key.write_all(self.key.as_slice())?;
        key.sync_all()?;
        // The completion marker is written last. Missing marker means incomplete.
        let mut manifest = private_file(&destination.join("backup.json"), true)?;
        serde_json::to_writer_pretty(
            &mut manifest,
            &serde_json::json!({
                "format":1,"treasury_id":self.id,"snapshot_revision":self.snapshot()?.0,
                "network":self.network.name(),"database":"state.sqlite","key":"key"
            }),
        )?;
        manifest.sync_all()?;
        File::open(destination)?.sync_all()?;
        if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}
