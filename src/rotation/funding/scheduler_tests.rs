//! Exercise the retained worker against actual durable funding operations.
use super::*;
use crate::rotation::{store::Store, transaction::TransactionFacts};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Semaphore, mpsc};

#[derive(Clone)]
struct Reads {
    credits: Arc<Semaphore>,
    credit_calls: Arc<AtomicUsize>,
    statuses: mpsc::UnboundedSender<String>,
    status_gates: Arc<std::collections::BTreeMap<String, Arc<Semaphore>>>,
    failed_job: Arc<std::sync::Mutex<Option<String>>>,
    processing: bool,
}
impl FundingObservations for Reads {
    type QuoteRequest = Quote;
    async fn quote(&self, quote: Quote) -> Result<Quote> {
        Ok(quote)
    }
    async fn credit(&self, _: FundingJob) -> Result<()> {
        self.credit_calls.fetch_add(1, Ordering::SeqCst);
        self.credits.acquire().await.unwrap().forget();
        Ok(())
    }
    async fn status(&self, quote: Quote) -> Result<SwapStatus> {
        let id = quote.request["fixture_job"].as_str().unwrap().to_owned();
        if let Some(gate) = self.status_gates.get(&id) {
            gate.acquire().await.unwrap().forget();
        }
        self.statuses.send(id.clone()).unwrap();
        anyhow::ensure!(
            self.failed_job.lock().unwrap().as_ref() != Some(&id),
            "fixture status unavailable"
        );
        Ok(if self.processing {
            SwapStatus::Processing
        } else {
            SwapStatus::Success
        })
    }
}
struct Source {
    store: StoreHandle,
    reads: Reads,
    gate: Arc<Semaphore>,
    started: mpsc::UnboundedSender<String>,
}
impl FundingBackend for Source {
    type Observations = Reads;
    fn observations(&self) -> Reads {
        self.reads.clone()
    }
    fn max_attempts(&self, _: &FundingJob) -> u32 {
        3
    }
    async fn quote_prerequisite(&mut self, job: &FundingJob) -> Result<Quote> {
        self.started.send(job.id.clone()).unwrap();
        self.gate.acquire().await.unwrap().forget();
        let (id, operation, target) =
            (job.id.clone(), job.operation_id.clone(), job.target.clone());
        self.store
            .call(move |s| s.select_quote_terms(&id, &operation, &target, &target))
            .await?;
        Ok(quote(job))
    }
    async fn prepare(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        anyhow::bail!("fixture forbids preparation")
    }
    async fn submit(&mut self, _: &FundingJob) -> Result<()> {
        panic!("fixture forbids new submission")
    }
    async fn reconcile(&mut self, job: &FundingJob) -> Result<bool> {
        self.started.send(job.id.clone()).unwrap();
        self.gate.acquire().await.unwrap().forget();
        Ok(false)
    }
}
fn quote(job: &FundingJob) -> Quote {
    Quote {
        request: serde_json::json!({"amount":job.target,"recipient":job.recipient,"fixture_job":job.id}),
        response: serde_json::json!({}),
        input: 50,
        deadline: u64::MAX,
        deposit: Some("fixture-deposit".into()),
    }
}

