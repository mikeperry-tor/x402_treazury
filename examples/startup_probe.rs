//! Explicit read-only startup measurement: no signers, wallets, listeners or paid calls.
use anyhow::Result;
use clap::Parser;
use serde_json::json;
use std::{path::PathBuf, time::Instant};
use x402_treazury::deployment::Deployment;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    meta_config: PathBuf,
    #[arg(long)]
    catalog_only: bool,
}
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("warn,x402_treazury::startup=info,x402_treazury::pricing=debug")
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args = Args::parse();
    let started = Instant::now();
    let mut deployment = Deployment::load(&args.meta_config).await?;
    let catalog_ms = started.elapsed().as_millis();
    let pricing_started = Instant::now();
    if !args.catalog_only {
        deployment.discover_prices().await?;
    }
    let pricing_ms = pricing_started.elapsed().as_millis();
    let inventory = deployment.inventory();
    println!(
        "{}",
        json!({"catalog_ms":catalog_ms,"pricing_ms":pricing_ms,
        "total_ms":started.elapsed().as_millis(),"catalog_only":args.catalog_only,
        "tool_count":inventory.iter().map(|i| i.tools.len()).sum::<usize>()})
    );
    Ok(())
}
