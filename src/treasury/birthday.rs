//! Read-only discovery before creating a new seed or any persistent state.
use anyhow::{Context, Result, ensure};
use std::time::Duration;
use zingo_netutils::Indexer;

/// Public mainnet endpoint used by the reference Zodl wallets.
pub const DEFAULT_INDEXER: &str = crate::rotation::config::DEFAULT_ZCASH_INDEXER;
/// Scan a small overlap to tolerate ordinary tip movement and shallow reorgs.
pub const BIRTHDAY_REWIND: u32 = 100;

pub async fn discover(endpoint: &str) -> Result<u32> {
    crate::rotation::base::secure_endpoint(endpoint)?;
    {
        tracing::info!("wallet init: connecting to birthday indexer");
        let mut client = crate::network::global()
            .grpc(&crate::network::IsolationId::bootstrap(), endpoint)
            .await
            .context("birthday indexer connection failed before wallet creation; check indexer configuration (--indexer-url-env, ZCASH_INDEXER_URL or treasury indexer settings), endpoint reachability and TLS; in Tor mode also check the SOCKS proxy")?;
        tracing::info!("wallet init: birthday indexer connected; checking mainnet network");
        let info = client
            .get_lightd_info(Duration::from_secs(15))
            .await
            .map_err(|status| super::diagnostics::tip_failure(status, "birthday_network_check"))
            .context("birthday network check failed before wallet creation")?;
        ensure!(
            info.chain_name == "main",
            "birthday indexer is not on mainnet"
        );
        tracing::info!("wallet init: mainnet network verified; querying chain tip");
        let tip = client
            .get_latest_block(Duration::from_secs(15))
            .await
            .map_err(|status| super::diagnostics::tip_failure(status, "birthday_tip_query"))
            .context("birthday tip query failed before wallet creation")?;
        // These are separate observations, potentially separated by a slow but
        // progressing RPC. Selecting the older height only increases scanning;
        // it does not require a quiet chain or grant spending authority.
        let tip = u32::try_from(tip.height.min(info.block_height))
            .context("birthday height exceeds supported range")?;
        ensure!(
            tip > BIRTHDAY_REWIND,
            "birthday indexer returned an invalid mainnet height"
        );
        tracing::info!(
            height = tip,
            "wallet init: chain tip received; checking indexer capabilities"
        );
        super::server::check(
            &mut client,
            &zingolib::config::ChainType::Mainnet,
            u64::from(tip),
        )
        .await?;
        tracing::info!(
            birthday = tip - BIRTHDAY_REWIND,
            rewind_blocks = BIRTHDAY_REWIND,
            "wallet init: indexer checks completed; birthday selected"
        );
        Ok(tip - BIRTHDAY_REWIND)
    }
}