#[test]
fn saturated_retry_records_preserve_active_work_and_unelapsed_backoff() {
    let job = FundingJob {
        id: "job".into(),
        pool_id: "pool".into(),
        pool_name: "pool".into(),
        wallet_id: "wallet".into(),
        recipient: "recipient".into(),
        target: "1".into(),
        phase: FundingPhase::Swapping,
        operation_id: "operation".into(),
        attempts: 1,
        started_at: None,
        error_streak: 0,
        timed_out: false,
        next_poll: 0,
        last_error: None,
    };
    for kind in [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote] {
        let mut state = State::default();
        let mut active = BTreeSet::new();
        let mut jobs = Vec::new();
        for n in 0..RECORDS {
            let mut job = job.clone();
            job.operation_id = format!("operation-{n:03}");
            let key = Key {
                operation: job.operation_id.clone(),
                kind,
            };
            assert_eq!(state.admit(&key, &job, 100, &active), Some(true));
            if n == 0 {
                active.insert(key);
            } else {
                state.complete(&job, kind, 100, 10, Some("fixture outage".into()));
            }
            jobs.push(job);
        }
        let next = Key {
            operation: "next-operation".into(),
            kind,
        };
        assert_eq!(state.admit(&next, &job, 100, &active), None);
        let due = state
            .records
            .values()
            .map(|record| record.due)
            .max()
            .unwrap();
        let protected = Key {
            operation: jobs[0].operation_id.clone(),
            kind,
        };
        let retry = Key {
            operation: jobs[1].operation_id.clone(),
            kind,
        };
        assert_eq!(state.admit(&retry, &jobs[1], due - 1, &active), Some(false));
        assert_eq!(state.admit(&next, &job, due - 1, &active), None);
        assert_eq!(state.admit(&next, &job, due, &active), Some(true));
        assert!(
            state.records.contains_key(&protected),
            "active ownership cannot be evicted"
        );
        assert_eq!(state.records.len(), RECORDS);
        // Completion still finds the active record after eviction pressure.
        state.complete(&jobs[0], kind, due, 10, None);
        active.remove(&protected);
        assert_eq!(state.admit(&protected, &jobs[0], due, &active), Some(false));
        // A new operation on the same job has independent retry metadata.
        let mut replacement = jobs[0].clone();
        replacement.operation_id = "replacement".into();
        let replacement_key = Key {
            operation: replacement.operation_id.clone(),
            kind,
        };
        assert_eq!(
            state.admit(&replacement_key, &replacement, due, &active),
            Some(true)
        );
        assert!(
            state.records.contains_key(&protected),
            "replacement cannot reset old backoff"
        );
    }
}

