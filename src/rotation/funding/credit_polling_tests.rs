//! Credit traffic follows accepted source work, independently of faster lanes.
use super::*;

fn polling_worker(
    store: StoreHandle,
    credits: Arc<AtomicUsize>,
) -> (
    FundingWorker<Source>,
    mpsc::UnboundedReceiver<String>,
    mpsc::UnboundedReceiver<String>,
) {
    let (started, source_events) = mpsc::unbounded_channel();
    let (statuses, status_events) = mpsc::unbounded_channel();
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store,
            reads: Reads {
                credits: Arc::new(Semaphore::new(4096)),
                credit_calls: credits,
                statuses,
                status_gates: Default::default(),
                failed_job: Default::default(),
                processing: true,
                quotes: None,
            },
            gate: Arc::new(Semaphore::new(4096)),
            started,
        },
        1,
    );
    (worker, source_events, status_events)
}

#[tokio::test]
async fn fresh_and_quoted_allocations_never_poll_credit_even_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let key = dir.path().join("key");
    let mut raw = Store::create(&path, &key, 1, b"fixture").unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
        &mut raw,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let jobs = raw.funding_jobs().unwrap();
    raw.save_funding_quote(&jobs[0].id, &serde_json::to_vec(&quote(&jobs[0])).unwrap())
        .unwrap();
    let treasury = raw.id().to_owned();
    let credits = Arc::new(AtomicUsize::new(0));
    for epoch in [1000, 2000] {
        let (store, owner) = StoreHandle::spawn(raw);
        let (worker, _source_events, _status_events) =
            polling_worker(store.clone(), credits.clone());
        worker.tick(epoch).await.unwrap();
        worker.tick(epoch + 120).await.unwrap();
        assert_eq!(
            credits.load(Ordering::SeqCst),
            0,
            "unstarted work must not call Base"
        );
        store
            .call(|s| {
                let status = s.status()?;
                assert!(status.treasury_operations.is_empty());
                assert!(!status.outgoing_pending);
                for job in s.funding_jobs()? {
                    let facts = s.funding_facts(&job.id, &job.operation_id)?;
                    assert!(!facts.credit_needed());
                    assert!(!facts.preparation_accepted && !facts.source_pending);
                    assert_eq!(facts.source_attempts, 0);
                    assert_eq!(s.funding_recovery_count(&job.id)?, 0);
                }
                Ok(())
            })
            .await
            .unwrap();
        drop(worker);
        drop(store);
        owner.await.unwrap();
        raw = Store::open(&path, &key, &treasury).unwrap();
    }
}

#[tokio::test]
async fn accepted_work_resumes_credit_immediately_then_waits_a_minute_despite_sibling_progress() {
    for stage in ["preparation", "submitted", "confirmed"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        let key = dir.path().join("key");
        let mut raw = Store::create(&path, &key, 1, b"fixture").unwrap();
        crate::rotation::store::test_support::declare_two_reserves(
            &mut raw,
            "pool",
            "5",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
        let job = raw.funding_jobs().unwrap().remove(0);
        if stage == "preparation" {
            raw.save_funding_quote(&job.id, &serde_json::to_vec(&quote(&job)).unwrap())
                .unwrap();
            raw.require_preparation_authority(
                &job.operation_id,
                Some(&crate::rotation::transaction::ManagedFundingAuthority {
                    job_id: job.id.clone(),
                    auto_fund: true,
                    max_attempts: 3,
                }),
            )
            .unwrap();
            raw.accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                .unwrap();
        } else {
            submitted(&mut raw, &job);
            if stage == "confirmed" {
                raw.confirm_spend(&job.operation_id).unwrap();
            }
        }
        let treasury = raw.id().to_owned();
        drop(raw);
        let raw = Store::open(&path, &key, &treasury).unwrap();
        assert!(
            raw.funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .credit_needed()
        );
        let (store, owner) = StoreHandle::spawn(raw);
        let credits = Arc::new(AtomicUsize::new(0));
        let (worker, _source_events, mut status_events) =
            polling_worker(store.clone(), credits.clone());
        worker.tick(1000).await.unwrap();
        assert_eq!(
            credits.load(Ordering::SeqCst),
            1,
            "{stage}: first observation is immediate"
        );
        for epoch in [1002, 1010, 1030, 1059] {
            worker.tick(epoch).await.unwrap();
            assert_eq!(
                credits.load(Ordering::SeqCst),
                1,
                "{stage}: credit polling must retain its own due time"
            );
        }
        if stage != "preparation" {
            let mut statuses = 0;
            while status_events.try_recv().is_ok() {
                statuses += 1;
            }
            assert!(statuses > 1, "provider status continues independently");
        }
        worker.tick(1061).await.unwrap();
        assert_eq!(
            credits.load(Ordering::SeqCst),
            2,
            "{stage}: credit polling resumes after a minute"
        );
        let id = job.clone();
        store
            .call(move |s| {
                let facts = s.funding_facts(&id.id, &id.operation_id)?;
                assert!(!facts.initial_credit_recorded);
                assert_eq!(facts.source_attempts, u32::from(stage != "preparation"));
                Ok(())
            })
            .await
            .unwrap();
        drop(worker);
        drop(store);
        owner.await.unwrap();
    }
}

#[test]
fn credit_failure_backoff_survives_sibling_success_and_uses_completion_time() {
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
    let credit = Key {
        operation: job.operation_id.clone(),
        kind: Kind::Credit,
    };
    assert_eq!(state.admit(&credit, &job, 1000, &active), Some(true));
    for kind in [Kind::Source, Kind::Status] {
        state
            .admit(
                &Key {
                    operation: job.operation_id.clone(),
                    kind,
                },
                &job,
                1000,
                &active,
            )
            .unwrap();
    }
    let jitter = job.id.bytes().map(u64::from).sum::<u64>() % 6;
    for failure in 1..=7 {
        let completed = 1000 + failure * 1000;
        state.complete(
            &job,
            Kind::Credit,
            completed,
            RetrySchedule::Poll(5),
            Some("RPC unavailable".into()),
            None,
        );
        let delay = (5u64 * (1u64 << failure.min(6))).clamp(60, 300);
        let due = completed + delay + jitter;
        for kind in [Kind::Source, Kind::Status] {
            state.complete(
                &job,
                kind,
                completed + 1,
                RetrySchedule::Poll(5),
                None,
                None,
            );
        }
        assert_eq!(state.admit(&credit, &job, due - 1, &active), Some(false));
        assert_eq!(state.admit(&credit, &job, due, &active), Some(true));
        assert_eq!(state.records[&credit].streak, failure as u32);
    }
    state.complete(
        &job,
        Kind::Credit,
        10000,
        RetrySchedule::Poll(5),
        None,
        None,
    );
    assert_eq!(state.records[&credit].streak, 0);
    assert_eq!(state.records[&credit].due, 10060 + jitter);
}
