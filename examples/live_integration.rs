//! Explicit opt-in qualification tooling. Planning never opens wallets or networks.
#[path = "live_integration/manifest.rs"]
mod manifest;
#[path = "live_integration/planner.rs"]
mod planner;
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
    /// Offline, credential-free scenario expansion; no catalogs, balances or quotes fetched.
    Plan {
        #[arg(long)]
        manifest: PathBuf,
    },
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("live integration: {error:#}");
        std::process::exit(4);
    }
}
async fn run() -> Result<()> {
    match Args::parse().command {
        Command::Plan { manifest } => {
            let input = manifest::Manifest::load(&manifest).await?;
            let plan = planner::plan(&input).await?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    }
    Ok(())
}
