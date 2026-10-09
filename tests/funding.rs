#![cfg(feature = "zcash")]
use anyhow::Result;
use x402_treazury::rotation::{
    funding::{FundingBackend, FundingWorker},
    near::{Quote, SwapStatus},
    store::{Store, StoreHandle, SyncObservation, SyncPhase, funding::FundingJob},
    transaction::TransactionFacts,
};
struct Fake {
    store: StoreHandle,
    sends: usize,
    reconcile_error: Option<&'static str>,
    recovery: Option<x402_treazury::treasury::recovery::RecoveryOutcome>,
    chain_credit: bool,
    status_error: bool,
    status_calls: usize,
    swap_status: fn() -> SwapStatus,
    credit_calls: usize,
    funds_available: bool,
    defer_prepare: bool,
    quote_deadline: u64,
    quotes: usize,
    quote_outage: bool,
    minimum: Option<String>,
    timeout: u64,
}
impl FundingBackend for Fake {
    async fn recover(
        &mut self,
        _: &FundingJob,
    ) -> Result<Option<x402_treazury::treasury::recovery::RecoveryOutcome>> {
        Ok(self.recovery.clone())
    }

    fn swap_timeout_seconds(&self) -> u64 {
        self.timeout
    }
    async fn ready(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        anyhow::ensure!(
            self.funds_available,
            "treasury_insufficient_spendable_funds"
        );
        Ok(())
    }
    async fn quote(&mut self, job: &FundingJob) -> Result<Quote> {
        self.quotes += 1;
        anyhow::ensure!(!self.quote_outage, "quote transport unavailable");
        Ok(Quote {
            request: serde_json::json!({"amount":self.minimum.as_ref().unwrap_or(&job.target)}),
            response: serde_json::json!({}),
            input: 50,
            deadline: self.quote_deadline,
            deposit: Some("test-deposit".into()),
        })
    }
    async fn prepare(&mut self, j: &FundingJob, quote: &Quote) -> Result<()> {
        if self.defer_prepare {
            return Err(x402_treazury::rotation::transaction::PreparationDeferred.into());
        }
        assert_eq!(quote.request["amount"], j.target);
        let id = j.operation_id.clone();
        let pool = j.pool_id.clone();
        let deadline = quote.deadline;
        self.store
            .call(move |s| {
                s.reserve(&id, Some(&pool), 1, 100, 1000)?;
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
                        deadline,
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
        if let Some(error) = self.reconcile_error {
            anyhow::bail!(error);
        }
        let id = j.operation_id.clone();
        self.store
            .call(move |s| s.confirm_spend(&id, 60, 1))
            .await?;
        Ok(true)
    }
    async fn status(&mut self, _: &Quote) -> Result<SwapStatus> {
        self.status_calls += 1;
        anyhow::ensure!(
            !self.status_error,
            "secret-token https://private.invalid/deposit/private-address"
        );
        Ok((self.swap_status)())
    }
    async fn credit(&mut self, j: &FundingJob) -> Result<()> {
        self.credit_calls += 1;
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
        assert_eq!(
            phase,
            x402_treazury::rotation::store::funding::FundingPhase::VerifyingCredit,
            "verification phase must commit before the external credit check"
        );
        anyhow::ensure!(self.chain_credit, "no independent Base credit");
        let wallet = j.wallet_id.clone();
        self.store
            .call(move |s| s.record_credit(&wallet, "5000000", "confirmed-block", 1))
            .await
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
    s.ensure_pool("a", "5").unwrap();
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
            max_age_seconds: 300,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 1000,
            spendable_shielded_zatoshis: 1000,
        }),
    )
    .unwrap();
    let (store, task) = StoreHandle::spawn(s);
    let mut worker = FundingWorker {
        store: store.clone(),
        backend: Fake {
            store: store.clone(),
            sends: 0,
            reconcile_error: None,
            recovery: None,
            chain_credit: false,
            status_error: false,
            status_calls: 0,
            swap_status: || SwapStatus::Success,
            credit_calls: 0,
            funds_available: true,
            defer_prepare: false,
            quote_deadline: u64::MAX,
            quotes: 0,
            quote_outage: false,
            minimum: None,
            timeout: u64::MAX,
        },
        poll_seconds: 1,
    };
    // Advances quote, preparation, ambiguous submission, restart-like recovery,
    // source confirmation, then API success. No second submit is permitted.
    for i in 0..8 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    assert_eq!(worker.backend.sends, 1);
    let status = store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::VerifyingCredit
    );
    assert_eq!(status.pools[0].addresses[0].role, "ALLOCATED");
    assert_eq!(status.treasury_operations[0].attempts, 1);
    worker.backend.chain_credit = true;
    worker.tick(now + 9000).await.unwrap();
    assert_eq!(
        store.call(|s| s.status()).await.unwrap().funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Complete
    );
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(logs.contains("funding phase changed"), "{logs}");
    assert!(logs.contains("to=Quoted"), "{logs}");
    assert!(logs.contains("to=Complete"), "{logs}");
    assert!(logs.contains("job_id="), "{logs}");
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
    let pool = s.ensure_pool("a", "5").unwrap();
    let jobs = s.funding_jobs().unwrap();
    let job = &jobs[0];
    s.save_funding_quote(&job.id, b"quote").unwrap();
    s.advance_funding(
        &job.id,
        x402_treazury::rotation::store::funding::FundingPhase::Quoted,
        x402_treazury::rotation::store::funding::FundingPhase::Preparing,
    )
    .unwrap();
    s.reserve(&job.operation_id, Some(&pool), 1, 100, 1000)
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
            deadline: u64::MAX,
        }),
    )
    .unwrap();
    s.request_broadcast(&job.operation_id, 1, 1, false).unwrap();
    s.record_credit(&job.wallet_id, "5000000", "base-block", 1)
        .unwrap();
    s.defer_funding(&jobs[1].id, i64::MAX as u64, None, false)
        .unwrap();
    let (store, task) = StoreHandle::spawn(s);
    let mut worker = FundingWorker {
        store: store.clone(),
        backend: Fake {
            store: store.clone(),
            sends: 0,
            reconcile_error: None,
            recovery: None,
            chain_credit: true,
            status_error: false,
            status_calls: 0,
            swap_status: || SwapStatus::Success,
            credit_calls: 0,
            funds_available: true,
            defer_prepare: false,
            quote_deadline: u64::MAX,
            quotes: 0,
            quote_outage: false,
            minimum: None,
            timeout: u64::MAX,
        },
        poll_seconds: 1,
    };
    worker.tick(1).await.unwrap();
    assert!(!store.call(|s| s.status()).await.unwrap().outgoing_pending);
    assert_eq!(worker.backend.sends, 0);
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
    s.ensure_pool("a", "5").unwrap();
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
            max_age_seconds: 3600,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 1000,
            spendable_shielded_zatoshis: 1000,
        }),
    )
    .unwrap();
    let (store, task) = StoreHandle::spawn(s);
    let worker = FundingWorker {
        backend: Fake {
            store: store.clone(),
            sends: 0,
            reconcile_error: None,
            recovery: None,
            chain_credit: false,
            status_error: false,
            status_calls: 0,
            swap_status: || SwapStatus::Success,
            credit_calls: 0,
            funds_available: true,
            defer_prepare: false,
            quote_deadline: u64::MAX,
            quotes: 0,
            quote_outage: false,
            minimum: None,
            timeout: u64::MAX,
        },
        store,
        poll_seconds: 1,
    };
    (dir, worker, task)
}
#[tokio::test]
async fn status_backoff_persists_independently_and_redacts_errors() {
    let (dir, mut worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    for i in 0..5 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    worker.backend.status_error = true;
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
        assert!(job.next_poll >= instant + (1 << streak));
        let text = serde_json::to_string(&status).unwrap();
        assert!(!text.contains("secret-token") && !text.contains("private.invalid"));
        let calls = worker.backend.status_calls;
        worker.tick(job.next_poll - 1).await.unwrap();
        assert_eq!(worker.backend.status_calls, calls);
        instant = job.next_poll;
    }
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
    assert_eq!(job.next_poll, instant);
}
#[tokio::test]
async fn timeout_degrades_pool_but_keeps_reconciling_without_resending() {
    let (_dir, mut worker, task) = fixture().await;
    worker.backend.timeout = 1;
    let now = x402_treazury::rotation::base::now().unwrap();
    for i in 0..6 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert!(status.funding_jobs[0].timed_out && status.pools[0].funding_degraded);
    assert!(status.funding_jobs[0].next_poll >= now + 5060);
    assert_eq!(worker.backend.sends, 1);
    worker.backend.chain_credit = true;
    worker.tick(now + 7000).await.unwrap();
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Complete
    );
    assert!(!status.funding_jobs[0].timed_out && !status.pools[0].funding_degraded);
    assert_eq!(worker.backend.sends, 1);
    drop(worker);
    task.await.unwrap();
}
#[tokio::test]
async fn insufficient_treasury_waits_and_unprepared_quote_refresh_preserves_recovery_authority() {
    let (_dir, mut worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.backend.quote_deadline = now + 400;
    worker.tick(now).await.unwrap();
    let original = worker
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0).operation_id))
        .await
        .unwrap();
    worker.backend.funds_available = false;
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
    assert!(message.contains("refill paused before transaction preparation"));
    assert!(message.contains("existing funded wallets remain usable"));
    assert_eq!(worker.backend.sends, 0);
    worker.backend.funds_available = true;
    for i in 2..8 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_ne!(status.funding_jobs[0].operation_id, original);
    assert_eq!(worker.backend.quotes, 4);
    assert_eq!(worker.backend.sends, 0);
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
    for remaining in [299, 300] {
        let (_dir, mut worker, task) = fixture().await;
        let now = x402_treazury::rotation::base::now().unwrap();
        worker.backend.quote_deadline = now + 100 + remaining;
        worker.tick(now).await.unwrap();
        worker.tick(now + 100).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        assert_eq!(
            status.funding_jobs[0].phase,
            if remaining == 300 {
                FundingPhase::Prepared
            } else {
                FundingPhase::Allocated
            }
        );
        assert_eq!(
            status.treasury_operations.len(),
            usize::from(remaining == 300)
        );
        assert_eq!(worker.backend.sends, 0);
        drop(worker);
        task.await.unwrap();
        let (_dir, mut worker, task) = fixture().await;
        worker.backend.quote_deadline = now + 200 + remaining;
        worker.tick(now).await.unwrap();
        worker.tick(now + 100).await.unwrap();
        worker.tick(now + 200).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        assert_eq!(
            status.funding_jobs[0].phase,
            if remaining == 300 {
                FundingPhase::Prepared
            } else {
                FundingPhase::RecoveryRequired
            }
        );
        if remaining < 300 {
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
        assert_eq!(worker.backend.sends, usize::from(remaining == 300));
        assert_eq!(status.treasury_operations.len(), 1);
        drop(worker);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn preparing_recovery_uses_saved_bytes_and_existing_intent_never_resubmits() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    for saved in [false, true] {
        let (dir, mut worker, task) = fixture().await;
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
                FundingPhase::Prepared
            } else {
                FundingPhase::RecoveryRequired
            }
        );
        assert_eq!(worker.backend.sends, 0);
        if saved {
            let id = status.funding_jobs[0].operation_id.clone();
            worker
                .store
                .call(move |s| {
                    s.request_broadcast(&id, now, 1, false)?;
                    Ok(())
                })
                .await
                .unwrap();
            worker.tick(now + 300).await.unwrap();
            assert_eq!(worker.backend.sends, 0);
            let after = worker.store.call(|s| s.status()).await.unwrap();
            assert_eq!(after.funding_jobs[0].phase, FundingPhase::DepositPending);
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
            let (_dir, mut worker, task) = fixture().await;
            let now = x402_treazury::rotation::base::now().unwrap();
            // Real quote/preparation/broadcast-intent/reconciliation transitions.
            for i in 0..5 {
                worker.tick(now + i * 100).await.unwrap();
            }
            if phase != FundingPhase::Swapping {
                worker.backend.swap_status = if phase == FundingPhase::RefundPending {
                    || SwapStatus::Refunded
                } else {
                    || SwapStatus::Success
                };
                worker.tick(now + 500).await.unwrap();
            }
            let before = worker.store.call(|s| s.status()).await.unwrap();
            assert_eq!(before.funding_jobs[0].phase, phase);
            let credit_calls = worker.backend.credit_calls;
            // Credit would succeed if called: refund quarantine must prevent it.
            worker.backend.chain_credit = true;
            worker.backend.swap_status = status;
            worker.tick(now + 600).await.unwrap();
            let after = worker.store.call(|s| s.status()).await.unwrap();
            let expected = match status() {
                SwapStatus::Success if phase != FundingPhase::RefundPending => {
                    FundingPhase::Complete
                }
                SwapStatus::Refunded | SwapStatus::Failed => FundingPhase::RefundPending,
                SwapStatus::IncompleteDeposit => FundingPhase::RecoveryRequired,
                _ => phase.clone(),
            };
            assert_eq!(
                after.funding_jobs[0].phase,
                expected,
                "{phase:?}: {:?}",
                status()
            );
            assert_eq!(
                worker.backend.credit_calls - credit_calls,
                usize::from(expected == FundingPhase::Complete)
            );
            assert_eq!(worker.backend.sends, 1);
            assert_eq!(
                after.funding_jobs[0].operation_id,
                before.funding_jobs[0].operation_id
            );
            assert_eq!(
                serde_json::to_value(&after.treasury_operations).unwrap(),
                serde_json::to_value(&before.treasury_operations).unwrap(),
                "provider status cannot change the source accounting"
            );
            if expected != FundingPhase::Complete {
                assert_eq!(after.pools[0].addresses[0].role, "ALLOCATED");
            }
            drop(worker);
            task.await.unwrap();
        }
    }
}

