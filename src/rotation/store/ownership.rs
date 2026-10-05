//! Explicitly release treasury ownership before closing its descriptor. A dup or
//! transient fork inheritance must not keep flock alive after Store has closed.
use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs::File;

pub(super) struct Owner(File);
impl Owner {
    pub(super) fn acquire(file: File) -> Result<Self> {
        file.try_lock_exclusive().context("state_in_use")?;
        Ok(Self(file))
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Err(error) = FileExt::unlock(&self.0) {
            tracing::warn!(category = "treasury_ownership_unlock_failed", %error,
                "Treasury ownership unlock failed; closing the owning descriptor");
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
        let owner = Owner::acquire(file).unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(contender.try_lock_exclusive().is_err());
        drop(owner);
        // The duplicate remains open, just as a concurrently spawned child may
        // briefly retain an inherited descriptor before exec closes CLOEXEC FDs.
        contender.try_lock_exclusive().unwrap();
        FileExt::unlock(&contender).unwrap();
        drop(duplicate);
    }
}