#[tokio::test]
async fn late_quote_completion_cannot_mutate_a_replacement_operation() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_and_allocate(
        &mut state,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let old = state.funding_jobs().unwrap().remove(0);
    state
        .close_unprepared_funding_for_operation(&old.id, &old.operation_id)
        .unwrap();
    state
        .reauthorize_closed_funding_for_operation(&old.id, &old.operation_id, 3)
        .unwrap();
    let replacement = state.funding_job(&old.id).unwrap().unwrap();
    assert_ne!(old.operation_id, replacement.operation_id);
    let (store, actor) = StoreHandle::spawn(state);
    let (statuses, _status_events) = mpsc::unbounded_channel();
    let (started, _source_events) = mpsc::unbounded_channel();
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: Arc::new(Semaphore::new(0)),
                credit_calls: Default::default(),
                statuses,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: false,
            },
            gate: Arc::new(Semaphore::new(0)),
            started,
        },
        1,
    );
    let lease = Arc::new(store.funding_owner().lock_owned().await);
    assert!(
        worker
            .publish_observation(
                &lease,
                Completion {
                    result: Ok(Observation::Quote(quote(&old))),
                    job: old.clone(),
                    kind: Kind::Quote,
                },
                1000
            )
            .await
            .unwrap()
            .is_none()
    );
    store
        .call(move |s| {
            let current = s.funding_job(&old.id)?.unwrap();
            assert_eq!(current.operation_id, replacement.operation_id);
            assert_eq!(current.phase, FundingPhase::Allocated);
            assert_eq!(current.next_poll, 0);
            let facts = s.funding_facts(&current.id, &current.operation_id)?;
            assert!(!facts.accepted_quote);
            assert_eq!(facts.source_attempts, 0);
            assert_eq!(s.funding_recovery_count(&current.id)?, 1);
            Ok(())
        })
        .await
        .unwrap();
    assert!(lease.state.lock().unwrap().records.is_empty());
    drop(lease);
    drop(worker);
    drop(store);
    actor.await.unwrap();
}
fn submitted(store: &mut Store, job: &FundingJob) {
    store
        .save_funding_quote(&job.id, &serde_json::to_vec(&quote(job)).unwrap())
        .unwrap();
    store
        .begin_funding_preparation(
            &job.operation_id,
            Some(&crate::rotation::transaction::ManagedFundingAuthority {
                job_id: job.id.clone(),
                auto_fund: true,
                max_attempts: 3,
            }),
        )
        .unwrap();
    store
        .reserve(&job.operation_id, Some(&job.pool_id), 1, 100)
        .unwrap();
    let revision = store.snapshot().unwrap().0;
    store
        .prepare_with_facts(
            &job.operation_id,
            revision,
            b"next",
            b"signed",
            Some(TransactionFacts {
                txid: job.id.clone(),
                expiry_height: 100,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: u64::MAX,
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
async fn wait_until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn saturated_inventory_survives_blocked_reads_and_cancelled_waiter() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    for i in 0..(RECORDS / 2 + 2) {
        crate::rotation::store::test_support::declare_and_allocate(
            &mut store,
            &format!("pool{i}"),
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
    }
    let mut jobs = store.funding_jobs().unwrap();
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    assert!(jobs.len() > RECORDS);
    let target = jobs.last().unwrap().clone();
    submitted(&mut store, &target);
    let store_id = store.id().to_owned();
    let (store, actor) = StoreHandle::spawn(store);
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();
    let (source_tx, mut source_rx) = mpsc::unbounded_channel();
    let credits = Arc::new(Semaphore::new(0));
    let source_gate = Arc::new(Semaphore::new(0));
    let credit_calls = Arc::new(AtomicUsize::new(0));
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: credits.clone(),
                credit_calls: credit_calls.clone(),
                statuses: status_tx,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: false,
            },
            gate: source_gate.clone(),
            started: source_tx,
        },
        1,
    );
    let waiter = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(1000).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), source_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let observed = tokio::time::timeout(std::time::Duration::from_secs(5), status_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observed, target.id);
    wait_until(|| credit_calls.load(Ordering::SeqCst) == 4).await;
    let id = target.id.clone();
    let operation = target.operation_id.clone();
    // Wait for the status callback to commit while every credit lane remains parked.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (id, operation) = (id.clone(), operation.clone());
            if store
                .call(move |s| Ok(s.funding_facts(&id, &operation)?.provider_success))
                .await
                .unwrap()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    waiter.abort();
    let _ = waiter.await;
    let error = worker.tick(1001).await.unwrap_err();
    assert!(error.is::<AlreadyOwned>());
    assert_eq!(credit_calls.load(Ordering::SeqCst), 4);
    source_gate.add_permits(jobs.len());
    credits.add_permits(jobs.len());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(lease) = store.funding_owner().try_lock_owned() {
                drop(lease);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(credit_calls.load(Ordering::SeqCst), jobs.len());
    let mut source_steps = 1; // The first accepted source step was observed above.
    while source_rx.try_recv().is_ok() {
        source_steps += 1;
    }
    assert_eq!(
        source_steps,
        jobs.len(),
        "a finite pass must not cap total serialized source progress"
    );
    let id = target.id.clone();
    let operation = target.operation_id.clone();
    let facts = store
        .call(move |s| s.funding_facts(&id, &operation))
        .await
        .unwrap();
    assert!(facts.provider_success);
    assert_eq!(facts.source_attempts, 1);
    assert!(!facts.readiness_recorded);
    drop(worker);
    drop(store);
    actor.await.unwrap();
    let reopened = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &store_id,
    )
    .unwrap();
    let facts = reopened
        .funding_facts(&target.id, &target.operation_id)
        .unwrap();
    assert!(facts.provider_success);
    assert_eq!(facts.source_attempts, 1);
    assert!(!facts.status_needed());
    let (store, actor) = StoreHandle::spawn(reopened);
    let (statuses, mut observed_status) = mpsc::unbounded_channel();
    let (started, _source_events) = mpsc::unbounded_channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let reopened_worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: Arc::new(Semaphore::new(jobs.len())),
                credit_calls: calls.clone(),
                statuses,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: false,
            },
            gate: Arc::new(Semaphore::new(jobs.len())),
            started,
        },
        1,
    );
    reopened_worker.tick(1001).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), jobs.len());
    assert!(observed_status.try_recv().is_err());
    let facts = store
        .call(move |s| s.funding_facts(&target.id, &target.operation_id))
        .await
        .unwrap();
    assert!(facts.provider_success);
    assert_eq!(facts.source_attempts, 1);
    drop(reopened_worker);
    drop(store);
    actor.await.unwrap();
}

