use anyhow::Result;
#[path = "support/capacity.rs"]
mod capacity_fixture;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[path = "support/rpc_evidence.rs"]
mod rpc_evidence;
use x402_treazury::rotation::{
    funding::{FundingBackend, FundingObservations, FundingWorker},
    near::{Quote, SwapStatus},
    store::{Store, StoreHandle, SyncObservation, SyncPhase, funding::FundingJob},
    transaction::TransactionFacts,
};
#[derive(Clone)]
struct Fake {
    focus_job: String,
    store: StoreHandle,
    sends: usize,
    reconcile_error: Option<&'static str>,
    recovery: Option<x402_treazury::treasury::recovery::RecoveryOutcome>,
    recovery_calls: usize,
    recovery_observed: bool,
    reconcile_calls: usize,
    chain_credit: bool,
    external_credit: bool,
    status_error: bool,
    status_calls: Arc<AtomicUsize>,
    swap_status: fn() -> SwapStatus,
    credit_calls: Arc<AtomicUsize>,
    funds_available: bool,
    defer_prepare: bool,
    quote_deadline: u64,
    quotes: Arc<AtomicUsize>,
    quote_outage: bool,
    minimum: Option<String>,
    timeout: u64,
}
impl FundingObservations for Fake {
    type QuoteRequest = (Quote, bool);
    async fn quote(&self, (quote, outage): Self::QuoteRequest) -> Result<Quote> {
        self.quotes.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(!outage, "quote transport unavailable");
        Ok(quote)
    }

    async fn status(&self, _: Quote) -> Result<SwapStatus> {
        self.status_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(
            !self.status_error,
            "secret-token https://private.invalid/deposit/private-address"
        );
        Ok((self.swap_status)())
    }
    async fn credit(&self, j: FundingJob) -> Result<()> {
        self.credit_calls.fetch_add(1, Ordering::SeqCst);
        let id = j.id.clone();
        let phase = self
            .store
            .call(move |s| {
                Ok(s.funding_jobs()?
                    .into_iter()
                    .find(|job| job.id == id)
                    .unwrap()
                    .phase)
            })
            .await?;
        if phase != x402_treazury::rotation::store::funding::FundingPhase::VerifyingCredit
            && !self.external_credit
        {
            return Ok(());
        }
        anyhow::ensure!(self.chain_credit, "no independent Base credit");
        let wallet = j.wallet_id.clone();
        rpc_evidence::credit_owned(&self.store, wallet, "5000000", 13).await
    }
}
impl FundingBackend for Fake {
    type Observations = Self;
    fn observations(&self) -> Self {
        self.clone()
    }
    async fn recover(
        &mut self,
        _: &FundingJob,
    ) -> Result<Option<x402_treazury::treasury::recovery::RecoveryObservation>> {
        use x402_treazury::treasury::recovery::RecoveryObservation;
        self.recovery_calls += 1;
        Ok(self.recovery.clone().map(|outcome| RecoveryObservation {
            source_observed: self.recovery_observed,
            outcome,
        }))
    }

    fn swap_timeout_seconds(&self) -> u64 {
        self.timeout
    }
    async fn ready(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        anyhow::ensure!(
            self.funds_available,
            x402_treazury::rotation::error::FundingCondition::InsufficientFunds
        );
        Ok(())
    }
    async fn quote_prerequisite(&mut self, job: &FundingJob) -> Result<(Quote, bool)> {
        anyhow::ensure!(job.id == self.focus_job, "fixture_job_disabled");
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        let target = job.target.clone();
        let ceiling = self.minimum.clone().unwrap_or_else(|| job.target.clone());
        self.store
            .call(move |s| s.select_quote_terms(&id, &operation, &target, &ceiling))
            .await?;
        Ok((
            Quote {
                request: serde_json::json!({"amount":self.minimum.as_ref().unwrap_or(&job.target),"recipient":job.recipient}),
                response: serde_json::json!({}),
                input: 50,
                deadline: self.quote_deadline,
                deposit: Some("test-deposit".into()),
            },
            self.quote_outage,
        ))
    }
    async fn prepare(&mut self, j: &FundingJob, quote: &Quote) -> Result<()> {
        if self.defer_prepare {
            return Err(x402_treazury::rotation::transaction::PreparationDeferred.into());
        }
        assert!(quote.request["amount"].as_str().is_some());
        let id = j.operation_id.clone();
        let pool = j.pool_id.clone();
        let deadline = quote.deadline;
        self.store
            .call(move |s| {
                s.accept_preparation(&id, Some(&pool), 60, None)?;
                let revision = s.snapshot()?.0;
                s.prepare_with_facts(
                    &id,
                    revision,
                    b"next",
                    b"signed",
                    Some(TransactionFacts {
                        txid: "test".into(),
                        expiry_height: 100,
                        amount_zatoshis: 50,
                        fee_zatoshis: 10,
                        deadline: Some(deadline),
                    }),
                )?;
                Ok(())
            })
            .await
    }
    async fn submit(&mut self, j: &FundingJob) -> Result<()> {
        self.sends += 1;
        let id = j.operation_id.clone();
        // Simulate a process crash after durable send intent, before the response.
        self.store
            .call(move |s| {
                s.request_broadcast(&id, 1, 1, false)?;
                Ok(())
            })
            .await?;
        anyhow::bail!("ambiguous send")
    }
    async fn reconcile(&mut self, j: &FundingJob) -> Result<bool> {
        self.reconcile_calls += 1;
        let operation = j.operation_id.clone();
        if self
            .store
            .call(move |s| Ok(s.operation(&operation)?.attempts == 0))
            .await?
        {
            return Ok(false);
        }
        if let Some(error) = self.reconcile_error {
            if error == "treasury_confirmation_pending" {
                return Err(x402_treazury::rotation::transaction::RecoveryEvidenceError::ConfirmationPending.into());
            }
            anyhow::bail!(error);
        }
        let id = j.operation_id.clone();
        self.store.call(move |s| s.confirm_spend(&id)).await?;
        Ok(true)
    }