#[tokio::test]
async fn coordinator_prepares_the_committed_bridge_target_without_requoting() {
    let (_dir, mut worker, task) = fixture().await;
    worker.backend.minimum = Some("6000000".into());
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.tick(now).await.unwrap();
    let state = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(state.funding_jobs[0].target, "6000000");
    assert_eq!(state.pools[0].addresses[0].target, "6000000");
    assert!(state.treasury_operations.is_empty());
    worker.tick(now + 5).await.unwrap();
    assert_eq!(worker.backend.quotes, 1);
    assert_eq!(worker.backend.sends, 0);
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
    let (_dir, mut worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.tick(now).await.unwrap();
    let original = worker
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    worker.backend.defer_prepare = true;
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
                .contains("no calculation started")
        );
        assert!(status.treasury_operations.is_empty());
    }
    worker.backend.defer_prepare = false;
    worker.tick(now + 100).await.unwrap();
    assert_eq!(worker.backend.quotes, 1);
    assert_eq!(worker.backend.sends, 0);
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
async fn uncertain_preparation_stays_quarantined_and_retains_failure_evidence() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    let (_dir, mut worker, task) = fixture().await;
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
    assert_eq!(
        status.funding_jobs[0].last_error.as_deref(),
        Some("preparation_fixture_failure")
    );
    assert_eq!(worker.backend.sends, 0);
    assert!(status.treasury_operations.is_empty());
    drop(worker);
    task.await.unwrap();
}

