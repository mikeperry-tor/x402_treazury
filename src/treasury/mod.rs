//! Embedded treasury with encrypted sync and explicit durable transaction operations.
pub mod actor;
pub mod birthday;
pub(crate) mod diagnostics;
mod expiry;
mod freshness;
#[cfg(all(test, feature = "zcash-testutils"))]
mod lifecycle_tests;
pub(crate) mod ownership;
pub mod recovery;
mod refunds;
#[cfg(all(test, feature = "zcash-regtest"))]
mod regtest;
mod send;
mod server;
mod source_support;
mod sync_reuse;
use crate::rotation::store::TreasuryNetwork;
impl TreasuryNetwork {
    fn chain(self) -> zingolib::config::ChainType {
        match self {
            Self::Mainnet => zingolib::config::ChainType::Mainnet,
            #[cfg(all(test, feature = "zcash-testutils"))]
            Self::Synthetic => zingolib::config::ChainType::Regtest(Default::default()),
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
    #[cfg(test)]
    accounting_test_hook: Option<std::sync::Arc<source_support::AccountingTestGate>>,
    #[cfg(test)]
    final_checkpoint_test_hook: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    network: TreasuryNetwork,
    _scratch: tempfile::TempDir,
    client: Option<LightClient>,
    sync_owner: Option<ownership::SyncOwner>,
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    revision: i64,
    healthy: bool,
    sync_settings: Option<SyncSettings>,
    reusable_sync_revision: Option<i64>,
    // Scheduling only; never used to date or authorize spending evidence.
    last_completed_sync: Option<tokio::time::Instant>,
}
/// Shared state remains exclusively owned when only SDK reconstruction fails.
/// Serving can use that authenticated store independently of the SDK capability.
pub(crate) enum OpenedTreasury {
    Ready(Box<Treasury>),
    SnapshotUnavailable(Box<Store>),
}
#[derive(Debug)]
pub(crate) struct SnapshotUnavailable;
impl std::fmt::Display for SnapshotUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("corrupt or incompatible treasury snapshot")
    }
}
impl std::error::Error for SnapshotUnavailable {}

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
        tracing::info!(
            import = mnemonic.is_some(),
            "wallet init: creating offline wallet"
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
        tracing::info!("wallet init: offline wallet created; deriving shielded receive address");
        client
            .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
            .await
            .map_err(|_| anyhow::anyhow!("address derivation failed"))?;
        tracing::info!("wallet init: receive address derived; serializing wallet");
        let bytes = snapshot(&client).await?;
        tracing::info!("wallet init: wallet serialized; writing encrypted state and key");
        let store = tokio::task::spawn_blocking(move || {
            Store::create_with_network(&dir, &key, birthday, &bytes, network)
        })
        .await??;
        tracing::info!("wallet init: encrypted state and key saved; initialization completed");
        let (store, worker) = StoreHandle::spawn(store);
        Ok(Self {
            network,
            _scratch: scratch,
            client: Some(client),
            sync_owner: None,
            #[cfg(test)]
            final_checkpoint_test_hook: None,
            #[cfg(test)]
            accounting_test_hook: None,
            store,
            worker,
            revision: 1,
            healthy: true,
            sync_settings: None,
            reusable_sync_revision: None,
            last_completed_sync: None,
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
        let store = Self::open_shared_state(dir, key, id, network).await?;
        match Self::reconstruct(store).await? {
            OpenedTreasury::Ready(owner) => Ok(*owner),
            OpenedTreasury::SnapshotUnavailable(store) => {
                drop(store);
                Err(SnapshotUnavailable.into())
            }
        }
    }
    /// Shared identity, schema, key, snapshot authentication and exclusive owner
    /// validation precede any capability-local connection/preparation fallback.
    pub(crate) async fn open_shared_state(
        dir: PathBuf,
        key: PathBuf,
        id: String,
        network: TreasuryNetwork,
    ) -> Result<Store> {
        tokio::task::spawn_blocking(move || {
            let mut store = Store::open_with_network(&dir, &key, &id, network)?;
            store.set_sync_phase(SyncPhase::Offline)?;
            Ok(store)
        })
        .await?
    }
    /// SQL/decryption/configuration failures remain outer errors. Only the SDK's
    /// inability to reconstruct authenticated bytes yields the retained store.
    pub(crate) async fn reconstruct(store: Store) -> Result<OpenedTreasury> {
        let network = store.network();
        let (store, revision, bytes) = tokio::task::spawn_blocking(move || {
            let (revision, bytes) = store.snapshot()?;
            Ok::<_, anyhow::Error>((store, revision, bytes))
        })
        .await??;
        let scratch = tempfile::tempdir()?;
        // Borrow a zeroizing buffer instead of handing an unzeroized Vec to upstream.
        let client = match LightClient::from_reader(
            bytes.as_slice(),
            config(scratch.path(), WalletConfig::Read, network)?,
        )
        .await
        {
            Ok(client) => client,
            Err(_) => return Ok(OpenedTreasury::SnapshotUnavailable(Box::new(store))),
        };
        ensure!(
            client.indexer_uri().is_none(),
            "offline treasury must not connect to an indexer"
        );
        let (store, worker) = StoreHandle::spawn(store);
        let owner = Self {
            network,
            _scratch: scratch,
            client: Some(client),
            sync_owner: None,
            #[cfg(test)]
            final_checkpoint_test_hook: None,
            #[cfg(test)]
            accounting_test_hook: None,
            store,
            worker,
            revision,
            healthy: true,
            sync_settings: None,
            reusable_sync_revision: None,
            last_completed_sync: None,
        };
        owner.finish_open_handoff().await?;
        Ok(OpenedTreasury::Ready(Box::new(owner)))
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
    /// Declare configured capacity without allocating wallets or funding work.
    pub async fn ensure_capacity_group(
        &self,
        name: String,
        reserves: crate::rotation::config::ReservesConfig,
        cap: Option<alloy_primitives::U256>,
        retirement_atomic: u64,
    ) -> Result<String> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed persistence"
        );
        self.store
            .call(move |s| s.ensure_capacity_group(&name, &reserves, cap, retirement_atomic))
            .await
    }
    /// Allocate one exact vacancy under current funding authority.
    pub async fn allocate_capacity_kind(
        &self,
        pool: String,
        kind: crate::rotation::store::AllocationKind,
        auto_fund: bool,
    ) -> Result<Option<String>> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed persistence"
        );
        self.store
            .call(move |s| s.allocate_capacity_kind(&pool, kind, auto_fund))
            .await
    }
    pub fn configure_sync(&mut self, settings: SyncSettings) {
        self.reusable_sync_revision = None;
        self.last_completed_sync = None;
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
    #[cfg(all(test, feature = "zcash-testutils"))]
    async fn checkpoint_observed(
        &mut self,
        observation: SyncObservation,
        start: crate::rotation::store::SyncObservationStart,
    ) -> Result<bool> {
        self.checkpoint_observed_source(observation, start, None)
            .await
    }
    async fn checkpoint_observed_source(
        &mut self,
        observation: SyncObservation,
        start: crate::rotation::store::SyncObservationStart,
        source: Option<crate::rotation::store::source_support::Pass>,
    ) -> Result<bool> {
        let bytes = snapshot(
            self.client
                .as_ref()
                .context("treasury client unavailable")?,
        )
        .await?;
        let expected = self.revision;
        #[cfg(test)]
        if let Some(hook) = &self.final_checkpoint_test_hook
            && hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        {
            start.expire_for_test();
        }
        self.revision = self
            .store
            .call(move |s| {
                s.save_observed_source_snapshot(
                    expected,
                    &bytes,
                    observation,
                    start,
                    source.as_ref(),
                )
            })
            .await?;
        let current = self.store.call(|s| s.treasury_sync_state()).await?;
        Ok(current
            .sync
            .as_ref()
            .map(|sync| sync.fresh(now()?, current.snapshot_revision))
            .transpose()?
            .unwrap_or(false))
    }
    /// Cancellation must be requested through `stop`, then this future awaited.
    /// Adapter errors are sanitized; endpoints are not persisted in status.
    pub async fn sync_once(&mut self, stop: &CancellationToken) -> Result<()> {
        self.reusable_sync_revision = None;
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
        let mut retained_source_pass = None;
        loop {
            let mut observation = self
                .store
                .call(|s| s.treasury_sync_state())
                .await?
                .sync
                .map(|sync| sync.into_observation())
                .unwrap_or(SyncObservation {
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
            tracing::info!("treasury sync: synchronizing wallet with indexer");
            self.checkpoint(observation.clone()).await?;
            let session = SyncSession {
                #[cfg(test)]
                accounting_test_hook: self.accounting_test_hook.clone(),
                identity: crate::network::IsolationId::treasury(
                    &self.store.call(|s| Ok(s.id().to_owned())).await?,
                ),
                network: self.network,
                client: self
                    .client
                    .take()
                    .context("treasury sync interrupted; reopen required")?,
                store: self.store.clone(),
                revision: self.revision,
                observation_start: None,
                source_pass: retained_source_pass.take(),
            };
            let cancelled = stop.child_token();
            let _cancel_on_drop = cancelled.clone().drop_guard();
            // Upstream sync launches detached fetch/mempool tasks. Isolate each cycle
            // so cancellation/error cannot leave those tasks connected on the MCP runtime.
            let scan_settings = settings.clone();
            self.sync_owner = Some(ownership::SyncOwner::start(
                session,
                scan_settings,
                observation,
                cancelled,
            ));
            let (session, mut observation, outcome) = self.finish_sync_owner().await?;
            self.revision = session.revision;
            let observation_start = session.observation_start;
            let source_pass = session.source_pass;
            self.client = Some(session.client);
            observation.phase = if stop.is_cancelled() {
                SyncPhase::Offline
            } else if outcome.is_ok() {
                SyncPhase::Ready
            } else {
                SyncPhase::Failed
            };
            observation.last_error = outcome.as_ref().err().map(|e| e.to_string());
            tracing::debug!("treasury sync: saving final wallet checkpoint");
            let height = observation.height;
            let observed_tip_height = observation.observed_tip_height;
            let scanned_blocks = observation.scanned_blocks;
            if observation.phase == SyncPhase::Ready {
                let start =
                    observation_start.context("successful sync lacks observation origin")?;
                let fresh = self
                    .checkpoint_observed_source(observation, start, source_pass.clone())
                    .await?;
                if !fresh && !stop.is_cancelled() {
                    retained_source_pass = source_pass;
                    tracing::info!(
                        stage = "freshness_catch_up",
                        "Treasury checkpoint retained; acquiring fresh spending evidence"
                    );
                    tokio::task::yield_now().await;
                    continue;
                }
            } else {
                self.checkpoint(observation).await?;
            }
            self.healthy = true;
            if outcome.is_ok() && !stop.is_cancelled() {
                self.reusable_sync_revision = Some(self.revision);
                self.last_completed_sync = Some(tokio::time::Instant::now());
                tracing::info!(
                    height,
                    observed_tip_height,
                    scanned_blocks,
                    "treasury sync: synchronization completed; wallet checkpoint saved"
                );
            } else {
                tracing::debug!(
                    "treasury sync: wallet checkpoint saved after interruption or failure"
                );
            }
            return outcome;
        }
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
            let reconstructing = !self.healthy;
            let result = if reconstructing {
                self.reconstruct_live(&stop).await
            } else {
                self.sync_once(&stop).await
            };
            if let Err(error) = result
                && !stop.is_cancelled()
            {
                if reconstructing && !ownership::retryable_reconstruction(&error) {
                    return Err(error);
                }
                diagnostics::warn_sync_failure(&error);
            }
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
            }
        }
        self.close().await
    }
    pub async fn close(mut self) -> Result<()> {
        let result = self.retire_sdk().await.map(|_| ());
        drop(self.store);
        self.worker.await.context("store worker failed")?;
        result
    }
}

