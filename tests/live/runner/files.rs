//! Private, bounded evidence and registry filesystem operations.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
pub const DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
pub const CATALOG_BYTES: usize = x402_treazury::deployment::QUALIFICATION_SNAPSHOT_BYTES;
pub fn regular(path: &Path) -> Result<()> {
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "expected ordinary file: {}",
        path.display()
    );
    #[cfg(unix)]
    ensure!(
        m.nlink() == 1 && m.mode() & 0o077 == 0 && m.uid() == unsafe { libc::geteuid() },
        "file must be owner-only and singly linked: {}",
        path.display()
    );
    Ok(())
}
pub fn directory(path: &Path) -> Result<()> {
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_dir() && !m.file_type().is_symlink(),
        "expected ordinary directory"
    );
    #[cfg(unix)]
    ensure!(
        m.mode() & 0o077 == 0 && m.uid() == unsafe { libc::geteuid() },
        "directory must be owner-only"
    );
    Ok(())
}
pub fn create_dir(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)?;
    File::open(path.parent().context("missing parent")?)?.sync_all()?;
    Ok(())
}
pub fn create_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    Ok(options.open(path)?)
}
pub struct Ownership(File);
impl Drop for Ownership {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
pub fn lock(path: &Path) -> Result<Ownership> {
    regular(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options.open(path)?;
    file.try_lock_exclusive()
        .context("another process owns this registry/treasury")?;
    Ok(Ownership(file))
}
pub fn read(path: &Path) -> Result<Vec<u8>> {
    read_bounded(path, DOCUMENT_BYTES)
}
pub fn read_catalog(path: &Path) -> Result<Vec<u8>> {
    read_bounded(path, CATALOG_BYTES)
}
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "document exceeds {limit} byte limit: {}",
        path.display()
    );
    Ok(bytes)
}
pub fn hash(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
pub fn hash_file(path: &Path) -> Result<String> {
    const LIMIT: u64 = 2 * 1024 * 1024 * 1024;
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        ensure!(count <= LIMIT, "file hash exceeds {LIMIT} byte limit");
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    publish_bounded(path, bytes, DOCUMENT_BYTES)
}
pub fn publish_catalog(path: &Path, bytes: &[u8]) -> Result<()> {
    publish_bounded(path, bytes, CATALOG_BYTES)
}
fn publish_bounded(path: &Path, bytes: &[u8], limit: usize) -> Result<()> {
    ensure!(bytes.len() <= limit, "evidence exceeds {limit} byte limit");
    let parent = path.parent().context("evidence parent missing")?;
    directory(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(path)
        .map_err(|e| e.error)
        .context("evidence already exists; refusing overwrite")?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_documents_refuse_overflow_and_never_publish_partial_evidence() {
        let tmp = tempfile::tempdir().unwrap();
        let private = tmp.path().join("private");
        create_dir(&private).unwrap();
        let path = private.join("evidence");
        let error = publish_bounded(&path, b"12345", 4).unwrap_err();
        assert!(error.to_string().contains("4 byte limit"));
        assert!(!path.exists());
        publish_bounded(&path, b"1234", 4).unwrap();
        assert_eq!(read_bounded(&path, 4).unwrap(), b"1234");
        assert!(
            read_bounded(&path, 3)
                .unwrap_err()
                .to_string()
                .contains("3 byte limit")
        );
        assert!(publish_bounded(&path, b"12", 4).is_err());
        assert_eq!(read_bounded(&path, 4).unwrap(), b"1234");
    }
}
