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
}
impl FundingBackend for Fake {
    async fn quote(&mut self, _: &FundingJob) -> Result<Quote> {
        Ok(Quote {
            request: serde_json::json!({}),
            response: serde_json::json!({}),
            input: 50,
            deadline: u64::MAX,
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