struct SyncSession {
    #[cfg(test)]
    accounting_test_hook: Option<std::sync::Arc<source_support::AccountingTestGate>>,
    source_pass: Option<crate::rotation::store::source_support::Pass>,
    observation_start: Option<crate::rotation::store::SyncObservationStart>,
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
        loop {
            match self.scan_observation(settings, observation).await {
                Err(error) if error.is::<freshness::FreshObservationRequired>() => {
                    // Persist completed scan work, but never renew its timestamp
                    // or publish it as spend-ready. The next pass obtains new
                    // observations and scans only the remaining history.
                    self.checkpoint(observation.clone()).await?;
                    tracing::info!(
                        stage = "freshness_catch_up",
                        scanned_blocks = observation.scanned_blocks,
                        "Treasury scan retained; acquiring fresh spending evidence"
                    );
                    tokio::task::yield_now().await;
                }
                result => return result,
            }
        }
    }
    async fn scan_observation(
        &mut self,
        settings: &SyncSettings,
        observation: &mut SyncObservation,
    ) -> Result<()> {
        tracing::debug!("treasury sync: connecting to indexer");
        let mut indexer = crate::network::global()
            .grpc(&self.identity, &settings.endpoint)
            .await
            .context("indexer connection failed")?;
        tracing::debug!("treasury sync: indexer connected; checking network and chain tip");
        self.client.set_indexer(indexer.clone());
        let started =
            crate::rotation::store::SyncObservationStart::capture(settings.max_age_seconds)?;
        self.observation_start = None;
        let info = indexer
            .get_lightd_info(zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT)
            .await
            .map_err(|status| diagnostics::tip_failure(status, "starting_tip"))?;
        ensure!(
            info.chain_name == self.network.rpc_name(),
            "indexer is not on mainnet"
        );
        let target_height = info.block_height;
        let checked_at = started.checked_at();
        tracing::debug!(
            target_height,
            "treasury sync: network and chain tip verified; checking indexer capabilities"
        );
        observation.target_height = Some(target_height);
        observation.observed_tip_height = None;
        server::check_endpoint(
            &settings.endpoint,
            &self.identity,
            &self.network.chain(),
            info.block_height,
        )
        .await?;
        tracing::debug!(
            target_height,
            "treasury sync: indexer checks completed; scanning wallet history"
        );
        self.client
            .sync()
            .await
            .map_err(|error| diagnostics::client_failure(error, true))?;
        let result = loop {
            tokio::select! {
                result = self.client.await_sync() => break result.map_err(|error| {
                    diagnostics::client_failure(error, false).context(diagnostics::SyncProgress {
                        phase: "scan",
                        elapsed_ms: started.elapsed().unwrap_or_default().as_millis() as u64,
                        target_height,
                        scanned_blocks: self.client.latest_sync_status().map_or(0, |p| u64::from(p.total_blocks_scanned)),
                    })
                })?,
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    if let Some(progress) = self.client.latest_sync_status() {
                        observation.scanned_blocks = progress.total_blocks_scanned;
                    }
                    self.checkpoint(observation.clone()).await?;
                    tracing::info!(
                        target_height,
                        scanned_blocks = observation.scanned_blocks,
                        elapsed_seconds = started.elapsed()?.as_secs(),
                        "treasury sync: scan in progress; wallet checkpoint saved"
                    );
                }
            }
        };
        observation.scanned_blocks = self
            .client
            .latest_sync_status()
            .map_or(result.blocks_scanned, |progress| {
                progress.total_blocks_scanned
            });
        tracing::debug!(
            scanned_blocks = observation.scanned_blocks,
            "treasury sync: scan completed; verifying final chain tip and observation freshness"
        );
        let info = indexer
            .get_lightd_info(zingolib::lightclient::DEFAULT_REQUEST_TIMEOUT)
            .await
            .map_err(|status| diagnostics::tip_failure(status, "final_tip"))?;
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
            started.elapsed()?,
            settings.max_age_seconds,
        )?;
        server::check_endpoint(
            &settings.endpoint,
            &self.identity,
            &self.network.chain(),
            height,
        )
        .await?;
        // Retain protection against still-live signed source authority after a
        // fork. Once exact signed bytes have a buried expiry, their historical
        // disposition is accounting evidence, not a veto on current spendability.
        // This never releases a pending reservation or changes source accounting.
        tracing::debug!(
            "treasury sync: chain checks completed; reconciling prior spending and balances"
        );
        let retained = match self.source_pass.take() {
            Some(pass) => {
                self.retain_source_support(pass, &mut indexer, height)
                    .await?
            }
            None => None,
        };
        self.source_pass = Some(match retained {
            Some(pass) => pass,
            None => {
                let returns = self
                    .reconcile_refunds(height, settings.confirmations.get())
                    .await?;
                self.recheck_source_support(height, settings.confirmations.get(), returns)
                    .await?
            }
        });
        let wallet = self.client.wallet().read().await;
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
        tracing::debug!("treasury sync: spending and balances checked");
        freshness::validate(
            target_height,
            height,
            info.block_height,
            started.elapsed()?,
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
        if !started.fresh(now()?)? {
            return Err(freshness::FreshObservationRequired.into());
        }
        self.observation_start = Some(started);
        observation.checked_at = Some(checked_at);
        observation.height = Some(height);
        Ok(())
    }
}