#[test]
fn credit_completion_clears_old_errors_and_rejects_late_stale_deferrals() {
    use x402_treazury::rotation::base::{Anchor, ChainView};
    for via_view in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = s.ensure_pool("a", "5").unwrap();
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
            s.reconcile_pool(
                &pool,
                ChainView {
                    admission_valid_until: u64::MAX,
                    anchor: Anchor {
                        height: 1,
                        hash: format!("0x{:064x}", 1),
                    },
                    balances: jobs
                        .iter()
                        .map(|j| {
                            (
                                j.wallet_id.clone(),
                                alloy_primitives::U256::from(5_000_000u64),
                            )
                        })
                        .collect(),
                    released: vec![],
                    resolutions: Default::default(),
                },
            )
            .unwrap();
        } else {
            for j in &jobs {
                s.record_credit(&j.wallet_id, "5000000", "block", 1)
                    .unwrap();
            }
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
        s.ensure_pool("b", "5").unwrap();
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
        s.record_credit(&pending.wallet_id, "5000000", "block", 1)
            .unwrap();
        assert!(
            s.funding_jobs()
                .unwrap()
                .into_iter()
                .find(|j| j.id == pending.id)
                .unwrap()
                .last_error
                .unwrap()
                .starts_with("source_reconciliation_failed;")
        );
    }
}

