//! Bootstrap participation defers new claims, never existing source ownership.
use super::*;
use crate::rotation::{store::Store, transaction::TransactionFacts};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Clone)]
struct Reads;
impl FundingObservations for Reads {
    type QuoteRequest = ();
    async fn quote(&self, _: ()) -> Result<Quote> {
        panic!("fixture never prepares new quotes")
    }
    async fn status(&self, _: Quote) -> Result<SwapStatus> {
        Ok(SwapStatus::Processing)
    }
    async fn credit(&self, _: FundingJob) -> Result<()> {
        // No positive observation: the original operation remains pending.
        Ok(())
    }
}

struct Backend {
    source: mpsc::UnboundedSender<tokio::time::Instant>,
    groups: Vec<String>,
}
impl FundingBackend for Backend {
    type Observations = Reads;
    fn observations(&self) -> Reads {
        Reads
    }
    fn allocation_groups(&self) -> Vec<String> {
        self.groups.clone()
    }
    async fn quote_prerequisite(&mut self, _: &FundingJob) -> Result<()> {
        anyhow::bail!("unfunded fixture does not obtain new quotes")
    }
    async fn prepare(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        panic!("new preparation is forbidden")
    }
    async fn submit(&mut self, _: &FundingJob) -> Result<()> {
        panic!("new disclosure is forbidden")
    }
    async fn reconcile(&mut self, job: &FundingJob) -> Result<bool> {
        assert_eq!(job.pool_name, "existing");
        self.source.send(tokio::time::Instant::now()).unwrap();
        Ok(false)
    }
}

// Keep paused time from advancing while the blocking Store actor makes progress.
async fn local<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            value = &mut future => return value,
            _ = tokio::task::yield_now() => {},
        }
    }
}

