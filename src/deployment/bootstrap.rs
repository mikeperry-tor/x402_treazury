//! Catalog-free initial funding, using the ordinary durable worker and owner.
use super::*;
use crate::rotation::store::{Status, funding::FundingPhase};

#[derive(Serialize)]
pub struct BootstrapSummary {
    /// Initial pairs have received confirmed credit; this is not a live balance report.
    pub bootstrapped_wallets: Vec<String>,
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
        let mut task = tokio::spawn(supervise(initialized, names, stop.clone()));
        tokio::select! {
            result = &mut task => result.context("bootstrap supervisor failed")?,
            signal = crate::wallet_cli::shutdown_signal() => {
                stop.cancel();
                tracing::warn!("Wallet bootstrap stopping; waiting for accepted financial work to drain");
                let result = task.await.context("bootstrap supervisor failed")?;
                signal?;
                result
            }
        }
    }
}

fn complete(status: &Status, names: &BTreeSet<String>) -> Result<bool> {
    let mut ready = true;
    for name in names {
        let pool = status
            .pools
            .iter()
            .find(|pool| &pool.name == name)
            .context("bootstrap pool missing from state")?;
        ensure!(pool.enabled, "wallet {name}: pool is disabled");
        let blocked: Vec<_> = status
            .funding_jobs
            .iter()
            .filter(|job| {
                job.pool_id == pool.id
                    // Confirmed destination credit completes bootstrap even if
                    // source accounting still retains a historical timeout.
                    // The funding worker reconciles that outbox independently.
                    && job.phase != FundingPhase::Complete
                    && (job.timed_out
                        || matches!(
                            job.phase,
                            FundingPhase::RecoveryRequired | FundingPhase::RefundPending
                        ))
            })
            .collect();
        if pool.bootstrapped && blocked.is_empty() {
            continue;
        }
        let automatic = !blocked.is_empty()
            && blocked
                .iter()
                .all(|job| crate::treasury::recovery::can_recover(status, job));
        let manual: Vec<_> = blocked
            .into_iter()
            .filter(|job| !crate::treasury::recovery::can_recover(status, job))
            .collect();
        if !manual.is_empty() {
            let details = manual
                .into_iter()
                .map(|job| blocked_job(status, job))
                .collect::<Vec<_>>()
                .join("\n");
            bail!("wallet {name}: bootstrap blocked; completed funding is retained\n{details}");
        }
        ensure!(
            !pool.funding_degraded
                || automatic
                || status.funding_jobs.iter().any(|job| {
                    job.pool_id == pool.id && job.phase == FundingPhase::Complete && job.timed_out
                }),
            "wallet {name}: funding is degraded with no recovery job identified; inspect wallet status --config FILE; completed funding is retained"
        );
        ready = false;
    }
    Ok(ready)
}

fn blocked_job(status: &Status, job: &crate::rotation::store::funding::FundingJob) -> String {
    let operation = status
        .treasury_operations
        .iter()
        .find(|op| op.operation_id == job.operation_id);
    let phase = serde_json::to_value(&job.phase).expect("funding phase serializes");
    let phase = phase.as_str().expect("funding phase is a string");
    let source = operation
        .map(|op| format!("{} (submission attempts={})", op.submission, op.attempts))
        .unwrap_or_else(|| {
            "not recorded; this alone does not prove preparation never started".into()
        });
    let next = "Run wallet recover --config FILE for a guarded recovery assessment. Eligible jobs recover automatically during bootstrap; unresolved preparation, refunds and conflicting evidence require review. Completed funding and reservations are retained.";
    format!(
        "  job={} phase={} timed_out={} quote_attempts={}\n  last_error={}\n  source_operation={}\n  {}",
        job.id,
        phase,
        job.timed_out,
        job.attempts,
        job.last_error
            .as_deref()
            .unwrap_or("not recorded; the original cause cannot be inferred from this phase"),
        source,
        next
    )
}