    fn max_attempts(&self, _: &FundingJob) -> u32 {
        3
    }
}
#[tokio::test]
async fn ambiguous_submission_never_repeats_and_api_success_cannot_fund_wallet() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let writer = log.reopen().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::sync::Mutex::new(writer))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"seed",
    )
    .unwrap();
    crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000)),
        0
    )
    .unwrap();
    let jobs = s.funding_jobs().unwrap();
    s.defer_funding(&jobs[1].id, i64::MAX as u64, None, false)
        .unwrap();
    let now = x402_treazury::rotation::base::now().unwrap();
    s.save_sync_snapshot(
        1,
        b"ready",
        Some(SyncObservation {
            phase: SyncPhase::Ready,
            last_error: None,
            snapshot_revision: 1,
            checked_at: Some(now),
            checkpoint_at: now,
            scanned_blocks: 0,
            target_height: Some(1),
            observed_tip_height: None,
            height: Some(1),
            confirmations: 1,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 1000,
            spendable_shielded_zatoshis: 1000,
        }),
    )
    .unwrap();
    let (store, task) = StoreHandle::spawn(s);
    let worker = FundingWorker::new(
        store.clone(),
        Fake {
            focus_job: jobs[0].id.clone(),
            store: store.clone(),
            sends: 0,
            reconcile_error: None,
            recovery: None,
            recovery_calls: 0,
            recovery_observed: false,
            reconcile_calls: 0,
            chain_credit: false,
            external_credit: false,
            status_error: false,
            status_calls: Arc::default(),
            swap_status: || SwapStatus::Success,
            credit_calls: Arc::default(),
            funds_available: true,
            defer_prepare: false,
            quote_deadline: u64::MAX,
            quotes: Arc::default(),
            quote_outage: false,
            minimum: None,
            timeout: u64::MAX,
        },
        1,
    );
    // Advances quote, preparation, ambiguous submission, restart-like recovery,
    // source confirmation, then API success. No second submit is permitted.
    for i in 0..8 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    assert_eq!(worker.backend.lock().await.sends, 1);
    let status = store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::VerifyingCredit
    );
    assert_eq!(status.pools[0].addresses[0].role, "ALLOCATED");
    assert_eq!(status.treasury_operations[0].attempts, 1);
    worker.backend.lock().await.chain_credit = true;
    worker.tick(now + 9000).await.unwrap();
    assert_eq!(
        store.call(|s| s.status()).await.unwrap().funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Complete
    );
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        logs.contains("Funding progress wallet=a size=small status=Quoted"),
        "{logs}"
    );
    assert!(
        logs.contains("Funding progress wallet=a size=small status=Complete"),
        "{logs}"
    );
    assert!(
        !logs.contains("job_id="),
        "INFO must omit raw correlations: {logs}"
    );
    assert!(
        !logs.contains("pool_id="),
        "INFO must omit raw correlations: {logs}"
    );
    drop(worker);
    drop(store);
    task.await.unwrap();
}

#[tokio::test]
async fn base_credit_before_source_confirmation_does_not_strand_the_outbox() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"seed",
    )
    .unwrap();
    let pool = crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000)),
        0
    )
    .unwrap();
    let jobs = s.funding_jobs().unwrap();
    let job = &jobs[0];
    s.save_funding_quote(
        &job.id,
        &serde_json::to_vec(
            &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
        )
        .unwrap(),
    )
    .unwrap();
    s.advance_funding(
        &job.id,
        x402_treazury::rotation::store::funding::FundingPhase::Quoted,
        x402_treazury::rotation::store::funding::FundingPhase::Preparing,
    )
    .unwrap();
    s.accept_preparation(&job.operation_id, Some(&pool), 60, None)
        .unwrap();
    s.prepare_with_facts(
        &job.operation_id,
        1,
        b"next",
        b"signed",
        Some(TransactionFacts {
            txid: "test".into(),
            expiry_height: 100,
            amount_zatoshis: 50,
            fee_zatoshis: 10,
            deadline: Some(u64::MAX),
        }),
    )
    .unwrap();
    s.request_broadcast(&job.operation_id, 1, 1, false).unwrap();
    rpc_evidence::credit(&mut s, &job.wallet_id, "5000000", 13).unwrap();
    s.defer_funding(&jobs[1].id, i64::MAX as u64, None, false)
        .unwrap();
    let (store, task) = StoreHandle::spawn(s);
    let worker = FundingWorker::new(
        store.clone(),
        Fake {
            focus_job: jobs[0].id.clone(),
            store: store.clone(),
            sends: 0,
            reconcile_error: None,
            recovery: None,
            recovery_calls: 0,
            recovery_observed: false,
            reconcile_calls: 0,
            chain_credit: true,
            external_credit: false,
            status_error: false,
            status_calls: Arc::default(),
            swap_status: || SwapStatus::Success,
            credit_calls: Arc::default(),
            funds_available: true,
            defer_prepare: false,
            quote_deadline: u64::MAX,
            quotes: Arc::default(),
            quote_outage: false,
            minimum: None,
            timeout: u64::MAX,
        },
        1,
    );
    worker.tick(1).await.unwrap();
    assert!(!store.call(|s| s.status()).await.unwrap().outgoing_pending);
    assert_eq!(worker.backend.lock().await.sends, 0);
    drop(worker);
    drop(store);
    task.await.unwrap();
}

