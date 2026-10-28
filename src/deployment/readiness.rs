//! One advisory startup announcement; never gates serving or financial work.
use super::*;
use crate::rotation::manager::ManagedPool;
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::sync::Arc;
use tokio::sync::oneshot;

pub(super) type FundingPools = Option<Vec<Arc<ManagedPool>>>;

impl Deployment {
    pub(super) fn endpoint_readiness(&self, name: &str) -> Option<Result<()>> {
        let sources = &self.listeners[name].sources;
        if let Some(reason) = self.selection_errors.get(name) {
            return Some(Err(anyhow::anyhow!(
                "endpoint tool selection failed: {reason}"
            )
            .context(catalog::LoadStage::Selection)));
        }
        if let Some(id) = sources.iter().find(|id| {
            !self.pending_sources.contains_key(*id)
                && !self.sources.contains_key(*id)
                && !self.config.sources[*id].optional_startup
        }) {
            return Some(Err(anyhow::anyhow!("required catalog unavailable: {id}")));
        }
        sources
            .iter()
            .all(|id| !self.pending_sources.contains_key(id))
            .then_some(Ok(()))
    }
}

pub(super) fn funding_pools(
    deployment: &Deployment,
    wallets: &BTreeMap<String, PaidClient>,
) -> FundingPools {
    let mut needed = BTreeSet::new();
    for (server, tools) in &deployment.selected {
        for (source, _) in tools {
            needed.insert(
                deployment.wallet_resolution.bindings[server][source]
                    .wallet
                    .clone(),
            );
        }
    }
    needed.extend(crate::discovery::policy::wallets(&deployment.config));
    needed
        .into_iter()
        .map(|name| wallets.get(&name)?.startup_funding_pool())
        .collect()
}

pub(super) async fn announce(
    catalogs: oneshot::Receiver<FundingPools>,
    listening: Vec<oneshot::Receiver<()>>,
) {
    let Ok(Some(pools)) = catalogs.await else {
        return;
    };
    for listener in listening {
        if listener.await.is_err() {
            return;
        }
    }
    if wait_for_funding(&pools).await.is_err() {
        tracing::debug!(
            "Startup funding readiness could not be checked; MCP service remains available"
        );
        return;
    }
    tracing::info!(
        funding_basis = "saved USDC balance estimates",
        "All MCP endpoints listening, funded, and ready"
    );
}

