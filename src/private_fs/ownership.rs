//! Release logical ownership even when a dup or transient fork retains the file.
use fs2::FileExt;
use std::{fs::File, io};

pub(crate) struct ExclusiveLock {
    file: File,
    owner: &'static str,
}

impl ExclusiveLock {
    pub(crate) fn acquire(file: File, owner: &'static str) -> io::Result<Self> {
        file.try_lock_exclusive()?;
        Ok(Self { file, owner })
    }

    #[cfg(test)]
    pub(crate) fn try_clone(&self) -> io::Result<File> {
        self.file.try_clone()
    }
}

impl Drop for ExclusiveLock {
    fn drop(&mut self) {
        if let Err(error) = FileExt::unlock(&self.file) {
            tracing::warn!(category = "state_ownership_unlock_failed", owner = self.owner, %error,
                "State ownership could not be released; closing the owning descriptor");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    #[test]
    fn duplicate_descriptor_cannot_retain_ownership_after_owner_drops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let duplicate = file.try_clone().unwrap();
        let owner = ExclusiveLock::acquire(file, "fixture").unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(contender.try_lock_exclusive().is_err());
        drop(owner);
        // The duplicate remains open, like an inherited descriptor before exec.
        contender.try_lock_exclusive().unwrap();
        FileExt::unlock(&contender).unwrap();
        drop(duplicate);
    }
}
