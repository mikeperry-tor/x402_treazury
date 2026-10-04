//! Shared build-time/runtime fingerprint of local compilation inputs, without secrets.
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};
fn walk(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("build input must not be a symlink"));
    }
    if metadata.is_dir() {
        let mut entries = std::fs::read_dir(path)?
            .map(|e| e.map(|e| e.path()))
            .collect::<io::Result<Vec<_>>>()?;
        entries.sort();
        for entry in entries {
            if entry
                .file_name()
                .is_some_and(|n| n == ".git" || n == "target" || n == ".DS_Store")
            {
                continue;
            }
            walk(&entry, files)?;
        }
    } else if metadata.is_file() {
        files.push(path.to_owned());
    } else {
        return Err(io::Error::other("build input must be an ordinary file"));
    }
    Ok(())
}
pub fn file_hash(path: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn source_hash(root: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    for name in [
        "src",
        "examples/live_integration.rs",
        "examples/live_integration",
        "vendor",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "rust-toolchain.toml",
    ] {
        walk(&root.join(name), &mut files)?;
    }
    files.sort();
    let mut hash = Sha256::new();
    hash.update(b"x402_treazury-build-inputs-v1\0");
    for file in files {
        let name = file
            .strip_prefix(root)
            .map_err(io::Error::other)?
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF8 build path"))?;
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        hash.update(file_hash(&file)?.as_bytes());
    }
    Ok(format!("{:x}", hash.finalize()))
}
