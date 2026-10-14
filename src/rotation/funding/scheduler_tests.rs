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
                    1000
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
            assert!(credits.load(Ordering::SeqCst) >= 2);
            // Reuse the actual owner after the raced pass. Any new work must
            // bind to the replacement; captured stale work cannot prepare/send.
            worker.tick(2000).await.unwrap();
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
                    assert!(!facts.provider_success && !facts.readiness_recorded);
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

#[tokio::test]
async fn malformed_readiness_is_local_while_independent_source_work_drains() {
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state");
    let key_path = dir.path().join("key");
    let mut state = Store::create(&state_path, &key_path, 1, b"fixture").unwrap();
    for name in ["damaged", "healthy"] {
        let pool = state
            .ensure_capacity_group(
                name,
                &crate::rotation::store::test_support::reserves("5"),
                Some(alloy_primitives::U256::from(1_000_000)),
                0,
            )
            .unwrap();
        state
            .allocate_capacity_kind(&pool, crate::rotation::store::AllocationKind::Small, true)
            .unwrap();
    }
    let jobs = state.funding_jobs().unwrap();
    let damaged = jobs
        .iter()
        .find(|job| job.pool_name == "damaged")
        .unwrap()
        .clone();
    let healthy = jobs
        .iter()
        .find(|job| job.pool_name == "healthy")
        .unwrap()
        .clone();
    submitted(&mut state, &damaged);
    state.confirm_spend(&damaged.operation_id, 60, 1).unwrap();
    state
        .reconcile_funding_phase(&damaged.id, &damaged.operation_id)
        .unwrap();
    state
        .record_destination_readiness(&damaged.id, &damaged.target, "fixture", 100)
        .unwrap();
    submitted(&mut state, &healthy);
    // Corrupt only this wallet's immutable proof, preserving operation/source facts.
    let db = rusqlite::Connection::open(state_path.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE funding_jobs SET readiness_support='{}' WHERE id=?1",
        [&damaged.id],
    )
    .unwrap();
    drop(db);
    let treasury = state.id().to_owned();
    let (store, actor) = StoreHandle::spawn(state);
    let (statuses, _status_events) = mpsc::unbounded_channel();
    let (started, mut source_events) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            reads: Reads {
                credits: Arc::new(Semaphore::new(32)),
                credit_calls: Default::default(),
                statuses,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: false,
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
        async move { worker.schedule(1000, stop, false).await }
    });
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), source_events.recv())
            .await
            .unwrap()
            .unwrap(),
        healthy.id
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (bad, good) = (damaged.clone(), healthy.clone());
            if store
                .call(move |s| {
                    let bad = s.funding_job(&bad.id)?.unwrap();
                    Ok(bad.last_error.as_deref().is_some_and(|error| {
                        error.starts_with(
                            "funding_integrity_conflict; malformed_durable_evidence_support",
                        )
                    }) && s
                        .funding_facts(&good.id, &good.operation_id)?
                        .provider_success)
                })
                .await
                .unwrap()
            {
                break;
            }
            assert!(
                !waiter.is_finished(),
                "wallet-local error stopped shared funding"
            );
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    waiter.abort();
    let _ = waiter.await;
    stop.cancel();
    assert!(worker.tick(1001).await.unwrap_err().is::<AlreadyOwned>());
    gate.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(lease) = store.funding_owner().try_lock_owned() {
                let state = lease.state.lock().unwrap();
                let retry = &state.records[&Key {
                    operation: damaged.operation_id.clone(),
                    kind: Kind::Status,
                }];
                assert_eq!(retry.streak, 1);
                assert!(retry.due > 1000);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        source_events.try_recv().is_err(),
        "no replacement or duplicate source work"
    );
    drop(worker);
    drop(store);
    actor.await.unwrap();
    let reopened = Store::open(&state_path, &key_path, &treasury).unwrap();
    for job in [&damaged, &healthy] {
        let saved = reopened.funding_job(&job.id).unwrap().unwrap();
        assert_eq!(saved.operation_id, job.operation_id);
        assert_ne!(saved.phase, FundingPhase::Complete);
        assert_eq!(reopened.funding_recovery_count(&job.id).unwrap(), 0);
        assert_eq!(reopened.operation(&job.operation_id).unwrap().attempts, 1);
    }
    assert!(
        reopened
            .funding_job(&damaged.id)
            .unwrap()
            .unwrap()
            .last_error
            .unwrap()
            .contains("malformed_durable_evidence_support")
    );
    assert_eq!(
        reopened
            .operation(&healthy.operation_id)
            .unwrap()
            .submission,
        "BROADCAST_REQUESTED"
    );
    assert!(reopened.operation_pending(&healthy.operation_id).unwrap());
    assert_eq!(
        reopened
            .prepared_bytes(&healthy.operation_id)
            .unwrap()
            .as_slice(),
        b"signed"
    );
}

#[tokio::test]
async fn completion_proof_failure_backs_off_each_kind_but_storage_failure_is_fatal() {
    for malformed in [true, false] {
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
        if malformed {
            db.execute(
                "UPDATE funding_jobs SET readiness_support='{}' WHERE id=?1",
                [&job.id],
            )
            .unwrap();
        } else {
            db.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE OF state ON funding_jobs BEGIN SELECT RAISE(ABORT,'fixture storage failure'); END;").unwrap();
        }
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
                1,
                Some("funding_quote_failed; fixture outage".into()),
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
                    1000,
                )
                .await;
            if malformed {
                assert!(
                    result.unwrap().is_none(),
                    "rejected completion cannot start quote HTTP"
                );
                {
                    let state = lease.state.lock().unwrap();
                    assert_eq!(
                        state.records[&key].streak,
                        if kind == Kind::Source { 2 } else { 1 }
                    );
                    assert!(state.records[&key].due > 1000);
                }
                let id = job.id.clone();
                let saved = store
                    .call(move |s| Ok(s.funding_job(&id)?.unwrap()))
                    .await
                    .unwrap();
                assert!(
                    saved.last_error.unwrap().starts_with(
                        "funding_integrity_conflict; malformed_durable_evidence_support"
                    )
                );
            } else {
                assert!(result.err().unwrap().is::<rusqlite::Error>());
            }
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
                quotes: None,
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
    wait_until(|| credit_calls.load(Ordering::SeqCst) == jobs.len()).await;
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
        raw.confirm_spend(&initial[0].operation_id, 60, 1).unwrap();
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
                    s.confirm_spend(&job.operation_id, 60, 1)?;
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
            assert_eq!(credits.load(Ordering::SeqCst), initial.len());
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
