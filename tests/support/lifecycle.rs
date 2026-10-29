//! Real payment admission/signing/RPC plus a deterministic funding adapter.
//! Consensus calculation, shielding and expiry are qualified separately in regtest.
#[path = "zcash_transaction.rs"]
mod zcash_transaction;
use super::*;
use anyhow::Result;
use x402_treazury::rotation::{
    funding::{FundingBackend, FundingObservations, FundingWorker},
    near::{Quote, SwapStatus},
    store::{
        SyncObservation, SyncPhase,
        funding::{FundingJob, FundingPhase},
    },
    transaction::TransactionFacts,
};
#[derive(Clone)]
pub(super) struct Funding {
    store: StoreHandle,
    fake: Fake,
    base: String,
    sends: Arc<Mutex<std::collections::BTreeSet<String>>>,
    available: Arc<AtomicBool>,
    refund_pool: Option<String>,
    hold_confirmation_pool: Option<String>,
    pub(super) capital: Arc<std::sync::atomic::AtomicU64>,
}
pub(super) fn synced(s: &mut Store) -> Result<()> {
    synced_capacity(s, 10000)
}
pub(super) fn synced_capacity(s: &mut Store, capacity: u64) -> Result<()> {
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
            observed_tip_height: None,
            height: Some(1),
            confirmations: 1,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 10000,
            spendable_shielded_zatoshis: capacity,
        }),
    )?;
    Ok(())
}
impl FundingObservations for Funding {
    type QuoteRequest = Quote;
    async fn quote(&self, quote: Quote) -> Result<Quote> {
        Ok(quote)
    }

