//! Explicit same-day continuation without resetting any financial authority.
use super::*;
use fs2::FileExt;

pub async fn renew(path: &Path, manifest: &Manifest, digest: &str, expires_at: u64) -> Result<()> {
    let instant = now()?;
    let mut updated = manifest.clone();
    updated.expires_at = expires_at;
    updated.validate(instant)?;
    ensure!(
        manifest.expires_at <= instant
            && expires_at > manifest.expires_at
            && manifest.expires_at / 86400 == instant / 86400,
        "renewal requires an expired run from this UTC day"
    );
    let shown = settings(manifest).await?;
    let dir = directory(&shown)?;
    let mut ledger = ledger::Ledger::open(&dir)?;
    let old = ledger.prepared()?;
    ensure!(
        old["manifest_hash"] == digest && old["manifest"] == serde_json::to_value(manifest)?,
        "manifest differs from pinned run"
    );
    ensure!(
        old["resolved_config_hash"] == hash(serde_json::to_vec(&shown)?),
        "configuration changed; renewal cannot repin it"
    );
    let owner_path = dir
        .parent()
        .context("missing treasury directory")?
        .join("owner.lock");
    ledger::regular(&owner_path)?;
    let owner = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(owner_path)?;
    owner
        .try_lock_exclusive()
        .context("stop treasury owner before renewal")?;
    let bytes = toml::to_string_pretty(&updated)?;
    let mut replacement = old.clone();
    replacement["manifest"] = serde_json::to_value(&updated)?;
    replacement["manifest_hash"] = json!(hash(&bytes));
    ledger.check_renewal(&old, &replacement, instant)?;
    ledger::regular(path)?;
    let evidence = dir.join("window-renewal");
    std::fs::create_dir(&evidence)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&evidence, std::fs::Permissions::from_mode(0o700))?;
    }
    for (name, data) in [
        ("manifest-before.toml", std::fs::read(path)?),
        ("pins-before.json", serde_json::to_vec(&old)?),
    ] {
        let mut file = ledger::private_file(&evidence.join(name))?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    std::fs::File::open(&evidence)?.sync_all()?;
    std::fs::File::open(&dir)?.sync_all()?;
    // A crash before the ledger commit leaves mismatched pins and fails closed.
    // Retained evidence supports manual inspection, never automatic replay/reset.
    std::fs::write(path, &bytes)?;
    std::fs::File::open(path)?.sync_all()?;
    ledger.renew_window(&old, &replacement, instant)?;
    println!(
        "{}",
        json!({"renewed":true,"expires_at":expires_at,"budgets_cases_and_reservations_unchanged":true,"evidence":evidence})
    );
    Ok(())
}