async fn wait_for_funding(pools: &[Arc<ManagedPool>]) -> Result<()> {
    loop {
        let mut changes = FuturesUnordered::new();
        for pool in pools {
            let mut changed = Box::pin(pool.discovery_changes().await?.notified_owned());
            changed.as_mut().enable();
            changes.push(changed);
        }
        if ManagedPool::startup_funding_ready(pools).await? {
            return Ok(());
        }
        changes.next().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::store::{Store, StoreHandle, funding::FundingJob};
    use alloy_primitives::U256;
    use tracing::instrument::WithSubscriber;

    fn fixture() -> (
        tempfile::TempDir,
        StoreHandle,
        tokio::task::JoinHandle<()>,
        Vec<Arc<ManagedPool>>,
        Vec<FundingJob>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let mut ids = Vec::new();
        for name in ["one", "two"] {
            ids.push(
                crate::rotation::test_capacity::declare_and_allocate!(
                    store,
                    name,
                    crate::rotation::test_capacity::small_reserve("2"),
                    Some(U256::from(1_000_000)),
                    0
                )
                .unwrap(),
            );
        }
        let jobs = store.funding_jobs().unwrap();
        let (store, worker) = StoreHandle::spawn(store);
        let pools = ids
            .into_iter()
            .map(|id| {
                Arc::new(
                    ManagedPool::new(
                        store.clone(),
                        id,
                        crate::rotation::base::BaseRpc::new("https://rpc.example", 3).unwrap(),
                        SpendPolicy::dollars("1").unwrap(),
                    )
                    .unwrap(),
                )
            })
            .collect();
        (dir, store, worker, pools, jobs)
    }
    async fn balance(store: &StoreHandle, job: &FundingJob, amount: u64) {
        let job = job.clone();
        store
            .call(move |s| {
                s.record_funding_balance(
                    &job.id,
                    &job.operation_id,
                    &job.wallet_id,
                    &job.recipient,
                    U256::from(amount),
                )
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn effective_source_bindings_exclude_unused_defaults_and_filtered_sources() {
        let (dir, store, worker, pools, _jobs) = fixture();
        std::fs::write(
            dir.path().join("spec.json"),
            r#"{"operations":[{"method":"GET","path":"/read"}]}"#,
        )
        .unwrap();
        let config = dir.path().join("deployment.toml");
        std::fs::write(
            &config,
            r#"version=1
[wallets.default]
mode="static"
private_key_env="UNREAD"
[wallets.source]
mode="static"
private_key_env="UNREAD"
[sources.local]
spec="spec.json"
base_url="https://api.example"
wallet="source"
[servers.one]
listen="127.0.0.1:0"
auth=false
wallet="default"
sources=["local"]
"#,
        )
        .unwrap();
        let mut deployment = Deployment::load_for_serving(&config).await.unwrap();
        let mut wallets = BTreeMap::from([
            ("source".into(), PaidClient::managed(pools[0].clone())),
            ("default".into(), PaidClient::unsigned()),
            ("unused".into(), PaidClient::managed(pools[1].clone())),
        ]);
        assert_eq!(funding_pools(&deployment, &wallets).unwrap().len(), 1);
        deployment
            .config
            .servers
            .get_mut("one")
            .unwrap()
            .raw_url_calls = true;
        assert!(
            funding_pools(&deployment, &wallets).is_none(),
            "management uses the default payer"
        );
        deployment
            .config
            .servers
            .get_mut("one")
            .unwrap()
            .raw_url_calls = false;
        // Final selected inventory excludes an omitted optional source or a
        // catalog whose tools were all filtered, even if its client was loaded.
        deployment.selected.get_mut("one").unwrap().clear();
        wallets.insert("source".into(), PaidClient::unsigned());
        assert!(funding_pools(&deployment, &wallets).unwrap().is_empty());
        assert!(PaidClient::unsigned().startup_funding_pool().is_none());
        assert!(
            PaidClient::unavailable(crate::rotation::error::AdmissionError::FundingUnavailable(
                "fixture"
            ))
            .startup_funding_pool()
            .is_none()
        );
        let payer = Payer::new(&format!("{:064x}", 2), SpendPolicy::dollars("1").unwrap()).unwrap();
        assert!(PaidClient::new(payer).startup_funding_pool().is_none());
        let unlimited = Arc::new(
            ManagedPool::new(
                store.clone(),
                pools[0].hint_group_scope().1.to_owned(),
                crate::rotation::base::BaseRpc::new("https://rpc.example", 3).unwrap(),
                SpendPolicy { max_atomic: None },
            )
            .unwrap(),
        );
        assert!(
            PaidClient::managed(unlimited)
                .startup_funding_pool()
                .is_none()
        );
        drop(wallets);
        drop(pools);
        drop(store);
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn announcement_waits_for_catalogs_listeners_and_current_funding_once() {
        if crate::test_log_capture::run_in_child(
            "deployment::readiness::tests::announcement_waits_for_catalogs_listeners_and_current_funding_once",
        ) {
            return;
        }
        let (_dir, store, worker, pools, jobs) = fixture();
        balance(&store, &jobs[0], 1_000_000).await;
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let (catalog_send, catalogs) = oneshot::channel();
        let (one_send, one) = oneshot::channel();
        let (two_send, two) = oneshot::channel();
        let mut task = tokio::spawn(announce(catalogs, vec![one, two]).with_subscriber(subscriber));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut task)
                .await
                .is_err()
        );
        assert!(catalog_send.send(Some(pools)).is_ok());
        one_send.send(()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut task)
                .await
                .is_err()
        );
        two_send.send(()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut task)
                .await
                .is_err()
        );
        // A pool that was sufficient earlier must be checked again while its
        // sibling's swap/credit finishes. No per-pool success is latched.
        balance(&store, &jobs[0], 0).await;
        balance(&store, &jobs[1], 1_000_000).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut task)
                .await
                .is_err()
        );
        assert!(
            !std::fs::read_to_string(log.path())
                .unwrap()
                .contains("All MCP endpoints")
        );
        balance(&store, &jobs[0], 1_000_000).await;
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        balance(&store, &jobs[0], 2_000_000).await;
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            logs.matches("All MCP endpoints listening, funded, and ready")
                .count(),
            1,
            "{logs}"
        );
        assert!(
            logs.contains("INFO") && logs.contains("saved USDC balance estimates"),
            "{logs}"
        );
        store
            .call(|s| {
                assert!(s.status()?.treasury_operations.is_empty());
                assert_eq!(s.funding_jobs()?.len(), 2);
                Ok(())
            })
            .await
            .unwrap();
        drop(store);
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn closed_catalog_or_listener_and_cancellation_release_store_ownership() {
        let (_dir, store, worker, pools, _jobs) = fixture();
        let (catalog_send, catalogs) = oneshot::channel();
        drop(catalog_send);
        announce(catalogs, vec![]).await;
        let (catalog_send, catalogs) = oneshot::channel();
        assert!(catalog_send.send(Some(pools.clone())).is_ok());
        let (listener_send, listener) = oneshot::channel();
        drop(listener_send);
        announce(catalogs, vec![listener]).await;
        let (catalog_send, catalogs) = oneshot::channel();
        assert!(catalog_send.send(Some(pools.clone())).is_ok());
        let mut pending = Box::pin(announce(catalogs, vec![]));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut pending)
                .await
                .is_err()
        );
        drop(pending);
        drop(pools);
        drop(store);
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
    }
}
