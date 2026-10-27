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
    quotes: Option<mpsc::UnboundedSender<QuoteRead>>,
}
impl FundingObservations for Reads {
    type QuoteRequest = Quote;
    async fn quote(&self, quote: Quote) -> Result<Quote> {
        if let Some(quotes) = &self.quotes {
            let (finish, completed) = tokio::sync::oneshot::channel();
            quotes
                .send(QuoteRead {
                    job: quote.request["fixture_job"].as_str().unwrap().to_owned(),
                    finish,
                })
                .unwrap();
            completed.await.unwrap()?;
        }
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

#[tokio::test(start_paused = true)]
async fn acquisition_due_rounds_outward_from_the_root_clock_anchor() {
    let clock = TickClock {
        epoch: 1000,
        started: tokio::time::Instant::now(),
    };
    tokio::time::advance(std::time::Duration::from_millis(550)).await;
    let due = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    assert_eq!(clock.ceil_epoch(due), 1061);
    assert_eq!(
        clock.ceil_epoch(clock.started + std::time::Duration::from_secs(60)),
        1060
    );
    assert_eq!(clock.ceil_epoch(clock.started), 1000);
    tokio::time::advance(std::time::Duration::from_millis(59_450)).await;
    assert_eq!(clock.now(), 1060);
    assert!(clock.now() < clock.ceil_epoch(due));
}

#[test]
fn typed_conditions_survive_sibling_success_until_their_own_lane_clears() {
    use crate::rotation::error::FundingCondition;
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
    let mut state = State::default();
    let active = BTreeSet::new();
    for kind in [Kind::Source, Kind::Credit] {
        assert_eq!(
            state.admit(
                &Key {
                    operation: job.operation_id.clone(),
                    kind
                },
                &job,
                100,
                &active
            ),
            Some(true)
        );
    }
    state.complete(
        &job,
        Kind::Source,
        100,
        RetrySchedule::Poll(10),
        Some("shortage".into()),
        Some(FundingCondition::InsufficientFunds),
    );
    state.complete(&job, Kind::Credit, 101, RetrySchedule::Poll(10), None, None);
    assert_eq!(
        state.condition(&job.operation_id, [true; 4]),
        Some(FundingCondition::InsufficientFunds)
    );
    let mut applicable = [true; 4];
    applicable[Kind::Source as usize] = false;
    assert_eq!(state.condition(&job.operation_id, applicable), None);
    assert_eq!(state.condition("another-operation", [true; 4]), None);
    state.complete(&job, Kind::Source, 200, RetrySchedule::Poll(10), None, None);
    assert_eq!(state.condition(&job.operation_id, [true; 4]), None);
}

#[test]
fn saturated_retry_records_preserve_active_work_and_unelapsed_backoff() {
    if crate::test_log_capture::run_in_child(
        "rotation::funding::scheduler::tests::saturated_retry_records_preserve_active_work_and_unelapsed_backoff",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
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
                state.complete(
                    &job,
                    kind,
                    100,
                    RetrySchedule::Poll(10),
                    Some("fixture outage".into()),
                    None,
                );
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
        state.complete(&jobs[0], kind, due, RetrySchedule::Poll(10), None, None);
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
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        logs.lines()
            .any(|line| line.contains("DEBUG") && line.contains("due inactive record evicted"))
    );
    assert!(logs.contains("limit_records=256"));
    assert!(!logs.contains("next-operation"));
}

#[tokio::test]
async fn occupied_observation_lanes_report_once_per_pass_and_preserve_pending_work() {
    if crate::test_log_capture::run_in_child(
        "rotation::funding::scheduler::tests::occupied_observation_lanes_report_once_per_pass_and_preserve_pending_work",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let mut raw = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
        &mut raw,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = raw.funding_jobs().unwrap().remove(0);
    let (store, actor) = StoreHandle::spawn(raw);
    let (statuses, _status_events) = mpsc::unbounded_channel();
    let (started, _source_events) = mpsc::unbounded_channel();
    let reads = Reads {
        credits: Arc::new(Semaphore::new(0)),
        credit_calls: Arc::new(AtomicUsize::new(0)),
        statuses,
        status_gates: Default::default(),
        failed_job: Default::default(),
        processing: false,
        quotes: None,
    };
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: reads.clone(),
            gate: Arc::new(Semaphore::new(0)),
            started,
        },
        60,
    );
    let lease = Arc::new(store.funding_owner().lock_owned().await);
    let clock = TickClock {
        epoch: 1000,
        started: tokio::time::Instant::now(),
    };
    let mut flight = InFlight::new();
    // Joining an existing quote is normal coalescing, even though the same
    // prerequisite remains pending until that observation finishes.
    let quote_key = Key {
        operation: job.operation_id.clone(),
        kind: Kind::Quote,
    };
    flight.active.insert(quote_key.clone());
    let mut duplicate = VecDeque::from([Pending {
        job: job.clone(),
        lane: Kind::Quote,
    }]);
    worker.dispatch_pending(&mut duplicate, &mut flight, &reads, &lease, &clock);
    assert_eq!(duplicate.len(), 1);
    assert!(std::fs::read_to_string(log.path()).unwrap().is_empty());
    flight.active.remove(&quote_key);
    flight.credit_active = CREDIT_READS;
    flight.status_active = STATUS_READS;
    flight.quote_active = QUOTE_READS;
    flight.source_active = true;
    let mut pending: VecDeque<_> = [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote]
        .into_iter()
        .map(|lane| Pending {
            job: job.clone(),
            lane,
        })
        .collect();
    worker.dispatch_pending(&mut pending, &mut flight, &reads, &lease, &clock);
    assert_eq!(pending.len(), 4);
    assert!(flight.reads.is_empty());
    assert_eq!(reads.credit_calls.load(Ordering::SeqCst), 0);
    let logs = std::fs::read_to_string(log.path()).unwrap();
    let diagnostics: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("Funding observation capacity reached"))
        .collect();
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains("DEBUG"));
    for field in [
        "credit_deferred=true",
        "status_deferred=true",
        "source_deferred=true",
        "quote_deferred=true",
    ] {
        assert!(diagnostics[0].contains(field));
    }
    assert!(!logs.contains(&job.operation_id));
    flight.credit_active = 0;
    pending.clear();
    for index in 0..RECORDS {
        let mut existing = job.clone();
        existing.operation_id = format!("retained-{index}");
        let key = Key {
            operation: existing.operation_id.clone(),
            kind: Kind::Credit,
        };
        assert_eq!(
            lease
                .state
                .lock()
                .unwrap()
                .admit(&key, &existing, clock.now(), &flight.active),
            Some(true)
        );
        flight.active.insert(key);
    }
    pending.push_back(Pending {
        job: job.clone(),
        lane: Kind::Credit,
    });
    worker.dispatch_pending(&mut pending, &mut flight, &reads, &lease, &clock);
    assert_eq!(pending.len(), 1);
    assert!(flight.reads.is_empty());
    assert_eq!(reads.credit_calls.load(Ordering::SeqCst), 0);
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(logs.lines().any(|line| line.contains("DEBUG")
        && line.contains("metadata_deferred=true")
        && line.contains("metadata_capacity_per_kind=256")));
    assert!(!logs.contains(&job.operation_id));
    drop(lease);
    drop(worker);
    drop(store);
    actor.await.unwrap();
}

#[tokio::test]
async fn obsolete_completions_cannot_mutate_a_replacement_operation() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
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
                quotes: None,
            },
            gate: Arc::new(Semaphore::new(0)),
            started,
        },
        1,
    );
    let lease = Arc::new(store.funding_owner().lock_owned().await);
    for (kind, result) in [
        (Kind::Quote, Ok(Observation::Quote(quote(&old)))),
        (Kind::Source, Ok(Observation::Prerequisite(quote(&old)))),
        (Kind::Status, Ok(Observation::Status(SwapStatus::Success))),
        (Kind::Credit, Ok(Observation::None)),
        (
            Kind::Source,
            Err(anyhow::anyhow!("obsolete source failure")),
        ),
    ] {
        assert!(
            worker
                .publish_observation(
                    &lease,
                    Completion {
                        result,
                        job: old.clone(),
                        kind,
                    },
                    &TickClock {
                        epoch: 1000,
                        started: tokio::time::Instant::now(),
                    }
                )
                .await
                .unwrap()
                .is_none()
        );
    }
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

