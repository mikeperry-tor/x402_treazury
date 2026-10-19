//! Bounded command queue for the single mutable wallet owner. Executing commands
//! retain their durable results when reply receivers disappear. Shutdown discards
//! queued work and cancels interruptible sync/submission according to their contracts.
use super::Treasury;
use crate::rotation::transaction::{
    PrepareRequest, PreparedTransaction, SubmissionOutcome, TransactionPresence,
    TransactionSubmission,
};
use anyhow::{Context, Result};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

enum Command {
    #[cfg(test)]
    Interrupt(oneshot::Sender<()>, oneshot::Receiver<()>),
    RecoverFunding(
        String,
        u32,
        bool,
        oneshot::Sender<Result<super::recovery::RecoveryObservation>>,
    ),
    Refund(String, oneshot::Sender<Result<String>>),
    Sync(oneshot::Sender<Result<()>>),
    Prepare(
        Box<PrepareRequest>,
        Option<crate::rotation::base::BaseRpc>,
        oneshot::Sender<Result<PreparedTransaction>>,
    ),
    Submit(String, bool, oneshot::Sender<Result<SubmissionOutcome>>),
    Reconcile(String, oneshot::Sender<Result<TransactionPresence>>),
}
#[derive(Clone)]
pub struct TreasuryHandle {
    sender: mpsc::Sender<Command>,
}
pub struct TreasuryCommands {
    receiver: mpsc::Receiver<Command>,
}
pub fn channel() -> (TreasuryHandle, TreasuryCommands) {
    let (sender, receiver) = mpsc::channel(8);
    (TreasuryHandle { sender }, TreasuryCommands { receiver })
}
impl TreasuryHandle {
    pub async fn recover_funding(
        &self,
        id: String,
        max_attempts: u32,
        allow_new_attempt: bool,
    ) -> Result<super::recovery::RecoveryObservation> {
        self.call(|reply| Command::RecoverFunding(id, max_attempts, allow_new_attempt, reply))
            .await
    }

