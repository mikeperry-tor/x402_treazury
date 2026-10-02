//! Reject indexers that omit the activated Ironwood commitment tree.
use anyhow::{Result, ensure};
use std::time::Duration;
use zcash_protocol::consensus::{NetworkUpgrade, Parameters};
use zingo_netutils::{GrpcIndexer, Indexer, lightwallet_protocol::BlockId};

pub(crate) async fn check(
    client: &mut impl Indexer,
    chain: &impl Parameters,
    height: u64,
) -> Result<()> {
    let height32 = u32::try_from(height)
        .map_err(|_| anyhow::anyhow!("indexer height exceeds supported range"))?;
    if !chain.is_nu_active(NetworkUpgrade::Nu6_3, height32.into()) {
        return Ok(());
    }
    let tree = client
        .get_tree_state(
            BlockId {
                height,
                hash: vec![],
            },
            Duration::from_secs(15),
        )
        .await
        .map_err(|_| anyhow::anyhow!("indexer Ironwood capability check failed"))?;
    ensure!(
        tree.height == height,
        "indexer returned the wrong Ironwood tree height"
    );
    ensure!(
        !tree.ironwood_tree.is_empty(),
        "indexer missing Ironwood support"
    );
    Ok(())
}

pub(crate) async fn check_endpoint(
    endpoint: &str,
    chain: &impl Parameters,
    height: u64,
) -> Result<()> {
    if !chain.is_nu_active(NetworkUpgrade::Nu6_3, u32::try_from(height)?.into()) {
        return Ok(());
    }
    tokio::time::timeout(Duration::from_secs(30), async {
        let uri = endpoint
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid indexer URI"))?;
        let mut client = tokio::time::timeout(Duration::from_secs(15), GrpcIndexer::new(uri))
            .await
            .map_err(|_| anyhow::anyhow!("indexer capability connection timed out"))?
            .map_err(|_| anyhow::anyhow!("indexer capability connection failed"))?;
        check(&mut client, chain, height).await
    })
    .await
    .map_err(|_| anyhow::anyhow!("indexer Ironwood capability check timed out"))?
}