#[tokio::test]
async fn operation_rollover_after_inventory_capture_keeps_scheduler_and_siblings_running() {
    for lane in [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote] {
        for refresh in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut state = Store::create(
                &dir.path().join("state"),
                &dir.path().join("key"),
                1,
                b"fixture",
            )
            .unwrap();
            crate::rotation::store::test_support::declare_two_reserves(
                &mut state,
                "pool",
                "5",
                Some(alloy_primitives::U256::from(1_000_000)),
                0,
            )
            .unwrap();
            let jobs = state.funding_jobs().unwrap();
            let old = jobs[0].clone();
            let sibling = jobs[1].clone();
            state
                .save_funding_quote(&old.id, &serde_json::to_vec(&quote(&old)).unwrap())
                .unwrap();
            submitted(&mut state, &sibling);
            let treasury = state.id().to_owned();
            let (store, actor) = StoreHandle::spawn(state);
            let (statuses, mut status_events) = mpsc::unbounded_channel();
            let (started, _source_events) = mpsc::unbounded_channel();
            let credits = Arc::new(AtomicUsize::new(0));
            let mut worker = FundingWorker::new(
                store.clone(),
                Source {
                    store: store.clone(),
                    reads: Reads {
                        credits: Arc::new(Semaphore::new(64)),
                        credit_calls: credits.clone(),
                        statuses,
                        status_gates: Default::default(),
                        failed_job: Default::default(),
                        processing: false,
                        quotes: None,
                    },
                    gate: Arc::new(Semaphore::new(64)),
                    started,
                },
                1,
            );
            let (captured, captured_rx) = tokio::sync::oneshot::channel();
            let (resume, resume_rx) = tokio::sync::oneshot::channel();
            worker.inventory_test_hook = Some(Arc::new(InventoryTestHook {
                lane,
                barrier: Mutex::new(Some((captured, resume_rx))),
            }));
            let running = tokio::spawn({
                let worker = worker.clone();
                async move { worker.tick(1000).await }
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), captured_rx)
                .await
                .unwrap()
                .unwrap();
            // The same store owner commits the transitions used by source
            // recovery/refresh while the scheduler owns an older inventory page.
            let target = old.clone();
            let replacement = store
                .call(move |s| {
                    if refresh {
                        s.refresh_unprepared_quote_for_operation(&target.id, &target.operation_id)?;
                    } else {
                        s.close_unprepared_funding_for_operation(&target.id, &target.operation_id)?;
                        s.reauthorize_closed_funding_for_operation(
                            &target.id,
                            &target.operation_id,
                            3,
                        )?;
                    }
                    Ok(s.funding_job(&target.id)?.unwrap().operation_id)
                })
                .await
                .unwrap();
            assert_ne!(replacement, old.operation_id);
            resume.send(()).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), running)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(status_events.try_recv().unwrap(), sibling.id);
            // Only the accepted sibling can receive credit; neither the old
            // unprepared quote nor its replacement has disclosed source work.
            assert_eq!(credits.load(Ordering::SeqCst), 1);
            // Reuse the actual owner after the raced pass. Any new work must
            // bind to the replacement; captured stale work cannot prepare/send.
            worker.tick(2000).await.unwrap();
            assert_eq!(credits.load(Ordering::SeqCst), 2);
            let old_check = old.clone();
            let sibling_check = sibling.clone();
            let expected = replacement.clone();
            store
                .call(move |s| {
                    let current = s.funding_job(&old_check.id)?.unwrap();
                    assert_eq!(current.operation_id, expected);
                    assert_eq!(current.phase, FundingPhase::Quoted);
                    let facts = s.funding_facts(&current.id, &current.operation_id)?;
                    assert!(facts.accepted_quote);
                    assert!(!facts.provider_success && !facts.initial_credit_recorded);
                    assert!(facts.source_submission.is_none());
                    assert_eq!(facts.source_attempts, 0);
                    assert_eq!(s.funding_recovery_count(&current.id)?, u64::from(!refresh));
                    assert!(
                        s.funding_facts(&sibling_check.id, &sibling_check.operation_id)?
                            .provider_success
                    );
                    let operations = s.status()?.treasury_operations;
                    assert_eq!(operations.len(), 1);
                    assert_eq!(operations[0].operation_id, sibling_check.operation_id);
                    assert_eq!(operations[0].attempts, 1);
                    Ok(())
                })
                .await
                .unwrap();
            drop(worker);
            drop(store);
            actor.await.unwrap();
            let reopened = Store::open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &treasury,
            )
            .unwrap();
            assert_eq!(
                reopened.funding_job(&old.id).unwrap().unwrap().operation_id,
                replacement
            );
            assert_eq!(
                reopened.funding_recovery_count(&old.id).unwrap(),
                u64::from(!refresh)
            );
            assert_eq!(reopened.status().unwrap().treasury_operations.len(), 1);
        }
    }
}
fn submitted(store: &mut Store, job: &FundingJob) {
    store
        .save_funding_quote(&job.id, &serde_json::to_vec(&quote(job)).unwrap())
        .unwrap();
    store
        .require_preparation_authority(
            &job.operation_id,
            Some(&crate::rotation::transaction::ManagedFundingAuthority {
                job_id: job.id.clone(),
                auto_fund: true,
                max_attempts: 3,
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
            b"signed",
            Some(TransactionFacts {
                txid: job.id.clone(),
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

#[tokio::test]
async fn accepted_source_is_observed_despite_complete_phase_disabled_pool_and_retired_wallet() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let key = dir.path().join("key");
    let mut state = Store::create(&path, &key, 1, b"fixture").unwrap();
    crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "stale",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(alloy_primitives::U256::from(100)),
        0
    )
    .unwrap();
    let job = state.funding_jobs().unwrap().remove(0);
    submitted(&mut state, &job);
    state.disable_pool("stale").unwrap();
    let db = rusqlite::Connection::open(path.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE funding_jobs SET state='COMPLETE' WHERE id=?1",
        [&job.id],
    )
    .unwrap();
    db.execute("UPDATE funding_progress SET phase='\"COMPLETE\"',provider_success=1,provider_refunded=1 WHERE job_id=?1",[&job.id]).unwrap();
    db.execute(
        "UPDATE wallets SET role='RETIRED' WHERE id=?1",
        [&job.wallet_id],
    )
    .unwrap();
    let (store, owner) = StoreHandle::spawn(state);
    let (started, mut observations) = mpsc::unbounded_channel();
    let (statuses, _) = mpsc::unbounded_channel();
    let backend = Source {
        store: store.clone(),
        gate: Arc::new(Semaphore::new(1)),
        started,
        reads: Reads {
            credits: Arc::new(Semaphore::new(100)),
            credit_calls: Arc::default(),
            statuses,
            status_gates: Arc::default(),
            failed_job: Arc::default(),
            processing: true,
            quotes: None,
        },
    };
    let worker = FundingWorker::new(store.clone(), backend, 60);
    worker.tick(now().unwrap()).await.unwrap();
    assert_eq!(observations.try_recv().unwrap(), job.id);
    assert!(observations.try_recv().is_err());
    assert_eq!(
        db.query_row(
            "SELECT attempts FROM treasury_operations WHERE id=?1",
            [&job.operation_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM outgoing WHERE state='PREPARED'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM funding_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(db);
    drop(worker);
    drop(store);
    owner.await.unwrap();
}

#[tokio::test]
async fn completion_storage_failure_is_fatal() {
    {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        crate::rotation::store::test_support::declare_two_reserves(
            &mut state,
            "pool",
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
        let job = state.funding_jobs().unwrap().remove(0);
        state
            .record_destination_readiness(&job.id, &job.target, "fixture", 100)
            .unwrap();
        state
            .close_unprepared_funding_for_operation(&job.id, &job.operation_id)
            .unwrap();
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE OF state ON funding_jobs BEGIN SELECT RAISE(ABORT,'fixture storage failure'); END;").unwrap();
        drop(db);
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
                    quotes: None,
                },
                gate: Arc::new(Semaphore::new(0)),
                started,
            },
            1,
        );
        let lease = Arc::new(store.funding_owner().lock_owned().await);
        {
            let mut state = lease.state.lock().unwrap();
            state.admit(
                &Key {
                    operation: job.operation_id.clone(),
                    kind: Kind::Source,
                },
                &job,
                900,
                &BTreeSet::new(),
            );
            state.complete(
                &job,
                Kind::Source,
                900,
                RetrySchedule::Poll(1),
                Some("funding_quote_failed; fixture outage".into()),
                None,
            );
        }
        for kind in [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote] {
            let key = Key {
                operation: job.operation_id.clone(),
                kind,
            };
            lease
                .state
                .lock()
                .unwrap()
                .admit(&key, &job, 1000, &BTreeSet::new());
            let result = worker
                .publish_observation(
                    &lease,
                    Completion {
                        job: job.clone(),
                        kind,
                        result: Ok(if kind == Kind::Source {
                            Observation::Prerequisite(quote(&job))
                        } else {
                            Observation::None
                        }),
                    },
                    &TickClock {
                        epoch: 1000,
                        started: tokio::time::Instant::now(),
                    },
                )
                .await;
            assert!(result.err().unwrap().is::<rusqlite::Error>());
        }
        drop(lease);
        drop(worker);
        drop(store);
        actor.await.unwrap();
    }
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
        crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            &format!("pool{i}"),
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
    }
    let unsubmitted = store.funding_jobs().unwrap();
    assert!(unsubmitted.len() > RECORDS);
    // Credit inventory must represent accepted work. Keep a separate equally
    // large unprepared inventory to exercise uncapped serialized quote progress.
    for i in 0..(RECORDS / 2 + 2) {
        crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            &format!("accepted{i}"),
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
    }
    let mut jobs: Vec<_> = store
        .funding_jobs()
        .unwrap()
        .into_iter()
        .filter(|job| job.pool_name.starts_with("accepted"))
        .collect();
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    assert!(jobs.len() > RECORDS);
    let target = jobs.last().unwrap().clone();
    for job in &jobs {
        submitted(&mut store, job);
        if job.id != target.id {
            store.confirm_spend(&job.operation_id).unwrap();
            store
                .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 900)
                .unwrap();
        }
    }
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
                quotes: None,
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
    source_gate.add_permits(unsubmitted.len() + 1);
    credits.add_permits(jobs.len());
    // More than RECORDS accepted credits must cross a minute cadence before
    // completed scheduling records can be evicted. This is a fixture allowance
    // for cache rollover, never a deadline for accepted runtime work.
    let rollover_allowance = std::time::Duration::from_secs(OBSERVATION_POLL_SECONDS + 15);
    tokio::time::timeout(rollover_allowance, async {
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
        unsubmitted.len() + 1,
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
    assert!(!facts.initial_credit_recorded);
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
                quotes: None,
            },
            gate: Arc::new(Semaphore::new(jobs.len())),
            started,
        },
        1,
    );
    tokio::time::timeout(rollover_allowance, reopened_worker.tick(1001))
        .await
        .unwrap()
        .unwrap();
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
    crate::rotation::store::test_support::declare_two_reserves(
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
    store.confirm_spend(&jobs[0].operation_id).unwrap();
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
                quotes: None,
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
        assert!(!facts.initial_credit_recorded);
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
    crate::rotation::store::test_support::declare_two_reserves(
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
                quotes: None,
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
    assert_eq!(event(&mut status_rx).await, jobs[1].id);
    // Persisted health is written after State::complete anchors the cooldown.
    // Channel delivery and provider-observed publication happen before that point.
    let focus = jobs[1].id.clone();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let id = focus.clone();
            if store
                .call(move |s| Ok(s.funding_job(&id)?.unwrap().next_poll > 0))
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
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(59)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(
        status_rx.try_recv().is_err(),
        "provider polling has a minute floor"
    );
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    tokio::time::resume();
    assert_eq!(event(&mut status_rx).await, jobs[1].id);
    // The unprepared sibling owns source work but has no credit work yet.
    assert_eq!(credit_calls.load(Ordering::SeqCst), 1);
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
    assert!(!facts.initial_credit_recorded);
    drop(store);
    actor.await.unwrap();
}

/// Each read announces admission and stays pending until the test supplies its
/// result. No elapsed-time assumption controls completion ordering.
#[derive(Debug)]
struct QuoteRead {
    job: String,
    finish: tokio::sync::oneshot::Sender<Result<()>>,
}

async fn event<T>(receiver: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
        .await
        .expect("scheduler event stalled")
        .expect("scheduler event channel closed")
}

#[tokio::test]
async fn quote_saturation_releases_failed_slot_and_single_pass_drains_all_reads() {
    let dir = tempfile::tempdir().unwrap();
    let mut raw = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    for index in 0..3 {
        crate::rotation::store::test_support::declare_two_reserves(
            &mut raw,
            &format!("pool{index}"),
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
    }
    let mut jobs = raw.funding_jobs().unwrap();
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    let submitted_job = jobs.last().unwrap().clone();
    submitted(&mut raw, &submitted_job);
    let (store, actor) = StoreHandle::spawn(raw);
    let (statuses, mut status_events) = mpsc::unbounded_channel();
    let (started, mut source_events) = mpsc::unbounded_channel();
    let (quotes, mut quote_events) = mpsc::unbounded_channel();
    let credit_calls = Arc::new(AtomicUsize::new(0));
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: Arc::new(Semaphore::new(jobs.len())),
                credit_calls: credit_calls.clone(),
                statuses,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: false,
                quotes: Some(quotes),
            },
            gate: Arc::new(Semaphore::new(jobs.len())),
            started,
        },
        60,
    );
    let task = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(1000).await }
    });
    let mut pending = Vec::new();
    for _ in 0..QUOTE_READS {
        pending.push(event(&mut quote_events).await);
    }
    assert_eq!(event(&mut status_events).await, submitted_job.id);
    // Unsigned quote work must not create destination balance polling.
    wait_until(|| credit_calls.load(Ordering::SeqCst) == 1).await;
    assert!(
        !task.is_finished(),
        "once must retain outstanding quote reads"
    );
    assert!(worker.tick(1000).await.unwrap_err().is::<AlreadyOwned>());
    assert!(quote_events.try_recv().is_err());

    // Complete the most recently admitted read first, with an unsigned outage.
    // Its slot must become usable while all earlier reads remain blocked.
    let failed = pending.pop().unwrap();
    let failed_id = failed.job.clone();
    failed
        .finish
        .send(Err(anyhow::anyhow!("fixture quote outage")))
        .unwrap();
    pending.push(event(&mut quote_events).await);
    let mut seen = BTreeSet::from([failed_id.clone()]);
    for read in &pending {
        assert!(seen.insert(read.job.clone()), "duplicate quote admission");
    }
    assert_eq!(seen.len(), jobs.len() - 1);
    assert!(!task.is_finished());
    for read in pending.into_iter().rev() {
        read.finish.send(Ok(())).unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        quote_events.try_recv().is_err(),
        "single pass must not retry failed quotes"
    );
    let mut sources = BTreeSet::new();
    while let Ok(id) = source_events.try_recv() {
        assert!(sources.insert(id), "duplicate source admission");
    }
    assert_eq!(sources.len(), jobs.len());
    for job in jobs {
        let was_submitted = job.id == submitted_job.id;
        let failed = job.id == failed_id;
        store
            .call(move |s| {
                let facts = s.funding_facts(&job.id, &job.operation_id)?;
                assert_eq!(facts.accepted_quote, !failed);
                assert_eq!(facts.source_attempts, u32::from(was_submitted));
                assert_eq!(facts.provider_success, was_submitted);
                if failed {
                    let current = s.funding_job(&job.id)?.unwrap();
                    assert_eq!(current.phase, FundingPhase::Allocated);
                    assert_eq!(current.attempts, 0);
                    assert!(current.error_streak > 0);
                    assert!(current.next_poll > 1000);
                }
                Ok(())
            })
            .await
            .unwrap();
    }
    drop(worker);
    drop(store);
    actor.await.unwrap();
}