#[tokio::test]
async fn reverse_status_completion_and_operation_backoff_survive_independent_source_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_and_allocate(
        &mut store,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let mut jobs = store.funding_jobs().unwrap();
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    submitted(&mut store, &jobs[0]);
    // Source ownership remains singleton: this first source is canonically
    // confirmed before another operation receives prepared bytes.
    store.confirm_spend(&jobs[0].operation_id, 60, 1).unwrap();
    submitted(&mut store, &jobs[1]);
    let (store, actor) = StoreHandle::spawn(store);
    let gates = Arc::new(
        jobs.iter()
            .map(|job| (job.id.clone(), Arc::new(Semaphore::new(0))))
            .collect::<std::collections::BTreeMap<_, _>>(),
    );
    let fail = Arc::new(std::sync::Mutex::new(Some(jobs[0].id.clone())));
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();
    let (source_tx, mut source_rx) = mpsc::unbounded_channel();
    let source_gate = Arc::new(Semaphore::new(0));
    let credits = Arc::new(AtomicUsize::new(0));
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: Arc::new(Semaphore::new(100)),
                credit_calls: credits.clone(),
                statuses: status_tx,
                status_gates: gates.clone(),
                failed_job: fail.clone(),
                processing: false,
            },
            gate: source_gate.clone(),
            started: source_tx,
        },
        1,
    );
    let task = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(1000).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), source_rx.recv())
        .await
        .unwrap()
        .unwrap();
    gates[&jobs[1].id].add_permits(1);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), status_rx.recv())
            .await
            .unwrap()
            .unwrap(),
        jobs[1].id
    );
    let (first, second) = (jobs[0].clone(), jobs[1].clone());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (first, second) = (first.clone(), second.clone());
            let (a, b) = store
                .call(move |s| {
                    Ok((
                        s.funding_facts(&first.id, &first.operation_id)?
                            .provider_success,
                        s.funding_facts(&second.id, &second.operation_id)?
                            .provider_success,
                    ))
                })
                .await
                .unwrap();
            assert!(!a);
            if b {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    gates[&jobs[0].id].add_permits(1);
    assert_eq!(status_rx.recv().await.unwrap(), jobs[0].id);
    source_gate.add_permits(100);
    task.await.unwrap().unwrap();
    // The failure backs off only its status observation. It neither retries at
    // the immediate tick nor prevents other source/credit work from advancing.
    worker.tick(1001).await.unwrap();
    assert!(status_rx.try_recv().is_err());
    *fail.lock().unwrap() = None;
    gates[&jobs[0].id].add_permits(1);
    worker.tick(2000).await.unwrap();
    assert_eq!(status_rx.try_recv().unwrap(), jobs[0].id);
    assert!(status_rx.try_recv().is_err());
    for job in jobs {
        let facts = store
            .call(move |s| s.funding_facts(&job.id, &job.operation_id))
            .await
            .unwrap();
        assert!(facts.provider_success);
        assert_eq!(facts.source_attempts, 1);
        assert!(!facts.readiness_recorded);
    }
    assert!(credits.load(Ordering::SeqCst) >= 4);
    drop(worker);
    drop(store);
    actor.await.unwrap();
}

#[tokio::test]
async fn continuous_status_cadence_keeps_rolling_while_other_lanes_stay_owned() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_and_allocate(
        &mut store,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let mut jobs = store.funding_jobs().unwrap();
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    submitted(&mut store, &jobs[1]);
    let (store, actor) = StoreHandle::spawn(store);
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();
    let (source_tx, mut source_rx) = mpsc::unbounded_channel();
    let source_gate = Arc::new(Semaphore::new(0));
    let credit_calls = Arc::new(AtomicUsize::new(0));
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: Arc::new(Semaphore::new(0)),
                credit_calls: credit_calls.clone(),
                statuses: status_tx,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: true,
            },
            gate: source_gate.clone(),
            started: source_tx,
        },
        1,
    );
    let stop = CancellationToken::new();
    let task = tokio::spawn(worker.run(stop.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(5), source_rx.recv())
        .await
        .unwrap()
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(8), status_rx.recv())
                .await
                .unwrap()
                .unwrap(),
            jobs[1].id
        );
    }
    assert_eq!(credit_calls.load(Ordering::SeqCst), 2);
    stop.cancel();
    source_gate.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let job = jobs[1].clone();
    let facts = store
        .call(move |s| s.funding_facts(&job.id, &job.operation_id))
        .await
        .unwrap();
    assert_eq!(facts.source_attempts, 1);
    assert!(!facts.provider_success);
    assert!(!facts.readiness_recorded);
    drop(store);
    actor.await.unwrap();
}
