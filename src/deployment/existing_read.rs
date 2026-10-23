//! Open only the selected, already existing authority for an operator document read.
use super::{MetaConfig, StoreOnlyOwner, WalletConfig};
use crate::payment::{PaidClient, Payer, SpendPolicy};
use anyhow::{Context, Result, ensure};
use std::sync::Arc;

pub(crate) struct ExistingRead {
    client: Option<PaidClient>,
    owner: Option<StoreOnlyOwner>,
}
impl ExistingRead {
    pub(crate) async fn load(config: &MetaConfig, listener: &str) -> Result<Self> {
        Self::load_from(config, listener, |name| std::env::var(name).ok()).await
    }
    async fn load_from(
        config: &MetaConfig,
        listener: &str,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let policy = config
            .source_management
            .as_ref()
            .context("source management not configured")?;
        let listener = config
            .servers
            .get(listener)
            .filter(|server| server.source_management)
            .context("source_management_disabled")?;
        let name = crate::discovery::policy::wallet(policy, listener);
        let wallet = config
            .wallets
            .get(name)
            .context("source management wallet unavailable")?;
        match wallet {
            WalletConfig::Static {
                private_key_env,
                max_api_payment_usdc,
            } => {
                let key = env(private_key_env).context("selected signing key unavailable")?;
                let client = PaidClient::new(Payer::new(
                    &key,
                    SpendPolicy::dollars(max_api_payment_usdc)?,
                )?);
                Ok(Self {
                    client: Some(client),
                    owner: None,
                })
            }
            WalletConfig::ZcashRotation {
                max_api_payment_usdc,
                ..
            } => {
                let treasury = config
                    .treasury
                    .as_ref()
                    .context("treasury not configured")?;
                let funding = config
                    .funding
                    .as_ref()
                    .context("Base configuration unavailable")?;
                funding.validate_base()?;
                let urls = funding.base_rpc_urls(env)?;
                let base = crate::rotation::base::BaseRpc::with_fallbacks(
                    &urls,
                    funding.base_confirmations,
                )?;
                let store = crate::treasury::Treasury::open_shared_state(
                    treasury.state_dir.clone(),
                    treasury.key_file.clone(),
                    treasury.runtime_id()?,
                    crate::rotation::store::TreasuryNetwork::Mainnet,
                )
                .await?;
                let pools = store.advisory_pools(&[name.to_owned()])?;
                ensure!(pools.len() == 1, "existing managed pool unavailable");
                let spend = SpendPolicy::dollars(max_api_payment_usdc)?;
                let owner = StoreOnlyOwner::new(store);
                let result = crate::rotation::manager::ManagedPool::new(
                    owner.store.clone(),
                    pools[0].clone(),
                    base,
                    spend,
                );
                match result {
                    Ok(pool) => Ok(Self {
                        client: Some(PaidClient::managed(Arc::new(pool))),
                        owner: Some(owner),
                    }),
                    Err(error) => {
                        owner.close().await?;
                        Err(error)
                    }
                }
            }
        }
    }
    pub(crate) fn client(&self) -> PaidClient {
        self.client
            .as_ref()
            .expect("live existing authority")
            .clone()
    }
    pub(crate) async fn close(mut self) -> Result<()> {
        if let Some(client) = self.client.take() {
            client.drain().await;
            drop(client);
        }
        if let Some(owner) = self.owner.take() {
            owner.close().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn config() -> MetaConfig {
        serde_json::from_value(json!({"version":1,
            "wallets":{
                "shared":{"mode":"static","private_key_env":"MISSING_SHARED"},
                "selected":{"mode":"static","private_key_env":"SELECTED_KEY"},
                "unused":{"mode":"static","private_key_env":"MISSING_UNUSED"}},
            "source_management":{"wallet":"shared"},
            "servers":{"owner":{"listen":"127.0.0.1:0","source_management":true,"wallet":"selected"},
                "disabled":{"listen":"127.0.0.1:0","source_management":false,"wallet":"unused"}}
        })).unwrap()
    }
    #[tokio::test]
    async fn selected_static_authority_reads_only_its_key() {
        let calls = Mutex::new(Vec::new());
        let owner = ExistingRead::load_from(&config(), "owner", |name| {
            calls.lock().unwrap().push(name.to_owned());
            (name == "SELECTED_KEY").then(|| format!("{:064x}", 1))
        })
        .await
        .unwrap();
        let client = owner.client();
        assert!(client.documentation_identity().await.is_ok());
        drop(client);
        owner.close().await.unwrap();
        assert_eq!(*calls.lock().unwrap(), ["SELECTED_KEY"]);
        assert!(
            ExistingRead::load_from(&config(), "disabled", |_| panic!(
                "disabled owner read a key"
            ))
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn existing_managed_pool_needs_no_sdk_funding_credentials_or_allocation() {
        use crate::rotation::store::Store;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"invalid SDK snapshot").unwrap();
        let id = store.id().to_owned();
        let reserves =
            serde_json::from_value(json!({"small":{"funding_amount_usdc":"5"}})).unwrap();
        let pool = store
            .ensure_capacity_group(
                "selected",
                &reserves,
                Some(alloy_primitives::U256::from(5_000_000)),
                0,
            )
            .unwrap();
        assert!(store.funding_jobs().unwrap().is_empty());
        drop(store);
        let mut cfg = config();
        cfg.wallets.insert("selected".into(), serde_json::from_value(json!({"mode":"zcash_rotation","max_api_payment_usdc":"5","max_conversion_overhead_percent":5,"reserves":{"small":{"funding_amount_usdc":"5"}}})).unwrap());
        cfg.treasury = Some(serde_json::from_value(json!({"state_dir":state,"key_file":key,"id":id,"indexer_url_env":"ABSENT_INDEXER","submission_url_env":"ABSENT_SUBMIT"})).unwrap());
        cfg.funding = Some(
            serde_json::from_value(
                json!({"base_rpc_url_env":"BASE_ONLY","base_rpc_fallback_url_envs":[]}),
            )
            .unwrap(),
        );
        let calls = Mutex::new(Vec::new());
        let owner = ExistingRead::load_from(&cfg, "owner", |name| {
            calls.lock().unwrap().push(name.to_owned());
            (name == "BASE_ONLY").then(|| "http://127.0.0.1:1".into())
        })
        .await
        .unwrap();
        let client = owner.client();
        assert!(
            client.documentation_identity().await.is_err(),
            "an empty pool must not allocate a documentation wallet"
        );
        drop(client);
        tokio::time::timeout(std::time::Duration::from_secs(5), owner.close())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*calls.lock().unwrap(), ["BASE_ONLY"]);
        let store = Store::open(&state, &key, &id).unwrap();
        assert_eq!(
            store
                .advisory_pools(&["selected".into(), "unused".into(), "shared".into()])
                .unwrap(),
            [pool]
        );
        assert!(store.funding_jobs().unwrap().is_empty());
        let db = rusqlite::Connection::open_with_flags(
            state.join("state.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM wallets", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
