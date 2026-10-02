#![cfg(feature = "zcash")]
use anyhow::Result;
use x402_mcp_prototype::rotation::{
    funding::{FundingBackend, FundingWorker},
    near::{Quote, SwapStatus},
    store::{Store, StoreHandle, SyncObservation, SyncPhase, funding::FundingJob},
    transaction::TransactionFacts,
};
struct Fake {
    store: StoreHandle,
    sends: usize,
    chain_credit: bool,
    status_error: bool,
    status_calls: usize,
    funds_available: bool,
    quote_deadline: u64,
    quotes: usize,
    timeout: u64,
}
impl FundingBackend for Fake {
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
    async fn quote(&mut self, _: &FundingJob) -> Result<Quote> {
        self.quotes += 1;
        Ok(Quote {
            request: serde_json::json!({}),
            response: serde_json::json!({}),
            input: 50,
            deadline: self.quote_deadline,
            deposit: Some("test-deposit".into()),
        })
    }
    async fn prepare(&mut self, j: &FundingJob, _: &Quote) -> Result<()> {
        let id = j.operation_id.clone();
        let pool = j.pool_id.clone();
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
                        deadline: u64::MAX,
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
        Ok(SwapStatus::Success)
    }
    async fn credit(&mut self, j: &FundingJob) -> Result<()> {
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
    let now = x402_mcp_prototype::rotation::base::now().unwrap();
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
            height: Some(1),
            confirmations: 1,
            max_age_seconds: 300,
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
            chain_credit: false,
            status_error: false,
            status_calls: 0,
            funds_available: true,
            quote_deadline: u64::MAX,
            quotes: 0,
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
        x402_mcp_prototype::rotation::store::funding::FundingPhase::VerifyingCredit
    );
    assert_eq!(status.pools[0].addresses[0].role, "ALLOCATED");
    assert_eq!(status.treasury_operations[0].attempts, 1);
    worker.backend.chain_credit = true;
    worker.tick(now + 9000).await.unwrap();
    assert_eq!(
        store.call(|s| s.status()).await.unwrap().funding_jobs[0].phase,
        x402_mcp_prototype::rotation::store::funding::FundingPhase::Complete
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
    let pool = s.ensure_pool("a", "5").unwrap();
    let jobs = s.funding_jobs().unwrap();
    let job = &jobs[0];
    s.save_funding_quote(&job.id, b"quote").unwrap();
    s.advance_funding(
        &job.id,
        x402_mcp_prototype::rotation::store::funding::FundingPhase::Quoted,
        x402_mcp_prototype::rotation::store::funding::FundingPhase::Preparing,
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
            chain_credit: true,
            status_error: false,
            status_calls: 0,
            funds_available: true,
            quote_deadline: u64::MAX,
            quotes: 0,
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
    let now = x402_mcp_prototype::rotation::base::now().unwrap();
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
            height: Some(1),
            confirmations: 1,
            max_age_seconds: 3600,
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
            chain_credit: false,
            status_error: false,
            status_calls: 0,
            funds_available: true,
            quote_deadline: u64::MAX,
            quotes: 0,
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
    let now = x402_mcp_prototype::rotation::base::now().unwrap();
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
    let now = x402_mcp_prototype::rotation::base::now().unwrap();
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
        x402_mcp_prototype::rotation::store::funding::FundingPhase::Complete
    );
    assert!(!status.funding_jobs[0].timed_out && !status.pools[0].funding_degraded);
    assert_eq!(worker.backend.sends, 1);
    drop(worker);
    task.await.unwrap();
}
#[tokio::test]
async fn insufficient_treasury_waits_and_only_unprepared_quotes_refresh_with_a_bound() {
    let (_dir, mut worker, task) = fixture().await;
    let now = x402_mcp_prototype::rotation::base::now().unwrap();
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
        x402_mcp_prototype::rotation::store::funding::FundingPhase::Quoted
    );
    assert!(
        status.funding_jobs[0]
            .last_error
            .as_ref()
            .unwrap()
            .contains("insufficient_spendable")
    );
    assert!(status.treasury_operations.is_empty());
    worker.backend.funds_available = true;
    for i in 2..8 {
        worker.tick(now + i * 1000).await.unwrap();
    }
    let status = worker.store.call(|s| s.status()).await.unwrap();
    assert_ne!(status.funding_jobs[0].operation_id, original);
    assert_eq!(worker.backend.quotes, 3);
    assert_eq!(worker.backend.sends, 0);
    assert_eq!(
        status.funding_jobs[0].phase,
        x402_mcp_prototype::rotation::store::funding::FundingPhase::RecoveryRequired
    );
    assert!(
        status.funding_jobs[0]
            .last_error
            .as_ref()
            .unwrap()
            .contains("quote_refresh_exhausted")
    );
    drop(worker);
    task.await.unwrap();
}
