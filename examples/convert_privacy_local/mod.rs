use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use rusqlite::Connection;
#[path = "../../src/rotation/store/preflight.rs"]
mod preflight_snapshot;
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    AlreadyConverted,
}
struct Owner(File);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
fn regular(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path).context("source_metadata_unavailable")?;
    ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "source_path_type_invalid"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "source_permissions_not_private"
        );
    }
    Ok(())
}
pub fn inspect(dir: &Path) -> Result<Outcome> {
    regular(dir, true)?;
    // Match the production flock; never create missing state or lock files.
    let path = dir.join("owner.lock");
    regular(&path, false)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .context("source_owner_unavailable")?;
    file.try_lock_exclusive().context("state_in_use")?;
    let _owner = Owner(file);
    let path = dir.join("state.sqlite");
    regular(&path, false)?;
    // Clean state is read immutably. Recovery, if needed, touches only a private
    // disposable copy of the database and journals, without decrypting payloads.
    let snapshot = preflight_snapshot::snapshot(dir)?;
    let db = &snapshot.db;
    db.execute_batch("PRAGMA foreign_keys=ON; BEGIN;")?;
    preflight(db)
}
fn count(db: &Connection, sql: &str) -> Result<i64> {
    Ok(db.query_row(sql, [], |r| r.get(0))?)
}
fn column(db: &Connection, table: &str, name: &str) -> Result<bool> {
    // Table names below are authored constants, never command input.
    Ok(db
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|v| v == name))
}
fn preflight(db: &Connection) -> Result<Outcome> {
    let version = count(db, "PRAGMA user_version")?;
    ensure!(matches!(version, 12 | 16), "unsupported_source_schema");
    let integrity: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    ensure!(integrity == "ok", "source_integrity_failed");
    ensure!(
        db.prepare("PRAGMA foreign_key_check")?
            .query([])?
            .next()?
            .is_none(),
        "source_foreign_key_failed"
    );
    if version == 16 {
        ensure!(
            column(db, "reserve_claims", "generation")?
                && column(db, "pools", "payment_limit")?
                && column(db, "funding_jobs", "readiness_support")?
                && column(db, "wallets", "unresolved_exposure")?
                && column(db, "payment_admission_evidence", "payer_role")?
                && column(db, "payment_admission_evidence", "reserve_slot")?
                && column(db, "payment_admission_evidence", "claim_generation")?,
            "converted_schema_shape_invalid"
        );
        return Ok(Outcome::AlreadyConverted);
    }
    // Only the inspected source layout is supported. Merely adding a field with
    // purported evidence must not create a proof-import or legacy repair mode.
    ensure!(
        !column(db, "funding_jobs", "readiness_support")?,
        "unsupported_source_shape"
    );
    ensure!(
        count(
            db,
            "SELECT COUNT(*) FROM instance WHERE version=1 AND network='mainnet' AND account=0"
        )? == 1,
        "source_identity_ambiguous"
    );
    ensure!(
        count(db, "SELECT COUNT(*) FROM pools")? == 3
            && count(db, "SELECT COUNT(*) FROM wallets")? == 6
            && count(db, "SELECT COUNT(*) FROM funding_jobs")? == 6,
        "unsupported_source_inventory"
    );
    ensure!(
        count(
            db,
            "SELECT COUNT(*) FROM pools p WHERE (SELECT COUNT(*) FROM wallets w WHERE w.pool_id=p.id AND w.role='ACTIVE')<>1 OR (SELECT COUNT(*) FROM wallets w WHERE w.pool_id=p.id AND w.role='READY')<>1"
        )? == 0,
        "legacy_wallet_ownership_ambiguous"
    );
    ensure!(
        count(
            db,
            "SELECT COUNT(*) FROM funding_jobs j LEFT JOIN funding_progress p ON p.job_id=j.id WHERE j.state<>'COMPLETE' OR p.job_id IS NULL OR p.phase<>'\"VERIFYING_CREDIT\"'"
        )? == 0,
        "unsupported_source_funding_phase"
    );
    ensure!(
        count(
            db,
            "SELECT COUNT(*) FROM funding_progress p LEFT JOIN treasury_operations o ON o.id=p.operation_id LEFT JOIN outgoing x ON x.id=p.operation_id LEFT JOIN budget_entries b ON b.id=p.operation_id WHERE o.id IS NULL OR x.id IS NULL OR b.id IS NULL OR o.submission<>'CONFIRMED' OR x.state<>'RESOLVED' OR b.reserved<>0"
        )? == 0,
        "legacy_complete_source_obligation_unresolved"
    );
    ensure!(
        count(db, "SELECT COUNT(*) FROM budget_entries WHERE reserved<>0")? == 0
            && count(db, "SELECT COUNT(*) FROM outgoing WHERE state='PREPARED'")? == 0,
        "unsupported_source_pending_operation"
    );
    ensure!(
        count(
            db,
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'"
        )? == 0,
        "unsupported_source_signed_liability"
    );
    ensure!(
        count(
            db,
            "SELECT COUNT(*) FROM payment_attempts a LEFT JOIN payment_resolutions r ON r.attempt_id=a.id WHERE a.state='RESOLVED' AND (r.attempt_id IS NULL OR r.height<0 OR length(r.hash)<>66 OR substr(r.hash,1,2)<>'0x' OR substr(r.hash,3) GLOB '*[^0-9a-fA-F]*' OR r.block_time<0)"
        )? == 0,
        "legacy_payment_resolution_support_missing"
    );
    ensure!(
        count(
            db,
            "SELECT COUNT(*) FROM payment_attempts WHERE state='RESOLVED'"
        )? == 5
            && count(
                db,
                "SELECT COUNT(*) FROM payment_resolutions WHERE outcome='USED'"
            )? == 5,
        "unsupported_source_payment_history"
    );
    ensure!(
        count(db, "SELECT COUNT(*) FROM funding_recovery")? == 2
            && count(db, "SELECT COUNT(*) FROM refund_outputs")? == 0,
        "unsupported_source_return_history"
    );
    // record_credit overwrote wallets.balance/block; complete_job saved only a
    // phase. Neither is immutable operation-correlated destination support.
    // Current balances, encrypted quotes and successful source submission cannot
    // substitute for D. No schema change, balance reset, ADMITTED deletion or
    // backup/key access has happened before this diagnostic.
    anyhow::bail!("legacy_destination_readiness_support_missing")
}
#[cfg(test)]
mod tests;