    async fn call<T>(&self, make: impl FnOnce(oneshot::Sender<Result<T>>) -> Command) -> Result<T> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(make(send))
            .await
            .map_err(|_| anyhow::anyhow!("treasury owner stopped"))?;
        recv.await
            .context("treasury command interrupted; accepted work remains owned")?
    }
    pub async fn refund_address(&self, job: String) -> Result<String> {
        self.call(|reply| Command::Refund(job, reply)).await
    }
    pub async fn sync(&self) -> Result<()> {
        self.call(Command::Sync).await
    }
    pub async fn prepare(&self, request: PrepareRequest) -> Result<PreparedTransaction> {
        self.call(|reply| Command::Prepare(Box::new(request), None, reply))
            .await
    }
    pub(crate) async fn prepare_funding(
        &self,
        request: PrepareRequest,
        base: crate::rotation::base::BaseRpc,
    ) -> Result<PreparedTransaction> {
        self.call(|reply| Command::Prepare(Box::new(request), Some(base), reply))
            .await
    }
    pub async fn submit(&self, id: String, retry: bool) -> Result<SubmissionOutcome> {
        self.call(|reply| Command::Submit(id, retry, reply)).await
    }
    pub async fn reconcile(&self, id: String) -> Result<TransactionPresence> {
        self.call(|reply| Command::Reconcile(id, reply)).await
    }
}
impl Treasury {
    pub async fn run_commands(
        self,
        commands: TreasuryCommands,
        submission: impl TransactionSubmission + Send + 'static,
        stop: CancellationToken,
    ) -> Result<()> {
        tokio::spawn(self.run_commands_owned(commands, submission, stop))
            .await
            .context("treasury command driver stopped")?
    }
    async fn run_commands_owned(
        mut self,
        mut commands: TreasuryCommands,
        mut submission: impl TransactionSubmission,
        stop: CancellationToken,
    ) -> Result<()> {
        use futures_util::FutureExt;
        let result = loop {
            let outcome = std::panic::AssertUnwindSafe(self.command_loop(
                &mut commands,
                &mut submission,
                &stop,
            ))
            .catch_unwind()
            .await;
            match outcome {
                Ok(result) => break result,
                Err(_) => {
                    self.healthy = false;
                    tracing::warn!(
                        category = "treasury_command_interrupted",
                        "treasury command interrupted; failed command will not replay; accepted work retained"
                    );
                    if stop.is_cancelled() {
                        break Ok(());
                    }
                    // An interrupted SDK must not turn reconstruction into a
                    // busy loop. Reuse the existing maintenance retry cadence.
                    tokio::select! {
                        _=stop.cancelled()=>break Ok(()),
                        _=tokio::time::sleep(std::time::Duration::from_secs(super::publication::MAINTENANCE_SECONDS))=>{},
                    }
                }
            }
        };
        commands.receiver.close();
        drop(commands);
        // Fatal shared-state errors stop their dependants. SDK-local failures
        // reconstruct inside command_loop without cancelling this shutdown token.
        if result.is_err() {
            stop.cancel();
        }
        let closed = self.close().await;
        result.and(closed)
    }
    async fn command_loop(
        &mut self,
        commands: &mut TreasuryCommands,
        submission: &mut impl TransactionSubmission,
        stop: &CancellationToken,
    ) -> Result<()> {
        let changed = self.store.treasury_changes();
        let mut observed_sync = None;
        loop {
            if self.completed.is_some() {
                tokio::select! {
                    _=stop.cancelled()=>break,
                    _=tokio::time::sleep(std::time::Duration::from_secs(super::publication::MAINTENANCE_SECONDS))=>{}
                }
                if self.publish_completed().await.is_err() {
                    Self::warn_publication_pending();
                }
                if self.completed.is_some() {
                    continue;
                }
            }
            if stop.is_cancelled() {
                break;
            }
            if !self.healthy {
                match self.reconstruct_sdk(stop).await {
                    Ok(()) => {}
                    Err(_) if stop.is_cancelled() => break,
                    Err(error) if super::reconstruction::retryable(&error) => {
                        tracing::warn!(
                            category = "treasury_reconstruction_pending",
                            "treasury SDK reconstruction deferred; independent Base service remains available"
                        );
                        tokio::select! { _=stop.cancelled()=>break, _=tokio::time::sleep(std::time::Duration::from_secs(super::publication::MAINTENANCE_SECONDS))=>{} }
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            let requested = if self.startup_discovery {
                Some(true)
            } else {
                let groups = self.funding_groups.clone();
                crate::rotation::store::inventory::read_result(
                    self.store
                        .call(move |s| s.treasury_observation_needed(&groups))
                        .await,
                    &mut None,
                )?
            };
            let due = if self.startup_discovery {
                tokio::time::Instant::now()
            } else if requested.is_none() {
                // Unknown demand is not permission to scan. A read-only busy
                // inventory defers reevaluation while accepted commands stay live.
                tokio::time::Instant::now()
                    + std::time::Duration::from_secs(super::publication::MAINTENANCE_SECONDS)
            } else {
                self.observation.due()
            };
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                command = commands.receiver.recv() => {
                    let Some(command) = command else { break };
                    match command {
                        #[cfg(test)]
                        Command::Interrupt(entered, release) => { let _ = entered.send(()); release.await.unwrap(); },
                        Command::RecoverFunding(id, max_attempts, allow_new_attempt, reply) => { let _ = reply.send(self.recover_funding_observed(id, max_attempts, allow_new_attempt, submission, stop, true).await); }

                        Command::Refund(job, reply) => { let _ = reply.send(self.refund_address(job).await); }
                        Command::Sync(reply) => { let _ = reply.send(self.sync_once(stop).await); }
                        Command::Prepare(request, base, reply) => {
                            let operation = request.operation_id.clone();
                            let result = match self.sync_for_observation(stop).await {
                                Err(error) if !self.healthy => Err(error),
                                _ if stop.is_cancelled() => Err(anyhow::anyhow!("treasury stopping")),
                                // A missing reusable scan is not a new readiness
                                // gate: local SDK selection may establish valid
                                // work, whose canonical action checks stay immediate.
                                _ => self.prepare_with_observation(*request, base, Some(stop)).await,
                            };
                            let result = match result {
                                Err(error) if self.healthy && !error.is::<crate::rotation::transaction::PreparationDeferred>() => {
                                    // Healthy SDK state alone cannot prove that
                                    // this operation has no earlier accepted owner.
                                    if self.store.call(move |s| s.require_unstarted_preparation(&operation)).await.is_ok() {
                                        Err(error.context(crate::rotation::transaction::PreparationDeferred))
                                    } else { Err(error) }
                                }
                                result => result,
                            };
                            let _ = reply.send(result);
                        }
                        Command::Submit(id, retry, reply) => { let _ = reply.send(self.submit_prepared(id, submission, retry, stop).await); }
                        Command::Reconcile(id, reply) => { let _ = reply.send(self.reconcile_for_observation(id, submission, stop).await); }
                    }
                }
                _ = tokio::time::sleep_until(due), if requested != Some(false) => {
                    if requested == Some(true) {
                        self.startup_discovery = false;
                        if let Err(error) = self.sync_for_observation(stop).await
                            && !stop.is_cancelled() {
                                super::diagnostics::warn_sync_failure(
                                    &error,
                                    Some(self.observation.due().saturating_duration_since(tokio::time::Instant::now())),
                                );
                        }
                    }
                }
                // Capacity traffic must not starve an already-due scan.
                _ = changed.notified() => {},
            }
            if self.healthy
                && self.last_completed_sync.is_some()
                && self.last_completed_sync != observed_sync
            {
                if self
                    .maintain_owned_transparent(submission, stop)
                    .await
                    .is_err()
                    && !stop.is_cancelled()
                {
                    tracing::warn!(
                        category = "owned_transparent_maintenance_unavailable",
                        "accepted shielding remains owned; independent Base service remains available"
                    );
                }
                // Maintenance may itself complete canonical validation/proof
                // scans. They are part of this pass, not another wakeup.
                observed_sync = self.last_completed_sync;
            }
        }
        Ok(())
    }
}

// Observe the real command boundary without substituting a fake FundingBackend.
#[cfg(test)]
pub(crate) enum ObservedCommand {
    RecoverFunding(String, u32, bool),
    Prepare(PrepareRequest),
    Submit(String, bool),
    Reconcile(String),
}
#[cfg(test)]
impl TreasuryCommands {
    pub(crate) fn test_is_empty(&self) -> bool {
        self.receiver.is_empty()
    }
    pub(crate) async fn test_observe(&mut self) -> ObservedCommand {
        match self.receiver.recv().await.expect("command channel closed") {
            Command::RecoverFunding(id, limit, allow_new_attempt, reply) => {
                let _ = reply.send(Ok(super::recovery::RecoveryObservation {
                    outcome: super::recovery::RecoveryOutcome::Waiting(
                        "waiting_for_transaction_expiry",
                    ),
                    source_observed: true,
                }));
                ObservedCommand::RecoverFunding(id, limit, allow_new_attempt)
            }
            Command::Prepare(request, _base, reply) => {
                let _ = reply.send(Err(anyhow::anyhow!("fixture command observed")));
                ObservedCommand::Prepare(*request)
            }
            Command::Submit(id, retry, reply) => {
                let _ = reply.send(Err(anyhow::anyhow!("fixture command observed")));
                ObservedCommand::Submit(id, retry)
            }
            Command::Reconcile(id, reply) => {
                let _ = reply.send(Err(anyhow::anyhow!("fixture command observed")));
                ObservedCommand::Reconcile(id)
            }
            _ => panic!("unexpected treasury command"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoSubmission;
    impl TransactionSubmission for NoSubmission {
        async fn submit(
            &mut self,
            _: crate::rotation::transaction::BroadcastTransaction,
        ) -> Result<SubmissionOutcome> {
            panic!("unexpected submission")
        }
        async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
            panic!("unexpected lookup")
        }
    }

    #[tokio::test]
    async fn due_observation_precedes_ready_capacity_notifications() {
        use futures_util::FutureExt;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let dir = tempfile::tempdir().unwrap();
        let mut owner = Treasury::create(
            dir.path().join("state"),
            dir.path().join("key"),
            2_000_000,
            Some(zeroize::Zeroizing::new(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(),
            )),
        )
        .await
        .unwrap();
        crate::rotation::test_capacity::treasury_pair(&owner, "notifications", "5", None, 0)
            .await
            .unwrap();
        owner.configure_funding_discovery(vec!["notifications".into()]);
        owner.startup_discovery = false;
        // Make the shared observation unambiguously overdue before starting the
        // real owner, without waiting through the production minute interval.
        tokio::time::pause();
        owner.observation.complete(true);
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::time::resume();

        let changed = owner.store.treasury_changes();
        assert!(changed.notified().now_or_never().is_some());
        changed.notify_one();
        let pending_at_scan = Arc::new(AtomicBool::new(false));
        let recorded = pending_at_scan.clone();
        let stop = CancellationToken::new();
        let request_stop = stop.clone();
        let app = axum::Router::new().fallback(move || {
            let changed = changed.clone();
            let recorded = recorded.clone();
            let stop = request_stop.clone();
            async move {
                // Both branches were ready. The scan must start without first
                // consuming the permit that concurrent calls can keep refilling.
                recorded.store(
                    changed.notified().now_or_never().is_some(),
                    Ordering::SeqCst,
                );
                stop.cancel();
                (
                    [("content-type", "application/grpc"), ("grpc-status", "14")],
                    Vec::<u8>::new(),
                )
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        owner.configure_sync(
            super::super::SyncSettings::new(
                format!("http://{}", listener.local_addr().unwrap()),
                3,
            )
            .unwrap(),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (handle, commands) = channel();
        owner
            .run_commands(commands, NoSubmission, stop)
            .await
            .unwrap();
        drop(handle);
        server.abort();
        assert!(
            pending_at_scan.load(Ordering::SeqCst),
            "capacity notifications must not run ahead of an already-due observation"
        );
    }

    #[tokio::test]
    async fn busy_demand_inventory_keeps_commands_and_accepted_input_ownership_live() {
        use crate::rotation::store::inventory::Read;
        for code in [rusqlite::ffi::SQLITE_BUSY, rusqlite::ffi::SQLITE_LOCKED] {
            let dir = tempfile::tempdir().unwrap();
            let mut owner = Treasury::create(dir.path().join("state"), dir.path().join("key"), 2_000_000, Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
            owner.startup_discovery = false;
            owner.observation.complete(true);
            owner.configure_sync(
                super::super::SyncSettings::new("http://127.0.0.1:1".into(), 3).unwrap(),
            );
            let store = owner.store.clone();
            let revision = owner.revision;
            store
                .call(move |s| {
                    s.accept_preparation("retained", None, 60, None)?;
                    s.fail_next_inventory_read(Read::TreasuryDemand, code);
                    Ok(())
                })
                .await
                .unwrap();
            let (handle, commands) = channel();
            let stop = CancellationToken::new();
            let task = tokio::spawn(owner.run_commands(commands, NoSubmission, stop.clone()));
            while store
                .call(|s| Ok(s.inventory_fault_pending()))
                .await
                .unwrap()
            {
                tokio::task::yield_now().await;
            }
            assert!(
                handle
                    .recover_funding("missing".into(), 1, false)
                    .await
                    .is_err()
            );
            assert!(!stop.is_cancelled() && !task.is_finished());
            store
                .call(move |s| {
                    assert_eq!(
                        s.snapshot()?.0,
                        revision,
                        "unknown demand cannot authorize a scan"
                    );
                    assert!(s.require_unstarted_preparation("retained").is_err());
                    assert!(s.abandon_unprepared("retained").is_err());
                    Ok(())
                })
                .await
                .unwrap();
            stop.cancel();
            drop(handle);
            drop(store);
            task.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn dropped_receivers_preserve_mutations_but_shutdown_discards_queued_work() {
        for stopped in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut treasury=Treasury::create(dir.path().join("state"),dir.path().join("key"),2_000_000,Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
            crate::rotation::test_capacity::treasury_pair(
                &treasury,
                "actor",
                "5",
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
            .await
            .unwrap();
            let status = treasury.status().await.unwrap();
            let id = status.treasury_id;
            let job = status.funding_jobs[0].id.clone();
            treasury.configure_sync(
                super::super::SyncSettings::new("http://127.0.0.1:1".into(), 3).unwrap(),
            );
            let (handle, commands) = channel();
            let stop = CancellationToken::new();
            let (send, recv) = oneshot::channel();
            handle
                .sender
                .send(Command::Refund(job.clone(), send))
                .await
                .unwrap();
            drop(recv);
            let (send, reply) = oneshot::channel();
            handle
                .sender
                .send(Command::Refund(job.clone(), send))
                .await
                .unwrap();
            if stopped {
                stop.cancel();
            }
            let task = tokio::spawn(treasury.run_commands(commands, NoSubmission, stop.clone()));
            let address = tokio::time::timeout(std::time::Duration::from_secs(5), reply)
                .await
                .unwrap();
            if stopped {
                assert!(address.is_err());
            } else {
                assert!(address.unwrap().unwrap().starts_with('t'));
            }
            stop.cancel();
            task.await.unwrap().unwrap();
            assert!(handle.refund_address(job.clone()).await.is_err());
            let restored = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
                .await
                .unwrap();
            let saved = restored
                .store
                .call(move |s| s.refund_address(&job))
                .await
                .unwrap();
            assert_eq!(saved.is_some(), !stopped);
            assert!(
                restored
                    .status()
                    .await
                    .unwrap()
                    .treasury_operations
                    .is_empty()
            );
            restored.close().await.unwrap();
        }
    }
    #[tokio::test]
    async fn interrupted_command_retains_queue_claim_and_owner_after_driver_waiter_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let mut owner = Treasury::create(dir.path().join("state"), dir.path().join("key"), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        crate::rotation::test_capacity::treasury_pair(&owner, "actor_recovery", "5", None, 0)
            .await
            .unwrap();
        let status = owner.status().await.unwrap();
        let identity = status.treasury_id;
        let job = status.funding_jobs[0].id.clone();
        let store = owner.store_handle();
        store
            .call(|s| s.accept_preparation("uncertain", None, 100, None))
            .await
            .unwrap();
        owner.configure_sync(
            super::super::SyncSettings::new("http://127.0.0.1:1".into(), 3).unwrap(),
        );
        let (handle, commands) = channel();
        let (entered, ready) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        handle
            .sender
            .send(Command::Interrupt(entered, wait))
            .await
            .unwrap();
        let (send, reply) = oneshot::channel();
        handle
            .sender
            .send(Command::Refund(job.clone(), send))
            .await
            .unwrap();
        let stop = CancellationToken::new();
        let driver = tokio::spawn(owner.run_commands(commands, NoSubmission, stop.clone()));
        ready.await.unwrap();
        // The spawned cleanup owner outlives this caller, including its panic recovery.
        driver.abort();
        let _ = driver.await;
        drop(release);
        let address = tokio::time::timeout(std::time::Duration::from_secs(75), reply)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(address.starts_with('t'));
        store
            .call(move |s| {
                assert_eq!(s.refund_address(&job)?.as_deref(), Some(address.as_str()));
                assert!(s.require_unstarted_preparation("uncertain").is_err());
                assert!(s.accept_preparation("duplicate", None, 100, None).is_err());
                Ok(())
            })
            .await
            .unwrap();
        stop.cancel();
        // Channel closure acknowledges that the retained owner has reached its drain.
        handle.sender.closed().await;
        drop(handle);
        drop(store);
        // The exclusive Store lock must eventually be released by that owner.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(restored) = Treasury::open(
                    dir.path().join("state"),
                    dir.path().join("key"),
                    identity.clone(),
                )
                .await
                {
                    restored.close().await.unwrap();
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn failed_sync_returns_typed_unstarted_response_without_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let mut treasury = Treasury::create(
            dir.path().join("state"), dir.path().join("key"), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))
        ).await.unwrap();
        treasury.configure_sync(
            super::super::SyncSettings::new("http://127.0.0.1:1".into(), 3).unwrap(),
        );
        let id = treasury.status().await.unwrap().treasury_id;
        let (handle, commands) = channel();
        let stop = CancellationToken::new();
        let request = PrepareRequest {
            managed_funding: None,

            operation_id: uuid::Uuid::new_v4().to_string(),
            pool_id: None,

            deadline: u64::MAX,
            recipient: "not-used-before-sync".into(),
            amount_zatoshis: 1,
            max_fee_zatoshis: 10000,
            max_input_zatoshis: 20000,
        };
        let (send, reply) = oneshot::channel();
        handle
            .sender
            .send(Command::Prepare(Box::new(request), None, send))
            .await
            .unwrap();
        let task = tokio::spawn(treasury.run_commands(commands, NoSubmission, stop.clone()));
        let result = reply.await.unwrap();
        assert!(
            result
                .err()
                .unwrap()
                .is::<crate::rotation::transaction::PreparationDeferred>()
        );
        stop.cancel();
        task.await.unwrap().unwrap();
        let restored = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
            .await
            .unwrap();
        assert!(
            restored
                .status()
                .await
                .unwrap()
                .treasury_operations
                .is_empty()
        );
        restored.close().await.unwrap();
    }

    #[tokio::test]
    async fn rejected_accepted_preparation_never_reports_unstarted_or_releases_its_owner() {
        let dir = tempfile::tempdir().unwrap();
        let mut owner = Treasury::create(dir.path().join("state"), dir.path().join("key"), 2_000_000, Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        owner.startup_discovery = false;
        owner.observation.complete(true);
        owner.configure_sync(
            super::super::SyncSettings::new("http://127.0.0.1:1".into(), 3).unwrap(),
        );
        crate::rotation::test_capacity::treasury_pair(
            &owner,
            "accepted",
            "5",
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0,
        )
        .await
        .unwrap();
        let job = owner.status().await.unwrap().funding_jobs.remove(0);
        let retained = job.clone();
        let store = owner.store.clone();
        store
            .call(move |s| {
                s.accept_preparation(&retained.operation_id, Some(&retained.pool_id), 60, None)
            })
            .await
            .unwrap();
        let (handle, commands) = channel();
        let stop = CancellationToken::new();
        let task = tokio::spawn(owner.run_commands(commands, NoSubmission, stop.clone()));
        for (recipient, pool, auto_fund) in [
            ("invalid", job.pool_id.clone(), true),
            (
                "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F",
                "wrong-pool".into(),
                true,
            ),
            (
                "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F",
                job.pool_id.clone(),
                false,
            ),
        ] {
            let error = handle
                .prepare(PrepareRequest {
                    managed_funding: Some(crate::rotation::transaction::ManagedFundingAuthority {
                        job_id: job.id.clone(),
                        auto_fund,
                        max_attempts: 1,
                    }),
                    operation_id: job.operation_id.clone(),
                    pool_id: Some(pool),
                    deadline: u64::MAX,
                    recipient: recipient.into(),
                    amount_zatoshis: 1,
                    max_fee_zatoshis: 10000,
                    max_input_zatoshis: 20000,
                })
                .await
                .err()
                .unwrap();
            assert!(
                !error.is::<crate::rotation::transaction::PreparationDeferred>(),
                "{error:#}"
            );
        }
        store
            .call(move |s| {
                assert!(s.require_unstarted_preparation(&job.operation_id).is_err());
                assert!(s.abandon_unprepared(&job.operation_id).is_err());
                assert!(s.require_preparation_available().is_err());
                assert!(s.status()?.treasury_operations.is_empty());
                Ok(())
            })
            .await
            .unwrap();
        stop.cancel();
        drop(handle);
        drop(store);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cancellation_before_queue_admission_leaves_no_command() {
        let (handle, commands) = channel();
        for _ in 0..8 {
            let (send, _) = oneshot::channel();
            handle.sender.send(Command::Sync(send)).await.unwrap();
        }
        let mut pending = Box::pin(handle.sync());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        drop(pending);
        assert_eq!(commands.receiver.len(), 8);
    }
}