#[tokio::test]
async fn in_flight_credit_completion_cannot_overwrite_reconciled_or_promoted_wallets() {
    use x402_treazury::rotation::base::{Anchor, ChainView};
    use x402_treazury::rotation::store::funding::FundingPhase;
    for promote_again in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = s.ensure_pool("a", "5").unwrap();
        for j in s.funding_jobs().unwrap() {
            s.record_credit(&j.wallet_id, "5000000", "boot", 1).unwrap();
        }
        s.promote(&pool, 0).unwrap();
        let job = s
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.phase != FundingPhase::Complete)
            .unwrap();
        let (store, worker) = StoreHandle::spawn(s);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let late_store = store.clone();
        let wallet = job.wallet_id.clone();
        let late_credit = tokio::spawn(async move {
            // Pause at the real boundary: the RPC view was started, persistence has not run.
            started.send(()).unwrap();
            released.await.unwrap();
            late_store
                .call(move |s| s.record_credit(&wallet, "1", "old-in-flight-block", 2))
                .await
        });
        ready.await.unwrap();
        let p = pool.clone();
        store
            .call(move |s| {
                let query = s.chain_query(&p)?;
                s.reconcile_pool(
                    &p,
                    ChainView {
                        admission_valid_until: u64::MAX,
                        anchor: Anchor {
                            height: 3,
                            hash: format!("0x{:064x}", 3),
                        },
                        balances: query
                            .wallets
                            .iter()
                            .map(|(id, _)| (id.clone(), alloy_primitives::U256::from(5_000_000)))
                            .collect(),
                        released: vec![],
                        resolutions: Default::default(),
                    },
                )?;
                if promote_again {
                    s.promote(&p, 1)?;
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
            assert!(
                store
                    .call(move |s| s.record_credit(&wallet, "1", "fresh", 4))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("insufficient confirmed credit")
            );
        }
        drop(store);
        worker.await.unwrap();
    }
}

