//! Catalog-free initial funding, using the ordinary durable worker and owner.
use super::*;
use crate::rotation::store::{
    StoreHandle,
    bootstrap::{GROUP_PAGE, StartupClaim, StartupDisposition},
};

#[derive(Serialize)]
pub struct BootstrapSummary {
    pub schema_version: u32,
    pub serving_ready_groups: Vec<String>,
    pub inventory_ready_groups: Vec<String>,
    pub captured_completed_groups: Vec<String>,
    pub incomplete_captured_jobs: Vec<String>,
}

async fn bootstrap_summary(
    store: &StoreHandle,
    names: &BTreeSet<String>,
) -> Result<BootstrapSummary> {
    let mut summary = BootstrapSummary {
        schema_version: 2,
        serving_ready_groups: Vec::new(),
        inventory_ready_groups: Vec::new(),
        captured_completed_groups: names.iter().cloned().collect(),
        incomplete_captured_jobs: Vec::new(),
    };
    let names: Vec<_> = names.iter().cloned().collect();
    for page in names.chunks(GROUP_PAGE) {
        let page = page.to_vec();
        for group in store.call(move |s| s.startup_groups(&page)).await? {
            if group.serving_ready {
                summary.serving_ready_groups.push(group.name.clone());
            }
            if group.inventory_ready {
                summary.inventory_ready_groups.push(group.name);
            }
        }
    }
    Ok(summary)
}

/// Explicit funding authority still requires the deployment's auto_fund policy.
/// No listener tokens, static signing keys, catalogs or pricing are loaded.
pub async fn bootstrap_wallets(path: &Path) -> Result<BootstrapSummary> {
    ensure!(
        !crate::qualification::active() && !catalog_evidence::collecting(),
        "wallet bootstrap is unavailable during qualification"
    );
    let mut config: MetaConfig =
        toml::from_str(&tokio::fs::read_to_string(path).await?).context("invalid meta-config")?;
    if let Some(policy) = &mut config.source_management {
        policy.resolve(path);
    }
    let wallet_resolution = config.validate()?;
    crate::network::install(config.network.clone())?;
    if let Some(treasury) = &mut config.treasury {
        treasury.resolve(path);
    }
    crate::discovery::policy::validate_registry_path(&config, path)?;
    let deployment = Deployment {
        unavailable_sources: Default::default(),
        config,
        wallet_resolution,
        initialized: None,
        sources: BTreeMap::new(),
        selected: BTreeMap::new(),
        config_path: path.to_owned(),
    };
    deployment.bootstrap(&std::env::vars().collect()).await
}

