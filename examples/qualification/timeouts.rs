//! One audited timeout-only amendment before any treasury or API activity.
use super::*;
use fs2::FileExt;

fn compatible_hash(shown: &Value, expected: &Value) -> Result<()> {
    let expected = expected.as_str().context("missing pinned config hash")?;
    if hash(serde_json::to_vec(shown)?) == expected {
        return Ok(());
    }
    // The older inspector did not expose the newly introduced request floor.
    let mut legacy = shown.clone();
    legacy["network"]
        .as_object_mut()
        .context("network missing")?
        .remove("request_timeout_seconds");
    ensure!(
        hash(serde_json::to_vec(&legacy)?) == expected,
        "pinned configuration changed beyond timeout inspection"
    );
    Ok(())
}

pub async fn amend(path: &Path, manifest: &Manifest, digest: &str) -> Result<()> {
    manifest.validate(now()?)?;
    let shown = settings(manifest).await?;
    let dir = directory(&shown)?;
    let mut ledger = ledger::Ledger::open(&dir)?;
    let old = ledger.prepared()?;
    ensure!(
        old["manifest_hash"] == digest && old["manifest"] == serde_json::to_value(manifest)?,
        "manifest differs from pinned run"
    );
    compatible_hash(&shown, &old["resolved_config_hash"])?;
    // Lock the same ownership file as the treasury without loading encryption keys.
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
        .context("stop treasury owner before amending timeouts")?;
    let state = store::status(dir.parent().unwrap())?;
    ensure!(
        state.pools.is_empty()
            && state.funding_jobs.is_empty()
            && state.treasury_operations.is_empty(),
        "timeout amendment requires a never-started treasury run"
    );
    ledger.require_unstarted()?;
    let original_manifest = std::fs::read(path)?;
    let original_config = std::fs::read(&manifest.deployment)?;
    let mut table: toml::Value = toml::from_str(std::str::from_utf8(&original_config)?)?;
    let network = table.get_mut("network").context("network missing")?;
    ensure!(network["mode"].as_str() == Some("tor"), "Tor required");
    // Fixed reviewed increase, not a general config-edit or budget-reset facility.
    ensure!(
        shown["network"]["connect_timeout_seconds"]
            .as_u64()
            .is_some_and(|v| v <= 120)
            && shown["network"]["request_timeout_seconds"]
                .as_u64()
                .is_some_and(|v| v <= 240)
            && manifest.timeout_seconds <= 900,
        "amendment must not reduce timeouts"
    );
    network
        .as_table_mut()
        .context("invalid network table")?
        .insert("connect_timeout_seconds".into(), toml::Value::Integer(120));
    network
        .as_table_mut()
        .context("invalid network table")?
        .insert("request_timeout_seconds".into(), toml::Value::Integer(240));
    let config_bytes = toml::to_string_pretty(&table)?;
    let mut updated = manifest.clone();
    updated.timeout_seconds = 900; // unsigned + admission + signed request, each up to 240s
    let manifest_bytes = toml::to_string_pretty(&updated)?;
    let evidence = dir.join("timeout-amendment");
    std::fs::create_dir(&evidence)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&evidence, std::fs::Permissions::from_mode(0o700))?;
    }
    for (name, bytes) in [
        ("manifest-before.toml", original_manifest.as_slice()),
        ("deployment-before.toml", original_config.as_slice()),
        ("pins-before.json", &serde_json::to_vec(&old)?),
    ] {
        let mut file = ledger::private_file(&evidence.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    // A crash between file updates and the ledger commit fails hash validation;
    // the retained old files/pins permit explicit investigation, never auto-reset.
    std::fs::write(&manifest.deployment, &config_bytes)?;
    std::fs::File::open(&manifest.deployment)?.sync_all()?;
    std::fs::write(path, &manifest_bytes)?;
    std::fs::File::open(path)?.sync_all()?;
    let new_shown = settings(&updated).await?;
    let mut pinned = old.clone();
    pinned["manifest"] = serde_json::to_value(&updated)?;
    pinned["manifest_hash"] = json!(hash(manifest_bytes));
    pinned["resolved_config_hash"] = json!(hash(serde_json::to_vec(&new_shown)?));
    ledger.amend_timeouts(&old, &pinned)?;
    println!(
        "{}",
        json!({"amended":true,"connect_seconds":120,"request_seconds":240,"driver_seconds":900,"budgets_and_cases_unchanged":true,"evidence":evidence})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_compatibility_does_not_ignore_other_config_changes() {
        let old = json!({"network":{"mode":"tor","connect_timeout_seconds":30},"deposit":"1.74"});
        let digest = json!(hash(serde_json::to_vec(&old).unwrap()));
        let mut new = old.clone();
        new["network"]["request_timeout_seconds"] = json!(240);
        compatible_hash(&new, &digest).unwrap();
        new["deposit"] = json!("5");
        assert!(compatible_hash(&new, &digest).is_err());
    }
}