#[tokio::test]
async fn cancellation_drains_owned_prerequisite_without_starting_quote_or_next_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut raw = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
        &mut raw,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let jobs = raw.funding_jobs().unwrap();
    let (store, actor) = StoreHandle::spawn(raw);
    let (statuses, mut status_events) = mpsc::unbounded_channel();
    let (started, mut source_events) = mpsc::unbounded_channel();
    let (quotes, mut quote_events) = mpsc::unbounded_channel();
    let source_gate = Arc::new(Semaphore::new(0));
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: Arc::new(Semaphore::new(0)),
                credit_calls: Arc::new(AtomicUsize::new(0)),
                statuses,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: false,
                quotes: Some(quotes),
            },
            gate: source_gate.clone(),
            started,
        },
        60,
    );
    let stop = CancellationToken::new();
    let waiter = tokio::spawn({
        let worker = worker.clone();
        let stop = stop.clone();
        async move { worker.schedule(1000, stop, true).await }
    });
    let accepted = event(&mut source_events).await;
    stop.cancel();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(worker.tick(1000).await.unwrap_err().is::<AlreadyOwned>());
    assert!(quote_events.try_recv().is_err());
    source_gate.add_permits(1);
    // Acquiring the actual owner gate proves the detached scheduler and source
    // have drained. Keep it held while inspecting forbidden follow-up effects.
    let lease = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store.funding_owner().lock_owned(),
    )
    .await
    .unwrap();
    assert!(source_events.try_recv().is_err());
    assert!(quote_events.try_recv().is_err());
    assert!(status_events.try_recv().is_err());
    let accepted_job = jobs.iter().find(|job| job.id == accepted).unwrap();
    {
        let state = lease.state.lock().unwrap();
        let record = state
            .records
            .get(&Key {
                operation: accepted_job.operation_id.clone(),
                kind: Kind::Source,
            })
            .expect("accepted source ownership lost");
        assert!(
            record.due > 1000,
            "accepted source completion was not published"
        );
        assert!(record.error.is_none());
    }
    for job in jobs {
        store
            .call(move |s| {
                let facts = s.funding_facts(&job.id, &job.operation_id)?;
                assert!(!facts.accepted_quote);
                assert_eq!(facts.source_attempts, 0);
                let current = s.funding_job(&job.id)?.unwrap();
                assert_eq!(current.phase, FundingPhase::Allocated);
                assert_eq!(current.attempts, 0);
                Ok(())
            })
            .await
            .unwrap();
    }
    drop(lease);
    drop(worker);
    drop(store);
    actor.await.unwrap();
}

