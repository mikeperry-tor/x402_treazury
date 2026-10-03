//! Real payment admission/signing/RPC plus a deterministic funding adapter.
//! Consensus calculation, shielding and expiry are qualified separately in regtest.
use super::*;
use anyhow::Result;
use x402_treazury::rotation::{
    funding::{FundingBackend, FundingWorker},
    near::{Quote, SwapStatus},
    store::{
        SyncObservation, SyncPhase,
        funding::{FundingJob, FundingPhase},
    },
    transaction::TransactionFacts,
};
#[derive(Clone)]
struct Funding {
    store: StoreHandle,
    fake: Fake,
    base: String,
    sends: Arc<Mutex<std::collections::BTreeSet<String>>>,
    available: Arc<AtomicBool>,
    refund_pool: Option<String>,
}
fn synced(s: &mut Store) -> Result<()> {
    let revision = s.snapshot()?.0;
    let instant = now()?;
    s.save_sync_snapshot(
        revision,
        b"synced fixture",
        Some(SyncObservation {
            phase: SyncPhase::Ready,
            last_error: None,
            snapshot_revision: revision,
            checked_at: Some(instant),
            checkpoint_at: instant,
            scanned_blocks: 1,
            target_height: Some(1),
            height: Some(1),
            confirmations: 1,
            max_age_seconds: 3600,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 10000,
            spendable_shielded_zatoshis: 10000,
        }),
    )?;
    Ok(())
}
impl FundingBackend for Funding {
    fn swap_timeout_seconds(&self) -> u64 {
        u64::MAX
    }
    fn max_attempts(&self, _: &FundingJob) -> u32 {
        3
    }
    async fn ready(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        anyhow::ensure!(
            self.available.load(Ordering::SeqCst),
            "treasury_insufficient_spendable_funds"
        );
        self.store
            .call(|s| s.check_funding_capacity(now()?, 100, 10000))
            .await
    }
    async fn quote(&mut self, job: &FundingJob) -> Result<Quote> {
        Ok(Quote {
            request: json!({"recipient":job.recipient,"target":job.target,"pool":job.pool_name}),
            response: json!({}),
            input: 80,
            deadline: u64::MAX,
            deposit: Some("fixture deposit".into()),
        })
    }
    async fn prepare(&mut self, job: &FundingJob, _: &Quote) -> Result<()> {
        let job = job.clone();
        self.store
            .call(move |s| {
                s.reserve(
                    &job.operation_id,
                    Some(&job.pool_id),
                    u32::try_from(now()? / 86400)?,
                    100,
                    10000,
                )?;
                let revision = s.snapshot()?.0;
                s.prepare_with_facts(
                    &job.operation_id,
                    revision,
                    b"calculated fixture",
                    job.operation_id.as_bytes(),
                    Some(TransactionFacts {
                        txid: job.operation_id.clone(),
                        expiry_height: 100,
                        amount_zatoshis: 80,
                        fee_zatoshis: 20,
                        deadline: u64::MAX,
                    }),
                )?;
                Ok(())
            })
            .await
    }
    async fn submit(&mut self, job: &FundingJob) -> Result<()> {
        let id = job.operation_id.clone();
        self.store
            .call(move |s| {
                let attempt = s.request_broadcast(&id, now()?, 1, false)?;
                s.broadcast_result(&id, attempt, true)
            })
            .await?;
        assert!(
            self.sends.lock().unwrap().insert(job.operation_id.clone()),
            "duplicate source send"
        );
        Ok(())
    }
    async fn reconcile(&mut self, job: &FundingJob) -> Result<bool> {
        let id = job.operation_id.clone();
        self.store
            .call(move |s| {
                s.confirm_spend(&id, 100, u32::try_from(now()? / 86400)?)?;
                synced(s)
            })
            .await?;
        Ok(true)
    }
    async fn status(&mut self, quote: &Quote) -> Result<SwapStatus> {
        if self.refund_pool.as_deref() == quote.request["pool"].as_str() {
            return Ok(SwapStatus::Refunded);
        }
        self.fake.balances.lock().unwrap().insert(
            quote.request["recipient"].as_str().unwrap().into(),
            quote.request["target"].as_str().unwrap().parse()?,
        );
        Ok(SwapStatus::Success)
    }
    async fn credit(&mut self, job: &FundingJob) -> Result<()> {
        let pool = job.pool_id.clone();
        let query = self.store.call(move |s| s.chain_query(&pool)).await?;
        let view = BaseRpc::new(&format!("{}/rpc", self.base), 12, 120)?
            .view(query)
            .await?;
        let balance = view.balances[&job.wallet_id].to_string();
        let wallet = job.wallet_id.clone();
        self.store
            .call(move |s| {
                s.record_credit(
                    &wallet,
                    &balance,
                    &view.anchor.hash,
                    i64::try_from(view.anchor.height)?,
                )
            })
            .await
    }
}
fn funder(
    h: &Harness,
    sends: Arc<Mutex<std::collections::BTreeSet<String>>>,
) -> FundingWorker<Funding> {
    FundingWorker {
        store: h.store.clone(),
        backend: Funding {
            store: h.store.clone(),
            fake: h.f.clone(),
            base: h.base.clone(),
            sends,
            available: Arc::new(AtomicBool::new(true)),
            refund_pool: None,
        },
        poll_seconds: 1,
    }
}
async fn tick(worker: &mut FundingWorker<Funding>, instant: &mut u64) {
    worker.tick(*instant).await.unwrap();
    *instant += 100;
}

