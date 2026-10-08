//! Embedded treasury with encrypted sync and explicit durable transaction operations.
pub mod actor;
pub mod birthday;
pub(crate) mod diagnostics;
mod expiry;
mod freshness;
pub mod recovery;
mod refunds;
#[cfg(all(test, feature = "zcash-regtest"))]
mod regtest;
mod send;
mod server;
use crate::rotation::store::TreasuryNetwork;
impl TreasuryNetwork {
    fn chain(self) -> zingolib::config::ChainType {
        match self {
            Self::Mainnet => zingolib::config::ChainType::Mainnet,
            #[cfg(all(test, feature = "zcash-regtest"))]
            Self::Regtest => zingolib::config::ChainType::Regtest(regtest::heights()),
        }
    }
}
pub mod submission;
use crate::rotation::{
    base::now,
    store::{Status, Store, StoreHandle, SyncObservation, SyncPhase},
};
use anyhow::{Context, Result, ensure};
use std::time::Duration;
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;
use zingo_netutils::Indexer;
use zingolib::{
    config::{ClientConfig, WalletConfig},
    lightclient::LightClient,
    wallet::{WalletSettings, keys::unified::ReceiverSelection},
};

pub struct Treasury {
    network: TreasuryNetwork,
    _scratch: tempfile::TempDir,
    client: Option<LightClient>,
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    revision: i64,
    healthy: bool,
    sync_settings: Option<SyncSettings>,
}
/// Endpoint is intentionally excluded from Debug/serialization and status output.
#[derive(Clone)]
pub struct SyncSettings {
    endpoint: String,
    confirmations: NonZeroU32,
    max_age_seconds: u64,
}
impl SyncSettings {
    pub fn new(endpoint: String, confirmations: u64, max_age_seconds: u64) -> Result<Self> {
        crate::rotation::base::secure_endpoint(&endpoint)?;
        let confirmations = NonZeroU32::new(u32::try_from(confirmations)?)
            .context("confirmations must be positive")?;
        ensure!(max_age_seconds > 0, "sync age must be positive");
        Ok(Self {
            endpoint,
            confirmations,
            max_age_seconds,
        })
    }
}
fn config(dir: &Path, wallet: WalletConfig, network: TreasuryNetwork) -> Result<ClientConfig> {
    ClientConfig::builder()
        .set_chain_type(network.chain())
        .set_wallet_dir(dir.to_path_buf())
        .set_wallet_config(wallet)
        .build()
        .map_err(|_| anyhow::anyhow!("invalid offline wallet configuration"))
}
async fn snapshot(client: &LightClient) -> Result<Zeroizing<Vec<u8>>> {
    let wallet = client.wallet();
    let mut wallet = wallet.write().await;
    wallet.mark_dirty();
    Ok(Zeroizing::new(
        wallet
            .save()?
            .context("wallet did not produce a snapshot")?,
    ))
}
impl Treasury {
    pub async fn create(
        dir: PathBuf,
        key: PathBuf,
        birthday: u32,
        mnemonic: Option<Zeroizing<String>>,
    ) -> Result<Self> {
        Self::create_with_network(dir, key, birthday, mnemonic, TreasuryNetwork::Mainnet).await
    }
    async fn create_with_network(
        dir: PathBuf,
        key: PathBuf,
        birthday: u32,
        mnemonic: Option<Zeroizing<String>>,
        network: TreasuryNetwork,
    ) -> Result<Self> {
        ensure!(birthday > 0, "birthday must be positive");
        ensure!(
            !dir.exists() && !key.exists(),
            "refusing to overwrite treasury state or key"
        );
        let wallet = match mnemonic {
            Some(seed) => WalletConfig::MnemonicPhrase {
                mnemonic_phrase: seed.trim().to_owned(),
                no_of_accounts: NonZeroU32::new(1).unwrap(),
                birthday,
                wallet_settings: WalletSettings::default(),
            },
            None => WalletConfig::NewSeed {
                no_of_accounts: NonZeroU32::new(1).unwrap(),
                chain_height: birthday,
                wallet_settings: WalletSettings::default(),
            },
        };
        let scratch = tempfile::tempdir()?;
        let mut client = LightClient::new(config(scratch.path(), wallet, network)?, false)
            .await
            .map_err(|_| anyhow::anyhow!("offline treasury initialization failed"))?;
        ensure!(
            client.indexer_uri().is_none(),
            "offline treasury must not connect to an indexer"
        );
        client
            .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
            .await
            .map_err(|_| anyhow::anyhow!("address derivation failed"))?;
        let bytes = snapshot(&client).await?;
        let store = tokio::task::spawn_blocking(move || {
            Store::create_with_network(&dir, &key, birthday, &bytes, network)
        })
        .await??;
        let (store, worker) = StoreHandle::spawn(store);
        Ok(Self {
            network,
            _scratch: scratch,
            client: Some(client),
            store,
            worker,
            revision: 1,
            healthy: true,
            sync_settings: None,
        })
    }
    pub async fn open(dir: PathBuf, key: PathBuf, id: String) -> Result<Self> {
        Self::open_with_network(dir, key, id, TreasuryNetwork::Mainnet).await
    }
    pub(crate) async fn open_with_network(
        dir: PathBuf,
        key: PathBuf,
        id: String,
        network: TreasuryNetwork,
    ) -> Result<Self> {
        let path = dir.clone();
        let (store, revision, bytes) = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut store = Store::open_with_network(&path, &key, &id, network)?;
            store.set_sync_phase(SyncPhase::Offline)?;
            let (revision, bytes) = store.snapshot()?;
            Ok((store, revision, bytes))
        })
        .await??;
        let scratch = tempfile::tempdir()?;
        // Borrow a zeroizing buffer instead of handing an unzeroized Vec to upstream.
        let client = LightClient::from_reader(
            bytes.as_slice(),
            config(scratch.path(), WalletConfig::Read, network)?,
        )
        .await
        .map_err(|_| anyhow::anyhow!("corrupt or incompatible treasury snapshot"))?;
        ensure!(
            client.indexer_uri().is_none(),
            "offline treasury must not connect to an indexer"
        );
        let (store, worker) = StoreHandle::spawn(store);
        Ok(Self {
            network,
            _scratch: scratch,
            client: Some(client),
            store,
            worker,
            revision,
            healthy: true,
            sync_settings: None,
        })
    }
    pub(crate) fn store_handle(&self) -> StoreHandle {
        self.store.clone()
    }
    pub async fn status(&self) -> Result<Status> {
        self.store.call(|s| s.status()).await
    }
    pub async fn addresses(&self) -> Result<serde_json::Value> {
        address_info(
            self.client
                .as_ref()
                .context("treasury sync interrupted; reopen required")?,
        )
        .await
    }
    /// Inspect an encrypted snapshot without changing its revision or sync readiness.
    pub async fn inspect_addresses(
        dir: PathBuf,
        key: PathBuf,
        expected_id: Option<String>,
    ) -> Result<serde_json::Value> {
        let (state, bytes) = tokio::task::spawn_blocking(move || -> Result<_> {
            let id = expected_id.unwrap_or(crate::rotation::store::status(&dir)?.treasury_id);
            let store = Store::open(&dir, &key, &id)?;
            let state = store.status()?;
            let (_, bytes) = store.snapshot()?;
            Ok((state, bytes))
        })
        .await??;
        let scratch = tempfile::tempdir()?;
        let client = LightClient::from_reader(
            bytes.as_slice(),
            config(scratch.path(), WalletConfig::Read, TreasuryNetwork::Mainnet)?,
        )
        .await
        .map_err(|_| anyhow::anyhow!("corrupt or incompatible treasury snapshot"))?;
        ensure!(
            client.indexer_uri().is_none(),
            "offline treasury must not connect to an indexer"
        );
        Ok(serde_json::json!({"state": state, "receive_addresses": address_info(&client).await?}))
    }
    pub async fn derive_address(&mut self) -> Result<serde_json::Value> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed persistence"
        );
        self.healthy = false;
        self.client
            .as_mut()
            .context("treasury sync interrupted; reopen required")?
            .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
            .await
            .map_err(|_| anyhow::anyhow!("address derivation failed"))?;
        let bytes = snapshot(
            self.client
                .as_ref()
                .context("treasury sync interrupted; reopen required")?,
        )
        .await?;
        let expected = self.revision;
        self.revision = self
            .store
            .call(move |s| s.save_snapshot(expected, &bytes))
            .await?;
        self.healthy = true;
        self.addresses().await
    }
    /// Persist the complete derivation range and its job binding before a quote
    /// can expose this address to a remote service.
    pub async fn refund_address(&mut self, job: String) -> Result<String> {
        let lookup = job.clone();
        if let Some(address) = self.store.call(move |s| s.refund_address(&lookup)).await? {
            return Ok(address);
        }
        ensure!(self.healthy, "treasury requires reopen");
        self.healthy = false;
        let client = self
            .client
            .as_mut()
            .context("treasury client unavailable")?;
        let (_, address) = client
            .generate_transparent_address(zip32::AccountId::ZERO, false)
            .await
            .map_err(|_| anyhow::anyhow!("refund derivation failed"))?;
        let address =
            zcash_keys::address::Address::Transparent(address).encode(&self.network.chain());
        let bytes = snapshot(client).await?;
        let expected = self.revision;
        let saved = address.clone();
        self.revision = self
            .store
            .call(move |s| s.save_refund_address(&job, &saved, expected, &bytes))
            .await?;
        self.healthy = true;
        Ok(address)
    }
    pub async fn ensure_pool(&self, name: String, funding_amount_usdc: String) -> Result<String> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed persistence"
        );
        self.store
            .call(move |s| s.ensure_pool(&name, &funding_amount_usdc))
            .await
    }
    pub fn configure_sync(&mut self, settings: SyncSettings) {
        self.sync_settings = Some(settings);
    }
    async fn checkpoint(&mut self, observation: SyncObservation) -> Result<()> {
        let bytes = snapshot(
            self.client
                .as_ref()
                .context("treasury sync interrupted; reopen required")?,
        )
        .await?;
        let expected = self.revision;
        self.revision = self
            .store
            .call(move |s| s.save_sync_snapshot(expected, &bytes, Some(observation)))
            .await?;
        Ok(())
    }
    /// Cancellation must be requested through `stop`, then this future awaited.
    /// Adapter errors are sanitized; endpoints are not persisted in status.
    pub async fn sync_once(&mut self, stop: &CancellationToken) -> Result<()> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed persistence"
        );
        let settings = self
            .sync_settings
            .clone()
            .context("sync endpoint not configured")?;
        self.healthy = false;
        self.client
            .as_mut()
            .context("treasury sync interrupted; reopen required")?
            .wallet()
            .write()
            .await
            .wallet_settings
            .min_confirmations = settings.confirmations;
        let mut observation = self.status().await?.sync.unwrap_or(SyncObservation {
            phase: SyncPhase::Syncing,
            last_error: None,
            snapshot_revision: self.revision,
            checked_at: None,
            checkpoint_at: 0,
            scanned_blocks: 0,
            target_height: None,
            observed_tip_height: None,
            height: None,
            confirmations: settings.confirmations.get(),
            max_age_seconds: settings.max_age_seconds,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 0,
            spendable_shielded_zatoshis: 0,
        });
        observation.phase = SyncPhase::Syncing;
        observation.confirmations = settings.confirmations.get();
        observation.max_age_seconds = settings.max_age_seconds;
        self.checkpoint(observation.clone()).await?;
        let session = SyncSession {
            identity: crate::network::IsolationId::treasury(&self.status().await?.treasury_id),
            network: self.network,
            client: self
                .client
                .take()
                .context("treasury sync interrupted; reopen required")?,
            store: self.store.clone(),
            revision: self.revision,
        };
        let cancelled = stop.child_token();
        let _cancel_on_drop = cancelled.clone().drop_guard();
        // Upstream sync launches detached fetch/mempool tasks. Isolate each cycle
        // so cancellation/error cannot leave those tasks connected on the MCP runtime.
        let (session, mut observation, outcome) =
            tokio::task::spawn_blocking(move || session.run(settings, observation, cancelled))
                .await
                .context("treasury sync worker failed")??;
        self.revision = session.revision;
        self.client = Some(session.client);
        observation.phase = if stop.is_cancelled() {
            SyncPhase::Offline
        } else if outcome.is_ok() {
            SyncPhase::Ready
        } else {
            SyncPhase::Failed
        };
        observation.last_error = outcome.as_ref().err().map(|e| e.to_string());
        self.checkpoint(observation).await?;
        self.healthy = true;
        outcome
    }
    /// One owner periodically syncs while Base payment admission remains independent.
    pub async fn run_sync(mut self, stop: CancellationToken) -> Result<()> {
        let delay = self
            .sync_settings
            .as_ref()
            .context("sync endpoint not configured")?
            .max_age_seconds
            .saturating_div(2)
            .clamp(1, 60);
        while !stop.is_cancelled() {
            if let Err(error) = self.sync_once(&stop).await {
                if !self.healthy {
                    if let Some(client) = &mut self.client {
                        client.go_offline().await;
                    }
                    stop.cancel();
                    tracing::error!("treasury persistence failed; funding disabled until restart");
                    // Persistence failure is fatal to treasury ownership, not Base payments.
                    self.close().await?;
                    return Err(error);
                }
                if !stop.is_cancelled() {
                    diagnostics::warn_sync_failure(&error);
                }
            }
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
            }
        }
        self.close().await
    }
    pub async fn close(mut self) -> Result<()> {
        if let Some(client) = &mut self.client {
            client.go_offline().await;
        }
        let result = self
            .store
            .call(|s| s.set_sync_phase(SyncPhase::Offline))
            .await;
        drop(self.store);
        self.worker.await.context("store worker failed")?;
        result
    }
}