#[tokio::test]
async fn qualification_denial_preserves_quote_and_records_actionable_status() {
    use x402_treazury::rotation::{base::now, store::funding::FundingPhase};
    let (_dir, mut worker, task) = fixture().await;
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
    let reason = after.funding_jobs[0].last_error.as_deref().unwrap();
    assert!(reason.contains("qualification_funding_denied"), "{reason}");
    assert!(reason.contains("limit is zero"), "{reason}");
    assert_eq!(after.snapshot_revision, before.snapshot_revision);
    assert!(after.treasury_operations.is_empty());
    assert!(!after.outgoing_pending);
    assert_eq!(worker.backend.sends, 0);
    assert_eq!(worker.backend.quotes, 1);
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
    let (_dir, mut worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    for i in 0..4 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    let baseline = std::fs::read_to_string(log.path()).unwrap().len();
    worker.backend.reconcile_error = Some("treasury_confirmation_pending");
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
        assert_eq!(worker.backend.sends, 1);
        assert_ne!(status.treasury_operations[0].submission, "CONFIRMED");
    }
    let output = std::fs::read_to_string(log.path()).unwrap();
    let waiting = &output[baseline..];
    assert!(waiting.contains("zcash_confirmations_pending"));
    assert!(!waiting.contains("WARN"), "{waiting}");
    worker.backend.reconcile_error = Some("private upstream error");
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
    let failed = &output[baseline..];
    assert!(failed.contains("WARN"));
    assert!(failed.contains("funding check failed"));
    assert!(!failed.contains("private upstream error"));
    assert!(!failed.contains("source_confirmation_pending"));
    worker.backend.reconcile_error = None;
    worker.tick(now + 8000).await.unwrap();
    assert_eq!(worker.backend.sends, 1);
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
    let (_dir, mut worker, task) = fixture().await;
    let now = x402_treazury::rotation::base::now().unwrap();
    worker.backend.quote_deadline = now + 450;
    worker.tick(now).await.unwrap();
    worker.tick(now + 100).await.unwrap();
    worker.tick(now + 200).await.unwrap();
    worker.backend.recovery = Some(RecoveryOutcome::Waiting("waiting_for_transaction_expiry"));
    for i in 3..6 {
        worker.tick(now + i * 1000).await.unwrap();
        let status = worker.store.call(|s| s.status()).await.unwrap();
        assert_eq!(status.funding_jobs[0].phase, FundingPhase::RecoveryRequired);
        assert_eq!(status.treasury_operations[0].submission, "PREPARED");
        assert_eq!(status.treasury_operations[0].attempts, 0);
        assert_eq!(worker.backend.sends, 0);
    }
    worker.backend.recovery = Some(RecoveryOutcome::OperatorRequired(
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
    assert_eq!(worker.backend.sends, 0);
    drop(worker);
    task.await.unwrap();
}

#[tokio::test]
async fn repeated_quote_outages_do_not_consume_recovery_attempts_or_allocate_again() {
    let (_dir, mut worker, task) = fixture().await;
    worker.backend.quote_outage = true;
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
        assert!(job.next_poll > instant);
        instant = job.next_poll;
    }
    worker.backend.quote_outage = false;
    worker.tick(instant).await.unwrap();
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_treazury::rotation::store::funding::FundingPhase::Quoted
    );
    assert_eq!(status.funding_jobs[0].operation_id, id);
    assert_eq!(worker.backend.sends, 0);
    worker
        .store
        .call(move |s| s.require_unstarted_preparation(&id))
        .await
        .unwrap();
    drop(worker);
    task.await.unwrap();
}
