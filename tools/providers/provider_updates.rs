//! Explicit public-evidence maintenance; no serving, wallet or payment paths.
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
use x402_treazury::{catalog::Config, network};

include!("../../src/providers/assets.rs");
#[path = "update_compare.rs"]
mod compare;

#[derive(Parser)]
#[command(about = "Capture complete public provider evidence or replay a frozen sweep offline")]
struct Args {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Capture {
        /// Bundled IDs; omit for the entire shipped manifest.
        #[arg(long)]
        provider: Vec<String>,
        #[arg(long)]
        network_config: Option<PathBuf>,
        /// Capture local inputs and explicitly mark remote evidence unavailable.
        #[arg(long)]
        offline: bool,
    },
    Replay {
        run: PathBuf,
        #[arg(long)]
        prior: Option<PathBuf>,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    intent: Intent,
    guidance: String,
    #[serde(default)]
    sources: Vec<Source>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Intent {
    Authored,
    VendorCopy,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Source {
    url: String,
    purpose: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Read {
    source: String,
    purpose: String,
    hash: Option<String>,
    outcome: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Provider {
    id: String,
    path: String,
    config: Option<Config>,
    inputs: Vec<Read>,
    asset: Option<Read>,
    evidence: Option<Evidence>,
    outcome: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct Run {
    format: u32,
    version: String,
    tool_hash: String,
    baseline: String,
    dirty: String,
    network: serde_json::Value,
    metadata: Read,
    providers: Vec<Provider>,
}
fn inventory() -> Vec<(&'static str, &'static str)> {
    macro_rules! entries {
        ($(($id:literal, $path:literal, $kind:ident $(($local:literal))?)),* $(,)?) => {
            vec![$(($id, $path)),*]
        };
    }
    provider_manifest!(entries)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).output()?;
    ensure!(output.status.success(), "cannot record checkout provenance");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
fn store(run: &Path, bytes: &[u8]) -> Result<String> {
    let digest = hash(bytes);
    std::fs::write(run.join(&digest), bytes)?;
    Ok(digest)
}
fn bytes(run: &Path, read: &Read) -> Result<Vec<u8>> {
    let digest = read.hash.as_deref().context("input was unavailable")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid capture hash"
    );
    let bytes = std::fs::read(run.join(digest))?;
    ensure!(hash(&bytes) == digest, "capture hash mismatch");
    Ok(bytes)
}
fn local(run: &Path, source: &str, purpose: &str) -> Result<Read> {
    let bytes = std::fs::read(source)?;
    Ok(Read {
        source: source.into(),
        purpose: purpose.into(),
        hash: Some(store(run, &bytes)?),
        outcome: "captured".into(),
    })
}
fn public_source(source: &str) -> Result<()> {
    let url = network::public_url(source)?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "evidence URL must be public and credential-free, without query or fragment"
    );
    Ok(())
}
fn complete_response(status: u16, content_range: bool) -> Result<()> {
    ensure!(
        (200..300).contains(&status) && status != 206 && !content_range,
        "complete successful response required (HTTP {status})"
    );
    Ok(())
}
async fn download(source: &str, cfg: &Config) -> Result<Vec<u8>> {
    public_source(source)?;
    let http = network::global().http_policy(
        &network::IsolationId::discovery(source)?,
        source,
        None,
        true,
        cfg.transport(),
    )?;
    let response = http.get(source).send().await?;
    complete_response(
        response.status().as_u16(),
        response
            .headers()
            .contains_key(reqwest::header::CONTENT_RANGE),
    )?;
    Ok(response.bytes().await?.to_vec())
}
// Only fixed diagnostic categories are persisted; reqwest errors may contain URLs/prose.
async fn remote(
    run: &Path,
    source: &Source,
    cfg: &Config,
    offline: bool,
    cache: &mut BTreeMap<(String, bool, bool), Read>,
) -> Result<Read> {
    let key = (source.url.clone(), cfg.allow_http1, cfg.allow_tls12);
    if let Some(read) = cache.get(&key) {
        return Ok(Read {
            purpose: source.purpose.clone(),
            ..read.clone()
        });
    }
    if public_source(&source.url).is_err() {
        return Ok(Read {
            source: "invalid public evidence URL".into(),
            purpose: source.purpose.clone(),
            hash: None,
            outcome: "source unavailable: invalid public evidence URL".into(),
        });
    }
    let (digest, outcome) = if offline {
        (None, "offline: not attempted".into())
    } else {
        match download(&source.url, cfg).await {
            Ok(raw) => (Some(store(run, &raw)?), "captured".into()),
            Err(_) => (
                None,
                "source unavailable: public read failed (no payment attempted)".into(),
            ),
        }
    };
    let read = Read {
        source: source.url.clone(),
        purpose: source.purpose.clone(),
        hash: digest,
        outcome,
    };
    cache.insert(key, read.clone());
    Ok(read)
}
async fn capture_provider(
    run: &Path,
    id: &str,
    path: &str,
    evidence: Option<Evidence>,
    offline: bool,
    cache: &mut BTreeMap<(String, bool, bool), Read>,
) -> Result<Provider> {
    let raw = std::fs::read(path)?;
    let table: toml::Table = toml::from_str(std::str::from_utf8(&raw)?)?;
    // Bundled definitions cannot extend files; never discover unselected inputs.
    ensure!(
        !table.contains_key("extends"),
        "bundled provider inheritance is unsupported"
    );
    let cfg = x402_treazury::config::resolve(table, Path::new(path))
        .await?
        .settings;
    let mut inputs = vec![Read {
        source: path.into(),
        purpose: "provider TOML".into(),
        hash: Some(store(run, &raw)?),
        outcome: "captured".into(),
    }];
    let asset = if cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://") {
        inputs.push(
            remote(
                run,
                &Source {
                    url: cfg.spec.clone(),
                    purpose: "schema".into(),
                },
                &cfg,
                offline,
                cache,
            )
            .await?,
        );
        None
    } else {
        Some(local(run, &cfg.spec, "committed asset")?)
    };
    if let Some(url) = &cfg.help_url {
        inputs.push(
            remote(
                run,
                &Source {
                    url: url.clone(),
                    purpose: "help".into(),
                },
                &cfg,
                offline,
                cache,
            )
            .await?,
        );
    }
    if let Some(metadata) = &evidence {
        let guidance = metadata
            .guidance
            .split('#')
            .next()
            .context("missing guidance path")?;
        inputs.push(local(run, guidance, "maintenance guidance")?);
        for source in &metadata.sources {
            inputs.push(remote(run, source, &cfg, offline, cache).await?);
        }
    }
    let outcome = if inputs.iter().any(|r| r.hash.is_none()) {
        "source unavailable"
    } else if asset.is_some() && evidence.is_none() {
        "missing maintenance metadata"
    } else {
        "captured; review pending"
    }
    .into();
    Ok(Provider {
        id: id.into(),
        path: path.into(),
        config: Some(cfg),
        inputs,
        asset,
        evidence,
        outcome,
    })
}
async fn capture(ids: Vec<String>, offline: bool) -> Result<PathBuf> {
    let inventory = inventory();
    for id in &ids {
        ensure!(
            inventory.iter().any(|(known, _)| known == id),
            "unknown bundled provider ID"
        );
    }
    let metadata: BTreeMap<String, Evidence> =
        toml::from_str(&std::fs::read_to_string("tools/providers/evidence.toml")?)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let run = PathBuf::from(format!("target/provider-updates/{stamp}"));
    std::fs::create_dir_all("target/provider-updates")?;
    std::fs::create_dir(&run)?;
    let mut report = Run {
        format: 1,
        version: env!("CARGO_PKG_VERSION").into(),
        tool_hash: hash(&std::fs::read(std::env::current_exe()?)?),
        baseline: git(&["rev-parse", "HEAD"])?,
        dirty: git(&["status", "--porcelain"])?,
        network: network::global().policy.inspection(),
        metadata: local(
            &run,
            "tools/providers/evidence.toml",
            "maintenance metadata",
        )?,
        providers: vec![],
    };
    let mut cache = BTreeMap::new();
    for (id, path) in inventory
        .into_iter()
        .filter(|(id, _)| ids.is_empty() || ids.iter().any(|selected| selected == id))
    {
        let evidence = metadata.get(id).cloned();
        let provider =
            match capture_provider(&run, id, path, evidence.clone(), offline, &mut cache).await {
                Ok(provider) => provider,
                Err(_) => Provider {
                    id: id.into(),
                    path: path.into(),
                    config: None,
                    inputs: vec![],
                    asset: None,
                    evidence,
                    outcome: "capture unavailable: configuration or local input invalid".into(),
                },
            };
        eprintln!("{}: {}", provider.id, provider.outcome);
        report.providers.push(provider);
        std::fs::write(
            run.join("capture.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
    }
    Ok(run)
}
fn replay(run: &Path) -> Result<Run> {
    let report: Run = serde_json::from_slice(&std::fs::read(run.join("capture.json"))?)?;
    ensure!(report.format == 1, "unsupported capture version");
    bytes(run, &report.metadata)?;
    Ok(report)
}
#[tokio::main]
async fn main() -> Result<()> {
    match Args::parse().command {
        Action::Capture {
            provider,
            network_config,
            offline,
        } => {
            if let Some(path) = network_config {
                network::install(network::NetworkPolicy::load(&path)?)?;
            }
            let run = capture(provider, offline).await?;
            println!("{}", serde_json::json!({"run":serde_json::to_value(&run)?}));
        }
        Action::Replay { run, prior } => {
            compare::review(&run, prior.as_deref())?;
            let report = replay(&run)?;
            println!(
                "{}",
                serde_json::json!({"run":serde_json::to_value(&run)?,"report":serde_json::to_value(run.join("review.json"))?,"providers":report.providers.len()})
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn frozen_local_inputs_missing_metadata_and_independent_remote_failure() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        std::fs::create_dir(&run).unwrap();
        let spec = dir.path().join("schema.json");
        std::fs::write(&spec, br#"{"paths":{}}"#).unwrap();
        let path = dir.path().join("provider.toml");
        std::fs::write(
            &path,
            "spec = 'schema.json'\nprefix = 'demo'\nhelp_url = 'https://example.com/guide'\n",
        )
        .unwrap();
        let provider = capture_provider(
            &run,
            "synthetic",
            path.to_str().unwrap(),
            None,
            true,
            &mut BTreeMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(provider.outcome, "source unavailable");
        assert_eq!(provider.inputs.last().unwrap().hash, None);
        let original = bytes(&run, provider.asset.as_ref().unwrap()).unwrap();
        std::fs::write(&spec, "changed").unwrap();
        std::fs::write(&path, "changed").unwrap();
        assert_eq!(
            bytes(&run, provider.asset.as_ref().unwrap()).unwrap(),
            original
        );
        assert_eq!(provider.config.unwrap().prefix.as_deref(), Some("demo"));
        let mut sibling_cfg = Config::default();
        sibling_cfg.spec = "https://example.com/openapi.json".into();
        let read = remote(
            &run,
            &Source {
                url: sibling_cfg.spec.clone(),
                purpose: "schema".into(),
            },
            &sibling_cfg,
            true,
            &mut BTreeMap::new(),
        )
        .await
        .unwrap();
        assert!(read.hash.is_none());
        assert_eq!(std::fs::read_to_string(&spec).unwrap(), "changed");
        let stored = local(&run, spec.to_str().unwrap(), "test").unwrap();
        std::fs::write(run.join(stored.hash.as_ref().unwrap()), "corrupt").unwrap();
        assert!(bytes(&run, &stored).is_err());
    }
    #[test]
    fn complete_capture_rejects_partial_and_payment_responses() {
        assert!(complete_response(200, false).is_ok());
        for (status, range) in [(206, false), (200, true), (402, false), (500, false)] {
            assert!(complete_response(status, range).is_err());
        }
    }
    #[tokio::test]
    async fn missing_metadata_and_frozen_config_use_exact_declaring_bytes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("spec.json"), "{}").unwrap();
        let path = dir.path().join("provider.toml");
        let raw = "spec='spec.json'\nprefix='frozen'\n";
        std::fs::write(&path, raw).unwrap();
        let provider = capture_provider(
            dir.path(),
            "demo",
            path.to_str().unwrap(),
            None,
            true,
            &mut BTreeMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(provider.outcome, "missing maintenance metadata");
        std::fs::write(&path, "prefix='changed'").unwrap();
        assert_eq!(
            bytes(dir.path(), &provider.inputs[0]).unwrap(),
            raw.as_bytes()
        );
        assert_eq!(provider.config.unwrap().prefix.as_deref(), Some("frozen"));
        std::fs::write(&path, "extends='private.toml'\nspec='spec.json'").unwrap();
        assert!(
            capture_provider(
                dir.path(),
                "demo",
                path.to_str().unwrap(),
                None,
                true,
                &mut BTreeMap::new()
            )
            .await
            .is_err()
        );
    }
    #[test]
    fn public_evidence_rejects_private_and_credential_urls() {
        for url in [
            "http://127.0.0.1/a",
            "https://user:pass@example.com/a",
            "https://example.com/a?token=x",
            "file:///tmp/a",
        ] {
            assert!(public_source(url).is_err());
        }
        assert!(public_source("https://example.com/a").is_ok());
    }
}