struct SyncSession {
    identity: crate::network::IsolationId,
    network: TreasuryNetwork,
    client: LightClient,
    store: StoreHandle,
    revision: i64,
}
impl SyncSession {
    fn run(
        mut self,
        settings: SyncSettings,
        mut observation: SyncObservation,
        cancelled: CancellationToken,
    ) -> Result<(Self, SyncObservation, Result<()>)> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let outcome = runtime.block_on(async {
            let outcome = tokio::select! {
                biased;
                _ = cancelled.cancelled() => Err(anyhow::anyhow!("treasury sync cancelled")),
                result = self.sync_inner(&settings, &mut observation) => result,
            };
            self.client.go_offline().await;
            crate::network::global().release_grpc(&self.identity).await;
            outcome
        });
        // Join blocking workers and tear down detached async tasks before the
        // owner reads the final snapshot, including on unwind from an SDK panic.
        drop(runtime);
        Ok((self, observation, outcome))
    }
    async fn checkpoint(&mut self, observation: SyncObservation) -> Result<()> {
        let bytes = snapshot(&self.client).await?;
        let expected = self.revision;
        self.revision = self
            .store
            .call(move |s| s.save_sync_snapshot(expected, &bytes, Some(observation)))
            .await?;
        Ok(())
    }
    async fn sync_inner(
        &mut self,
        settings: &SyncSettings,
        observation: &mut SyncObservation,
    ) -> Result<()> {
        let mut indexer = tokio::time::timeout(
            crate::network::global().connection_timeout(Duration::from_secs(30)),
            crate::network::global().grpc(&self.identity, &settings.endpoint),
        )
        .await
        .map_err(|_| anyhow::anyhow!("indexer connection timed out"))?
        .context("indexer connection failed")?;
        self.client.set_indexer(indexer.clone());
        let info = tokio::time::timeout(
            crate::network::global().operation_timeout(Duration::from_secs(15)),
            indexer.get_lightd_info(
                crate::network::global()
                    .operation_timeout(zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT),
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("indexer tip check timed out"))?
        .map_err(|_| anyhow::anyhow!("indexer tip check failed"))?;
        ensure!(
            info.chain_name == self.network.rpc_name(),
            "indexer is not on mainnet"
        );
        let target_height = info.block_height;
        let checked_at = now()?;
        let started = std::time::Instant::now();
        observation.target_height = Some(target_height);
        observation.observed_tip_height = None;
        server::check_endpoint(
            &settings.endpoint,
            &self.identity,
            &self.network.chain(),
            info.block_height,
        )
        .await?;
        self.client
            .sync()
            .await
            .map_err(|error| diagnostics::client_failure(error, true))?;
        let result = loop {
            tokio::select! {
                result = self.client.await_sync() => break result.map_err(|error| diagnostics::client_failure(error, false))?,
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    if let Some(progress) = self.client.latest_sync_status() {
                        observation.scanned_blocks = progress.total_blocks_scanned;
                    }
                    self.checkpoint(observation.clone()).await?;
                }
            }
        };
        observation.scanned_blocks = self
            .client
            .latest_sync_status()
            .map_or(result.blocks_scanned, |progress| {
                progress.total_blocks_scanned
            });
        let info = tokio::time::timeout(
            crate::network::global().operation_timeout(Duration::from_secs(15)),
            indexer.get_lightd_info(
                crate::network::global()
                    .operation_timeout(zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT),
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("indexer tip check timed out"))?
        .map_err(|_| anyhow::anyhow!("indexer tip check failed"))?;
        let height = u64::from(u32::from(result.sync_end_height));
        ensure!(
            info.chain_name == self.network.rpc_name(),
            "indexer is not on mainnet"
        );
        observation.observed_tip_height = Some(info.block_height);
        let lag = freshness::validate(
            target_height,
            height,
            info.block_height,
            started.elapsed(),
            settings.max_age_seconds,
        )?;
        server::check_endpoint(
            &settings.endpoint,
            &self.identity,
            &self.network.chain(),
            height,
        )
        .await?;
        // A later fork can invalidate a spend whose source cost was already
        // accounted. Keep that cost consumed and fail closed until the same
        // transaction regains the required depth; never make it spendable again
        // merely because its outgoing journal entry was resolved earlier.
        let confirmed = self
            .store
            .call(|s| {
                Ok(s.status()?
                    .treasury_operations
                    .into_iter()
                    .filter(|op| op.submission == "CONFIRMED")
                    .map(|op| op.facts.txid)
                    .collect::<Vec<_>>())
            })
            .await?;
        let wallet = self.client.wallet();
        let wallet = wallet.read().await;
        for txid in confirmed {
            let txid = zcash_primitives::transaction::TxId::from_hex(&txid)
                .context("invalid confirmed transaction identity")?;
            let confirmed_height = wallet
                .wallet_transactions
                .get(&txid)
                .and_then(|record| record.status().get_confirmed_height())
                .map(|h| u64::from(u32::from(h)));
            ensure!(
                confirmed_height.is_some_and(|h| h > 0
                    && height >= h
                    && height - h + 1 >= u64::from(settings.confirmations.get())),
                "treasury_confirmed_spend_reorg: source spend requires recovery"
            );
        }
        let expired = self
            .store
            .call(|s| {
                Ok(s.status()?
                    .treasury_operations
                    .into_iter()
                    .filter(|o| o.submission == "EXPIRED")
                    .collect::<Vec<_>>())
            })
            .await?;
        for operation in expired {
            let txid = zcash_primitives::transaction::TxId::from_hex(&operation.facts.txid)
                .context("invalid transaction identity")?;
            ensure!(
                height
                    >= u64::from(operation.facts.expiry_height)
                        + u64::from(settings.confirmations.get())
                    && !wallet
                        .wallet_transactions
                        .get(&txid)
                        .is_some_and(|r| r.status().is_confirmed()),
                "treasury_expiry_reorg: recovered source requires review"
            );
        }
        let balance = wallet
            .account_balance(zip32::AccountId::ZERO)
            .map_err(|_| anyhow::anyhow!("treasury balance unavailable"))?;
        observation.confirmed_pool_balances_zatoshis = Some(crate::rotation::store::PoolBalances {
            ironwood: balance.confirmed_ironwood_balance.map(|z| z.into_u64()),
            orchard: balance.confirmed_orchard_balance.map(|z| z.into_u64()),
            sapling: balance.confirmed_sapling_balance.map(|z| z.into_u64()),
        });
        observation.confirmed_shielded_zatoshis = [
            balance.confirmed_ironwood_balance,
            balance.confirmed_orchard_balance,
            balance.confirmed_sapling_balance,
        ]
        .into_iter()
        .flatten()
        .map(|z| z.into_u64())
        .sum();
        observation.spendable_shielded_zatoshis = wallet
            .shielded_spendable_balance(zip32::AccountId::ZERO, false)
            .map_err(|_| anyhow::anyhow!("treasury spendable balance unavailable"))?
            .into_u64();
        drop(wallet);
        self.reconcile_refunds(height, settings.confirmations.get())
            .await?;
        freshness::validate(
            target_height,
            height,
            info.block_height,
            started.elapsed(),
            settings.max_age_seconds,
        )?;
        if lag > 0 {
            tracing::debug!(
                scanned_height = height,
                observed_tip_height = info.block_height,
                lag_blocks = lag,
                max_lag_blocks = freshness::MAX_LAG_BLOCKS,
                "treasury sync accepted with bounded tip lag; background sync will continue"
            );
        }
        observation.checked_at = Some(checked_at);
        observation.height = Some(height);
        Ok(())
    }
}

async fn address_info(client: &LightClient) -> Result<serde_json::Value> {
    let mut addresses: serde_json::Value =
        serde_json::from_str(&client.unified_addresses_json().await.to_string())?;
    for address in addresses.as_array_mut().context("invalid address list")? {
        let address = address.as_object_mut().context("invalid address record")?;
        let orchard = address.remove("has_orchard").unwrap_or(false.into());
        let sapling = address.remove("has_sapling").unwrap_or(false.into());
        let transparent = address.remove("has_transparent").unwrap_or(false.into());
        address.insert(
            "receiver_capabilities".into(),
            serde_json::json!({
                "orchard_protocol": orchard,
                "sapling": sapling,
                "transparent": transparent,
            }),
        );
        address.insert(
            "orchard_protocol_pools".into(),
            if orchard.as_bool() == Some(true) {
                serde_json::json!(["orchard", "ironwood"])
            } else {
                serde_json::json!([])
            },
        );
    }
    Ok(addresses)
}
