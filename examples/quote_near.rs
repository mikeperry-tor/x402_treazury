//! Read-only route demo: public test addresses, dry=true, no credentials or wallet.
use anyhow::Result;
use x402_treazury::rotation::{
    base::now,
    near::{Limits, NearClient, request},
};
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    #[derive(clap::Parser)]
    struct Args {
        #[arg(long)]
        network_config: Option<std::path::PathBuf>,
        /// Exact destination USDC amount; this remains a dry quote.
        #[arg(long, default_value = "1.00")]
        amount_usdc: String,
        /// Print bounded public API diagnostics for this synthetic dry request.
        #[arg(long)]
        diagnose: bool,
    }
    let args = <Args as clap::Parser>::parse();
    if let Some(path) = &args.network_config {
        x402_treazury::network::install(x402_treazury::network::NetworkPolicy::load(path)?)?;
    }
    let amount = x402_treazury::rotation::config::positive_usdc(&args.amount_usdc)?;
    let client = NearClient::new(None)?;
    let assets = client.assets().await?;
    let instant = now()?;
    let request = request(
        &assets,
        "0x0000000000000000000000000000000000000001",
        "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F",
        &amount.to_string(),
        "public",
        100,
        instant + 1800,
        true,
    )?;
    if args.diagnose {
        let origin = "https://1click.chaindefuser.com/v0/quote";
        let http = x402_treazury::network::global().http(
            &x402_treazury::network::IsolationId::evm(
                "0x0000000000000000000000000000000000000001",
            )?,
            origin,
            std::time::Duration::from_secs(30),
        )?;
        let mut response = http.post(origin).json(&request).send().await?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                chunk.len() <= 2_000_000usize.saturating_sub(bytes.len()),
                "dry quote diagnostic exceeds 2000000-byte limit; response rejected"
            );
            bytes.extend_from_slice(&chunk);
        }
        println!(
            "{}",
            serde_json::json!({"dry":true,"http_status":status.as_u16(),
            "destination_usdc":args.amount_usdc,"diagnostic_only":true,
            "response":serde_json::from_slice::<serde_json::Value>(&bytes)?})
        );
        return Ok(());
    }
    let quote = client
        .quote(
            request,
            &Limits {
                max_input: 2_000_000,
                max_fee: 100_000,
                max_fee_bps: 500,
            },
            instant,
        )
        .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "mode":"public", "dry":true, "destination_usdc":args.amount_usdc,
            "input_zatoshis":quote.input, "input_usd":quote.response["quote"]["amountInUsd"],
            "local_deadline":quote.deadline, "deposit_allocated":quote.deposit.is_some()
        }))?
    );
    Ok(())
}