#[tokio::test]
async fn bootstrap_payment_promotion_pending_refill_restart_and_pool_isolation() {
    let mut h = Harness::new().await;
    h.f.balances.lock().unwrap().clear();
    h.store
        .call(|s| {
            s.ensure_pool("other", "5")?;
            synced(s)
        })
        .await
        .unwrap();
    let sends = Arc::default();
    let mut worker = funder(&h, Arc::clone(&sends));
    let mut instant = now().unwrap();
    assert!(h.client.execute(h.route()).await.is_err());
    assert!(h.f.signed.lock().unwrap().is_empty());
    for _ in 0..60 {
        tick(&mut worker, &mut instant).await;
        if h.store
            .call(|s| Ok(s.status()?.pools.iter().all(|p| p.bootstrapped)))
            .await
            .unwrap()
        {
            break;
        }
    }
    let status = h.store.call(|s| s.status()).await.unwrap();
    assert!(status.pools.iter().all(|p| p.bootstrapped));
    assert_eq!(sends.lock().unwrap().len(), 4);
    let research = status.pools.iter().find(|p| p.id == h.pool).unwrap();
    let old_active = research
        .addresses
        .iter()
        .find(|a| a.role == "ACTIVE")
        .unwrap()
        .address
        .clone();
    let standby = research
        .addresses
        .iter()
        .find(|a| a.role == "READY")
        .unwrap()
        .address
        .clone();
    h.client.execute(h.route()).await.unwrap();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(old_active.clone(), 0);
    h.client.execute(h.route()).await.unwrap();
    let signed = h.f.signed.lock().unwrap().clone();
    assert_eq!(
        signed[0]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .parse::<Address>()
            .unwrap(),
        old_active.parse::<Address>().unwrap()
    );
    assert_eq!(
        signed[1]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .parse::<Address>()
            .unwrap(),
        standby.parse::<Address>().unwrap()
    );
    h.f.balances
        .lock()
        .unwrap()
        .insert(standby.clone(), 2_000_000);
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    // Stop with the replacement transaction prepared but not sent.
    for _ in 0..8 {
        tick(&mut worker, &mut instant).await;
        if h.store
            .call(|s| Ok(s.status()?.outgoing_pending))
            .await
            .unwrap()
        {
            break;
        }
    }
    let status = h.store.call(|s| s.status()).await.unwrap();
    assert!(status.outgoing_pending);
    let replacement = status
        .funding_jobs
        .iter()
        .find(|j| j.phase == FundingPhase::Prepared)
        .unwrap()
        .clone();
    let id = status.treasury_id;
    assert_eq!(sends.lock().unwrap().len(), 4);
    drop(worker);
    drop(h.client);
    drop(h.store);
    h.worker.await.unwrap();
    let s = Store::open(&h.dir.path().join("state"), &h.dir.path().join("key"), &id).unwrap();
    (h.store, h.worker) = StoreHandle::spawn(s);
    h.client = make_client(h.store.clone(), h.pool.clone(), &h.base);
    let mut worker = funder(&h, Arc::clone(&sends));
    // Payment from the promoted active wallet works while Zcash refill is pending.
    h.client.execute(h.route()).await.unwrap();
    for _ in 0..12 {
        tick(&mut worker, &mut instant).await;
        if h.store
            .call(|s| {
                Ok(s.status()?
                    .funding_jobs
                    .iter()
                    .all(|j| j.phase == FundingPhase::Complete))
            })
            .await
            .unwrap()
        {
            break;
        }
    }
    let status = h.store.call(|s| s.status()).await.unwrap();
    assert!(
        status
            .funding_jobs
            .iter()
            .all(|j| j.phase == FundingPhase::Complete)
    );
    assert_eq!(sends.lock().unwrap().len(), 5);
    assert!(sends.lock().unwrap().contains(&replacement.operation_id));
    let research = status.pools.iter().find(|p| p.id == h.pool).unwrap();
    assert_eq!(research.generation, 1);
    assert_eq!(
        research
            .addresses
            .iter()
            .filter(|a| a.role == "READY")
            .count(),
        1
    );
    assert_eq!(
        research
            .addresses
            .iter()
            .find(|a| a.role == "ACTIVE")
            .unwrap()
            .address,
        standby
    );
    let other = status.pools.iter().find(|p| p.name == "other").unwrap();
    assert_eq!(other.generation, 0);
    assert_eq!(other.addresses.len(), 2);
    assert!(
        research
            .addresses
            .iter()
            .all(|a| other.addresses.iter().all(|b| a.address != b.address))
    );
    drop(worker);
    h.close().await;
}

