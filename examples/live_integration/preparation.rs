use super::{
    files,
    manifest::Manifest,
    planner::{self, Plan},
    registry::Pins,
};
use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Command};
/// No executable is launched, no catalog/network/secret is read.
pub async fn collect(manifest: &Manifest, state: &Path) -> Result<(Plan, Pins)> {
    let config = x402_treazury::deployment::Deployment::show_config(&manifest.deployment).await?;
    let plan = planner::build(manifest, &config)?;
    let configured = Path::new(
        config["treasury"]["state_dir"]
            .as_str()
            .context("preparation requires an existing treasury state reference")?,
    );
    ensure!(
        configured.canonicalize()? == state.canonicalize()?,
        "registry state does not match deployment treasury path"
    );
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let revision = git(repo, &["rev-parse", "HEAD"])?;
    let dirty = git(repo, &["status", "--porcelain", "--untracked-files=normal"])?;
    let pins = Pins {
        binary_sha256: files::hash_file(&manifest.binary)?,
        cargo_lock_sha256: files::hash_file(&repo.join("Cargo.lock"))?,
        source_revision: String::from_utf8(revision)?.trim().to_owned(),
        source_dirty: !dirty.is_empty(),
        resolved_config: config,
        qualification: "configuration_only".into(),
        catalogs: None,
    };
    Ok((plan, pins))
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut out = Vec::new();
    let read = child
        .stdout
        .take()
        .context("missing git stdout")?
        .take(files::DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut out);
    if read.is_err() || out.len() > files::DOCUMENT_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        read?;
        anyhow::bail!("git evidence exceeds {} byte limit", files::DOCUMENT_BYTES);
    }
    ensure!(child.wait()?.success(), "cannot record checkout evidence");
    Ok(out)
}

#[path = "preparation_catalogs.rs"]
mod catalogs;
pub use catalogs::{CatalogPins, collect_catalogs, collect_catalogs_confined};
