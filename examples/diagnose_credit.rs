//! Read-only Base verification against public wallet identities from an existing
//! state directory. Never opens the encryption key, signs, funds, or updates state.
use anyhow::{Context, Result};
use clap::Parser;
use std::{path::PathBuf, time::Duration};
use x402_treazury::{
    network::{self, NetworkPolicy},
    rotation::{
        base::{BaseRpc, ChainQuery, safe_diagnostic},
        store,
    },
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    network_config: PathBuf,
    #[arg(long)]
    state_dir: PathBuf,
    /// Explicit primary (no implicit fallbacks when supplied). Omit for PublicNode/dRPC/Base defaults.
    #[arg(long)]
    rpc_url: Option<String>,
    /// Explicit read-only fallback endpoints, in order; repeat at most twice.
    #[arg(long)]
    fallback_rpc_url: Vec<String>,
    #[arg(long, default_value_t=3, value_parser=clap::value_parser!(u32).range(1..=20))]
    rounds: u32,
    /// Maximum simultaneous views; queued views are delayed, never dropped.
    #[arg(long, default_value_t=4, value_parser=clap::value_parser!(u32).range(1..=16))]
    concurrency: u32,
}
#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("warn")
        .init();
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("diagnose-credit: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
async fn run() -> Result<()> {
    let args = Args::parse();
    network::install(NetworkPolicy::load(&args.network_config)?)?;
    let state = store::status(&args.state_dir)?;
    anyhow::ensure!(!state.pools.is_empty(), "no pools");
    let urls = if let Some(primary) = args.rpc_url {
        std::iter::once(primary)
            .chain(args.fallback_rpc_url)
            .collect::<Vec<_>>()
    } else {
        anyhow::ensure!(
            args.fallback_rpc_url.is_empty(),
            "--fallback-rpc-url requires --rpc-url; default list is otherwise PublicNode/dRPC/Base"
        );
        x402_treazury::rotation::config::DEFAULT_BASE_RPC_URLS
            .iter()
            .map(|s| (*s).into())
            .collect()
    };
    let rpc = BaseRpc::with_fallbacks(&urls, 12, 120)?;
    eprintln!(
        "Read-only RPC policy: {} endpoints; wallet-address queries may use configured fallbacks",
        urls.len()
    );
    let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(args.concurrency as usize));
    eprintln!(
        "Read-only diagnostic: {} rounds, at most {} concurrent views; all queued views will be reported",
        args.rounds, args.concurrency
    );
    let mut failed = 0usize;
    for round in 0..args.rounds {
        let mut tasks = tokio::task::JoinSet::new();
        // Two independent background views plus overlapping funding credit checks.
        for (index, pool) in state.pools.iter().enumerate() {
            for lane in 0..2 {
                let query = ChainQuery {
                    wallets: pool
                        .addresses
                        .iter()
                        .map(|w| (w.id.clone(), w.address.clone()))
                        .collect(),
                    pending: vec![],
                    anchor: None,
                };
                let rpc = rpc.clone();
                let limit = limit.clone();
                tasks.spawn(async move {
                    let _permit = limit.acquire_owned().await.expect("diagnostic semaphore closed");
                    let started=std::time::Instant::now();
                    let result=rpc.view(query).await;
                    let outcome=match result {Ok(v)=>serde_json::json!({"verified_block":v.anchor.height,"balances":v.balances.values().map(ToString::to_string).collect::<Vec<_>>()}),Err(e)=>serde_json::json!({"error":safe_diagnostic(&e)})};
                    serde_json::json!({"round":round,"pool_index":index,"lane":lane,"elapsed_ms":started.elapsed().as_millis(),"outcome":outcome})
                });
            }
        }
        while let Some(result) = tasks.join_next().await {
            let result = result.context("diagnostic task failed")?;
            failed += usize::from(result["outcome"].get("error").is_some());
            println!("{result}");
        }
        if round + 1 < args.rounds {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    anyhow::ensure!(
        failed == 0,
        "{failed} read-only verification views failed; see the complete JSON records above"
    );
    Ok(())
}