    async fn status(&self, quote: Quote) -> Result<SwapStatus> {
        if self.refund_pool.as_deref() == quote.request["pool"].as_str() {
            return Ok(SwapStatus::Refunded);
        }
        self.fake.balances.lock().unwrap().insert(
            quote.request["recipient"].as_str().unwrap().into(),
            quote.request["target"].as_str().unwrap().parse()?,
        );
        Ok(SwapStatus::Success)
    }
    async fn credit(&self, job: FundingJob) -> Result<()> {
        let pool = job.pool_id.clone();
        let query = self.store.call(move |s| s.chain_query(&pool)).await?;
        let view = BaseRpc::new(&format!("{}/rpc", self.base), 12)?
            .view(query)
            .await?;
        let wallet = job.wallet_id.clone();
        self.store
            .call(move |s| s.record_verified_credit(&wallet, view).map(|_| ()))
            .await
    }
}
impl FundingBackend for Funding {
    type Observations = Self;
    fn observations(&self) -> Self {
        self.clone()
    }
    fn allocation_groups(&self) -> Vec<String> {
        [
            "research",
            "other",
            "lifecycle",
            "background",
            "refund",
            "consumption",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }
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
        self.store.call(|s| s.require_preparation_available()).await
    }
    async fn quote_prerequisite(&mut self, job: &FundingJob) -> Result<Quote> {
        let id = job.id.clone();
        let operation = job.operation_id.clone();
        let target = job.target.clone();
        self.store
            .call(move |s| s.select_quote_terms(&id, &operation, &target, &target))
            .await?;
        Ok(Quote {
            sizing_preview: None,
            request: json!({"recipient":job.recipient,"target":job.target,"amount":job.target,"pool":job.pool_name}),
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
                s.accept_preparation(&job.operation_id, Some(&job.pool_id), 100, None)?;
                let revision = s.snapshot()?.0;
                s.prepare_with_facts(
                    &job.operation_id,
                    revision,
                    b"calculated fixture",
                    zcash_transaction::raw(100).as_slice(),
                    Some(TransactionFacts {
                        txid: zcash_transaction::txid(100),
                        expiry_height: 100,
                        amount_zatoshis: 80,
                        fee_zatoshis: 20,
                        deadline: Some(u64::MAX),
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
        if self.hold_confirmation_pool.as_deref() == Some(job.pool_name.as_str()) {
            return Ok(false);
        }
        let id = job.operation_id.clone();
        let capital = self.capital.clone();
        self.store
            .call(move |s| {
                let previously = s.operation(&id)?.submission == "CONFIRMED";
                s.confirm_spend(&id)?;
                if !previously {
                    capital.fetch_sub(100, Ordering::SeqCst);
                }
                synced_capacity(s, capital.load(Ordering::SeqCst))
            })
            .await?;
        Ok(true)
    }
}
pub(super) fn funder(
    h: &Harness,
    sends: Arc<Mutex<std::collections::BTreeSet<String>>>,
) -> FundingWorker<Funding> {
    FundingWorker::new(
        h.store.clone(),
        Funding {
            store: h.store.clone(),
            fake: h.f.clone(),
            base: h.base.clone(),
            sends,
            available: Arc::new(AtomicBool::new(true)),
            refund_pool: None,
            hold_confirmation_pool: None,
            capital: Arc::new(std::sync::atomic::AtomicU64::new(10000)),
        },
        1,
    )
}
pub(super) async fn tick(worker: &mut FundingWorker<Funding>, instant: &mut u64) {
    worker.tick(*instant).await.unwrap();
    *instant += 100;
}

#[tokio::test]
async fn bootstrap_payment_promotion_pending_refill_restart_and_pool_isolation() {
    let mut h = Harness::new().await;
    h.pool = h
        .store
        .call(|s| {
            crate::capacity_fixture::declare_and_allocate!(
                s,
                "lifecycle",
                crate::capacity_fixture::two_reserves("5"),
                Some(alloy_primitives::U256::from(5_000_000)),
                0
            )
        })
        .await
        .unwrap();
    h.client = make_client(h.store.clone(), h.pool.clone(), &h.base);
    h.store
        .call(|s| {
            crate::capacity_fixture::declare_and_allocate!(
                s,
                "other",
                crate::capacity_fixture::two_reserves("5"),
                Some(alloy_primitives::U256::from(5_000_000)),
                0
            )?;
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
            .call(|s| Ok(s.status()?.pools.iter().all(|p| p.inventory_ready)))
            .await
            .unwrap()
        {
            break;
        }
    }
    let status = h.store.call(|s| s.status()).await.unwrap();
    assert!(status.pools.iter().all(|p| p.inventory_ready));
    assert_eq!(sends.lock().unwrap().len(), 4);
    let research = status.pools.iter().find(|p| p.id == h.pool).unwrap();
    let old_active = research
        .addresses
        .iter()
        .find(|a| a.role == "READY")
        .unwrap()
        .address
        .clone();
    let standby = research
        .addresses
        .iter()
        .find(|a| a.role == "READY" && a.address != old_active)
        .unwrap()
        .address
        .clone();
    h.client.execute(h.route()).await.unwrap();
    h.f.used.store(true, Ordering::SeqCst);
    h.f.balances.lock().unwrap().insert(old_active.clone(), 0);
    reconcile_complete_history(&h).await;
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
    let operation = replacement.operation_id.clone();
    let (prepared_bytes, prepared_facts, lifecycle) = h
        .store
        .call(move |s| {
            Ok((
                s.prepared_bytes(&operation)?,
                x402_treazury::rotation::transaction::PreparedTransaction::load(s, &operation)?
                    .facts()
                    .cloned(),
                s.qualification_lifecycle()?,
            ))
        })
        .await
        .unwrap();
    let id = status.treasury_id;
    assert_eq!(sends.lock().unwrap().len(), 4);
    drop(worker);
    drop(h.client);
    drop(h.store);
    h.worker.await.unwrap();
    let s = Store::open(&h.dir.path().join("state"), &h.dir.path().join("key"), &id).unwrap();
    assert_eq!(
        &*s.prepared_bytes(&replacement.operation_id).unwrap(),
        &*prepared_bytes
    );
    assert_eq!(
        x402_treazury::rotation::transaction::PreparedTransaction::load(
            &s,
            &replacement.operation_id
        )
        .unwrap()
        .facts()
        .cloned(),
        prepared_facts
    );
    let reopened_lifecycle = s.qualification_lifecycle().unwrap();
    for field in ["source_operations", "funding_jobs"] {
        assert_eq!(reopened_lifecycle[field], lifecycle[field], "{field}");
    }
    assert_eq!(sends.lock().unwrap().len(), 4, "reopen never submits");
    (h.store, h.worker) = StoreHandle::spawn(s);
    h.client = make_client(h.store.clone(), h.pool.clone(), &h.base);
    let mut worker = funder(&h, Arc::clone(&sends));
    // Canonical history observes prior settlement independently of the refill.
    reconcile_complete_history(&h).await;
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
    assert_eq!(sends.lock().unwrap().len(), 6);
    assert!(sends.lock().unwrap().contains(&replacement.operation_id));
    let research = status.pools.iter().find(|p| p.id == h.pool).unwrap();
    assert_eq!(
        research
            .reserve_claims
            .iter()
            .find(|claim| claim.slot == "small")
            .unwrap()
            .generation,
        1
    );
    assert_eq!(
        research
            .addresses
            .iter()
            .filter(|a| a.role == "READY")
            .count(),
        2
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
    assert_eq!(
        other
            .reserve_claims
            .iter()
            .find(|claim| claim.slot == "small")
            .unwrap()
            .generation,
        0
    );
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
async fn failed_swap_retains_original_source_terms_while_another_pool_serves() {
    let h = Harness::new().await;
    // Existing research pool is funded independently; only the new pool swaps.
    h.client.execute(h.route()).await.unwrap();
    h.store
        .call(|s| {
            crate::capacity_fixture::declare_and_allocate!(
                s,
                "refund",
                crate::capacity_fixture::two_reserves("5"),
                Some(alloy_primitives::U256::from(5_000_000)),
                0
            )?;
            synced(s)
        })
        .await
        .unwrap();
    let sends = Arc::default();
    let mut worker = funder(&h, Arc::clone(&sends));
    {
        let mut backend = worker.backend.lock().await;
        backend.refund_pool = Some("refund".into());
        // Keep native confirmation pending so SUCCESS must preserve BROADCAST
        // rather than racing this fixture's immediate confirmation callback.
        backend.hold_confirmation_pool = Some("research".into());
    }
    let mut instant = now().unwrap();
    for _ in 0..35 {
        tick(&mut worker, &mut instant).await;
    }
    let status = h.store.call(|s| s.status()).await.unwrap();
    let refund = status.pools.iter().find(|p| p.name == "refund").unwrap();
    assert!(refund.funding_degraded && !refund.inventory_ready);
    assert!(
        status
            .funding_jobs
            .iter()
            .filter(|j| j.pool_name == "refund")
            .all(|j| j.phase == FundingPhase::RefundPending)
    );
    assert!(
        status.legacy_refunds.is_none(),
        "API status must never manufacture on-chain refund credit"
    );
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    // Accepted source terms are immutable audit data. Provider fulfillment
    // closes source work without fabricating an SDK confirmation for it.
    assert_eq!(status.treasury_operations.len(), 3);
    let journaled: u64 = status
        .treasury_operations
        .iter()
        .map(|operation| {
            assert_eq!(operation.facts.txid, zcash_transaction::txid(100));
            assert_eq!(operation.facts.amount_zatoshis, 80);
            assert_eq!(operation.facts.fee_zatoshis, 20);
            assert_eq!(operation.facts.expiry_height, 100);
            assert_eq!(operation.attempts, 1);
            operation.facts.amount_zatoshis + operation.facts.fee_zatoshis
        })
        .sum();
    assert_eq!(journaled, 300);
    let (confirmed,confirmed_terms):(i64,i64)=db.query_row("SELECT COUNT(*),SUM(json_extract(facts,'$.amount_zatoshis')+json_extract(facts,'$.fee_zatoshis')) FROM treasury_operations WHERE submission='CONFIRMED'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((confirmed, confirmed_terms), (2, 200));
    let (fulfilled_native,fulfilled_work):(String,String)=db.query_row("SELECT o.submission,x.state FROM funding_progress j JOIN treasury_operations o ON o.id=j.operation_id JOIN outgoing x ON x.id=o.id WHERE j.provider_success=1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        (fulfilled_native.as_str(), fulfilled_work.as_str()),
        ("BROADCAST", "RESOLVED")
    );
    let originals: Vec<_> = status
        .treasury_operations
        .iter()
        .map(|operation| operation.operation_id.clone())
        .collect();
    h.store
        .call(move |s| {
            for operation in originals {
                assert_eq!(
                    s.prepared_bytes(&operation)?.as_slice(),
                    zcash_transaction::raw(100).as_slice()
                );
                assert!(!s.operation_pending(&operation)?);
            }
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(sends.lock().unwrap().len(), 3);
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
    reconcile_complete_history(&h).await;
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
            let p = crate::capacity_fixture::declare_and_allocate!(
                s,
                "background",
                crate::capacity_fixture::two_reserves("5"),
                Some(alloy_primitives::U256::from(5_000_000)),
                0
            )?;
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
                .filter(|p| p.id == other || p.id == h.pool)
                .all(|p| p.inventory_ready)
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
            .inventory_ready
    );
    assert!(
        state
            .pools
            .iter()
            .find(|p| p.id == h.pool)
            .unwrap()
            .inventory_ready
    );
    assert_eq!(sends.lock().unwrap().len(), 3);
    assert!(
        !payment.is_finished(),
        "funding must complete before the held payment is released"
    );
    let independent = make_client(h.store.clone(), other, &h.base);
    assert_eq!(independent.execute(h.route()).await.unwrap(), "paid");
    // The unresolved authorization continues to consume its original wallet's
    // capacity; another READY wallet can independently satisfy the next call.
    h.f.release.notify_one();
    assert_eq!(payment.await.unwrap().unwrap(), "paid");
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    let signed = h.f.signed.lock().unwrap().clone();
    assert_eq!(signed.len(), 3);
    assert_ne!(
        signed[0]["payload"]["authorization"]["from"],
        signed[2]["payload"]["authorization"]["from"]
    );
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3,
        "cross-wallet progress must retain every unresolved signed liability"
    );
    drop(db);
    drop(independent);
    drop(worker);
    h.close().await;
}
