//! Explicit opt-in qualification tooling. Planning never opens wallets or networks.
#[path = "live_integration/files.rs"]
mod files;
#[path = "live_integration/manifest.rs"]
mod manifest;
#[path = "live_integration/planner.rs"]
mod planner;
#[path = "live_integration/preparation.rs"]
mod preparation;
#[path = "live_integration/process.rs"]
mod process;
#[path = "live_integration/registry.rs"]
mod registry;
#[path = "live_integration/schema.rs"]
mod schema;
#[cfg(test)]
#[path = "live_integration/tests.rs"]
mod tests;
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
    /// Initialize or explicitly revise cumulative registry authority; no spending.
    AuthorizeRegistry {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        authorization: PathBuf,
    },
    /// Archive an offline plan/config/binary identity; not yet executable qualification.
    Prepare {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Freeze catalogs and validate arguments through the pinned unsigned executable.
    PrepareCatalogs {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Review and append pin revisions before any case is reserved.
    RevisePins {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        run: String,
        #[arg(long)]
        expected_digest: String,
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
    },
    /// Report case eligibility only; never dispatches reserved/interrupted work.
    Resume {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        run: String,
    },
    /// Same read-only summary as report, without creating an output file.
    Status {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        run: Option<String>,
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
            let mut registry = registry::Registry::open(&state_dir, false)?;
            let (plan, pins) = preparation::collect(&input, &state_dir).await?;
            registry.prepare(&plan, &pins, now)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&registry.report(Some(&input.run_id), now)?)?
            );
        }
        Command::PrepareCatalogs {
            state_dir,
            manifest,
        } => {
            let input = manifest::Manifest::load(&manifest).await?;
            let mut registry = registry::Registry::open(&state_dir, false)?;
            let (plan, pins) = preparation::collect_catalogs(&input, &state_dir).await?;
            registry.prepare(&plan, &pins, now)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&registry.report(Some(&input.run_id), now)?)?
            );
        }
        Command::RevisePins {
            state_dir,
            run,
            expected_digest,
        } => {
            let mut registry = registry::Registry::open(&state_dir, false)?;
            let input = registry.manifest(&run)?;
            let (_, pins) = preparation::collect(&input, &state_dir).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&registry.revise(
                    &run,
                    &pins,
                    &expected_digest,
                    now
                )?)?
            );
        }
        Command::Report {
            state_dir,
            run,
            output,
        } => {
            let registry = registry::Registry::open(&state_dir, true)?;
            let report = registry.report(run.as_deref(), now)?;
            let bytes = serde_json::to_vec_pretty(&report)?;
            if let Some(path) = output {
                files::publish(&path, &bytes)?;
            }
            println!("{}", String::from_utf8(bytes)?);
        }
        Command::Status { state_dir, run } => {
            let registry = registry::Registry::open(&state_dir, true)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&registry.report(run.as_deref(), now)?)?
            );
        }
        Command::Resume { state_dir, run } => {
            let registry = registry::Registry::open(&state_dir, true)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&registry.report(Some(&run), now)?)?
            );
            eprintln!("observation complete; execution is not implemented (no cases dispatched)");
            return Ok(3);
        }
        Command::Plan { manifest } => {
            let input = manifest::Manifest::load(&manifest).await?;
            let plan = planner::plan(&input).await?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    }
    Ok(0)
}
