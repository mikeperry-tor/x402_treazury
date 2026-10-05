use super::*;
use std::collections::BTreeMap;
pub(super) fn capture(state: &Path, treasury: &str) -> Result<(files::Ownership, Value)> {
    let owner = files::lock(&state.join("owner.lock"))?;
    let mut snapshot = x402_treazury::rotation::store::qualification_state(state)?;
    ensure!(
        snapshot["treasury_status"]["treasury_id"] == treasury,
        "authorization treasury does not match existing state"
    );
    let mut hashes = BTreeMap::new();
    let mut locks = vec![];
    legacy(state, state, 0, &mut hashes, &mut locks)?;
    snapshot["legacy_file_hashes"] = serde_json::to_value(hashes)?;
    snapshot["meaning"] = json!("immutable historical baseline, not new spending authority");
    Ok((owner, snapshot))
}
fn legacy(
    root: &Path,
    dir: &Path,
    depth: usize,
    hashes: &mut BTreeMap<String, String>,
    locks: &mut Vec<files::Ownership>,
) -> Result<()> {
    ensure!(depth <= 8, "baseline directory traversal exceeds depth 8");
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path == root.join("live-integration") {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "baseline refuses symlink: {}",
            path.display()
        );
        if metadata.is_dir() {
            files::directory(&path)?;
            let owner = path.join("owner.lock");
            if owner.try_exists()? {
                locks.push(files::lock(&owner)?);
            }
            legacy(root, &path, depth + 1, hashes, locks)?;
        } else if path
            .extension()
            .is_some_and(|x| matches!(x.to_str(), Some("sqlite" | "json" | "jsonl" | "toml")))
        {
            ensure!(hashes.len() < 256, "baseline exceeds 256 evidence files");
            files::regular(&path)?;
            hashes.insert(
                path.strip_prefix(root)?.to_string_lossy().into_owned(),
                files::hash_file(&path)?,
            );
            // Journals affect a SQLite baseline too; never treat the main file hash as complete.
            if path.extension().is_some_and(|x| x == "sqlite") {
                for suffix in ["-wal", "-shm", "-journal"] {
                    let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
                    if sidecar.try_exists()? || sidecar.is_symlink() {
                        ensure!(
                            hashes.len() < 256,
                            "baseline exceeds 256 evidence files including SQLite sidecars"
                        );
                        files::regular(&sidecar)?;
                        hashes.insert(
                            sidecar.strip_prefix(root)?.to_string_lossy().into_owned(),
                            files::hash_file(&sidecar)?,
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