impl Deployment {
    pub(super) async fn bootstrap(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<BootstrapSummary> {
        ensure!(
            self.config.funding.as_ref().is_some_and(|f| f.auto_fund),
            "wallet bootstrap requires funding.auto_fund=true; funding remains paused"
        );
        let names: BTreeSet<String> = self
            .wallet_resolution
            .wallets
            .iter()
            .filter(|(_, wallet)| wallet.managed())
            .map(|(name, _)| name.clone())
            .collect();
        ensure!(
            !names.is_empty(),
            "wallet bootstrap requires at least one managed wallet"
        );
        let initialized = self
            .initialize_wallets(env, Default::default(), Some(&names), &BTreeSet::new())
            .await?;
        let stop = CancellationToken::new();
        let _cancel_on_drop = stop.clone().drop_guard();
        // The supervisor retains ownership and drains even if its caller disappears.
        let attempts = startup_attempts(&self.wallet_resolution.wallets);
        let task = tokio::spawn(supervise(initialized, names, attempts, stop.clone()));
        crate::wallet_cli::finish_on_shutdown("bootstrap", &stop, async {
            task.await.context("bootstrap supervisor failed")?
        })
        .await
    }
}

/// The same treasury and funding tasks survive startup readiness and catalog I/O.
/// Dropping a startup waiter cancels new work, then drains the owned tasks.
type FundingTask = tokio::task::JoinHandle<Result<()>>;
pub(super) type FundingTasks = (Option<FundingTask>, Option<FundingTask>);
pub(super) struct FundingSupervisor {
    pub(super) stop: CancellationToken,
    store: Option<crate::rotation::store::StoreHandle>,
    treasury: Option<tokio::task::JoinHandle<Result<()>>>,
    funding: Option<tokio::task::JoinHandle<Result<()>>>,
    attempts: BTreeMap<String, u32>,
}
impl FundingSupervisor {
    fn start(
        owner: crate::treasury::Treasury,
        runtime: FundingRuntime,
        attempts: BTreeMap<String, u32>,
    ) -> Self {
        let store = owner.store_handle();
        let stop = CancellationToken::new();
        let (commands, sender, worker) = runtime;
        let treasury = tokio::spawn({
            let stopped = stop.clone();
            async move {
                let _stop_on_exit = stopped.clone().drop_guard();
                owner.run_commands(commands, sender, stopped).await
            }
        });
        let funding = tokio::spawn({
            let stopped = stop.clone();
            async move {
                let _stop_on_exit = stopped.clone().drop_guard();
                worker.run(stopped).await
            }
        });
        Self {
            stop,
            store: Some(store),
            treasury: Some(treasury),
            funding: Some(funding),
            attempts,
        }
    }
    pub(super) fn store(&self) -> crate::rotation::store::StoreHandle {
        self.store.as_ref().expect("owned funding store").clone()
    }
    pub(super) fn into_tasks(mut self) -> FundingTasks {
        self.store.take();
        (self.treasury.take(), self.funding.take())
    }
    pub(super) async fn shutdown(mut self) -> Result<()> {
        self.stop.cancel();
        self.store.take();
        let funding = self.funding.take();
        let treasury = self.treasury.take();
        let funding_result = match funding {
            Some(task) => task
                .await
                .context("startup funding drain failed")
                .and_then(|result| result),
            None => Ok(()),
        };
        let treasury_result = match treasury {
            Some(task) => task
                .await
                .context("startup treasury drain failed")
                .and_then(|result| result),
            None => Ok(()),
        };
        funding_result?;
        treasury_result
    }
    async fn wait_inventory(&self, names: &BTreeSet<String>) -> Result<()> {
        let mut next_progress = tokio::time::Instant::now();
        loop {
            ensure!(
                !self.stop.is_cancelled(),
                "startup funding owner stopped; persisted work retained"
            );
            let progress = tokio::time::Instant::now() >= next_progress;
            if check_startup(&self.store(), names, &self.attempts, None, progress).await? {
                return Ok(());
            }
            if progress {
                next_progress = tokio::time::Instant::now() + Duration::from_secs(60);
            }
            tokio::select! {
                _ = self.stop.cancelled() => bail!("startup funding owner stopped; persisted work retained"),
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }
}
impl Drop for FundingSupervisor {
    fn drop(&mut self) {
        if self.treasury.is_none() && self.funding.is_none() {
            return;
        }
        self.stop.cancel();
        self.store.take();
        let treasury = self.treasury.take();
        let funding = self.funding.take();
        // Accepted financial commands retain their owner after a cancelled caller.
        tokio::spawn(async move {
            if let Some(task) = funding
                && !matches!(task.await, Ok(Ok(())))
            {
                tracing::warn!(
                    category = "startup_funding_drain_failed",
                    "Startup funding drain failed; persisted financial work retained"
                );
            }
            if let Some(task) = treasury
                && !matches!(task.await, Ok(Ok(())))
            {
                tracing::warn!(
                    category = "startup_treasury_drain_failed",
                    "Startup treasury drain failed; persisted financial work retained"
                );
            }
        });
    }
}

impl Deployment {
    pub(super) async fn bootstrap_for_serving(
        &self,
        env: &BTreeMap<String, String>,
        restriction: crate::rotation::restriction::FundingRestriction,
    ) -> Result<InitializedWallets> {
        let names: BTreeSet<_> = self
            .wallet_resolution
            .wallets
            .iter()
            .filter(|(_, wallet)| wallet.managed())
            .map(|(name, _)| name.clone())
            .collect();
        let needed = self
            .config
            .discovery_relay
            .as_ref()
            .filter(|policy| policy.serve)
            .map(|policy| {
                crate::discovery_relay::resolve_wallets(
                    policy,
                    &self.config,
                    &self.wallet_resolution,
                    self.config.sources.keys().cloned(),
                )
            })
            .transpose()?
            .unwrap_or_default()
            .into_values()
            .collect();
        let mut initialized = self
            .initialize_wallets(env, restriction, None, &needed)
            .await?;
        let owner = initialized
            .treasury
            .take()
            .context("startup treasury unavailable")?;
        let runtime = initialized
            .funding_runtime
            .take()
            .context("startup funding worker unavailable")?;
        let supervisor = FundingSupervisor::start(
            owner,
            runtime,
            startup_attempts(&self.wallet_resolution.wallets),
        );
        if self.config.funding.as_ref().is_some_and(|f| f.auto_fund)
            && let Err(error) = supervisor.wait_inventory(&names).await
        {
            drop(initialized);
            supervisor.shutdown().await?;
            return Err(error);
        }
        initialized.supervisor = Some(supervisor);
        Ok(initialized)
    }
}

impl InitializedWallets {
    pub(super) async fn shutdown(mut self) -> Result<()> {
        let supervisor = self.supervisor.take();
        drop(self);
        if let Some(supervisor) = supervisor {
            supervisor.shutdown().await?;
        }
        Ok(())
    }
}

async fn supervise(
    initialized: InitializedWallets,
    names: BTreeSet<String>,
    attempts: BTreeMap<String, u32>,
    stop: CancellationToken,
) -> Result<BootstrapSummary> {
    let InitializedWallets {
        wallets,
        treasury,
        funding_runtime,
        supervisor,
        managed_pools,
    } = initialized;
    ensure!(
        supervisor.is_none(),
        "explicit bootstrap received a running supervisor"
    );
    let owner = treasury.context("bootstrap treasury unavailable")?;
    let store = owner.store_handle();
    // No paid clients or pool gates are needed. Drop their store handles before
    // closing the owner, whose close waits for the store worker to drain.
    drop(wallets);
    drop(managed_pools);
    let captured = match capture_startup(&store, &names).await {
        Ok(captured) => captured,
        Err(error) => {
            drop(funding_runtime);
            drop(store);
            owner.close().await?;
            return Err(error);
        }
    };
    let initial = check_startup(&store, &names, &attempts, Some(&captured), true).await;
    if !matches!(initial, Ok(false)) {
        let summary = match initial {
            Ok(_) => bootstrap_summary(&store, &names).await,
            Err(error) => Err(error),
        };
        drop(funding_runtime);
        drop(store);
        owner.close().await?;
        tracing::info!(
            wallets = names.len(),
            "Initial managed capacity bootstrap already complete"
        );
        return summary;
    }
    let (commands, sender, worker) =
        funding_runtime.context("bootstrap funding worker unavailable")?;
    tracing::info!(
        wallets = names.len(),
        "Bootstrapping initial managed wallet capacity before discovery; waiting for confirmed USDC credit"
    );
    let monitored_names = names.clone();
    let monitor = async move {
        let mut next_progress = tokio::time::Instant::now();
        loop {
            let progress = tokio::time::Instant::now() >= next_progress;
            if check_startup(
                &store,
                &monitored_names,
                &attempts,
                Some(&captured),
                progress,
            )
            .await?
            {
                return bootstrap_summary(&store, &monitored_names).await;
            }
            if progress {
                next_progress = tokio::time::Instant::now() + Duration::from_secs(60);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };
    let summary = run_workers(
        monitor,
        owner.run_commands(commands, sender, stop.clone()),
        worker.run(stop.clone()),
        stop,
    )
    .await?;
    tracing::info!(
        wallets = names.len(),
        "Initial managed capacity bootstrap complete"
    );
    Ok(summary)
}

fn startup_attempts(wallets: &BTreeMap<String, WalletConfig>) -> BTreeMap<String, u32> {
    wallets
        .iter()
        .filter_map(|(name, wallet)| match wallet {
            WalletConfig::ZcashRotation { max_attempts, .. } => Some((name.clone(), *max_attempts)),
            _ => None,
        })
        .collect()
}

async fn capture_startup(
    store: &StoreHandle,
    names: &BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<StartupClaim>>> {
    let mut captured = BTreeMap::new();
    let names: Vec<_> = names.iter().cloned().collect();
    for page in names.chunks(GROUP_PAGE) {
        let page = page.to_vec();
        for group in store.call(move |s| s.startup_groups(&page)).await? {
            captured.insert(group.name, group.claims);
        }
    }
    Ok(captured)
}

/// A serving pass checks destination ownership; explicit bootstrap checks only
/// its captured destination jobs. Neither waits for their source-only backlog.
async fn check_startup(
    store: &StoreHandle,
    names: &BTreeSet<String>,
    attempts: &BTreeMap<String, u32>,
    captured: Option<&BTreeMap<String, Vec<StartupClaim>>>,
    progress: bool,
) -> Result<bool> {
    let names: Vec<_> = names.iter().cloned().collect();
    let mut all_ready = true;
    for page in names.chunks(GROUP_PAGE) {
        let page = page.to_vec();
        let groups = store.call(move |s| s.startup_groups(&page)).await?;
        for group in groups {
            ensure!(group.enabled, "bootstrap group disabled");
            if captured.is_none() && group.serving_ready {
                continue;
            }
            let claims = captured
                .and_then(|c| c.get(&group.name))
                .unwrap_or(&group.claims)
                .clone();
            let maximum = *attempts
                .get(&group.name)
                .context("startup funding policy missing")?;
            let pool = group.pool.clone();
            let query_claims = claims.clone();
            let paths = store
                .call(move |s| s.startup_claim_paths(&pool, &query_claims))
                .await?;
            let PathAssessment {
                path_count,
                pending,
                actionable,
            } = assess_startup_paths(paths, maximum, &group, progress);
            if captured.is_some() && path_count == claims.len() && pending == 0 {
                continue;
            }
            all_ready = false;
            if !actionable && pending > 0 {
                let name = group.name.clone();
                let captured_claims = captured.map(|_| claims.clone());
                if let Some(obstructions) = store
                    .call(move |s| {
                        s.startup_obstruction(&name, maximum, captured_claims.as_deref())
                    })
                    .await?
                {
                    return Err(StartupObstructed {
                        group: group.name,
                        obstructions,
                    }
                    .into());
                }
            }
            // No job rows is incomplete, never an impossibility proof. Schema16
            // incorporates enabled vacant claims in this same structural query.
        }
    }
    Ok(all_ready)
}
struct PathAssessment {
    path_count: usize,
    pending: usize,
    actionable: bool,
}

fn assess_startup_paths(
    paths: Vec<crate::rotation::store::bootstrap::StartupPath>,
    maximum: u32,
    group: &crate::rotation::store::bootstrap::StartupGroup,
    progress: bool,
) -> PathAssessment {
    let mut path_count = 0;
    let mut pending = 0;
    let mut actionable = false;
    let mut logged = 0;
    for path in paths {
        path_count += 1;
        let disposition = path.disposition(maximum, group.source_pending);
        if disposition == StartupDisposition::DestinationReady {
            continue;
        }
        pending += 1;
        actionable |= !matches!(disposition, StartupDisposition::Obstructed(_));
        if progress && logged < 8 {
            logged += 1;
            tracing::info!(wallet=%group.name,job_id=%path.job.id,phase=?path.job.phase,disposition=?disposition,source_submission=path.operation.as_ref().map(|o|o.submission.as_str()),"Bootstrap awaiting destination inventory; original operation remains owned");
        }
    }
    PathAssessment {
        path_count,
        pending,
        actionable,
    }
}

#[derive(Debug)]
struct StartupObstructed {
    group: String,
    obstructions: Vec<(String, &'static str)>,
}
impl std::fmt::Display for StartupObstructed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "wallet {}: startup incomplete;", self.group)?;
        for (job, reason) in &self.obstructions {
            write!(f, " job={job} reason={reason};")?;
        }
        write!(
            f,
            " accepted work and reservations retained; inspect wallet recover --config FILE"
        )
    }
}
impl std::error::Error for StartupObstructed {}

async fn run_workers<T>(
    monitor: impl std::future::Future<Output = Result<T>>,
    treasury: impl std::future::Future<Output = Result<()>> + Send + 'static,
    funding: impl std::future::Future<Output = Result<()>> + Send + 'static,
    stop: CancellationToken,
) -> Result<T> {
    let mut treasury_task = tokio::spawn(treasury);
    let mut funding_task = tokio::spawn(funding);
    let mut treasury_finished = false;
    let mut funding_finished = false;
    let result: Result<T> = tokio::select! {
        result = monitor => result,
        _ = stop.cancelled() => Err(anyhow::anyhow!("wallet bootstrap interrupted; persisted funding progress retained")),
        result = &mut treasury_task => {
            treasury_finished = true;
            result.context("bootstrap treasury task failed").and_then(|r| r)
                .and(Err(anyhow::anyhow!("bootstrap treasury stopped")))
        },
        result = &mut funding_task => {
            funding_finished = true;
            result.context("bootstrap funding task failed").and_then(|r| r)
                .and(Err(anyhow::anyhow!("bootstrap funding stopped")))
        },
    };
    stop.cancel();
    tracing::info!("Bootstrap draining accepted financial work before closing treasury");
    // Always drain both, including when one fails. Never abort a funding task.
    let funding_result = if funding_finished {
        Ok(())
    } else {
        funding_task
            .await
            .context("bootstrap funding task failed")
            .and_then(|r| r)
    };
    let treasury_result = if treasury_finished {
        Ok(())
    } else {
        treasury_task
            .await
            .context("bootstrap treasury task failed")
            .and_then(|r| r)
    };
    let value = result?;
    funding_result?;
    treasury_result?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bootstrap_output_contract_v2_separates_current_and_captured_inventory() {
        let summary = BootstrapSummary {
            schema_version: 2,
            serving_ready_groups: vec!["web".into()],
            inventory_ready_groups: vec![],
            captured_completed_groups: vec!["web".into()],
            incomplete_captured_jobs: vec![],
        };
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/capacity/bootstrap-output-v2.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(summary).unwrap(), expected);
    }

    #[tokio::test]
    async fn scoped_startup_waits_for_actionable_sibling_and_reports_only_fixed_obstructions() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        crate::rotation::test_capacity::declare_and_allocate!(
            state,
            "web",
            crate::rotation::test_capacity::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let jobs = state.funding_jobs().unwrap();
        state
            .defer_funding(
                &jobs[0].id,
                0,
                Some("treasury_insufficient_spendable_funds; private fixture prose"),
                false,
            )
            .unwrap();
        let (store, owner) = StoreHandle::spawn(state);
        let names = BTreeSet::from(["web".to_owned()]);
        let attempts = BTreeMap::from([("web".to_owned(), 3)]);
        assert!(
            !check_startup(&store, &names, &attempts, None, false)
                .await
                .unwrap()
        );
        let blocked = jobs[1].id.clone();
        store
            .call(move |s| {
                s.defer_funding(
                    &blocked,
                    0,
                    Some("funding_amount_limit_exceeded; private fixture prose"),
                    false,
                )
            })
            .await
            .unwrap();
        let error = check_startup(&store, &names, &attempts, None, false)
            .await
            .unwrap_err();
        assert!(error.is::<StartupObstructed>());
        let message = error.to_string();
        for job in &jobs {
            assert!(message.contains(&job.id));
            assert!(!message.contains(&job.recipient));
        }
        assert!(!message.contains("private fixture prose"));
        let captured = capture_startup(&store, &names).await.unwrap();
        store
            .call(move |s| {
                for job in jobs {
                    s.record_credit(&job.wallet_id, &job.target, "fixture", 100)?;
                }
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            check_startup(&store, &names, &attempts, None, false)
                .await
                .unwrap()
        );
        assert!(
            check_startup(&store, &names, &attempts, Some(&captured), false)
                .await
                .unwrap()
        );
        drop(store);
        owner.await.unwrap();
    }

    #[tokio::test]
    async fn startup_handoff_keeps_the_same_store_and_workers_alive() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let (store, store_owner) = crate::rotation::store::StoreHandle::spawn(store);
        let stop = CancellationToken::new();
        let treasury_stop = stop.clone();
        let treasury = tokio::spawn(async move {
            treasury_stop.cancelled().await;
            store_owner.await?;
            Ok(())
        });
        let funding_stop = stop.clone();
        let funding_store = store.clone();
        let funding = tokio::spawn(async move {
            funding_stop.cancelled().await;
            drop(funding_store);
            Ok(())
        });
        let supervisor = FundingSupervisor {
            stop: stop.clone(),
            store: Some(store.clone()),
            treasury: Some(treasury),
            funding: Some(funding),
            attempts: BTreeMap::new(),
        };
        let (treasury, funding) = supervisor.into_tasks();
        assert!(
            !stop.is_cancelled(),
            "readiness handoff must not stop funding"
        );
        assert_eq!(store.call(|s| Ok(s.status()?.birthday)).await.unwrap(), 1);
        assert!(!treasury.as_ref().unwrap().is_finished());
        assert!(!funding.as_ref().unwrap().is_finished());
        drop(store);
        stop.cancel();
        funding.unwrap().await.unwrap().unwrap();
        treasury.unwrap().await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn dropped_startup_owner_drains_accepted_work_and_releases_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let (store, store_owner) = crate::rotation::store::StoreHandle::spawn(store);
        let stop = CancellationToken::new();
        let (finish, accepted) = tokio::sync::oneshot::channel::<()>();
        let (closed, closure) = tokio::sync::oneshot::channel();
        let treasury_stop = stop.clone();
        let treasury = tokio::spawn(async move {
            treasury_stop.cancelled().await;
            store_owner.await?;
            let _ = closed.send(());
            Ok(())
        });
        let funding_store = store.clone();
        let funding = tokio::spawn(async move {
            accepted.await?;
            funding_store
                .call(|s| {
                    crate::rotation::test_capacity::declare_and_allocate!(
                        s,
                        "accepted",
                        crate::rotation::test_capacity::two_reserves("2"),
                        Some(alloy_primitives::U256::from(1_000_000u64)),
                        0
                    )?;
                    Ok(())
                })
                .await?;
            drop(funding_store);
            Ok(())
        });
        drop(FundingSupervisor {
            stop: stop.clone(),
            store: Some(store),
            treasury: Some(treasury),
            funding: Some(funding),
            attempts: BTreeMap::new(),
        });
        assert!(stop.is_cancelled());
        finish.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), closure)
            .await
            .unwrap()
            .unwrap();
        let status = crate::rotation::store::status(&dir.path().join("state")).unwrap();
        assert!(status.pools.iter().any(|pool| pool.name == "accepted"));
    }