#[tokio::test]
async fn failed_swap_keeps_source_expense_while_another_pool_serves() {
    let h = Harness::new().await;
    // Existing research pool is funded independently; only the new pool swaps.
    h.client.execute(h.route()).await.unwrap();
    h.store
        .call(|s| {
            s.ensure_pool("refund", "5")?;
            synced(s)
        })
        .await
        .unwrap();
    let sends = Arc::default();
    let mut worker = funder(&h, Arc::clone(&sends));
    worker.backend.refund_pool = Some("refund".into());
    let mut instant = now().unwrap();
    for _ in 0..35 {
        tick(&mut worker, &mut instant).await;
    }
    let status = h.store.call(|s| s.status()).await.unwrap();
    let refund = status.pools.iter().find(|p| p.name == "refund").unwrap();
    assert!(refund.funding_degraded && !refund.bootstrapped);
    assert!(
        status
            .funding_jobs
            .iter()
            .filter(|j| j.pool_name == "refund")
            .all(|j| j.phase == FundingPhase::RefundPending)
    );
    assert!(
        status.refunds.is_empty(),
        "API status must never manufacture on-chain refund credit"
    );
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let consumed: i64 = db
        .query_row("SELECT SUM(consumed) FROM budget_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(consumed, 200);
    assert_eq!(sends.lock().unwrap().len(), 2);
    h.f.used.store(true, Ordering::SeqCst);
    let active = status
        .pools
        .iter()
        .find(|p| p.id == h.pool)
        .unwrap()
        .addresses
        .iter()
        .find(|a| a.role == "ACTIVE")
        .unwrap()
        .address
        .clone();
    h.f.balances.lock().unwrap().insert(active, 2_000_000);
    *h.f.challenge.lock().unwrap() = challenge("1000000");
    h.client.execute(h.route()).await.unwrap();
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    drop(worker);
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn funding_and_reconciliation_progress_while_another_pool_payment_is_held() {
    let h = Harness::new().await;
    h.f.hold.store(true, Ordering::SeqCst);
    let client = h.client.clone();
    let route = h.route();
    let payment = tokio::spawn(async move { client.execute(route).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), h.f.arrived.notified())
        .await
        .unwrap();
    let other = h
        .store
        .call(|s| {
            let p = s.ensure_pool("background", "5")?;
            synced(s)?;
            Ok(p)
        })
        .await
        .unwrap();
    let sends = Arc::default();
    let mut worker = funder(&h, Arc::clone(&sends));
    let mut instant = now().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for _ in 0..60 {
            tick(&mut worker, &mut instant).await;
            if h.store
                .call(|s| s.status())
                .await
                .unwrap()
                .pools
                .iter()
                .find(|p| p.id == other)
                .unwrap()
                .bootstrapped
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    let state = h.store.call(|s| s.status()).await.unwrap();
    assert!(
        state
            .pools
            .iter()
            .find(|p| p.id == other)
            .unwrap()
            .bootstrapped
    );
    assert_eq!(sends.lock().unwrap().len(), 2);
    assert!(
        !payment.is_finished(),
        "funding must complete before the held payment is released"
    );
    let independent = make_client(h.store.clone(), other, &h.base);
    assert_eq!(independent.execute(h.route()).await.unwrap(), "paid");
    // The unresolved held authorization still prevents over-admission on research.
    h.f.release.notify_one();
    assert_eq!(payment.await.unwrap().unwrap(), "paid");
    assert!(
        h.client
            .execute(h.route())
            .await
            .unwrap_err()
            .to_string()
            .contains("payment_pending")
    );
    assert_eq!(h.f.signed.lock().unwrap().len(), 2);
    drop(independent);
    drop(worker);
    h.close().await;
}