#[tokio::test]
async fn fatal_store_error_retains_active_source_lease_and_signed_liability() {
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state");
    let key_path = dir.path().join("key");
    let mut raw = Store::create(&state_path, &key_path, 1, b"fixture").unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
        &mut raw,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = raw.funding_jobs().unwrap().remove(0);
    submitted(&mut raw, &job);
    let treasury = raw.id().to_owned();
    let (store, actor) = StoreHandle::spawn(raw);
    let (statuses, _status_events) = mpsc::unbounded_channel();
    let (started, mut source_events) = mpsc::unbounded_channel();
    let source_gate = Arc::new(Semaphore::new(0));
    let credit_gate = Arc::new(Semaphore::new(0));
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: credit_gate.clone(),
                credit_calls: Default::default(),
                statuses,
                status_gates: Arc::new(std::collections::BTreeMap::from([(
                    job.id.clone(),
                    Arc::new(Semaphore::new(0)),
                )])),
                failed_job: Default::default(),
                processing: false,
                quotes: None,
            },
            gate: source_gate.clone(),
            started,
        },
        60,
    );
    let task = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(1000).await }
    });
    assert_eq!(event(&mut source_events).await, job.id);
    // Fail completion publication, only after mutable reconciliation owns its lease.
    let db = rusqlite::Connection::open(state_path.join("state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE ON funding_progress BEGIN SELECT RAISE(ABORT,'fixture storage failure'); END;").unwrap();
    credit_gate.add_permits(1);
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.is::<rusqlite::Error>());
    assert!(worker.tick(1001).await.unwrap_err().is::<AlreadyOwned>());
    db.execute_batch("DROP TRIGGER reject_completion").unwrap();
    drop(db);
    source_gate.add_permits(1);
    let lease = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store.funding_owner().lock_owned(),
    )
    .await
    .unwrap();
    assert!(
        source_events.try_recv().is_err(),
        "fatal error cannot admit replacement source work"
    );
    drop(lease);
    drop(worker);
    drop(store);
    actor.await.unwrap();
    let reopened = Store::open(&state_path, &key_path, &treasury).unwrap();
    assert_eq!(
        reopened.funding_job(&job.id).unwrap().unwrap().operation_id,
        job.operation_id
    );
    assert_eq!(
        reopened
            .prepared_bytes(&job.operation_id)
            .unwrap()
            .as_slice(),
        b"signed"
    );
    assert!(reopened.operation_pending(&job.operation_id).unwrap());
    let operation = reopened.operation(&job.operation_id).unwrap();
    assert_eq!(operation.attempts, 1);
    assert_eq!(operation.submission, "BROADCAST_REQUESTED");
    assert_eq!(reopened.funding_recovery_count(&job.id).unwrap(), 0);
}

#[tokio::test]
async fn cancellation_drops_unsigned_quotes_without_accepting_or_retrying_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut raw = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
        &mut raw,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let jobs = raw.funding_jobs().unwrap();
    let (store, actor) = StoreHandle::spawn(raw);
    let (statuses, mut status_events) = mpsc::unbounded_channel();
    let (started, mut source_events) = mpsc::unbounded_channel();
    let (quotes, mut quote_events) = mpsc::unbounded_channel();
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
                quotes: Some(quotes),
            },
            gate: Arc::new(Semaphore::new(jobs.len())),
            started,
        },
        60,
    );
    let stop = CancellationToken::new();
    let task = tokio::spawn({
        let worker = worker.clone();
        let stop = stop.clone();
        async move { worker.schedule(1000, stop, true).await }
    });
    let mut outstanding = Vec::new();
    for _ in &jobs {
        outstanding.push(event(&mut quote_events).await);
    }
    let mut accepted = BTreeSet::new();
    for _ in &jobs {
        assert!(accepted.insert(event(&mut source_events).await));
    }
    stop.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let lease = store
        .funding_owner()
        .try_lock_owned()
        .expect("unsigned observations must not retain owner");
    for read in outstanding {
        assert!(
            read.finish.is_closed(),
            "cancelled quote future remains alive"
        );
    }
    assert!(source_events.try_recv().is_err());
    assert!(quote_events.try_recv().is_err());
    assert!(status_events.try_recv().is_err());
    for job in jobs {
        store
            .call(move |s| {
                let facts = s.funding_facts(&job.id, &job.operation_id)?;
                assert!(!facts.accepted_quote);
                assert_eq!(facts.source_attempts, 0);
                let saved = s.funding_job(&job.id)?.unwrap();
                assert_eq!(saved.phase, FundingPhase::Allocated);
                assert_eq!(saved.attempts, 0);
                assert_eq!(s.funding_recovery_count(&job.id)?, 0);
                Ok(())
            })
            .await
            .unwrap();
    }
    drop(lease);
    drop(worker);
    drop(store);
    actor.await.unwrap();
}

