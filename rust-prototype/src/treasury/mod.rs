//! Offline embedded zingolib treasury. Never starts sync, save_task, or broadcast.
use crate::rotation::store::{Status, Store, StoreHandle};
use anyhow::{Context, Result, ensure};
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
use zingolib::{
    config::{ClientConfig, WalletConfig},
    lightclient::LightClient,
    wallet::{WalletSettings, keys::unified::ReceiverSelection},
};

pub struct Treasury {
    _scratch: tempfile::TempDir,
    client: LightClient,
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    revision: i64,
    healthy: bool,
}
fn config(dir: &Path, wallet: WalletConfig) -> Result<ClientConfig> {
    ClientConfig::builder()
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
        let mut client = LightClient::new(config(scratch.path(), wallet)?, false)
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
        let store =
            tokio::task::spawn_blocking(move || Store::create(&dir, &key, birthday, &bytes))
                .await??;
        let (store, worker) = StoreHandle::spawn(store);
        Ok(Self {
            _scratch: scratch,
            client,
            store,
            worker,
            revision: 1,
            healthy: true,
        })
    }
    pub async fn open(dir: PathBuf, key: PathBuf, id: String) -> Result<Self> {
        let path = dir.clone();
        let (store, revision, bytes) = tokio::task::spawn_blocking(move || -> Result<_> {
            let store = Store::open(&path, &key, &id)?;
            let (revision, bytes) = store.snapshot()?;
            Ok((store, revision, bytes))
        })
        .await??;
        let scratch = tempfile::tempdir()?;
        // Borrow a zeroizing buffer instead of handing an unzeroized Vec to upstream.
        let client = LightClient::from_reader(
            bytes.as_slice(),
            config(scratch.path(), WalletConfig::Read)?,
        )
        .await
        .map_err(|_| anyhow::anyhow!("corrupt or incompatible treasury snapshot"))?;
        ensure!(
            client.indexer_uri().is_none(),
            "offline treasury must not connect to an indexer"
        );
        let (store, worker) = StoreHandle::spawn(store);
        Ok(Self {
            _scratch: scratch,
            client,
            store,
            worker,
            revision,
            healthy: true,
        })
    }
    pub async fn status(&self) -> Result<Status> {
        self.store.call(|s| s.status()).await
    }
    pub async fn addresses(&self) -> Result<serde_json::Value> {
        Ok(serde_json::from_str(
            &self.client.unified_addresses_json().await.to_string(),
        )?)
    }
    pub async fn derive_address(&mut self) -> Result<serde_json::Value> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed persistence"
        );
        self.healthy = false;
        self.client
            .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
            .await
            .map_err(|_| anyhow::anyhow!("address derivation failed"))?;
        let bytes = snapshot(&self.client).await?;
        let expected = self.revision;
        self.revision = self
            .store
            .call(move |s| s.save_snapshot(expected, &bytes))
            .await?;
        self.healthy = true;
        self.addresses().await
    }
    pub async fn ensure_pool(&self, name: String, deposit_size: String) -> Result<String> {
        ensure!(
            self.healthy,
            "treasury requires reopen after failed persistence"
        );
        self.store
            .call(move |s| s.ensure_pool(&name, &deposit_size))
            .await
    }
    pub async fn close(self) -> Result<()> {
        drop(self.store);
        self.worker.await.context("store worker failed")?;
        Ok(())
    }
}