async fn supervise(
    initialized: InitializedWallets,
    names: BTreeSet<String>,
    stop: CancellationToken,
) -> Result<BootstrapSummary> {
    let InitializedWallets {
        wallets,
        treasury,
        funding_runtime,
        managed_pools,
    } = initialized;
    let owner = treasury.context("bootstrap treasury unavailable")?;
    let store = owner.store_handle();
    // No paid clients or pool gates are needed. Drop their store handles before
    // closing the owner, whose close waits for the store worker to drain.
    drop(wallets);
    drop(managed_pools);
    let initial = store
        .call(|s| s.status())
        .await
        .and_then(|s| {
            let recovering = s.funding_jobs.iter().filter(|job| names.contains(&job.pool_name)
                && crate::treasury::recovery::needs_recovery(job)
                && crate::treasury::recovery::can_recover(&s, job)).count();
            if recovering > 0 {
                tracing::info!(jobs = recovering, "Recovering existing funding before bootstrap; waiting for verified transaction expiry where required; no manual recovery command needed");
            }
            complete(&s, &names)
        });
    if !matches!(initial, Ok(false)) {
        drop(funding_runtime);
        drop(store);
        owner.close().await?;
        initial?;
        tracing::info!(
            wallets = names.len(),
            "Initial managed wallet bootstrap already complete"
        );
        return Ok(BootstrapSummary {
            bootstrapped_wallets: names.into_iter().collect(),
        });
    }
    let (commands, sender, worker) =
        funding_runtime.context("bootstrap funding worker unavailable")?;
    tracing::info!(
        wallets = names.len(),
        "Bootstrapping initial managed wallet pairs before discovery; waiting for confirmed USDC credit"
    );
    let monitored_names = names.clone();
    let monitor = async move {
        let mut next_progress = tokio::time::Instant::now();
        loop {
            let status = store.call(|s| s.status()).await?;
            if complete(&status, &monitored_names)? {
                return Ok(());
            }
            if tokio::time::Instant::now() >= next_progress {
                log_progress(&status, &monitored_names);
                next_progress = tokio::time::Instant::now() + Duration::from_secs(60);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };
    run_workers(
        monitor,
        owner.run_commands(commands, sender, stop.clone()),
        worker.run(stop.clone()),
        stop,
    )
    .await?;
    tracing::info!(
        wallets = names.len(),
        "Initial managed wallet bootstrap complete"
    );
    Ok(BootstrapSummary {
        bootstrapped_wallets: names.into_iter().collect(),
    })
}

fn log_progress(status: &Status, names: &BTreeSet<String>) {
    for job in &status.funding_jobs {
        if !names.contains(&job.pool_name) || job.phase == FundingPhase::Complete {
            continue;
        }
        let operation = status
            .treasury_operations
            .iter()
            .find(|op| op.operation_id == job.operation_id);
        tracing::info!(
            wallet = %job.pool_name,
            job_id = %job.id,
            phase = ?job.phase,
            timed_out = job.timed_out,
            source_submission = operation.map(|op| op.submission.as_str()),
            expiry_height = operation.map(|op| op.facts.expiry_height),
            sync_height = status.sync.as_ref().and_then(|sync| sync.height),
            sync_fresh = status.sync_fresh,
            "Bootstrap waiting for existing funding; progress checked automatically"
        );
    }
}

// The monitor owns its store handle. Dropping it before joining the owner is
// essential: owner.close waits for every store handle to disappear.
async fn run_workers(
    monitor: impl std::future::Future<Output = Result<()>>,
    treasury: impl std::future::Future<Output = Result<()>> + Send + 'static,
    funding: impl std::future::Future<Output = Result<()>> + Send + 'static,
    stop: CancellationToken,
) -> Result<()> {
    let mut treasury_task = tokio::spawn(treasury);
    let mut funding_task = tokio::spawn(funding);
    let mut treasury_finished = false;
    let mut funding_finished = false;
    let result: Result<()> = tokio::select! {
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
    result?;
    funding_result?;
    treasury_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::store::PoolStatus;
    fn status() -> Status {
        Status {
            treasury_id: "test".into(),
            birthday: 0,
            snapshot_revision: 0,
            pools: vec![PoolStatus {
                id: "pool".into(),
                name: "web".into(),
                deposit_atomic: "2000000".into(),
                generation: 0,
                enabled: true,
                bootstrapped: false,
                funding_degraded: false,
                addresses: vec![],
            }],
            sync: None,
            outgoing_pending: false,
            sync_fresh: false,
            treasury_operations: vec![],
            funding_jobs: vec![],
            refunds: vec![],
        }
    }
    #[test]
    fn bootstrap_requires_every_initial_pair_and_rejects_degraded_or_disabled_pools() {
        let names = BTreeSet::from(["web".to_owned()]);
        let mut status = status();
        assert!(!complete(&status, &names).unwrap());
        status.pools[0].funding_degraded = true;
        assert!(
            complete(&status, &names)
                .unwrap_err()
                .to_string()
                .contains("degraded")
        );
        status.pools[0].funding_degraded = false;
        status.pools[0].bootstrapped = true;
        assert!(complete(&status, &names).unwrap());
        status.pools[0].enabled = false;
        assert!(complete(&status, &names).is_err());
        assert!(complete(&status, &BTreeSet::from(["missing".to_owned()])).is_err());
    }
    #[test]
    fn bootstrap_completion_requires_confirmed_credit_for_both_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture snapshot",
        )
        .unwrap();
        store.ensure_pool("web", "2").unwrap();
        let names = BTreeSet::from(["web".to_owned()]);
        let jobs = store.funding_jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        store
            .record_credit(&jobs[0].wallet_id, &jobs[0].target, "fixture", 1)
            .unwrap();
        assert!(!complete(&store.status().unwrap(), &names).unwrap());
        store
            .record_credit(&jobs[1].wallet_id, &jobs[1].target, "fixture", 1)
            .unwrap();
        assert!(complete(&store.status().unwrap(), &names).unwrap());
        store.ensure_pool("web", "2").unwrap();
        assert_eq!(store.funding_jobs().unwrap().len(), 2);
    }

    #[test]
    fn completed_swap_timeout_does_not_block_bootstrap_or_clear_accounting() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture snapshot",
        )
        .unwrap();
        store.ensure_pool("web", "2").unwrap();
        let names = BTreeSet::from(["web".to_owned()]);
        let jobs = store.funding_jobs().unwrap();
        store.mark_funding_timeout(&jobs[0].id).unwrap();
        store
            .record_credit(&jobs[0].wallet_id, &jobs[0].target, "fixture", 1)
            .unwrap();
        assert!(!complete(&store.status().unwrap(), &names).unwrap());
        store
            .record_credit(&jobs[1].wallet_id, &jobs[1].target, "fixture", 1)
            .unwrap();
        let status = store.status().unwrap();
        assert_eq!(status.funding_jobs[0].phase, FundingPhase::Complete);
        assert!(status.funding_jobs[0].timed_out);
        assert!(complete(&status, &names).unwrap());
        assert!(store.status().unwrap().funding_jobs[0].timed_out);

        // A genuinely unfinished timeout still waits, even in a pool whose
        // initial pair was previously funded.
        let mut unfinished = status;
        unfinished.funding_jobs[0].phase = FundingPhase::Swapping;
        assert!(!complete(&unfinished, &names).unwrap());
        unfinished.funding_jobs[0].phase = FundingPhase::RefundPending;
        assert!(complete(&unfinished, &names).is_err());
    }

    #[test]
    fn recovery_diagnostic_names_job_before_generic_degraded_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        store.ensure_pool("web", "2").unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        store
            .advance_funding(
                &job.id,
                FundingPhase::Allocated,
                FundingPhase::RecoveryRequired,
            )
            .unwrap();
        let mut status = store.status().unwrap();
        status.pools[0].funding_degraded = true;
        let names = BTreeSet::from(["web".to_owned()]);
        let error = complete(&status, &names).unwrap_err().to_string();
        assert!(error.contains(&job.id));
        assert!(error.contains("RECOVERY_REQUIRED"));
        assert!(error.contains("last_error=not recorded"));
        assert!(error.contains("wallet recover --config FILE"));
        assert!(!error.contains(&job.recipient));
        status
            .treasury_operations
            .push(crate::rotation::transaction::OperationStatus {
                operation_id: job.operation_id.clone(),
                facts: crate::rotation::transaction::TransactionFacts {
                    txid: "private-txid".into(),
                    expiry_height: 100,
                    amount_zatoshis: 1,
                    fee_zatoshis: 1,
                    deadline: 100,
                },
                submission: "UNKNOWN".into(),
                attempts: 1,
            });
        assert!(!complete(&status, &names).unwrap());
        let error = blocked_job(&status, &status.funding_jobs[0]);
        assert!(error.contains("source_operation=UNKNOWN (submission attempts=1)"));
        assert!(error.contains("wallet recover --config FILE"));
        assert!(!error.contains("recover-unprepared"));
        assert!(!error.contains("private-txid"));
        assert!(!error.contains("--rebroadcast"));
        status.treasury_operations[0].submission = "PREPARED".into();
        status.treasury_operations[0].attempts = 0;
        assert!(!complete(&status, &names).unwrap());
        let error = blocked_job(&status, &status.funding_jobs[0]);
        assert!(error.contains("wallet recover --config FILE"));
        assert!(!error.contains("Run wallet reconcile"));
        status.funding_jobs[0].last_error =
            Some("funding_recovery_requires_operator; recovery_attempt_limit_reached".into());
        assert!(
            complete(&status, &names)
                .unwrap_err()
                .to_string()
                .contains("recovery_attempt_limit_reached")
        );
    }

    #[tokio::test]
    async fn cancellation_drains_accepted_funding_and_releases_monitor_before_owner() {
        let stop = CancellationToken::new();
        let (store, released) = tokio::sync::oneshot::channel::<()>();
        let (finish, accepted) = tokio::sync::oneshot::channel::<()>();
        let owner_stop = stop.clone();
        let (closing, owner_closing) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run_workers(
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
        let result = run_workers(
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
        run_workers(
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