#[tokio::test]
async fn captured_inventory_excludes_new_rows_until_a_continuous_pass() {
    for once in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let mut raw = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        crate::rotation::store::test_support::declare_two_reserves(
            &mut raw,
            "initial",
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
        let initial = raw.funding_jobs().unwrap();
        submitted(&mut raw, &initial[0]);
        raw.confirm_spend(&initial[0].operation_id).unwrap();
        let (store, actor) = StoreHandle::spawn(raw);
        let (statuses, mut status_events) = mpsc::unbounded_channel();
        let (started, mut source_events) = mpsc::unbounded_channel();
        let credits = Arc::new(AtomicUsize::new(0));
        let source_gate = Arc::new(Semaphore::new(if once { initial.len() } else { 0 }));
        let mut worker = FundingWorker::new(
            store.clone(),
            Source {
                store: store.clone(),
                reads: Reads {
                    credits: Arc::new(Semaphore::new(64)),
                    credit_calls: credits.clone(),
                    statuses,
                    status_gates: Default::default(),
                    failed_job: Default::default(),
                    processing: true,
                    quotes: None,
                },
                gate: source_gate.clone(),
                started,
            },
            1,
        );
        let (captured, captured_rx) = tokio::sync::oneshot::channel();
        let (resume, resume_rx) = tokio::sync::oneshot::channel();
        worker.inventory_test_hook = Some(Arc::new(InventoryTestHook {
            lane: Kind::Credit,
            barrier: Mutex::new(Some((captured, resume_rx))),
        }));
        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let worker = worker.clone();
            let stop = stop.clone();
            async move { worker.schedule(1000, stop, once).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), captured_rx)
            .await
            .unwrap()
            .unwrap();
        let added = store
            .call(|s| {
                crate::rotation::store::test_support::declare_two_reserves(
                    s,
                    "added",
                    "5",
                    Some(alloy_primitives::U256::from(1_000_000)),
                    0,
                )?;
                let added: Vec<_> = s
                    .funding_jobs()?
                    .into_iter()
                    .filter(|job| job.pool_name == "added")
                    .collect();
                for job in &added {
                    submitted(s, job);
                    s.confirm_spend(&job.operation_id)?;
                }
                Ok(added)
            })
            .await
            .unwrap();
        resume.send(()).unwrap();
        if once {
            tokio::time::timeout(std::time::Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let mut observed = BTreeSet::new();
            while let Ok(id) = status_events.try_recv() {
                assert!(observed.insert(id));
            }
            assert_eq!(observed, BTreeSet::from([initial[0].id.clone()]));
            assert_eq!(credits.load(Ordering::SeqCst), 1);
            let mut sources = BTreeSet::new();
            while let Ok(id) = source_events.try_recv() {
                assert!(sources.insert(id));
            }
            assert_eq!(sources, BTreeSet::from([initial[1].id.clone()]));
        } else {
            // An occupied mutable lane cannot prevent exhausted observation lanes
            // from capturing the new upper boundary on their next pass.
            let active = event(&mut source_events).await;
            assert!(initial.iter().any(|job| job.id == active));
            let expected: BTreeSet<_> = added.iter().map(|job| job.id.clone()).collect();
            let mut seen = BTreeSet::new();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !expected.is_subset(&seen) {
                    seen.insert(event(&mut status_events).await);
                }
            })
            .await
            .expect("continuous pass failed to include new inventory");
            assert!(source_events.try_recv().is_err());
            stop.cancel();
            source_gate.add_permits(1);
            tokio::time::timeout(std::time::Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(source_events.try_recv().is_err());
        }
        for job in added {
            store
                .call(move |s| {
                    assert_eq!(
                        s.funding_facts(&job.id, &job.operation_id)?.source_attempts,
                        1
                    );
                    assert!(!s.operation_pending(&job.operation_id)?);
                    assert_eq!(s.prepared_bytes(&job.operation_id)?.as_slice(), b"signed");
                    Ok(())
                })
                .await
                .unwrap();
        }
        drop(worker);
        drop(store);
        actor.await.unwrap();
    }
}

#[test]
fn sibling_observations_report_current_phase_once_per_operation() {
    let mut state = State::default();
    let record = || Record {
        reported_phase: None,
        due: 0,
        streak: 0,
        error: None,
        condition: None,
    };
    state.records.insert(
        Key {
            operation: "first".into(),
            kind: Kind::Source,
        },
        record(),
    );
    assert!(state.report_phase("first", &FundingPhase::Quoted));
    // A slower sibling starts/finishes with an old snapshot, but reports the
    // same durable phase; adding its record must not cause a duplicate message.
    state.records.insert(
        Key {
            operation: "first".into(),
            kind: Kind::Credit,
        },
        record(),
    );
    assert!(!state.report_phase("first", &FundingPhase::Quoted));
    assert!(state.report_phase("first", &FundingPhase::Prepared));
    assert!(!state.report_phase("first", &FundingPhase::Prepared));
    state.records.insert(
        Key {
            operation: "replacement".into(),
            kind: Kind::Source,
        },
        record(),
    );
    assert!(state.report_phase("replacement", &FundingPhase::Quoted));
    assert!(!state.report_phase("first", &FundingPhase::Prepared));
    // Real durable recovery changes remain visible; we do not impose a fake
    // monotonic ordering on the funding state machine just to tidy the logs.
    assert!(state.report_phase("first", &FundingPhase::RecoveryRequired));
}