    #[tokio::test]
    async fn cancellation_drains_accepted_funding_and_releases_monitor_before_owner() {
        let stop = CancellationToken::new();
        let (store, released) = tokio::sync::oneshot::channel::<()>();
        let (finish, accepted) = tokio::sync::oneshot::channel::<()>();
        let owner_stop = stop.clone();
        let (closing, owner_closing) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run_workers::<()>(
            async move {
                let _store = store;
                std::future::pending::<Result<()>>().await
            },
            async move {
                owner_stop.cancelled().await;
                let _ = released.await;
                closing.send(()).unwrap();
                Ok(())
            },
            async move {
                accepted.await?;
                Ok(())
            },
            stop.clone(),
        ));
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(2), owner_closing)
            .await
            .unwrap()
            .unwrap();
        assert!(!task.is_finished(), "accepted financial work must drain");
        finish.send(()).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("interrupted"));
    }
    #[tokio::test]
    async fn funding_failure_cancels_and_drains_owner() {
        let stop = CancellationToken::new();
        let owner_stop = stop.clone();
        let (closed, closure) = tokio::sync::oneshot::channel();
        let result = run_workers::<()>(
            std::future::pending(),
            async move {
                owner_stop.cancelled().await;
                closed.send(()).unwrap();
                Ok(())
            },
            async { anyhow::bail!("fixture funding failure") },
            stop.clone(),
        )
        .await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("fixture funding failure")
        );
        assert!(stop.is_cancelled());
        closure.await.unwrap();
    }
    #[tokio::test]
    async fn completed_bootstrap_stops_both_workers() {
        let stop = CancellationToken::new();
        let owner_stop = stop.clone();
        let funding_stop = stop.clone();
        run_workers::<()>(
            async { Ok(()) },
            async move {
                owner_stop.cancelled().await;
                Ok(())
            },
            async move {
                funding_stop.cancelled().await;
                Ok(())
            },
            stop.clone(),
        )
        .await
        .unwrap();
        assert!(stop.is_cancelled());
    }
}

#[cfg(test)]
#[path = "bootstrap_paused_tests.rs"]
mod paused_tests;

#[cfg(test)]
#[path = "bootstrap_actor_tests.rs"]
mod actor_tests;

#[cfg(test)]
#[path = "bootstrap_readiness_tests.rs"]
mod readiness_tests;
