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
        let mut client = crate::network::global()
            .grpc(&crate::network::IsolationId::bootstrap(), endpoint)
            .await
            .map_err(|_| anyhow::anyhow!("birthday indexer connection failed"))?;
        let info = client
            .get_lightd_info(Duration::from_secs(15))
            .await
            .map_err(|_| anyhow::anyhow!("birthday network check failed"))?;
        ensure!(
            info.chain_name == "main",
            "birthday indexer is not on mainnet"
        );
        let tip = client
            .get_latest_block(Duration::from_secs(15))
            .await
            .map_err(|_| anyhow::anyhow!("birthday tip query failed"))?;
        // These are separate observations, potentially separated by a slow but
        // progressing RPC. Selecting the older height only increases scanning;
        // it does not require a quiet chain or grant spending authority.
        let tip = u32::try_from(tip.height.min(info.block_height))
            .context("birthday height exceeds supported range")?;
        ensure!(
            tip > BIRTHDAY_REWIND,
            "birthday indexer returned an invalid mainnet height"
        );
        super::server::check(
            &mut client,
            &zingolib::config::ChainType::Mainnet,
            u64::from(tip),
        )
        .await?;
        Ok(tip - BIRTHDAY_REWIND)
    }
}