#[tokio::test]
async fn malformed_inventory_and_post_capture_payloads_preserve_sources_and_drain_siblings() {
    for (table, field, value) in [
        ("funding_progress", "phase", "'invalid'"),
        ("funding_progress", "provider_success", "'invalid'"),
        ("funding_health", "error_streak", "'invalid'"),
        ("funding_progress", "next_poll", "-1"),
    ] {
        for after_capture in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let state_path = dir.path().join("state");
            let key_path = dir.path().join("key");
            let mut raw = Store::create(&state_path, &key_path, 1, b"fixture").unwrap();
            for name in ["same_pool", "other_pool"] {
                crate::rotation::store::test_support::declare_two_reserves(
                    &mut raw,
                    name,
                    "5",
                    Some(alloy_primitives::U256::from(1_000_000)),
                    0,
                )
                .unwrap();
            }
            let jobs = raw.funding_jobs().unwrap();
            for (index, job) in jobs.iter().enumerate() {
                submitted(&mut raw, job);
                if index + 1 < jobs.len() {
                    raw.confirm_spend(&job.operation_id).unwrap();
                }
            }
            let bad = jobs[0].clone();
            let treasury = raw.id().to_owned();
            let db = rusqlite::Connection::open(state_path.join("state.sqlite")).unwrap();
            let corrupt = || {
                db.execute(
                    &format!("UPDATE {table} SET {field}={value} WHERE job_id=?1"),
                    [&bad.id],
                )
                .unwrap();
            };
            if !after_capture {
                corrupt();
            }
            let (store, actor) = StoreHandle::spawn(raw);
            let (statuses, mut status_events) = mpsc::unbounded_channel();
            let (started, mut source_events) = mpsc::unbounded_channel();
            let credit_calls = Arc::new(AtomicUsize::new(0));
            let mut worker = FundingWorker::new(
                store.clone(),
                Source {
                    store: store.clone(),
                    reads: Reads {
                        credits: Arc::new(Semaphore::new(64)),
                        credit_calls: credit_calls.clone(),
                        statuses,
                        status_gates: Default::default(),
                        failed_job: Default::default(),
                        processing: true,
                        quotes: None,
                    },
                    gate: Arc::new(Semaphore::new(64)),
                    started,
                },
                1,
            );
            let (captured, captured_rx) = tokio::sync::oneshot::channel();
            let (resume, resume_rx) = tokio::sync::oneshot::channel();
            if after_capture {
                worker.inventory_test_hook = Some(Arc::new(InventoryTestHook {
                    lane: Kind::Credit,
                    barrier: Mutex::new(Some((captured, resume_rx))),
                }));
            }
            let waiter = tokio::spawn({
                let worker = worker.clone();
                async move { worker.tick(1000).await }
            });
            if after_capture {
                captured_rx.await.unwrap();
                corrupt();
                resume.send(()).unwrap();
            }
            let error = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert!(
                error.is::<MalformedFunding>(),
                "{field} after_capture={after_capture}: {error:#}"
            );
            assert_eq!(
                credit_calls.load(Ordering::SeqCst),
                if after_capture { 4 } else { 3 }
            );
            let mut observed = BTreeSet::new();
            while let Ok(id) = source_events.try_recv() {
                assert_ne!(id, bad.id);
                observed.insert(id);
            }
            assert_eq!(observed.len(), 1);
            observed.clear();
            while let Ok(id) = status_events.try_recv() {
                assert_ne!(id, bad.id);
                observed.insert(id);
            }
            assert_eq!(observed.len(), 3);
            drop(db);
            drop(worker);
            drop(store);
            actor.await.unwrap();
            let reopened = Store::open(&state_path, &key_path, &treasury).unwrap();
            for job in &jobs {
                assert_eq!(reopened.funding_recovery_count(&job.id).unwrap(), 0);
                assert_eq!(reopened.operation(&job.operation_id).unwrap().attempts, 1);
                assert_eq!(
                    reopened.operation_pending(&job.operation_id).unwrap(),
                    job.id == jobs.last().unwrap().id
                );
                assert_eq!(
                    reopened
                        .prepared_bytes(&job.operation_id)
                        .unwrap()
                        .as_slice(),
                    b"signed"
                );
            }
            let page = reopened
                .funding_work_page("", reopened.funding_work_boundary().unwrap())
                .unwrap();
            assert_eq!(page.len(), 4);
            assert!(
                page.into_iter()
                    .find(|(id, _)| id == &bad.id)
                    .unwrap()
                    .1
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn transient_funding_inventory_preserves_source_owner_and_lane_progress() {
    use crate::rotation::store::inventory::{Read, transient_read};
    for read in [Read::FundingBoundary, Read::FundingPage] {
        for once in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut raw = Store::create(
                &dir.path().join("state"),
                &dir.path().join("key"),
                1,
                b"fixture",
            )
            .unwrap();
            crate::rotation::store::test_support::declare_two_reserves(
                &mut raw,
                "pool",
                "5",
                Some(alloy_primitives::U256::from(1_000_000)),
                0,
            )
            .unwrap();
            let jobs = raw.funding_jobs().unwrap();
            submitted(&mut raw, &jobs[0]);
            raw.confirm_spend(&jobs[0].operation_id).unwrap();
            submitted(&mut raw, &jobs[1]);
            let treasury = raw.id().to_owned();
            raw.fail_next_inventory_read(read, rusqlite::ffi::SQLITE_LOCKED);
            let (store, actor) = StoreHandle::spawn(raw);
            let (statuses, mut status_events) = mpsc::unbounded_channel();
            let (started, mut source_events) = mpsc::unbounded_channel();
            let credits = Arc::new(AtomicUsize::new(0));
            let gate = Arc::new(Semaphore::new(if once { 1 } else { 0 }));
            let worker = FundingWorker::new(
                store.clone(),
                Source {
                    store: store.clone(),
                    reads: Reads {
                        credits: Arc::new(Semaphore::new(if once { 64 } else { 0 })),
                        credit_calls: credits.clone(),
                        statuses,
                        status_gates: Default::default(),
                        failed_job: Default::default(),
                        processing: true,
                        quotes: None,
                    },
                    gate: gate.clone(),
                    started,
                },
                1,
            );
            let stop = CancellationToken::new();
            let waiter = tokio::spawn({
                let worker = worker.clone();
                let stop = stop.clone();
                async move { worker.schedule(1000, stop, once).await }
            });
            if once {
                let error = tokio::time::timeout(std::time::Duration::from_secs(4), waiter)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert!(transient_read(&error));
                if read == Read::FundingPage {
                    assert_eq!(source_events.try_recv().unwrap(), jobs[1].id);
                    let observed = event(&mut status_events).await;
                    assert!(jobs.iter().any(|job| job.id == observed));
                }
            } else {
                assert_eq!(event(&mut source_events).await, jobs[1].id);
                wait_until(|| credits.load(Ordering::SeqCst) >= 2).await;
                let mut first_statuses = BTreeSet::new();
                while first_statuses.len() < jobs.len() {
                    let observed = event(&mut status_events).await;
                    assert!(jobs.iter().any(|job| job.id == observed));
                    assert!(first_statuses.insert(observed));
                }
                // Health publication follows State::complete; provider outcome
                // publication alone does not establish the cooldown anchor.
                let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let published: i64 = db
                            .query_row(
                                "SELECT count(*) FROM funding_progress WHERE next_poll>0",
                                [],
                                |row| row.get(0),
                            )
                            .unwrap();
                        if published == jobs.len() as i64 {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                drop(db);
                let before = credits.load(Ordering::SeqCst);
                while status_events.try_recv().is_ok() {}
                store
                    .call(|s| {
                        s.fail_next_inventory_read(
                            Read::FundingBoundary,
                            rusqlite::ffi::SQLITE_BUSY,
                        );
                        Ok(())
                    })
                    .await
                    .unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while store
                        .call(|s| Ok(s.inventory_fault_pending()))
                        .await
                        .unwrap()
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                // Inventory recovery does not shorten the provider's minute
                // interval. Its next due read still progresses independently of
                // the held source and credit acquisitions.
                assert!(status_events.try_recv().is_err());
                tokio::time::pause();
                tokio::time::advance(std::time::Duration::from_secs(61)).await;
                tokio::time::resume();
                let observed = event(&mut status_events).await;
                assert!(jobs.iter().any(|job| job.id == observed));
                assert_eq!(credits.load(Ordering::SeqCst), before);
                assert!(worker.tick(1001).await.unwrap_err().is::<AlreadyOwned>());
                assert!(source_events.try_recv().is_err());
                stop.cancel();
                assert!(worker.tick(1002).await.unwrap_err().is::<AlreadyOwned>());
                gate.add_permits(1);
                tokio::time::timeout(std::time::Duration::from_secs(4), waiter)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
            }
            // Permanent store corruption never enters inventory retry.
            store
                .call(|s| {
                    s.fail_next_inventory_read(
                        Read::FundingBoundary,
                        rusqlite::ffi::SQLITE_CORRUPT,
                    );
                    Ok(())
                })
                .await
                .unwrap();
            let error = tokio::time::timeout(std::time::Duration::from_secs(2), worker.tick(1003))
                .await
                .unwrap()
                .unwrap_err();
            assert!(!transient_read(&error));
            drop(worker);
            drop(store);
            actor.await.unwrap();
            let reopened = Store::open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &treasury,
            )
            .unwrap();
            for job in &jobs {
                assert_eq!(reopened.funding_recovery_count(&job.id).unwrap(), 0);
                assert_eq!(reopened.operation(&job.operation_id).unwrap().attempts, 1);
                assert_eq!(
                    reopened
                        .prepared_bytes(&job.operation_id)
                        .unwrap()
                        .as_slice(),
                    b"signed"
                );
            }
            assert!(reopened.operation_pending(&jobs[1].operation_id).unwrap());
        }
    }
}

struct ExitingSource {
    reads: Reads,
    owners: mpsc::UnboundedSender<tokio::task::Id>,
    first: std::sync::atomic::AtomicBool,
}

#[derive(Clone)]
struct InterruptedReads {
    reads: Reads,
    interrupt: Arc<std::sync::atomic::AtomicBool>,
    gate: Arc<Semaphore>,
    exited: mpsc::UnboundedSender<()>,
}
impl FundingObservations for InterruptedReads {
    type QuoteRequest = Quote;
    async fn quote(&self, quote: Quote) -> Result<Quote> {
        self.reads.quote(quote).await
    }
    async fn status(&self, quote: Quote) -> Result<SwapStatus> {
        self.reads.status(quote).await
    }
    async fn credit(&self, job: FundingJob) -> Result<()> {
        if self.interrupt.swap(false, Ordering::SeqCst) {
            self.gate.acquire().await.unwrap().forget();
            self.exited.send(()).unwrap();
            panic!("fixture observation interrupted while source remains owned");
        }
        self.reads.credit(job).await
    }
}
struct RetainedSource {
    source: Source,
    reads: InterruptedReads,
    owners: mpsc::UnboundedSender<tokio::task::Id>,
}
impl FundingBackend for RetainedSource {
    type Observations = InterruptedReads;
    fn observations(&self) -> InterruptedReads {
        self.owners.send(tokio::task::id()).unwrap();
        self.reads.clone()
    }
    fn max_attempts(&self, _: &FundingJob) -> u32 {
        3
    }
    async fn quote_prerequisite(&mut self, _: &FundingJob) -> Result<Quote> {
        panic!("unexpected new quote")
    }
    async fn prepare(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        panic!("unexpected new calculation")
    }
    async fn submit(&mut self, _: &FundingJob) -> Result<()> {
        panic!("unexpected new submission")
    }
    async fn reconcile(&mut self, job: &FundingJob) -> Result<bool> {
        self.source.reconcile(job).await
    }
}

#[tokio::test]
async fn scheduler_replacement_waits_for_retained_source_and_preserves_backoff() {
    for stop_before_release in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut raw = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        crate::rotation::store::test_support::declare_two_reserves(
            &mut raw,
            "pool",
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
        let jobs = raw.funding_jobs().unwrap();
        let job = jobs[0].clone();
        submitted(&mut raw, &job);
        for other in jobs.into_iter().skip(1) {
            raw.record_credit(&other.wallet_id, "1", "fixture", 100)
                .unwrap();
        }
        let (store, actor) = StoreHandle::spawn(raw);
        let (statuses, _status_events) = mpsc::unbounded_channel();
        let reads = Reads {
            credits: Arc::new(Semaphore::new(0)),
            credit_calls: Default::default(),
            statuses,
            status_gates: Arc::new(BTreeMap::from([(
                job.id.clone(),
                Arc::new(Semaphore::new(0)),
            )])),
            failed_job: Default::default(),
            processing: false,
            quotes: None,
        };
        let (started, mut sources) = mpsc::unbounded_channel();
        let source_gate = Arc::new(Semaphore::new(0));
        let interrupt_gate = Arc::new(Semaphore::new(0));
        let (exited, mut exits) = mpsc::unbounded_channel();
        let (owners, mut tasks) = mpsc::unbounded_channel();
        {
            let gate = store.funding_owner();
            let lease = gate.lock().await;
            lease.state.lock().unwrap().records.insert(
                Key {
                    operation: "backoff".into(),
                    kind: Kind::Quote,
                },
                Record {
                    reported_phase: None,
                    due: u64::MAX,
                    streak: 3,
                    error: Some("retained outage".into()),
                    condition: None,
                },
            );
        }
        let worker = FundingWorker::new(
            store.clone(),
            RetainedSource {
                source: Source {
                    store: store.clone(),
                    reads: reads.clone(),
                    gate: source_gate.clone(),
                    started,
                },
                reads: InterruptedReads {
                    reads,
                    interrupt: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                    gate: interrupt_gate.clone(),
                    exited,
                },
                owners,
            },
            1,
        );
        let stop = CancellationToken::new();
        let running = tokio::spawn(worker.clone().run(stop.clone()));
        let first = event(&mut tasks).await;
        assert_eq!(event(&mut sources).await, job.id);
        interrupt_gate.add_permits(1);
        event(&mut exits).await;
        assert!(worker.tick(1000).await.unwrap_err().is::<AlreadyOwned>());
        assert!(
            tasks.try_recv().is_err(),
            "retained source excludes replacement"
        );
        assert!(sources.try_recv().is_err(), "no overlapping source");
        if stop_before_release {
            stop.cancel();
        }
        // A replacement may observe the same unresolved operation again, but
        // only after the original source releases ownership. Let it drain too.
        source_gate.add_permits(2);
        if !stop_before_release {
            assert_ne!(
                first,
                event(&mut tasks).await,
                "a new scheduler must resume after release"
            );
            stop.cancel();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let lease = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            store.funding_owner().lock_owned(),
        )
        .await
        .unwrap();
        assert!(tasks.try_recv().is_err());
        if stop_before_release {
            assert!(
                sources.try_recv().is_err(),
                "shutdown forbids replacement source"
            );
        }
        {
            let state = lease.state.lock().unwrap();
            let backoff = state
                .records
                .get(&Key {
                    operation: "backoff".into(),
                    kind: Kind::Quote,
                })
                .unwrap();
            assert_eq!(
                (backoff.due, backoff.streak, backoff.error.as_deref()),
                (u64::MAX, 3, Some("retained outage"))
            );
        }
        let operation = job.operation_id.clone();
        store
            .call(move |s| {
                assert_eq!(s.prepared_bytes(&operation)?.as_slice(), b"signed");
                assert!(s.operation_pending(&operation)?);
                assert_eq!(s.operation(&operation)?.attempts, 1);
                Ok(())
            })
            .await
            .unwrap();
        drop(lease);
        drop(worker);
        drop(store);
        actor.await.unwrap();
    }
}
impl FundingBackend for ExitingSource {
    type Observations = Reads;
    fn observations(&self) -> Reads {
        self.owners.send(tokio::task::id()).unwrap();
        assert!(
            !self.first.swap(false, Ordering::SeqCst),
            "fixture owner exit"
        );
        self.reads.clone()
    }
    fn max_attempts(&self, _: &FundingJob) -> u32 {
        3
    }
    async fn quote_prerequisite(&mut self, _: &FundingJob) -> Result<Quote> {
        panic!("unexpected quote")
    }
    async fn prepare(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        panic!("unexpected calculation")
    }
    async fn submit(&mut self, _: &FundingJob) -> Result<()> {
        panic!("unexpected submission")
    }
    async fn reconcile(&mut self, _: &FundingJob) -> Result<bool> {
        panic!("unexpected reconciliation")
    }
}
#[tokio::test]
async fn exited_funding_task_is_replaced_with_same_store_and_shutdown_stops_replacement() {
    for stop_before_retry in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let (store, actor) = StoreHandle::spawn(state);
        let (owners, mut tasks) = mpsc::unbounded_channel();
        let (statuses, _) = mpsc::unbounded_channel();
        let source = ExitingSource {
            reads: Reads {
                credits: Arc::new(Semaphore::new(0)),
                credit_calls: Default::default(),
                statuses,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: false,
                quotes: None,
            },
            owners,
            first: std::sync::atomic::AtomicBool::new(true),
        };
        let worker = FundingWorker::new(store.clone(), source, 1);
        let stop = CancellationToken::new();
        let running = tokio::spawn(worker.run(stop.clone()));
        let first = event(&mut tasks).await;
        if !stop_before_retry {
            let second = event(&mut tasks).await;
            assert_ne!(
                first, second,
                "real task replacement, not retained future retry"
            );
        }
        stop.cancel();
        running.await.unwrap().unwrap();
        assert!(tasks.try_recv().is_err());
        store
            .call(|s| {
                assert!(s.status()?.treasury_operations.is_empty());
                Ok(())
            })
            .await
            .unwrap();
        drop(store);
        actor.await.unwrap();
    }
}

#[path = "credit_polling_tests.rs"]
mod credit_polling;

#[path = "diagnostic_tests.rs"]
mod diagnostics;

#[test]
fn source_and_status_floor_and_failed_source_backoff_survive_local_wakes() {
    let dir = tempfile::tempdir().unwrap();
    let mut raw = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
        &mut raw,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = raw.funding_jobs().unwrap().remove(0);
    let mut state = State::default();
    let active = BTreeSet::new();
    for kind in [Kind::Source, Kind::Status, Kind::Quote] {
        let key = Key {
            operation: job.operation_id.clone(),
            kind,
        };
        assert_eq!(state.admit(&key, &job, 1000, &active), Some(true));
        state.complete(&job, kind, 1000, RetrySchedule::Poll(1), None, None);
        if kind != Kind::Quote {
            assert_eq!(state.admit(&key, &job, 1059, &active), Some(false));
            assert_eq!(state.admit(&key, &job, 1061, &active), Some(true));
        }
    }
    let source = Key {
        operation: job.operation_id.clone(),
        kind: Kind::Source,
    };
    state.complete(
        &job,
        Kind::Source,
        1100,
        RetrySchedule::Poll(100),
        Some("source observation unavailable".into()),
        None,
    );
    let due = state.records[&source].due;
    state.complete(&job, Kind::Status, 1101, RetrySchedule::Poll(1), None, None);
    state.complete(&job, Kind::Quote, 1102, RetrySchedule::Poll(1), None, None);
    state.advance_source(&job.operation_id, 1102);
    assert_eq!(state.records[&source].due, due);
    assert_eq!(state.records[&source].streak, 1);
    assert_eq!(
        state.records[&source].error.as_deref(),
        Some("source observation unavailable")
    );
    assert_eq!(state.admit(&source, &job, due - 1, &active), Some(false));
    // Only a successful completion of this exact source clears its failure.
    state.complete(&job, Kind::Source, due, RetrySchedule::Poll(1), None, None);
    state.advance_source(&job.operation_id, due);
    assert_eq!(state.admit(&source, &job, due, &active), Some(true));
}

struct LocalHandoff {
    store: StoreHandle,
    reads: Reads,
    events: Arc<std::sync::Mutex<Vec<&'static str>>>,
    commit_preparation: bool,
}
impl FundingBackend for LocalHandoff {
    type Observations = Reads;
    fn observations(&self) -> Reads {
        self.reads.clone()
    }
    fn max_attempts(&self, _: &FundingJob) -> u32 {
        3
    }
    async fn quote_prerequisite(&mut self, job: &FundingJob) -> Result<Quote> {
        self.events.lock().unwrap().push("prerequisite");
        let (id, operation, target) =
            (job.id.clone(), job.operation_id.clone(), job.target.clone());
        self.store
            .call(move |s| s.select_quote_terms(&id, &operation, &target, &target))
            .await?;
        Ok(quote(job))
    }
    async fn prepare(&mut self, job: &FundingJob, _: &Quote) -> Result<()> {
        self.events.lock().unwrap().push("prepare");
        if !self.commit_preparation {
            return Ok(());
        }
        let job = job.clone();
        self.store
            .call(move |s| {
                s.accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)?;
                let revision = s.snapshot()?.0;
                s.prepare_with_facts(
                    &job.operation_id,
                    revision,
                    b"next",
                    b"signed",
                    Some(TransactionFacts {
                        txid: job.id.clone(),
                        expiry_height: 100,
                        amount_zatoshis: 50,
                        fee_zatoshis: 10,
                        deadline: Some(u64::MAX),
                    }),
                )?;
                Ok(())
            })
            .await
    }
    async fn submit(&mut self, job: &FundingJob) -> Result<()> {
        self.events.lock().unwrap().push("submit");
        let job = job.clone();
        self.store
            .call(move |s| {
                s.request_broadcast(&job.operation_id, now()?, 1, false)?;
                s.reconcile_funding_phase(&job.id, &job.operation_id)
            })
            .await
    }
    async fn reconcile(&mut self, _: &FundingJob) -> Result<bool> {
        self.events.lock().unwrap().push("source");
        Ok(false)
    }
}