fn source_expiry_buried(expiry: u32, scanned_height: u64, confirmations: u32) -> bool {
    crate::rotation::transaction::expiry_buried(expiry, scanned_height, confirmations)
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

#[cfg(test)]
mod initialization_tests {
    use super::*;

    #[tokio::test]
    async fn sdk_reconstruction_failure_retains_authenticated_store_and_exclusive_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let store = Store::create(&state, &key, 1, b"synthetic invalid SDK bytes").unwrap();
        let id = store.id().to_owned();
        drop(store);
        let store = Treasury::open_shared_state(
            state.clone(),
            key.clone(),
            id.clone(),
            TreasuryNetwork::Mainnet,
        )
        .await
        .unwrap();
        let before = store.snapshot().unwrap();
        let OpenedTreasury::SnapshotUnavailable(store) =
            Treasury::reconstruct(store).await.unwrap()
        else {
            panic!("invalid fixture reconstructed")
        };
        assert_eq!(store.snapshot().unwrap(), before);
        assert_eq!(store.id(), id);
        assert!(store.funding_jobs().unwrap().is_empty());
        assert!(Store::open(&state, &key, &id).is_err());
        drop(store);
        assert!(
            Treasury::open(state, key, id)
                .await
                .err()
                .unwrap()
                .is::<SnapshotUnavailable>()
        );
    }

    #[tokio::test]
    async fn shared_state_failures_cannot_become_sdk_unavailability() {
        for fault in ["schema", "key", "identity", "owner"] {
            let dir = tempfile::tempdir().unwrap();
            let state = dir.path().join("state");
            let key = dir.path().join("key");
            let store = Store::create(&state, &key, 1, b"synthetic invalid SDK bytes").unwrap();
            let mut id = store.id().to_owned();
            let owner = if fault == "owner" {
                Some(store)
            } else {
                drop(store);
                None
            };
            match fault {
                "schema" => {
                    let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
                    db.execute("PRAGMA user_version=15", []).unwrap();
                }
                "key" => std::fs::write(&key, [0u8; 32]).unwrap(),
                "identity" => id = uuid::Uuid::new_v4().to_string(),
                _ => {}
            }
            let error = Treasury::open_shared_state(state, key, id, TreasuryNetwork::Mainnet)
                .await
                .err()
                .unwrap();
            assert!(!error.is::<SnapshotUnavailable>());
            drop(owner);
        }
    }
}
