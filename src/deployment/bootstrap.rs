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
    pub balance_estimates: Vec<BootstrapBalance>,
}

#[derive(Serialize)]
pub struct BootstrapBalance {
    pub wallet_name: String,
    pub wallet_type: String,
    pub balance_estimate_usdc: String,
}

async fn bootstrap_summary(
    store: &StoreHandle,
    names: &BTreeSet<String>,
    captured: &BTreeMap<String, Vec<StartupClaim>>,
) -> Result<BootstrapSummary> {
    let mut summary = BootstrapSummary {
        schema_version: 3,
        serving_ready_groups: Vec::new(),
        inventory_ready_groups: Vec::new(),
        captured_completed_groups: names.iter().cloned().collect(),
        incomplete_captured_jobs: Vec::new(),
        balance_estimates: Vec::new(),
    };
    let names: Vec<_> = names.iter().cloned().collect();
    for page in names.chunks(GROUP_PAGE) {
        let page = page.to_vec();
        for group in store.call(move |s| s.startup_groups(&page)).await? {
            let claims = captured
                .get(&group.name)
                .context("bootstrap captured claims missing")?
                .clone();
            let pool = group.pool.clone();
            for (wallet_type, balance_estimate_usdc) in store
                .call(move |s| s.startup_claim_balances(&pool, &claims))
                .await?
            {
                summary.balance_estimates.push(BootstrapBalance {
                    wallet_name: group.name.clone(),
                    wallet_type,
                    balance_estimate_usdc,
                });
            }
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
    bootstrap_wallets_with_network(path, None).await
}
pub async fn bootstrap_wallets_with_network(
    path: &Path,
    network_override: Option<&crate::network::NetworkConfig>,
) -> Result<BootstrapSummary> {
    ensure!(
        !crate::qualification::active() && !catalog_evidence::collecting(),
        "wallet bootstrap is unavailable during qualification"
    );
    let mut config = MetaConfig::parse_with_network(
        &tokio::fs::read_to_string(path).await.with_context(|| {
            format!("failed to read deployment config file '{}'", path.display())
        })?,
        network_override,
    )?;
    crate::discovery::policy::resolve(&mut config, path);
    let wallet_resolution = config.validate()?;
    crate::network::install(config.network.clone())?;
    if let Some(treasury) = &mut config.treasury {
        treasury.resolve(path);
    }
    crate::discovery::policy::validate_registry_path(&config, path)?;
    let deployment = Deployment {
        listeners: wallet_resolution.source_bindings.listeners(&config),
        search_relay: None,
        prebound: BTreeMap::new(),
        pending_sources: BTreeMap::new(),
        defer_discovery: false,
        unavailable_sources: Default::default(),
        config,
        wallet_resolution,
        initialized: None,
        sources: BTreeMap::new(),
        selected: BTreeMap::new(),
        conflicting_tools: BTreeMap::new(),
        selection_errors: BTreeMap::new(),
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

impl InitializedWallets {
    pub(super) async fn shutdown(self) -> Result<()> {
        let Self {
            wallets,
            runtime,
            managed_pools,
        } = self;
        drop(wallets);
        drop(managed_pools);
        runtime.close().await
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
        runtime,
        managed_pools,
    } = initialized;
    // No paid clients or pool gates are needed. Drop their store handles before
    // closing the owner, whose close waits for the store worker to drain.
    drop(wallets);
    drop(managed_pools);
    let treasury = match runtime {
        ManagedRuntime::Treasury(treasury) => treasury,
        runtime => {
            runtime.close().await?;
            bail!("explicit bootstrap requires treasury SDK");
        }
    };
    let store = treasury.owner.store_handle();
    let captured = match capture_startup(&store, &names).await {
        Ok(captured) => captured,
        Err(error) => {
            drop(store);
            ManagedRuntime::Treasury(treasury).close().await?;
            return Err(error);
        }
    };
    let initial = check_startup(&store, &names, &attempts, Some(&captured), true).await;
    if !matches!(initial, Ok(false)) {
        let summary = match initial {
            Ok(_) => bootstrap_summary(&store, &names, &captured).await,
            Err(error) => Err(error),
        };
        drop(store);
        ManagedRuntime::Treasury(treasury).close().await?;
        if summary.is_ok() {
            tracing::info!(
                wallets = names.len(),
                "Initial managed capacity bootstrap already complete"
            );
        }
        return summary;
    }
    let TreasuryRuntime {
        owner,
        funding: (commands, sender, worker),
    } = *treasury;
    tracing::info!(
        wallets = names.len(),
        "Explicit wallet bootstrap waiting for captured initial USDC credit"
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
                return bootstrap_summary(&store, &monitored_names, &captured).await;
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
            let waiting_for = if path.job.phase
                == crate::rotation::store::funding::FundingPhase::Quoted
                && group.source_pending
            {
                "another treasury transaction to confirm"
            } else {
                "initial USDC credit"
            };
            tracing::info!(wallet=%group.name,size=%path.wallet_type,status=?path.job.phase,waiting_for,"Bootstrap funding in progress");
            tracing::debug!(wallet=%group.name,job_id=%path.job.id,phase=?path.job.phase,disposition=?disposition,source_submission=path.operation.as_ref().map(|o|o.submission.as_str()),"Bootstrap funding observation details");
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
            let explanation = match *reason {
                "treasury_insufficient_spendable_funds" => {
                    "the treasury has insufficient spendable ZEC; fund it and allow confirmations before retrying"
                }
                "funding_disabled" | "funding_group_disabled" => {
                    "funding is disabled; check the deployment funding and wallet settings"
                }
                "qualification_funding_denied" => {
                    "this qualification run does not authorize funding"
                }
                "funding_policy_requires_review" => {
                    "funding was blocked by cost, amount or authentication policy; review the configured funding limits and credentials"
                }
                "funding_evidence_requires_review" | "destination_support_requires_review" => {
                    "saved funding evidence is inconsistent and requires operator review"
                }
                "recovery_attempt_limit_reached" => {
                    "automatic recovery attempts are exhausted; operator review is required"
                }
                "funding_outcome_requires_review" => {
                    "the swap failed, was refunded or has an uncertain outcome; operator review is required"
                }
                "preparation_or_swap_requires_review" => {
                    "transaction preparation or swap completion is unresolved; operator review is required"
                }
                _ => "funding cannot proceed; inspect the saved funding state",
            };
            write!(f, " {explanation} (job={job}, reason={reason});")?;
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
            schema_version: 3,
            serving_ready_groups: vec!["web".into()],
            inventory_ready_groups: vec![],
            captured_completed_groups: vec!["web".into()],
            incomplete_captured_jobs: vec![],
            balance_estimates: vec![],
        };
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/capacity/bootstrap-output-v3.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(summary).unwrap(), expected);
    }

    #[test]
    fn info_progress_uses_wallet_name_and_size_without_internal_identifiers() {
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
        let group = state.startup_groups(&["web".into()]).unwrap().remove(0);
        let paths = state
            .startup_claim_paths(&group.pool, &group.claims)
            .unwrap();
        let ids: Vec<_> = paths.iter().map(|p| p.job.id.clone()).collect();
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(log.reopen().unwrap())
            .finish();
        {
            let _guard = tracing::subscriber::set_default(subscriber);
            assess_startup_paths(paths, 3, &group, true);
        }
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(text.contains("wallet=web"), "{text}");
        assert!(
            text.contains("size=small") && text.contains("size=large"),
            "{text}"
        );
        for id in ids {
            assert!(!text.contains(&id));
        }
        assert!(!text.contains("job_id") && !text.contains(&group.pool));
    }

    #[tokio::test]
    async fn rejected_store_only_bootstrap_closes_owner_before_returning() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state");
        let key = dir.path().join("key");
        let state =
            crate::rotation::store::Store::create(&state_path, &key, 1, b"fixture").unwrap();
        let id = state.id().to_owned();
        let initialized = InitializedWallets {
            wallets: BTreeMap::new(),
            runtime: ManagedRuntime::StoreOnly(StoreOnlyOwner::new(state)),
            managed_pools: Vec::new(),
        };
        let error = supervise(
            initialized,
            BTreeSet::new(),
            BTreeMap::new(),
            CancellationToken::new(),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            "explicit bootstrap requires treasury SDK"
        );
        let reopened = crate::rotation::store::Store::open(&state_path, &key, &id).unwrap();
        assert!(reopened.funding_jobs().unwrap().is_empty());
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
        state
            .observe_funding_condition(
                &jobs[0].id,
                &jobs[0].operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
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
                )?;
                let operation = s.funding_job(&blocked)?.unwrap().operation_id;
                s.observe_funding_condition(
                    &blocked,
                    &operation,
                    Some(crate::rotation::error::FundingCondition::AmountLimit),
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
        assert!(message.contains("insufficient spendable ZEC"));
        assert!(message.contains("cost, amount or authentication policy"));
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