#[tokio::test]
async fn successful_quote_prepare_submit_handoffs_do_not_wait_for_observation_cooldown() {
    let dir = tempfile::tempdir().unwrap();
    let mut raw = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
        &mut raw,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let mut jobs = raw.funding_jobs().unwrap();
    let job = jobs.remove(0);
    for other in jobs {
        raw.record_credit(&other.wallet_id, "1", "fixture", 100)
            .unwrap();
    }
    let (store, actor) = StoreHandle::spawn(raw);
    let (statuses, _status_events) = mpsc::unbounded_channel();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let worker = FundingWorker::new(
        store.clone(),
        LocalHandoff {
            store: store.clone(),
            events: events.clone(),
            commit_preparation: true,
            reads: Reads {
                credits: Arc::new(Semaphore::new(100)),
                credit_calls: Default::default(),
                statuses,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: true,
                quotes: None,
            },
        },
        1,
    );
    worker.tick(1000).await.unwrap();
    assert_eq!(*events.lock().unwrap(), ["prerequisite"]);
    worker.tick(1000).await.unwrap();
    assert_eq!(*events.lock().unwrap(), ["prerequisite", "prepare"]);
    worker.tick(1000).await.unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        ["prerequisite", "prepare", "submit"]
    );
    let before = events.lock().unwrap().len();
    worker.tick(1059).await.unwrap();
    assert_eq!(
        events.lock().unwrap().len(),
        before,
        "accepted source waits at least a minute"
    );
    worker.tick(1061).await.unwrap();
    assert_eq!(events.lock().unwrap().last(), Some(&"source"));
    let operation = job.operation_id.clone();
    store
        .call(move |s| {
            assert_eq!(s.operation(&operation)?.attempts, 1);
            Ok(())
        })
        .await
        .unwrap();
    drop(worker);
    drop(store);
    actor.await.unwrap();
}