async fn published_after(store: &StoreHandle, id: &str, instant: u64) {
    local(async {
        loop {
            let id = id.to_owned();
            if store
                .call(move |s| Ok(s.funding_job(&id)?.unwrap().next_poll > instant))
                .await
                .unwrap()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

fn submitted(store: &mut Store, job: &FundingJob) {
    let quote = Quote {
        sizing_preview: None,
        request: serde_json::json!({"amount":job.target,"recipient":job.recipient}),
        response: serde_json::json!({}),
        input: 50,
        deadline: u64::MAX,
        deposit: Some("fixture-deposit".into()),
    };
    store
        .save_funding_quote(&job.id, &serde_json::to_vec(&quote).unwrap())
        .unwrap();
    store
        .require_preparation_authority(
            &job.operation_id,
            Some(&crate::rotation::transaction::ManagedFundingAuthority {
                job_id: job.id.clone(),
                auto_fund: true,
            }),
        )
        .unwrap();
    store
        .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
        .unwrap();
    let revision = store.snapshot().unwrap().0;
    store
        .prepare_with_facts(
            &job.operation_id,
            revision,
            b"next",
            crate::rotation::store::test_support::transaction_raw(100).as_slice(),
            Some(TransactionFacts {
                txid: crate::rotation::store::test_support::transaction_txid(100),
                expiry_height: 100,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: Some(u64::MAX),
            }),
        )
        .unwrap();
    store
        .request_broadcast(&job.operation_id, now().unwrap(), 1, false)
        .unwrap();
    store
        .reconcile_funding_phase(&job.id, &job.operation_id)
        .unwrap();
}

async fn exercise(paused: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let small = crate::rotation::test_capacity::small_reserve("4");
    let mut demanded = String::new();
    for name in ["bootstrap", "ordinary", "dormant", "demanded", "existing"] {
        let reserves = if name == "demanded" {
            crate::rotation::test_capacity::two_reserves("4")
        } else {
            small.clone()
        };
        let pool = state
            .ensure_capacity_group(
                name,
                &reserves,
                Some(alloy_primitives::U256::from(1_000_000)),
                0,
            )
            .unwrap();
        if name == "demanded" {
            demanded = pool;
        }
    }
    state.fill_funding_vacancies("existing").unwrap();
    let original = state.funding_jobs().unwrap().remove(0);
    submitted(&mut state, &original);
    assert!(!state.has_bootstrap_demand(&original.pool_id).unwrap());
    if paused {
        state.pause_new_funding();
    } else {
        state.fail_next_inventory_read(
            crate::rotation::store::inventory::Read::BootstrapEligibility,
            rusqlite::ffi::SQLITE_BUSY,
        );
    }
    let (store, actor) = StoreHandle::spawn(state);
    let (source, mut sources) = mpsc::unbounded_channel();
    let worker = FundingWorker::new(
        store.clone(),
        Backend {
            source,
            groups: ["bootstrap", "ordinary", "dormant", "demanded", "existing"]
                .map(str::to_owned)
                .into(),
        },
        5,
    )
    .with_bootstrap_participants(
        ["dormant", "demanded", "existing"]
            .map(str::to_owned)
            .into(),
    );
    let stop = CancellationToken::new();
    let task = tokio::spawn({
        let stop = stop.clone();
        let worker = worker.clone();
        async move { worker.schedule(1000, stop, false).await }
    });
    // Existing financial work runs even while all participants remain dormant.
    local(sources.recv()).await.unwrap();
    published_after(&store, &original.id, 1000).await;
    local(store.call(move |s| {
        assert!(
            !s.inventory_fault_pending(),
            "the eligibility read outage was consumed without stopping accepted work"
        );
        assert!(s.activate_bootstrap_demand(&demanded)?);
        assert_eq!(
            s.funding_jobs()?.len(),
            1,
            "activation must not allocate eagerly"
        );
        Ok(())
    }))
    .await
    .unwrap();
    tokio::time::advance(Duration::from_secs(11)).await;
    let armed_by = tokio::time::Instant::now();
    // Source polling uses at most its five-second poll plus five-second jitter.
    // This subsequent callback follows another opportunity reconciliation, so
    // participant demand has an armed due time no later than this instant.
    local(async { while sources.recv().await.unwrap() < armed_by {} }).await;
    published_after(&store, &original.id, 1011).await;
    // An iteration can capture t=0 before awaiting Store work and only dispatch
    // its due source at t=11. One further source round forces a new iteration
    // captured after the eligibility retry's t=5 due time.
    tokio::time::advance(Duration::from_secs(11)).await;
    let retried_by = tokio::time::Instant::now();
    local(async { while sources.recv().await.unwrap() < retried_by {} }).await;
    published_after(&store, &original.id, 1022).await;
    tokio::time::advance(Duration::from_secs(37)).await;
    local(store.call(|s| {
        assert!(
            s.funding_jobs()?
                .iter()
                .all(|job| !matches!(job.pool_name.as_str(), "dormant" | "demanded"))
        );
        Ok(())
    }))
    .await
    .unwrap();
    // Independent ordinary groups have opportunities within 30–120 seconds
    // after the failed eligibility read is retried by t=22.
    tokio::time::advance(Duration::from_secs(84)).await;
    if !paused {
        local(async {
            loop {
                if store
                    .call(|s| {
                        let jobs = s.funding_jobs()?;
                        Ok(["bootstrap", "ordinary"]
                            .iter()
                            .all(|name| jobs.iter().any(|job| job.pool_name == *name)))
                    })
                    .await
                    .unwrap()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    tokio::time::advance(Duration::from_secs(120)).await;
    if !paused {
        local(async {
            loop {
                if store
                    .call(|s| {
                        Ok(s.funding_jobs()?
                            .iter()
                            .filter(|job| job.pool_name == "demanded")
                            .count()
                            == 2)
                    })
                    .await
                    .unwrap()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    stop.cancel();
    local(task).await.unwrap().unwrap();
    while sources.try_recv().is_ok() {}
    // Reacquire the retained owner at a later observation epoch without granting
    // any new allocation selection. Existing source recovery remains independent
    // of bootstrap demand, dormant groups and the ordinary auto-funding pause.
    worker.backend.lock().await.groups.clear();
    let resumed_stop = CancellationToken::new();
    let resumed = tokio::spawn({
        let stop = resumed_stop.clone();
        let worker = worker.clone();
        async move { worker.schedule(2000, stop, false).await }
    });
    local(sources.recv())
        .await
        .expect("the original source must remain observable after owner replacement");
    resumed_stop.cancel();
    local(resumed).await.unwrap().unwrap();
    let operation = original.operation_id.clone();
    local(store.call(move |s| {
        let jobs = s.funding_jobs()?;
        assert!(jobs.iter().all(|job| job.pool_name != "dormant"));
        assert_eq!(jobs.len(), if paused { 1 } else { 5 });
        assert_eq!(
            jobs.iter()
                .find(|job| job.pool_name == "existing")
                .unwrap()
                .operation_id,
            operation
        );
        assert!(s.operation_pending(&operation)?);
        Ok(())
    }))
    .await
    .unwrap();
    drop(worker);
    drop(store);
    local(actor).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn credit_of_existing_undemanded_work_arms_ordinary_refill() {
    use crate::rotation::store::AllocationKind;
    use alloy_primitives::U256;
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = state
        .ensure_capacity_group(
            "existing",
            &crate::rotation::test_capacity::two_reserves("4"),
            None,
            0,
        )
        .unwrap();
    let job = state
        .allocate_capacity_kind(&pool, AllocationKind::Small, true)
        .unwrap()
        .unwrap();
    let job = state.funding_job(&job).unwrap().unwrap();
    submitted(&mut state, &job);
    let (store, actor) = StoreHandle::spawn(state);
    let (source, mut sources) = mpsc::unbounded_channel();
    let worker = FundingWorker::new(
        store.clone(),
        Backend {
            source,
            groups: vec!["existing".into()],
        },
        5,
    )
    .with_bootstrap_participants(["existing".into()].into());
    let stop = CancellationToken::new();
    let task = tokio::spawn({
        let stop = stop.clone();
        async move { worker.schedule(1000, stop, false).await }
    });
    local(sources.recv()).await.unwrap();
    published_after(&store, &job.id, 1000).await;
    let original = job.clone();
    local(store.call(move |s| {
        assert_eq!(s.funding_jobs()?.len(), 1);
        assert!(!s.has_bootstrap_demand(&pool)?);
        s.finish_balance(&pool, &original.wallet_id, Some(U256::ONE))
    }))
    .await
    .unwrap();

    // Let the owner observe credit while the original accepted operation is
    // still pending, then cross the entire ordinary opportunity window.
    let mut epoch = 1000;
    for seconds in [11, 11, 120, 11] {
        tokio::time::advance(Duration::from_secs(seconds)).await;
        epoch += seconds;
        let due = tokio::time::Instant::now();
        local(async { while sources.recv().await.unwrap() < due {} }).await;
        published_after(&store, &job.id, epoch).await;
    }
    local(store.call(move |s| {
        assert!(!s.has_bootstrap_demand(&job.pool_id)?);
        assert!(s.operation_pending(&job.operation_id)?);
        let jobs = s.funding_jobs()?;
        assert_eq!(
            jobs.len(),
            2,
            "existing credit must arm the other reserve without API demand"
        );
        let refill = jobs.iter().find(|refill| refill.id != job.id).unwrap();
        assert_eq!(s.funding_wallet_type(&refill.id)?, "large");
        Ok(())
    }))
    .await
    .unwrap();
    stop.cancel();
    local(task).await.unwrap().unwrap();
    drop(store);
    local(actor).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn bootstrap_demand_delays_both_slots_without_stalling_ordinary_or_owned_work() {
    exercise(false).await;
}

#[tokio::test(start_paused = true)]
async fn paused_bootstrap_demand_never_allocates_and_keeps_original_source_observation() {
    exercise(true).await;
}