async fn fixture() -> (
    tempfile::TempDir,
    FundingWorker<Fake>,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"seed",
    )
    .unwrap();
    crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000)),
        0
    )
    .unwrap();
    let jobs = s.funding_jobs().unwrap();
    s.defer_funding(&jobs[1].id, i64::MAX as u64, None, false)
        .unwrap();
    let now = x402_treazury::rotation::base::now().unwrap();
    s.save_sync_snapshot(
        1,
        b"ready",
        Some(SyncObservation {
            phase: SyncPhase::Ready,
            last_error: None,
            snapshot_revision: 1,
            checked_at: Some(now),
            checkpoint_at: now,
            scanned_blocks: 1,
            target_height: Some(1),
            observed_tip_height: None,
            height: Some(1),
            confirmations: 1,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 1000,
            spendable_shielded_zatoshis: 1000,
        }),
    )
    .unwrap();
    let (store, task) = StoreHandle::spawn(s);
    let worker = FundingWorker::new(
        store.clone(),
        Fake {
            focus_job: jobs[0].id.clone(),
            store: store.clone(),
            sends: 0,
            reconcile_error: None,
            recovery: None,
            recovery_calls: 0,
            recovery_observed: false,
            reconcile_calls: 0,
            chain_credit: false,
            external_credit: false,
            status_error: false,
            status_calls: Arc::default(),
            swap_status: || SwapStatus::Success,
            credit_calls: Arc::default(),
            funds_available: true,
            defer_prepare: false,
            quote_deadline: u64::MAX,
            quotes: Arc::default(),
            quote_outage: false,
            minimum: None,
            timeout: u64::MAX,
        },
        1,
    );
    (dir, worker, task)
}
#[tokio::test]
async fn status_backoff_persists_independently_and_redacts_errors() {
    let (dir, worker, task) = fixture().await;
    worker.backend.lock().await.swap_status = || SwapStatus::Processing;
    let now = x402_treazury::rotation::base::now().unwrap();
    for i in 0..5 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    worker.backend.lock().await.status_error = true;
    let mut instant = now + 5000;
    for streak in 1..=4 {
        worker.tick(instant).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        let job = &status.funding_jobs[0];
        assert_eq!(job.error_streak, streak);
        assert_eq!(
            job.attempts, 0,
            "status errors must not consume quote retries"
        );
        let text = serde_json::to_string(&status).unwrap();
        assert!(!text.contains("secret-token") && !text.contains("private.invalid"));
        let calls = worker
            .backend
            .lock()
            .await
            .status_calls
            .load(Ordering::SeqCst);
        // Aggregate next_poll may name another work kind. Test the retained
        // status owner's actual backoff, not the minimum of unrelated schedules.
        worker.tick(instant + 1).await.unwrap();
        assert_eq!(
            worker
                .backend
                .lock()
                .await
                .status_calls
                .load(Ordering::SeqCst),
            calls
        );
        instant += 100;
    }
    let saved_next_poll = worker
        .store
        .call(|s| Ok(s.status()?.funding_jobs[0].next_poll))
        .await
        .unwrap();
    let id = worker
        .store
        .call(|s| Ok(s.status()?.treasury_id))
        .await
        .unwrap();
    drop(worker);
    task.await.unwrap();
    let s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    let job = &s.status().unwrap().funding_jobs[0];
    assert_eq!(job.error_streak, 4);
    assert_eq!(job.next_poll, saved_next_poll);
}
#[tokio::test]
async fn unsigned_outage_past_health_threshold_does_not_authorize_recovery() {
    let (_dir, worker, task) = fixture().await;
    worker.backend.lock().await.timeout = 1;
    worker.backend.lock().await.quote_outage = true;
    let original = worker
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let now = x402_treazury::rotation::base::now().unwrap();
    for step in 0..5 {
        worker.tick(now + step * 1000).await.unwrap();
    }
    assert_eq!(worker.backend.lock().await.quotes.load(Ordering::SeqCst), 5);
    assert_eq!(worker.backend.lock().await.recovery_calls, 0);
    assert_eq!(worker.backend.lock().await.sends, 0);
    let id = original.id.clone();
    let (job, attempts) = worker
        .store
        .call(move |s| Ok((s.funding_job(&id)?.unwrap(), s.funding_recovery_count(&id)?)))
        .await
        .unwrap();
    assert_eq!(job.operation_id, original.operation_id);
    assert_eq!(attempts, 0);
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn timeout_degrades_pool_but_keeps_reconciling_without_resending() {
    let (_dir, worker, task) = fixture().await;
    worker.backend.lock().await.timeout = 1;
    let now = x402_treazury::rotation::base::now().unwrap();
    for i in 0..6 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert!(status.funding_jobs[0].timed_out && status.pools[0].funding_degraded);
    assert_eq!(worker.backend.lock().await.sends, 1);
    worker.backend.lock().await.chain_credit = true;
    worker.tick(now + 7000).await.unwrap();
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Complete
    );
    assert!(!status.funding_jobs[0].timed_out);
    assert!(status.funding_jobs[0].last_error.is_none());
    let completed = status.funding_jobs[0].id.clone();
    worker
        .store
        .call(move |s| s.mark_funding_timeout(&completed))
        .await
        .unwrap();
    assert!(
        !worker
            .store
            .call(|s| Ok(s.status()?.funding_jobs[0].timed_out))
            .await
            .unwrap()
    );
    // The intentionally disabled sibling fixture remains independently degraded.
    assert_eq!(worker.backend.lock().await.sends, 1);
    drop(worker);
    task.await.unwrap();
}
#[tokio::test]
async fn insufficient_treasury_waits_and_unprepared_quote_refresh_preserves_recovery_authority() {
    let (_dir, worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.backend.lock().await.quote_deadline = now + 400;
    worker.tick(now).await.unwrap();
    let original = worker
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0).operation_id))
        .await
        .unwrap();
    worker.backend.lock().await.funds_available = false;
    worker.tick(now + 1000).await.unwrap();
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Quoted
    );
    assert!(
        status.funding_jobs[0]
            .last_error
            .as_ref()
            .unwrap()
            .contains("insufficient_spendable")
    );
    assert!(status.treasury_operations.is_empty());
    let message = status.funding_jobs[0].last_error.as_deref().unwrap();
    assert!(message.contains("waiting for confirmed spendable ZEC"));
    assert!(message.contains("wallet status --config FILE"));
    assert_eq!(worker.backend.lock().await.sends, 0);
    worker.backend.lock().await.funds_available = true;
    for i in 2..8 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_ne!(status.funding_jobs[0].operation_id, original);
    assert_eq!(worker.backend.lock().await.quotes.load(Ordering::SeqCst), 4);
    assert_eq!(worker.backend.lock().await.sends, 0);
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Quoted
    );
    let job = status.funding_jobs[0].id.clone();
    assert_eq!(
        worker
            .store
            .call(move |s| s.funding_recovery_count(&job))
            .await
            .unwrap(),
        0
    );
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn quote_and_prepared_validity_boundaries_are_exact() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    for remaining in [0, 1, 299, 300] {
        let (_dir, worker, task) = fixture().await;
        let now = x402_treazury::rotation::base::now().unwrap();
        worker.backend.lock().await.quote_deadline = now + 100 + remaining;
        worker.tick(now).await.unwrap();
        worker.tick(now + 100).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        assert_eq!(
            status.funding_jobs[0].phase,
            if remaining > 0 {
                FundingPhase::Prepared
            } else {
                FundingPhase::Allocated
            }
        );
        assert_eq!(status.treasury_operations.len(), usize::from(remaining > 0));
        assert_eq!(worker.backend.lock().await.sends, 0);
        if remaining == 0 {
            let replacement = status.funding_jobs[0].operation_id.clone();
            // Refresh already replaced the operation. Its first unsigned step
            // needs no handoff from the obsolete completion or later clock tick.
            worker.tick(now + 100).await.unwrap();
            let next = worker.store.call(|s| s.status()).await.unwrap();
            assert_eq!(next.funding_jobs[0].operation_id, replacement);
            assert_eq!(next.funding_jobs[0].phase, FundingPhase::Quoted);
            assert_eq!(worker.backend.lock().await.quotes.load(Ordering::SeqCst), 2);
            assert!(next.treasury_operations.is_empty());
        }
        drop(worker);
        task.await.unwrap();
        let (_dir, worker, task) = fixture().await;
        worker.backend.lock().await.quote_deadline = now + 200 + remaining;
        worker.tick(now).await.unwrap();
        worker.tick(now + 100).await.unwrap();
        worker.tick(now + 200).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        assert_eq!(
            status.funding_jobs[0].phase,
            if remaining > 0 {
                FundingPhase::DepositPending
            } else {
                FundingPhase::RecoveryRequired
            }
        );
        if remaining == 0 {
            assert!(
                status.funding_jobs[0]
                    .last_error
                    .as_deref()
                    .unwrap()
                    .starts_with("prepared_quote_window_exhausted;")
            );
            assert_eq!(status.treasury_operations[0].submission, "PREPARED");
            assert_eq!(status.treasury_operations[0].attempts, 0);
        }
        assert_eq!(
            worker.backend.lock().await.sends,
            usize::from(remaining > 0)
        );
        assert_eq!(status.treasury_operations.len(), 1);
        drop(worker);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn preparing_recovery_uses_saved_bytes_and_existing_intent_never_resubmits() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    for saved in [false, true] {
        let (dir, worker, task) = fixture().await;
        let now = x402_treazury::rotation::base::now().unwrap();
        worker.tick(now).await.unwrap();
        if saved {
            worker.tick(now + 100).await.unwrap();
        }
        // Reproduce an interrupted/legacy phase record while preserving real encrypted bytes.
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        db.execute("UPDATE funding_progress SET phase='\"PREPARING\"' WHERE job_id=(SELECT id FROM funding_jobs ORDER BY rowid LIMIT 1)",[]).unwrap();
        drop(db);
        worker.tick(now + 200).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        assert_eq!(
            status.funding_jobs[0].phase,
            if saved {
                FundingPhase::DepositPending
            } else {
                FundingPhase::RecoveryRequired
            }
        );
        assert_eq!(worker.backend.lock().await.sends, usize::from(saved));
        if saved {
            worker.tick(now + 300).await.unwrap();
            assert_eq!(worker.backend.lock().await.sends, 1);
            let after = worker.store.call(|s| s.status()).await.unwrap();
            assert_eq!(after.funding_jobs[0].phase, FundingPhase::VerifyingCredit);
            assert_eq!(after.treasury_operations[0].attempts, 1);
        }
        drop(worker);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn swap_status_matrix_preserves_quarantine_accounting_and_single_submission() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    let statuses: [fn() -> SwapStatus; 8] = [
        || SwapStatus::PendingDeposit,
        || SwapStatus::KnownDeposit,
        || SwapStatus::Processing,
        || SwapStatus::Unknown,
        || SwapStatus::Success,
        || SwapStatus::Refunded,
        || SwapStatus::Failed,
        || SwapStatus::IncompleteDeposit,
    ];
    for phase in [
        FundingPhase::Swapping,
        FundingPhase::VerifyingCredit,
        FundingPhase::RefundPending,
    ] {
        for status in statuses {
            let (_dir, worker, task) = fixture().await;
            worker.backend.lock().await.swap_status = || SwapStatus::Processing;
            let now = x402_treazury::rotation::base::now().unwrap();
            for i in 0..5 {
                worker.tick(now + i * 100).await.unwrap();
            }
            if phase != FundingPhase::Swapping {
                worker.backend.lock().await.swap_status = if phase == FundingPhase::RefundPending {
                    || SwapStatus::Refunded
                } else {
                    || SwapStatus::Success
                };
                worker.tick(now + 500).await.unwrap();
            }
            assert_eq!(
                worker
                    .store
                    .call(|s| s.status())
                    .await
                    .unwrap()
                    .funding_jobs[0]
                    .phase,
                phase
            );
            let calls = worker
                .backend
                .lock()
                .await
                .status_calls
                .load(Ordering::SeqCst);
            worker.backend.lock().await.chain_credit = true;
            worker.backend.lock().await.swap_status = status;
            worker.tick(now + 600).await.unwrap();
            worker.tick(now + 700).await.unwrap();
            let after = worker.store.call(|s| s.status()).await.unwrap();
            let expected = match phase {
                FundingPhase::VerifyingCredit => FundingPhase::Complete,
                FundingPhase::RefundPending => FundingPhase::RefundPending,
                _ => match status() {
                    SwapStatus::Success => FundingPhase::Complete,
                    SwapStatus::Refunded | SwapStatus::Failed => FundingPhase::RefundPending,
                    SwapStatus::IncompleteDeposit => FundingPhase::RecoveryRequired,
                    _ => FundingPhase::Swapping,
                },
            };
            assert_eq!(after.funding_jobs[0].phase, expected);
            if phase != FundingPhase::Swapping {
                assert_eq!(
                    worker
                        .backend
                        .lock()
                        .await
                        .status_calls
                        .load(Ordering::SeqCst),
                    calls,
                    "saved SUCCESS/REFUNDED stops ordinary status polling"
                );
            }
            assert_eq!(worker.backend.lock().await.sends, 1);
            assert_eq!(after.treasury_operations[0].attempts, 1);
            assert_eq!(after.treasury_operations[0].submission, "CONFIRMED");
            assert_eq!(
                after.treasury_operations[0].facts.amount_zatoshis
                    + after.treasury_operations[0].facts.fee_zatoshis,
                60
            );
            drop(worker);
            task.await.unwrap();
        }
    }
}

#[tokio::test]
async fn coordinator_prepares_the_committed_bridge_target_without_requoting() {
    let (_dir, worker, task) = fixture().await;
    worker.backend.lock().await.minimum = Some("6000000".into());
    worker
        .store
        .call(|s| {
            let pool = s.status()?.pools[0].id.clone();
            let mut reserves = capacity_fixture::two_reserves("5");
            reserves.small.max_funding_amount_usdc = Some("6".into());
            s.configure_capacity_group(
                &pool,
                Some(alloy_primitives::U256::from(1_000_000)),
                0,
                &reserves,
            )
        })
        .await
        .unwrap();
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.tick(now).await.unwrap();
    let state = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(state.funding_jobs[0].target, "5000000");
    assert_eq!(state.pools[0].addresses[0].target, "5000000");
    let job = state.funding_jobs[0].id.clone();
    assert_eq!(
        worker
            .store
            .call(move |s| s.destination_requirement(&job))
            .await
            .unwrap()
            .amount,
        "6000000"
    );
    assert!(state.treasury_operations.is_empty());
    worker.tick(now + 5).await.unwrap();
    assert_eq!(worker.backend.lock().await.quotes.load(Ordering::SeqCst), 1);
    assert_eq!(worker.backend.lock().await.sends, 0);
    assert_eq!(
        worker
            .store
            .call(|s| s.status())
            .await
            .unwrap()
            .treasury_operations
            .len(),
        1
    );
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn pre_preparation_sync_failure_retries_same_quote_without_recovery_or_send() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    let (_dir, worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.tick(now).await.unwrap();
    let original = worker
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    worker.backend.lock().await.defer_prepare = true;
    for i in 1..=2 {
        worker.tick(now + i * 10).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        let job = &status.funding_jobs[0];
        assert_eq!(job.phase, FundingPhase::Quoted);
        assert_eq!(job.operation_id, original.operation_id);
        assert!(
            job.last_error
                .as_ref()
                .unwrap()
                .contains("treasury_preparation_deferred")
        );
        assert!(
            job.last_error
                .as_ref()
                .unwrap()
                .contains("existing work retained")
        );
        assert!(status.treasury_operations.is_empty());
        let operation = job.operation_id.clone();
        let id = job.id.clone();
        worker
            .store
            .call(move |s| {
                s.require_unstarted_preparation(&operation)?;
                assert_eq!(s.funding_recovery_count(&id)?, 0);
                Ok(())
            })
            .await
            .unwrap();
    }
    worker.backend.lock().await.defer_prepare = false;
    worker.tick(now + 100).await.unwrap();
    assert_eq!(worker.backend.lock().await.quotes.load(Ordering::SeqCst), 1);
    assert_eq!(worker.backend.lock().await.sends, 0);
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.treasury_operations.len(), 1);
    let job = status.funding_jobs[0].id.clone();
    assert!(
        worker
            .store
            .call(move |s| s.defer_unstarted_preparation(&job))
            .await
            .is_err(),
        "durable preparation must not be rewound"
    );
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn uncertain_preparation_stays_quarantined_after_observation_clears_diagnostic() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    let (_dir, worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.tick(now).await.unwrap();
    worker
        .store
        .call(move |s| {
            let job = s.funding_jobs()?.remove(0);
            s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Preparing)?;
            s.defer_funding(&job.id, now, Some("preparation_fixture_failure"), false)
        })
        .await
        .unwrap();
    worker.tick(now + 10).await.unwrap();
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.funding_jobs[0].phase, FundingPhase::RecoveryRequired);
    assert_eq!(status.funding_jobs[0].last_error, None);
    let operation = status.funding_jobs[0].operation_id.clone();
    worker.tick(now + 400).await.unwrap();
    let later = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(later.funding_jobs[0].operation_id, operation);
    assert_eq!(later.funding_jobs[0].phase, FundingPhase::RecoveryRequired);
    assert_eq!(
        later.funding_jobs[0].attempts,
        status.funding_jobs[0].attempts
    );
    assert_eq!(worker.backend.lock().await.sends, 0);
    assert!(status.treasury_operations.is_empty());
    assert!(later.treasury_operations.is_empty());
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn credit_completion_clears_old_errors_and_rejects_late_stale_deferrals() {
    for via_view in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::capacity_fixture::declare_and_allocate!(
            s,
            "a",
            crate::capacity_fixture::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000)),
            0
        )
        .unwrap();
        let jobs = s.funding_jobs().unwrap();
        for j in &jobs {
            s.defer_funding(
                &j.id,
                100,
                Some("base_credit_unverified; stale failure"),
                false,
            )
            .unwrap();
        }
        if via_view {
            let query = s.chain_query(&pool).unwrap();
            let balances = query
                .wallets
                .iter()
                .map(|(id, _)| (id.clone(), 5_000_000))
                .collect();
            let view = rpc_evidence::acquire(query, balances, 100).await;
            s.reconcile_pool(&pool, view).unwrap();
        } else {
            for j in &jobs {
                rpc_evidence::credit(&mut s, &j.wallet_id, "5000000", 13).unwrap();
            }
        }
        for job in s.funding_jobs().unwrap() {
            s.finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap();
        }
        assert!(
            s.funding_jobs()
                .unwrap()
                .iter()
                .all(|j| j.last_error.is_none() && j.error_streak == 0)
        );
        // Simulate a persisted completed row written by the older implementation.
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        db.execute("UPDATE funding_progress SET last_error='funding_quote_failed; historical' WHERE job_id=?1", [&jobs[0].id]).unwrap();
        assert!(s.funding_jobs().unwrap()[0].last_error.is_none());
        // A quote/credit response can finish after a separate reconciliation credits the wallet.
        s.defer_funding(
            &jobs[0].id,
            200,
            Some("funding_quote_failed; late result"),
            true,
        )
        .unwrap();
        assert!(s.funding_jobs().unwrap()[0].last_error.is_none());
        s.defer_funding(
            &jobs[0].id,
            201,
            Some("source_reconciliation_failed; retain reservation"),
            false,
        )
        .unwrap();
        assert!(
            s.funding_jobs().unwrap()[0]
                .last_error
                .as_deref()
                .unwrap()
                .contains("source_reconciliation_failed")
        );
        s.defer_funding(&jobs[0].id, 202, None, false).unwrap();
        assert!(s.funding_jobs().unwrap()[0].last_error.is_none());
        crate::capacity_fixture::declare_and_allocate!(
            s,
            "b",
            crate::capacity_fixture::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000)),
            0
        )
        .unwrap();
        let pending = s
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "b")
            .unwrap();
        s.defer_funding(
            &pending.id,
            203,
            Some("source_reconciliation_failed; retain reservation"),
            false,
        )
        .unwrap();
        rpc_evidence::credit(&mut s, &pending.wallet_id, "5000000", 13).unwrap();
        assert!(
            s.funding_job(&pending.id)
                .unwrap()
                .unwrap()
                .last_error
                .is_none(),
            "external completion has no source obligation"
        );
    }
}

