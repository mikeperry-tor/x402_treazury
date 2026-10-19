//! Log severity must not change retry health or persisted funding conditions.
use super::*;
use crate::rotation::{error::FundingCondition, transaction::PreparationDeferred};
use tracing::instrument::WithSubscriber;

#[tokio::test]
async fn expected_waits_are_info_but_retain_error_backoff_and_other_failures_warn() {
    let cases = [
        (
            anyhow::Error::new(FundingCondition::InsufficientFunds)
                .context("private upstream shortage"),
            "INFO",
            "waiting for confirmed spendable ZEC",
        ),
        (
            anyhow::anyhow!("indexer connection failed")
                .context(PreparationDeferred)
                .context("private dependency"),
            "INFO",
            "deferred until treasury synchronization succeeds",
        ),
        (
            anyhow::anyhow!("private unclassified proposal").context(PreparationDeferred),
            "WARN",
            "funding observation unavailable",
        ),
        (
            anyhow::Error::new(FundingCondition::CostLimit).context(PreparationDeferred),
            "WARN",
            "funding observation unavailable",
        ),
    ];
    for (error, level, message) in cases {
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
        raw.save_funding_quote(&job.id, &serde_json::to_vec(&quote(&job)).unwrap())
            .unwrap();
        let job = raw.funding_job(&job.id).unwrap().unwrap();
        let condition = funding_condition(&error);
        let category = safe_error(&error, &job.phase);
        let (store, owner) = StoreHandle::spawn(raw);
        let (started, _source_events) = mpsc::unbounded_channel();
        let (statuses, _status_events) = mpsc::unbounded_channel();
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
                    processing: true,
                    quotes: None,
                },
                gate: Arc::new(Semaphore::new(0)),
                started,
            },
            5,
        );
        let lease = Arc::new(store.funding_owner().lock_owned().await);
        let key = Key {
            operation: job.operation_id.clone(),
            kind: Kind::Source,
        };
        assert_eq!(
            lease
                .state
                .lock()
                .unwrap()
                .admit(&key, &job, 1000, &BTreeSet::new()),
            Some(true)
        );
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(log.reopen().unwrap())
            .finish();
        worker
            .publish_observation(
                &lease,
                Completion {
                    job: job.clone(),
                    kind: Kind::Source,
                    result: Err(error),
                },
                &TickClock {
                    epoch: 1000,
                    started: tokio::time::Instant::now(),
                },
            )
            .with_subscriber(subscriber)
            .await
            .unwrap();
        let logged = std::fs::read_to_string(log.path()).unwrap();
        assert!(
            logged.contains(level) && logged.contains(message),
            "{logged}"
        );
        if level == "INFO" {
            assert!(!logged.contains("WARN"), "{logged}");
        }
        assert!(
            logged.contains("observation=Source") && logged.contains("phase=Quoted"),
            "{logged}"
        );
        assert!(!logged.contains("private"), "{logged}");
        {
            let state = lease.state.lock().unwrap();
            let record = &state.records[&key];
            assert_eq!(record.error.as_deref(), Some(category.as_str()));
            assert_eq!(record.condition, condition);
            assert_eq!(record.streak, 1);
            assert!(
                (1060..=1065).contains(&record.due),
                "logging must not reset failure backoff"
            );
        }
        store
            .call(move |s| {
                let current = s.funding_job(&job.id)?.unwrap();
                assert_eq!(current.last_error.as_deref(), Some(category.as_str()));
                assert_eq!(current.error_streak, 1);
                assert!(s.status()?.treasury_operations.is_empty());
                assert_eq!(s.funding_recovery_count(&job.id)?, 0);
                Ok(())
            })
            .await
            .unwrap();
        drop(lease);
        drop(worker);
        drop(store);
        owner.await.unwrap();
    }
}
