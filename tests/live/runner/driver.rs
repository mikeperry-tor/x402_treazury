#[path = "accounting_baseline.rs"]
mod accounting_baseline;
// Shared at crate root by the executable example and its real-executable integration tests.
#[path = "accounting.rs"]
mod accounting;
#[path = "accounting_attribution.rs"]
mod accounting_attribution;
#[path = "catalog_stages.rs"]
mod catalog_stages;
#[path = "concurrency.rs"]
mod concurrency;
#[path = "cover_fixture.rs"]
mod cover_fixture;
#[path = "execution.rs"]
mod execution;
#[path = "fees.rs"]
mod fees;
#[path = "files.rs"]
mod files;
#[path = "help.rs"]
mod help;
#[path = "manifest.rs"]
mod manifest;
#[path = "offers.rs"]
mod offers;
#[path = "planner.rs"]
mod planner;
#[path = "preparation.rs"]
mod preparation;
#[cfg(test)]
#[path = "presets.rs"]
mod presets;
#[path = "pricing_stages.rs"]
mod pricing_stages;
#[path = "process.rs"]
mod process;
#[path = "receipt_recovery.rs"]
mod receipt_recovery;
#[path = "registry.rs"]
mod registry;
#[path = "reliability.rs"]
mod reliability;
#[path = "report_model.rs"]
mod report_model;
#[path = "reporting.rs"]
mod reporting;
#[path = "restart_report.rs"]
mod restart_report;
#[path = "resumption.rs"]
mod resumption;
#[path = "rotation_report.rs"]
mod rotation_report;
#[path = "schema.rs"]
mod schema;
#[path = "selection.rs"]
mod selection;
#[path = "semantics.rs"]
mod semantics;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[path = "tor.rs"]
mod tor;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Filter reviewed provider cases offline into a new, unauthorised manifest.
    SelectCases(selection::Options),
    /// Observe recorded seller receipts through owned Tor; no keys, signing or paid retries.
    ReconcileReceipts {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        run: String,
        #[arg(long)]
        evidence_dir: PathBuf,
    },
    #[command(hide = true)]
    ObserveReceipts {
        #[arg(long)]
        task: PathBuf,
        #[arg(long)]
        digest: String,
    },
    /// Write a zero-spend unsigned cover experiment; performs no live requests.
    CoverFixture(cover_fixture::Options),
    /// Qualify macOS egress confinement using loopback-only positive/negative controls.
    QualifyConfinement {
        #[arg(long)]
        evidence_dir: PathBuf,
        #[arg(long)]
        socks_port: u16,
        #[arg(long, required = true)]
        mcp_port: Vec<u16>,
    },
    /// Re-audit retained Tor events offline; never executes or replays paid cases.
    AuditTor {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        run: String,
        /// New private result file; existing records remain unchanged.
        #[arg(long)]
        output: PathBuf,
    },
    /// Internal loopback-only confinement probe; never contacts a provider.
    #[command(hide = true)]
    EgressProbe {
        #[arg(long)]
        tcp4: std::net::SocketAddr,
        #[arg(long)]
        tcp6: std::net::SocketAddr,
        #[arg(long)]
        udp4: std::net::SocketAddr,
        #[arg(long)]
        udp6: std::net::SocketAddr,
    },
    /// Initialize or explicitly revise cumulative registry authority; no spending.
    AuthorizeRegistry {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        authorization: PathBuf,
    },
    /// Freeze catalogs and validate reviewed cases; no payments or allocation.
    Prepare {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Read private registry evidence without keys, network or execution.
    Report {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        run: Option<String>,
        /// New owner-only JSON file; existing files are never overwritten.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Markdown redacts identifiers and raw evidence; requires --run.
        #[arg(long, value_enum, default_value = "json")]
        format: reporting::Format,
        /// Inspect case windows and the unstarted-run gate; never grants execution authority.
        #[arg(long, requires = "run")]
        eligibility: bool,
    },
    /// Supervise prepared MCP cases; funding requires an explicit flag and registry authority.
    Run {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        run: String,
        /// Permit registry-bounded source funding; never overrides production limits.
        #[arg(long)]
        allow_funding: bool,
    },
    /// Offline, credential-free scenario expansion; no catalogs, balances or quotes fetched.
    Plan {
        #[arg(long)]
        manifest: PathBuf,
    },
}
#[tokio::main]
async fn main() {
    let code = match run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("live integration: {error:#}");
            4
        }
    };
    if code != 0 {
        std::process::exit(code.into());
    }
}
async fn run() -> Result<u8> {
    execute(Args::parse().command).await
}
async fn execute(command: Command) -> Result<u8> {
    let now = i64::try_from(x402_treazury::rotation::base::now()?)?;
    match command {
        Command::SelectCases(options) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&selection::write(options).await?)?
            );
        }
        Command::ReconcileReceipts {
            state_dir,
            run,
            evidence_dir,
        } => {
            return receipt_recovery::reconcile(&state_dir, &run, &evidence_dir).await;
        }
        Command::ObserveReceipts { task, digest } => {
            return receipt_recovery::observe(&task, &digest).await;
        }
        Command::CoverFixture(options) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&cover_fixture::write(options)?)?
            );
            return Ok(0);
        }
        Command::QualifyConfinement {
            evidence_dir,
            socks_port,
            mcp_port,
        } => {
            files::create_dir(&evidence_dir)?;
            let profile = evidence_dir.join("client.sb");
            files::publish(
                &profile,
                tor::confinement::profile(socks_port, &mcp_port)?.as_bytes(),
            )?;
            let result = tor::confinement::qualify(
                &std::env::current_exe()?.canonicalize()?,
                &profile,
                &evidence_dir,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::AuditTor {
            state_dir,
            run,
            output,
        } => {
            return tor::archive::audit_run(&state_dir, &run, &output);
        }
        Command::EgressProbe {
            tcp4,
            tcp6,
            udp4,
            udp6,
        } => {
            let mut results = std::collections::BTreeMap::new();
            for (name, address, udp) in [
                ("tcp4", tcp4, false),
                ("tcp6", tcp6, false),
                ("udp4", udp4, true),
                ("udp6", udp6, true),
            ] {
                results.insert(
                    name,
                    x402_treazury::network::qualification_probe(address, udp).await?,
                );
            }
            println!("{}", serde_json::to_string(&results)?);
        }
        Command::AuthorizeRegistry {
            state_dir,
            authorization,
        } => {
            let auth: registry::Authorization =
                toml::from_str(std::str::from_utf8(&files::read(&authorization)?)?)?;
            let registry = registry::Registry::authorize(&state_dir, &auth, now)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&registry.report(None, now)?)?
            );
        }
        Command::Prepare {
            state_dir,
            manifest,
        } => {
            let input = manifest::Manifest::load(&manifest).await?;
            if input.network.tor_mode == manifest::TorMode::Owned {
                return tor::session::prepare(&state_dir, &input).await;
            }
            let mut registry = registry::Registry::open(&state_dir, false)?;
            let (plan, pins) = preparation::collect_catalogs(&input, &state_dir).await?;
            registry.prepare(&plan, &pins, now)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&registry.report(Some(&input.run_id), now)?)?
            );
        }
        Command::Report {
            state_dir,
            run,
            output,
            format,
            eligibility,
        } => {
            let registry = registry::Registry::open(&state_dir, true)?;
            let mut report = registry.report(run.as_deref(), now)?;
            if eligibility {
                let id = run
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("eligibility requires one --run"))?;
                let unstarted = registry.require_unstarted(id).is_ok();
                report["eligibility"] = match resumption::inspect(
                    &registry.manifest(id)?,
                    &report,
                    u64::try_from(now)?,
                ) {
                    Ok(windows) => {
                        serde_json::json!({"status":"observed","run_unstarted":unstarted,"execution_authorized":false,"windows":windows})
                    }
                    Err(error) => {
                        serde_json::json!({"status":"refused","run_unstarted":unstarted,"execution_authorized":false,"reason":error.to_string()})
                    }
                };
            }
            let bytes = match format {
                reporting::Format::Json => serde_json::to_vec_pretty(&report)?,
                reporting::Format::Markdown => {
                    anyhow::ensure!(run.is_some(), "Markdown reporting requires one --run");
                    reporting::markdown(&report)?.into_bytes()
                }
            };
            if let Some(path) = output {
                files::publish(&path, &bytes)?;
            }
            println!("{}", String::from_utf8(bytes)?);
        }
        Command::Run {
            state_dir,
            run,
            allow_funding,
        } => {
            let registry = registry::Registry::open(&state_dir, true)?;
            registry.require_unstarted(&run)?;
            let input = registry.manifest(&run)?;
            drop(registry);
            return if input.network.tor_mode == manifest::TorMode::Owned {
                tor::session::run(&state_dir, &input, allow_funding).await
            } else {
                execution::run_with_funding(&state_dir, &run, allow_funding).await
            };
        }
        Command::Plan { manifest } => {
            let input = manifest::Manifest::load(&manifest).await?;
            let plan = planner::plan(&input).await?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    }
    Ok(0)
}