#[tokio::test]
async fn successful_no_progress_keeps_the_source_observation_floor() {
    for blocked_by_owner in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut raw = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        crate::rotation::store::test_support::declare_two_reserves(
            &mut raw,
            "pool",
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
        let mut jobs = raw.funding_jobs().unwrap();
        let job = jobs.remove(0);
        for other in jobs {
            raw.record_credit(&other.wallet_id, "1", "fixture", 100)
                .unwrap();
        }
        let (store, owner) = StoreHandle::spawn(raw);
        let (statuses, _status_events) = mpsc::unbounded_channel();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let worker = FundingWorker::new(
            store.clone(),
            LocalHandoff {
                store: store.clone(),
                events: events.clone(),
                commit_preparation: false,
                reads: Reads {
                    credits: Arc::new(Semaphore::new(100)),
                    credit_calls: Default::default(),
                    statuses,
                    status_gates: Default::default(),
                    failed_job: Default::default(),
                    processing: true,
                    quotes: None,
                },
            },
            1,
        );
        worker.tick(1000).await.unwrap();
        if blocked_by_owner {
            store
                .call(|s| s.accept_preparation("unrelated-owned-calculation", None, 60, None))
                .await
                .unwrap();
        }
        // A blocked preparation and a backend's successful no-op are both
        // successful local returns, but neither produced a signed source handoff.
        worker.tick(1000).await.unwrap();
        let expected = if blocked_by_owner {
            vec!["prerequisite"]
        } else {
            vec!["prerequisite", "prepare"]
        };
        assert_eq!(*events.lock().unwrap(), expected);
        {
            let lease = store.funding_owner().lock_owned().await;
            let state = lease.state.lock().unwrap();
            let key = Key {
                operation: job.operation_id.clone(),
                kind: Kind::Source,
            };
            assert!(state.records[&key].due >= 1060);
        }
        for epoch in [1000, 1059] {
            worker.tick(epoch).await.unwrap();
            assert_eq!(*events.lock().unwrap(), expected);
        }
        worker.tick(1061).await.unwrap();
        if blocked_by_owner {
            assert_eq!(*events.lock().unwrap(), expected);
        } else {
            assert_eq!(
                *events.lock().unwrap(),
                ["prerequisite", "prepare", "prepare"]
            );
        }
        let operation = job.operation_id.clone();
        store
            .call(move |s| {
                s.require_unstarted_preparation(&operation)?;
                assert!(!s.operation_pending(&operation)?);
                Ok(())
            })
            .await
            .unwrap();
        drop(worker);
        drop(store);
        owner.await.unwrap();
    }
}
