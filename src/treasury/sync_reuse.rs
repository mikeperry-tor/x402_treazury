//! Reuse a completed scan only within its owner and against the unchanged chain.
use super::*;

impl Treasury {
    pub(crate) async fn sync_for_command(&mut self, stop: &CancellationToken) -> Result<()> {
        ensure!(!stop.is_cancelled(), "treasury sync cancelled");
        if self.can_reuse_sync(stop).await? {
            tracing::debug!(
                "treasury sync: reusing unchanged scan; original observation age retained"
            );
            return Ok(());
        }
        self.sync_once(stop).await
    }

    async fn can_reuse_sync(&self, stop: &CancellationToken) -> Result<bool> {
        if !self.healthy || self.reusable_sync_revision != Some(self.revision) {
            return Ok(false);
        }
        let saved = self.store.call(|s| s.treasury_sync_state()).await?;
        let Some(sync) = saved.sync else {
            return Ok(false);
        };
        if saved.snapshot_revision != self.revision || !sync.fresh(now()?, self.revision) {
            return Ok(false);
        }
        let Some(height) = sync.height.and_then(|h| u32::try_from(h).ok()) else {
            return Ok(false);
        };
        let client = self
            .client
            .as_ref()
            .context("treasury client unavailable")?;
        let anchor = client
            .wallet()
            .read()
            .await
            .wallet_blocks
            .get(&height.into())
            .map(|block| block.block_hash());
        let Some(anchor) = anchor else {
            return Ok(false);
        };
        let settings = self
            .sync_settings
            .as_ref()
            .context("sync endpoint not configured")?;
        let identity = crate::network::IsolationId::treasury(
            &self.store.call(|s| Ok(s.id().to_owned())).await?,
        );
        // These unsigned reads have the ordinary transport inactivity bounds.
        // Neither their completion nor reuse checkpoints/renews financial evidence.
        let observation = async {
            let mut indexer = crate::network::global()
                .grpc(&identity, &settings.endpoint)
                .await
                .context("indexer connection failed")?;
            let info = indexer
                .get_lightd_info(zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT)
                .await
                .map_err(|s| diagnostics::tip_failure(s, "reuse_network_tip"))?;
            ensure!(
                info.chain_name == self.network.rpc_name(),
                "indexer is not on mainnet"
            );
            if info.block_height != u64::from(height) {
                return Ok(false);
            }
            let tip = indexer
                .get_latest_block(zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT)
                .await
                .map_err(|s| diagnostics::tip_failure(s, "reuse_chain_anchor"))?;
            // Missing hashes and same-height reorgs both require a scan. A newer
            // tip must be scanned too, so reuse cannot delay source confirmation.
            Ok(tip.height == u64::from(height)
                && tip.hash.as_slice() == anchor.0.as_slice()
                && sync.observed_tip_height == Some(u64::from(height))
                && sync.fresh(now()?, self.revision))
        };
        let result = tokio::select! {
            biased;
            _ = stop.cancelled() => Err(anyhow::anyhow!("treasury sync cancelled")),
            result = observation => result,
        };
        crate::network::global().release_grpc(&identity).await;
        result
    }
}