#[tokio::test]
async fn in_flight_credit_completion_cannot_overwrite_reconciled_wallets() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    for disable_group in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::capacity_fixture::declare_and_allocate!(
            s,
            "a",
            crate::capacity_fixture::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000)),
            0
        )
        .unwrap();
        for j in s.funding_jobs().unwrap() {
            rpc_evidence::credit(&mut s, &j.wallet_id, "5000000", 13).unwrap();
        }
        let job = s
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.phase == FundingPhase::Complete)
            .unwrap();
        let (store, worker) = StoreHandle::spawn(s);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let late_store = store.clone();
        let wallet = job.wallet_id.clone();
        let late_credit = tokio::spawn(async move {
            // Hold a genuinely acquired old view across a newer publication.
            let id = wallet.clone();
            let query = late_store
                .call(move |s| rpc_evidence::credit_query(s, &id))
                .await
                .unwrap();
            let view = rpc_evidence::acquire(query, [(wallet.clone(), 1)].into(), 14).await;
            started.send(()).unwrap();
            released.await.unwrap();
            late_store
                .call(move |s| s.record_verified_credit(&wallet, view).map(|_| ()))
                .await
        });
        ready.await.unwrap();
        let p = pool.clone();
        let query = store.call(move |s| s.chain_query(&p)).await.unwrap();
        let balances = query
            .wallets
            .iter()
            .map(|(id, _)| (id.clone(), 5_000_000))
            .collect();
        let view = rpc_evidence::acquire(query, balances, 100).await;
        let p = pool.clone();
        store
            .call(move |s| {
                s.reconcile_pool(&p, view)?;
                if disable_group {
                    s.disable_pool("a")?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let before = serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap();
        release.send(()).unwrap();
        late_credit.await.unwrap().unwrap();
        assert_eq!(
            before,
            serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap()
        );
        let jobs = store.call(|s| s.funding_jobs()).await.unwrap();
        let completed = jobs.iter().find(|j| j.id == job.id).unwrap();
        assert_eq!(completed.phase, FundingPhase::Complete);
        assert!(completed.last_error.is_none());
        // Still-unfunded candidates must not gain readiness from a low balance.
        if let Some(pending) = jobs.iter().find(|j| j.phase != FundingPhase::Complete) {
            let wallet = pending.wallet_id.clone();
            rpc_evidence::credit_owned(&store, wallet, "1", 112)
                .await
                .unwrap();
            let pending_id = pending.id.clone();
            assert!(
                store
                    .call(move |s| Ok(s
                        .funding_jobs()?
                        .into_iter()
                        .find(|job| job.id == pending_id)
                        .unwrap()
                        .phase
                        != FundingPhase::Complete))
                    .await
                    .unwrap()
            );
        }
        drop(store);
        worker.await.unwrap();
    }
}

#[tokio::test]
async fn qualification_pause_preserves_quote_without_starting_new_work() {
    use x402_treazury::rotation::{base::now, store::funding::FundingPhase};
    let (_dir, worker, task) = fixture().await;
    let instant = now().unwrap();
    worker.tick(instant).await.unwrap();
    let before = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(before.funding_jobs[0].phase, FundingPhase::Quoted);
    worker
        .store
        .call(|s| {
            s.deny_new_funding();
            Ok(())
        })
        .await
        .unwrap();
    worker.tick(instant + 2).await.unwrap();
    let after = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(after.funding_jobs[0].phase, FundingPhase::Quoted);
    assert_eq!(
        after.funding_jobs[0].operation_id,
        before.funding_jobs[0].operation_id
    );
    assert!(
        after.funding_jobs[0].last_error.is_none(),
        "paused work is not a failed attempt"
    );
    assert_eq!(after.snapshot_revision, before.snapshot_revision);
    assert!(after.treasury_operations.is_empty());
    assert!(!after.outgoing_pending);
    assert_eq!(worker.backend.lock().await.sends, 0);
    assert_eq!(worker.backend.lock().await.quotes.load(Ordering::SeqCst), 1);
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn confirmation_wait_is_not_failure_but_reconciliation_errors_still_warn() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::sync::Mutex::new(log.reopen().unwrap()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (_dir, worker, task) = fixture().await;
    worker.backend.lock().await.reconcile_error = Some("treasury_confirmation_pending");
    worker.backend.lock().await.swap_status = || SwapStatus::Processing;
    let now = x402_treazury::rotation::base::now().unwrap();
    for i in 0..4 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    let focus = worker.backend.lock().await.focus_job.clone();
    let baseline = std::fs::read_to_string(log.path()).unwrap().len();
    worker.backend.lock().await.reconcile_error = Some("treasury_confirmation_pending");
    for i in 4..7 {
        worker.tick(now + i * 1000).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        let job = &status.funding_jobs[0];
        assert_eq!(
            job.phase,
            x402_treazury::rotation::store::funding::FundingPhase::DepositPending
        );
        assert_eq!(job.error_streak, 0);
        assert!(job.last_error.is_none());
        assert_eq!(worker.backend.lock().await.sends, 1);
        assert_ne!(status.treasury_operations[0].submission, "CONFIRMED");
    }
    let output = std::fs::read_to_string(log.path()).unwrap();
    let waiting = output[baseline..]
        .lines()
        // This fixture's focused job is a/small. WARN uses that readable
        // scope; raw job correlation is deliberately a separate DEBUG event.
        .filter(|line| {
            line.contains(&focus)
                || ((line.contains("wallet=a ") || line.contains("wallet=\"a\" "))
                    && (line.contains("size=small ") || line.contains("size=\"small\" ")))
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(waiting.contains("zcash_confirmations_pending"));
    assert!(!waiting.contains("WARN"), "{waiting}");
    worker.backend.lock().await.reconcile_error = Some("private upstream error");
    worker.tick(now + 7000).await.unwrap();
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.funding_jobs[0].error_streak, 1);
    assert!(
        status.funding_jobs[0]
            .last_error
            .as_ref()
            .unwrap()
            .contains("source_reconciliation_failed")
    );
    let output = std::fs::read_to_string(log.path()).unwrap();
    let failed = output[baseline..]
        .lines()
        // This fixture's focused job is a/small. WARN uses that readable
        // scope; raw job correlation is deliberately a separate DEBUG event.
        .filter(|line| {
            line.contains(&focus)
                || ((line.contains("wallet=a ") || line.contains("wallet=\"a\" "))
                    && (line.contains("size=small ") || line.contains("size=\"small\" ")))
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(failed.contains("WARN"), "{output}");
    assert!(
        failed.contains("Wallet funding observation unavailable"),
        "{output}"
    );
    assert!(!output.contains("private upstream error"));
    assert!(!failed.contains("source_confirmation_pending"));
    worker.backend.lock().await.reconcile_error = None;
    worker.tick(now + 8000).await.unwrap();
    assert_eq!(worker.backend.lock().await.sends, 1);
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Swapping
    );
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn recovery_wait_preserves_signed_operation_without_resubmitting() {
    use x402_treazury::{
        rotation::store::funding::FundingPhase, treasury::recovery::RecoveryOutcome,
    };
    let (_dir, worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.backend.lock().await.quote_deadline = now + 200;
    worker.tick(now).await.unwrap();
    worker.tick(now + 100).await.unwrap();
    worker.tick(now + 200).await.unwrap();
    worker.backend.lock().await.recovery =
        Some(RecoveryOutcome::Waiting("waiting_for_transaction_expiry"));
    for i in 3..6 {
        worker.tick(now + i * 1000).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        assert_eq!(status.funding_jobs[0].phase, FundingPhase::RecoveryRequired);
        assert_eq!(status.treasury_operations[0].submission, "PREPARED");
        assert_eq!(status.treasury_operations[0].attempts, 0);
        assert_eq!(worker.backend.lock().await.sends, 0);
    }
    worker.backend.lock().await.recovery = Some(RecoveryOutcome::OperatorRequired(
        "recovery_attempt_limit_reached",
    ));
    worker.tick(now + 6000).await.unwrap();
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert!(
        status.funding_jobs[0]
            .last_error
            .as_deref()
            .unwrap()
            .contains("recovery_attempt_limit_reached")
    );
    assert_eq!(worker.backend.lock().await.sends, 0);
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn repeated_quote_outages_do_not_consume_recovery_attempts_or_allocate_again() {
    let (_dir, worker, task) = fixture().await;
    worker.backend.lock().await.quote_outage = true;
    let initial = worker.store.call(|s| s.status()).await.unwrap();
    let id = initial.funding_jobs[0].operation_id.clone();
    let mut instant = x402_treazury::rotation::base::now().unwrap();
    for _ in 0..6 {
        worker.tick(instant).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        let job = &status.funding_jobs[0];
        assert_eq!(
            job.phase,
            x402_treazury::rotation::store::funding::FundingPhase::Allocated
        );
        assert_eq!(job.attempts, 0);
        assert_eq!(job.operation_id, id);
        assert_eq!(status.funding_jobs.len(), initial.funding_jobs.len());
        assert!(status.treasury_operations.is_empty());
        instant += 100;
    }
    worker.backend.lock().await.quote_outage = false;
    worker.tick(instant).await.unwrap();
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Quoted
    );
    assert_eq!(status.funding_jobs[0].operation_id, id);
    assert_eq!(worker.backend.lock().await.sends, 0);
    worker
        .store
        .call(move |s| s.require_unstarted_preparation(&id))
        .await
        .unwrap();
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn disabled_quote_closes_after_explicit_discovery_without_polling_or_new_source_work() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    let (_dir, worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.tick(now).await.unwrap();
    let before = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(before.funding_jobs[0].phase, FundingPhase::Quoted);
    let name = before.funding_jobs[0].pool_name.clone();
    worker
        .store
        .call(move |s| s.disable_pool(&name))
        .await
        .unwrap();
    let credit_calls = worker.backend.lock().await.credit_calls.clone();
    worker.tick(now + 100).await.unwrap();
    assert_eq!(
        credit_calls.load(Ordering::SeqCst),
        0,
        "unaccepted disabled quotes do not poll Base"
    );
    assert_eq!(worker.backend.lock().await.sends, 0);
    // Startup/explicit discovery owns this finite observation, independently of
    // the funding coordinator's accepted-source-only credit lane.
    use axum::{Json, Router, routing::post};
    use serde_json::{Value, json};
    let scalar_calls = Arc::new(AtomicUsize::new(0));
    let calls = scalar_calls.clone();
    let app = Router::new().route("/", post(move |Json(request): Json<Value>| {
        let calls = calls.clone();
        async move {
            assert_eq!(request["method"], "eth_call");
            assert_eq!(request["params"][1], "latest");
            calls.fetch_add(1, Ordering::SeqCst);
            Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":format!("0x{:064x}", 5_000_000u64)}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = x402_treazury::rotation::base::BaseRpc::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        12,
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let manager = x402_treazury::rotation::manager::ManagedPool::new(
        worker.store.clone(),
        before.funding_jobs[0].pool_id.clone(),
        base,
        x402_treazury::payment::SpendPolicy {
            max_atomic: Some(alloy_primitives::U256::from(1_000_000)),
        },
    )
    .unwrap();
    manager.reconcile().await.unwrap();
    drop(manager);
    assert_eq!(
        scalar_calls.load(Ordering::SeqCst),
        before.funding_jobs.len()
    );
    worker.tick(now + 200).await.unwrap();
    assert_eq!(credit_calls.load(Ordering::SeqCst), 0);
    let after = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(after.funding_jobs[0].phase, FundingPhase::Complete);
    assert_eq!(
        after.funding_jobs[0].operation_id,
        before.funding_jobs[0].operation_id
    );
    assert!(after.treasury_operations.is_empty());
    assert_eq!(worker.backend.lock().await.quotes.load(Ordering::SeqCst), 1);
    assert_eq!(worker.backend.lock().await.sends, 0);
    drop(worker);
    task.await.unwrap();
    server.abort();
}

#[tokio::test]
async fn timed_out_submitted_operation_dispatches_owned_recovery_without_replay() {
    use x402_treazury::treasury::recovery::RecoveryOutcome;
    assert_timed_out_source_observations(
        Some(RecoveryOutcome::Waiting("waiting_for_transaction_expiry")),
        0,
    )
    .await;
}

#[tokio::test]
async fn timed_out_backend_without_recovery_still_reconciles_once() {
    assert_timed_out_source_observations(None, 1).await;
}

#[tokio::test]
async fn operator_disposition_preserves_pending_source_reconciliation() {
    use x402_treazury::treasury::recovery::RecoveryOutcome;
    assert_timed_out_source_observations(
        Some(RecoveryOutcome::OperatorRequired(
            "conflicting_transaction_or_input_evidence",
        )),
        1,
    )
    .await;
}

async fn assert_timed_out_source_observations(
    recovery: Option<x402_treazury::treasury::recovery::RecoveryOutcome>,
    ordinary_reconciliations: usize,
) {
    use x402_treazury::rotation::store::funding::FundingPhase;
    let (_dir, worker, task) = fixture().await;
    worker.backend.lock().await.swap_status = || SwapStatus::Processing;
    worker.backend.lock().await.reconcile_error = Some("treasury_confirmation_pending");
    let now = x402_treazury::rotation::base::now().unwrap();
    for i in 0..3 {
        worker.tick(now + i * 100).await.unwrap();
    }
    let before = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(before.funding_jobs[0].phase, FundingPhase::DepositPending);
    let reconciles_before = worker.backend.lock().await.reconcile_calls;
    worker.tick(now + 350).await.unwrap();
    assert_eq!(
        worker.backend.lock().await.reconcile_calls,
        reconciles_before + 1
    );
    assert_eq!(worker.backend.lock().await.recovery_calls, 0);
    worker.backend.lock().await.timeout = 1;
    worker.backend.lock().await.recovery = recovery;
    worker.backend.lock().await.recovery_observed = ordinary_reconciliations == 0;
    let reconciles_before = worker.backend.lock().await.reconcile_calls;
    worker.tick(now + 1000).await.unwrap();
    let after = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(worker.backend.lock().await.recovery_calls, 1);
    assert_eq!(
        worker.backend.lock().await.reconcile_calls,
        reconciles_before + ordinary_reconciliations,
        "each pending source observation must have exactly one owner"
    );
    assert_eq!(worker.backend.lock().await.sends, 1);
    assert_eq!(worker.backend.lock().await.quotes.load(Ordering::SeqCst), 1);
    assert_eq!(
        after.funding_jobs[0].operation_id,
        before.funding_jobs[0].operation_id
    );
    assert!(after.outgoing_pending);
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn operator_disposition_after_source_observation_does_not_reconcile_again() {
    use x402_treazury::treasury::recovery::RecoveryOutcome;
    assert_timed_out_source_observations(
        Some(RecoveryOutcome::OperatorRequired(
            "conflicting_transaction_or_input_evidence",
        )),
        0,
    )
    .await;
}

#[tokio::test]
async fn recovery_wait_before_source_observation_preserves_ordinary_reconciliation() {
    use x402_treazury::treasury::recovery::RecoveryOutcome;
    assert_timed_out_source_observations(Some(RecoveryOutcome::Waiting("funding_in_progress")), 1)
        .await;
}
