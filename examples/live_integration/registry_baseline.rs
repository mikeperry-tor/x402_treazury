use super::*;
use std::collections::BTreeMap;
pub(super) fn capture(state: &Path, treasury: &str) -> Result<(files::Ownership, Value)> {
    let owner = files::lock(&state.join("owner.lock"))?;
    let status = x402_treazury::rotation::store::status(state)?;
    ensure!(
        status.treasury_id == treasury,
        "authorization treasury does not match existing state"
    );
    let db =
        Connection::open_with_flags(state.join("state.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = db.prepare(
        "SELECT id,pool_id,wallet_id,generation,amount,state FROM payment_attempts ORDER BY id",
    )?;
    let count: i64 = db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r.get(0))?;
    ensure!(
        count <= 10000,
        "baseline exceeds 10000 payment attempts; export/review support required"
    );
    let attempts=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"pool":r.get::<_,String>(1)?,"wallet":r.get::<_,String>(2)?,"generation":r.get::<_,i64>(3)?,"amount":r.get::<_,String>(4)?,"state":r.get::<_,String>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let count: i64 = db.query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r.get(0))?;
    ensure!(
        count <= 10000,
        "baseline exceeds 10000 source budget entries"
    );
    let mut stmt = db.prepare(
        "SELECT id,day,original_day,requested,reserved,consumed FROM budget_entries ORDER BY id",
    )?;
    let budget=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"day":r.get::<_,i64>(1)?,"original_day":r.get::<_,i64>(2)?,"requested":r.get::<_,i64>(3)?,"reserved":r.get::<_,i64>(4)?,"consumed":r.get::<_,i64>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut hashes = BTreeMap::new();
    let mut locks = vec![];
    legacy(state, state, 0, &mut hashes, &mut locks)?;
    Ok((
        owner,
        json!({"treasury_status":status,"payment_attempts":attempts,"source_budget_entries":budget,"legacy_file_hashes":hashes,"meaning":"immutable historical baseline, not new spending authority"}),
    ))
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
