//! Read-only route demo: public test addresses, dry=true, no credentials or wallet.
use anyhow::Result;
use x402_treazury::rotation::{
    base::now,
    near::{Limits, NearClient, request},
};
#[tokio::main]
async fn main() -> Result<()> {
    #[derive(clap::Parser)]
    struct Args {
        #[arg(long)]
        network_config: Option<std::path::PathBuf>,
    }
    let args = <Args as clap::Parser>::parse();
    if let Some(path) = &args.network_config {
        x402_treazury::network::install(x402_treazury::network::NetworkPolicy::load(path)?)?;
    }
    let client = NearClient::new(None)?;
    let assets = client.assets().await?;
    let instant = now()?;
    let request = request(
        &assets,
        "0x0000000000000000000000000000000000000001",
        "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F",
        "5000000",
        "public",
        100,
        instant + 1800,
        true,
    )?;
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
            "mode":"public", "dry":true, "destination_usdc":"5.00",
            "input_zatoshis":quote.input, "input_usd":quote.response["quote"]["amountInUsd"],
            "local_deadline":quote.deadline, "deposit_allocated":quote.deposit.is_some()
        }))?
    );
    Ok(())
}
