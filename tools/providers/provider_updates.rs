//! Explicit public-evidence maintenance; no serving, wallet or payment paths.
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use futures_util::{Stream, StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
use x402_treazury::{catalog::Config, network};

include!("../../src/providers/assets.rs");
#[path = "update_candidates.rs"]
mod candidates;
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
    selected: BTreeMap<String, String>,
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
    alloy_primitives::hex::encode(Sha256::digest(bytes))
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
// A stopped capture leaves either complete snapshot; no recovery journal is needed.
fn publish_capture(run: &Path, report: &Run) -> Result<()> {
    let mut snapshot = tempfile::NamedTempFile::new_in(run)?;
    serde_json::to_writer_pretty(snapshot.as_file_mut(), report)?;
    snapshot.persist(run.join("capture.json"))?;
    Ok(())
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
fn pending_read(source: &Source) -> Read {
    let valid = public_source(&source.url).is_ok();
    Read {
        source: if valid {
            source.url.clone()
        } else {
            "invalid public evidence URL".into()
        },
        purpose: source.purpose.clone(),
        hash: None,
        outcome: if valid {
            "offline: not attempted"
        } else {
            "source unavailable: invalid public evidence URL"
        }
        .into(),
    }
}
// Only fixed diagnostic categories are persisted; reqwest errors may contain URLs/prose.
async fn remote(run: &Path, source: &Source, cfg: &Config) -> Result<Read> {
    let (digest, outcome) = match download(&source.url, cfg).await {
        Ok(raw) => (Some(store(run, &raw)?), "captured"),
        Err(_) => (
            None,
            "source unavailable: public read failed (no payment attempted)",
        ),
    };
    Ok(Read {
        source: source.url.clone(),
        purpose: source.purpose.clone(),
        hash: digest,
        outcome: outcome.into(),
    })
}
async fn capture_provider(
    run: &Path,
    id: &str,
    path: &str,
    evidence: Option<Evidence>,
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
        inputs.push(pending_read(&Source {
            url: cfg.spec.clone(),
            purpose: "schema".into(),
        }));
        None
    } else {
        Some(local(run, &cfg.spec, "committed asset")?)
    };
    if let Some(url) = &cfg.help_url {
        inputs.push(pending_read(&Source {
            url: url.clone(),
            purpose: "help".into(),
        }));
    }
    if let Some(metadata) = &evidence {
        let guidance = metadata
            .guidance
            .split('#')
            .next()
            .context("missing guidance path")?;
        inputs.push(local(run, guidance, "maintenance guidance")?);
        for source in &metadata.sources {
            inputs.push(pending_read(source));
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
    let metadata_raw = std::fs::read("tools/providers/evidence.toml")?;
    let metadata: BTreeMap<String, Evidence> = toml::from_str(std::str::from_utf8(&metadata_raw)?)?;
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
        metadata: Read {
            source: "tools/providers/evidence.toml".into(),
            purpose: "maintenance metadata".into(),
            hash: Some(store(&run, &metadata_raw)?),
            outcome: "captured".into(),
        },
        selected: BTreeMap::new(),
        providers: vec![],
    };
    let selected: Vec<_> = inventory
        .into_iter()
        .filter(|(id, _)| ids.is_empty() || ids.iter().any(|selected| selected == id))
        .collect();
    capture_entries(&run, &mut report, &selected, &metadata, offline).await?;
    Ok(run)
}
async fn capture_entries(
    run: &Path,
    report: &mut Run,
    selected: &[(&str, &str)],
    metadata: &BTreeMap<String, Evidence>,
    offline: bool,
) -> Result<()> {
    report.selected = selected
        .iter()
        .map(|(id, path)| (id.to_string(), path.to_string()))
        .collect();
    publish_capture(run, report)?;
    // Resolve every local input before waiting for any remote evidence.
    for &(id, path) in selected {
        let evidence = metadata.get(id).cloned();
        let provider = match capture_provider(run, id, path, evidence.clone()).await {
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
        report.providers.push(provider);
        publish_capture(run, report)?;
    }
    if !offline {
        capture_remote(run, report).await?;
    }
    for provider in &report.providers {
        eprintln!("{}: {}", provider.id, provider.outcome);
    }
    publish_capture(run, report)?;
    Ok(())
}
fn pending_reads(providers: &[Provider]) -> BTreeMap<(String, bool, bool), (Source, Config)> {
    let mut pending = BTreeMap::new();
    for provider in providers {
        if let Some(cfg) = &provider.config {
            for read in provider
                .inputs
                .iter()
                .filter(|r| r.hash.is_none() && public_source(&r.source).is_ok())
            {
                pending
                    .entry((read.source.clone(), cfg.allow_http1, cfg.allow_tls12))
                    .or_insert_with(|| {
                        (
                            Source {
                                url: read.source.clone(),
                                purpose: read.purpose.clone(),
                            },
                            cfg.clone(),
                        )
                    });
            }
        }
    }
    pending
}
async fn capture_remote(run: &Path, report: &mut Run) -> Result<()> {
    let reads = pending_reads(&report.providers)
        .into_iter()
        .map(|(key, (source, cfg))| async move {
            let read = remote(run, &source, &cfg).await?;
            Ok((key, read))
        })
        .collect::<FuturesUnordered<_>>();
    capture_completions(run, report, reads).await
}
async fn capture_completions(
    run: &Path,
    report: &mut Run,
    reads: impl Stream<Item = Result<((String, bool, bool), Read)>>,
) -> Result<()> {
    futures_util::pin_mut!(reads);
    while let Some(result) = reads.next().await {
        let ((url, http1, tls12), captured) = result?;
        for provider in &mut report.providers {
            let Some(cfg) = &provider.config else {
                continue;
            };
            if cfg.allow_http1 != http1 || cfg.allow_tls12 != tls12 {
                continue;
            }
            for read in &mut provider.inputs {
                if read.hash.is_none() && read.source == url {
                    read.hash = captured.hash.clone();
                    read.outcome = captured.outcome.clone();
                }
            }
            provider.outcome = if provider.inputs.iter().any(|r| r.hash.is_none()) {
                "source unavailable"
            } else if provider.asset.is_some() && provider.evidence.is_none() {
                "missing maintenance metadata"
            } else {
                "captured; review pending"
            }
            .into();
        }
        publish_capture(run, report)?;
        eprintln!("Public evidence read finished: {}", captured.outcome);
    }
    Ok(())
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
        let provider = capture_provider(&run, "synthetic", path.to_str().unwrap(), None)
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
        let read = pending_read(&Source {
            url: "https://example.com/openapi.json".into(),
            purpose: "schema".into(),
        });
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
        let provider = capture_provider(dir.path(), "demo", path.to_str().unwrap(), None)
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
            capture_provider(dir.path(), "demo", path.to_str().unwrap(), None,)
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
    #[tokio::test]
    async fn interrupted_capture_replays_completed_reads_and_shared_aliases() {
        use std::task::Poll;
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let url = "https://example.com/schema";
        let pending_url = "https://example.com/pending";
        let provider = |id: &str, url: &str| Provider {
            id: id.into(),
            path: format!("{id}.toml"),
            config: Some(Config {
                spec: url.into(),
                prefix: Some("demo".into()),
                ..Default::default()
            }),
            inputs: vec![Read {
                source: url.into(),
                purpose: "schema".into(),
                hash: None,
                outcome: "offline: not attempted".into(),
            }],
            asset: None,
            evidence: None,
            outcome: "source unavailable".into(),
        };
        let mut report = Run {
            format: 1,
            version: "test".into(),
            tool_hash: "test".into(),
            baseline: "test".into(),
            dirty: String::new(),
            network: serde_json::json!({}),
            metadata: Read {
                source: "metadata".into(),
                purpose: "test".into(),
                hash: Some(store(run, b"metadata").unwrap()),
                outcome: "captured".into(),
            },
            selected: [
                ("first".into(), "first.toml".into()),
                ("alias".into(), "alias.toml".into()),
                ("pending".into(), "pending.toml".into()),
            ]
            .into(),
            providers: vec![
                provider("first", url),
                provider("alias", url),
                provider("pending", pending_url),
            ],
        };
        let mut different = provider("different", url);
        different.config.as_mut().unwrap().allow_http1 = true;
        report
            .selected
            .insert("different".into(), "different.toml".into());
        report.providers.push(different);
        assert_eq!(pending_reads(&report.providers).len(), 3);
        publish_capture(run, &report).unwrap();
        let raw = br#"{"openapi":"3.0.0","paths":{"/one":{"get":{}}}}"#;
        let mut finished = Some(Ok((
            (url.to_owned(), false, false),
            Read {
                source: url.into(),
                purpose: "schema".into(),
                hash: Some(store(run, raw).unwrap()),
                outcome: "captured".into(),
            },
        )));
        let (polled, pending) = tokio::sync::oneshot::channel();
        let mut polled = Some(polled);
        let reads = futures_util::stream::poll_fn(move |_| {
            if let Some(read) = finished.take() {
                return Poll::Ready(Some(read));
            }
            // Polling the sibling proves the preceding result was published.
            if let Some(polled) = polled.take() {
                polled.send(()).unwrap();
            }
            Poll::Pending
        });
        {
            let capture = capture_completions(run, &mut report, reads);
            tokio::pin!(capture);
            tokio::select! {
                result = &mut capture => panic!("pending sibling unexpectedly completed: {result:?}"),
                result = pending => result.unwrap(),
            }
        } // Cancel while the sibling is still pending.
        let frozen = replay(run).unwrap();
        assert_eq!(frozen.selected.len(), 4);
        for provider in &frozen.providers[..2] {
            assert_eq!(bytes(run, &provider.inputs[0]).unwrap(), raw);
            assert_eq!(provider.outcome, "captured; review pending");
        }
        assert!(frozen.providers[2].inputs[0].hash.is_none());
        assert!(
            frozen.providers[3].inputs[0].hash.is_none(),
            "incompatible transport must not receive the completed alias evidence"
        );
        compare::review(run, None).unwrap();
        let summary: serde_json::Value =
            serde_json::from_slice(&std::fs::read(run.join("review.json")).unwrap()).unwrap();
        assert_eq!(
            summary["providers"][0]["outcome"],
            "baseline established; current consistency review needed"
        );
        assert_eq!(
            summary["providers"][1]["outcome"],
            "baseline established; current consistency review needed"
        );
        assert_eq!(summary["providers"][2]["outcome"], "source unavailable");
        assert_eq!(summary["providers"][3]["outcome"], "source unavailable");
        assert_eq!(summary["accepted"], 0);
    }
    #[tokio::test]
    async fn shared_synthetic_sweep_remote_pinned_authored_and_failed_sibling() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        std::fs::create_dir(&run).unwrap();
        let raw=serde_json::to_vec(&json!({"openapi":"3.0.0","paths":{"/one":{"get":{"description":"upstream","responses":{"200":{"description":"full response"}}}}}})).unwrap();
        let guidance = dir.path().join("guide.md");
        std::fs::write(&guidance, "Authored maintenance evidence").unwrap();
        std::fs::write(dir.path().join("asset.json"), &raw).unwrap();
        let remote_path = dir.path().join("remote.toml");
        let pinned_path = dir.path().join("pinned.toml");
        let authored_path = dir.path().join("authored.toml");
        let failed_path = dir.path().join("failed.toml");
        std::fs::write(&remote_path,"spec='https://example.com/openapi.json'\nprefix='demo'\n[overrides.demo_one]\ndescription='authored description'\n").unwrap();
        for path in [&pinned_path, &authored_path] {
            std::fs::write(path, "spec='asset.json'\nprefix='demo'\n").unwrap();
        }
        std::fs::write(&failed_path, "broken TOML").unwrap();
        let schema = Source {
            url: "https://example.com/openapi.json".into(),
            purpose: "schema".into(),
        };
        let registry = Source {
            url: "https://example.com/registry".into(),
            purpose: "registry".into(),
        };
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "pinned".into(),
            Evidence {
                intent: Intent::VendorCopy,
                guidance: guidance.to_str().unwrap().into(),
                sources: vec![schema.clone()],
            },
        );
        metadata.insert(
            "authored".into(),
            Evidence {
                intent: Intent::Authored,
                guidance: guidance.to_str().unwrap().into(),
                sources: vec![schema.clone(), registry.clone()],
            },
        );
        let mut report = Run {
            format: 1,
            version: "test".into(),
            tool_hash: "test".into(),
            baseline: "test".into(),
            dirty: String::new(),
            network: json!({}),
            metadata: local(&run, guidance.to_str().unwrap(), "test metadata").unwrap(),
            selected: BTreeMap::new(),
            providers: vec![],
        };
        let paths = [
            ("remote", remote_path.to_str().unwrap()),
            ("failed", failed_path.to_str().unwrap()),
            ("pinned", pinned_path.to_str().unwrap()),
            ("authored", authored_path.to_str().unwrap()),
        ];
        capture_entries(&run, &mut report, &paths, &metadata, true)
            .await
            .unwrap();
        assert_eq!(report.providers.len(), 4);
        assert_eq!(pending_reads(&report.providers).len(), 2);
        let reads = [
            (schema, raw.as_slice()),
            (registry, b"{\"aliases\":[]}".as_slice()),
        ]
        .into_iter()
        .map(|(source, data)| {
            Ok((
                (source.url.clone(), false, false),
                Read {
                    source: source.url,
                    purpose: source.purpose,
                    hash: Some(store(&run, data).unwrap()),
                    outcome: "captured".into(),
                },
            ))
        })
        .collect::<Vec<_>>();
        capture_completions(&run, &mut report, futures_util::stream::iter(reads))
            .await
            .unwrap();
        assert!(report.providers[1].config.is_none());
        assert_eq!(
            report.providers[0].inputs[1].hash,
            report.providers[2].inputs.last().unwrap().hash
        );
        let frozen = std::fs::read(run.join("capture.json")).unwrap();
        std::fs::write(&remote_path, "intervening edit").unwrap();
        compare::review(&run, None).unwrap();
        let first = std::fs::read(run.join("review.json")).unwrap();
        compare::review(&run, None).unwrap();
        assert_eq!(first, std::fs::read(run.join("review.json")).unwrap());
        let summary: serde_json::Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(summary["attempted"], 4);
        assert_eq!(summary["accepted"], 0);
        assert_eq!(
            summary["providers"][2]["outcome"],
            "candidate ready; human review pending"
        );
        assert_eq!(std::fs::read(run.join("candidate-2.json")).unwrap(), raw);
        assert_eq!(std::fs::read(run.join("capture.json")).unwrap(), frozen);
        assert_eq!(
            std::fs::read_to_string(&remote_path).unwrap(),
            "intervening edit"
        );
        let packet: serde_json::Value =
            serde_json::from_slice(&std::fs::read(run.join("review-3.json")).unwrap()).unwrap();
        assert_eq!(packet["candidate"]["accepted"], false);
        assert!(!run.join("candidate-3.json").exists());
        // Two declarations share one URL/policy; differing transport remains distinct.
        let mut shared = report.providers[0].clone();
        shared.inputs[1].hash = None;
        let mut different = shared.clone();
        different.config.as_mut().unwrap().allow_http1 = true;
        assert_eq!(pending_reads(&[shared.clone(), shared, different]).len(), 2);
    }
}
