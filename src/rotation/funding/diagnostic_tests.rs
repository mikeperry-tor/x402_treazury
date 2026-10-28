//! Log severity must not change retry health or persisted funding conditions.
use super::*;
use crate::rotation::{error::FundingCondition, transaction::PreparationDeferred};
use tracing::instrument::WithSubscriber;

#[tokio::test]
async fn retrying_observations_are_debug_and_operator_requirements_warn_without_changing_backoff() {
    let cases = [
        (
            anyhow::Error::new(FundingCondition::InsufficientFunds)
                .context(crate::rotation::error::TreasuryDepositAddress(
                    "public-receive-address".into(),
                ))
                .context("private upstream shortage"),
            "INFO",
            "add ZEC to public-receive-address and allow confirmations",
        ),
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
            "DEBUG",
            "funding observation unavailable",
        ),
        (
            anyhow::Error::new(FundingCondition::CostLimit).context(PreparationDeferred),
            "DEBUG",
            "funding observation unavailable",
        ),
        (
            anyhow::Error::new(crate::rotation::transaction::ConfirmationObservation {
                lookup_height: 100,
                wallet_height: None,
                scanned_height: Some(100),
            })
            .context(RecoveryBlocked("conflicting_transaction_or_input_evidence")),
            "WARN",
            "requires operator review",
        ),
        (
            anyhow::Error::new(crate::rotation::transaction::ConfirmationObservation {
                lookup_height: 100,
                wallet_height: Some(98),
                scanned_height: Some(101),
            })
            .context(RecoveryBlocked("conflicting_transaction_or_input_evidence")),
            "WARN",
            "requires operator review",
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
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(log.reopen().unwrap())
            .finish();
        let subscriber = tracing::Dispatch::new(subscriber);
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
            .with_subscriber(subscriber.clone())
            .await
            .unwrap();
        let logged = std::fs::read_to_string(log.path()).unwrap();
        assert!(
            logged.contains(level) && logged.contains(message),
            "{logged}"
        );
        if level != "WARN" {
            assert!(!logged.contains("WARN"), "{logged}");
        }
        if level == "WARN" {
            assert!(logged.contains("lookup_height=100"), "{logged}");
            assert!(logged.contains("wallet_height="), "{logged}");
            assert!(logged.contains("scanned_height="), "{logged}");
            assert!(logged.contains(&job.operation_id), "{logged}");
            assert!(logged.contains("reservations retained"), "{logged}");
            let repeated =
                anyhow::Error::new(crate::rotation::transaction::ConfirmationObservation {
                    lookup_height: 100,
                    wallet_height: None,
                    scanned_height: Some(102),
                })
                .context(RecoveryBlocked("conflicting_transaction_or_input_evidence"));
            let refreshed = tracing::dispatcher::with_default(&subscriber, || {
                report_observation_failure(
                    &job,
                    "small",
                    Kind::Source,
                    &job.phase,
                    &repeated,
                    Some(&category),
                )
            });
            let repeated_logs = std::fs::read_to_string(log.path()).unwrap();
            let repeated_logs = &repeated_logs[logged.len()..];
            assert!(repeated_logs.contains("DEBUG"), "{repeated_logs}");
            assert!(!repeated_logs.contains("WARN"), "{repeated_logs}");
            assert!(
                repeated_logs.contains("scanned_height=102"),
                "{repeated_logs}"
            );
            assert!(refreshed.contains("scanned_height=102"));
            // A successful observation clears Record.error; recurrence must warn.
            tracing::dispatcher::with_default(&subscriber, || {
                report_observation_failure(&job, "small", Kind::Source, &job.phase, &repeated, None)
            });
            let recurrence_logs = std::fs::read_to_string(log.path()).unwrap();
            assert_eq!(
                recurrence_logs.matches("WARN").count(),
                2,
                "{recurrence_logs}"
            );
            assert_eq!(
                recurrence_logs.matches("requires operator review").count(),
                2,
                "{recurrence_logs}"
            );
        }
        if level != "INFO" {
            assert!(
                logged.contains("observation=Source") && logged.contains("phase=Quoted"),
                "{logged}"
            );
        }
        assert!(!logged.contains("private"), "{logged}");
        assert!(!category.contains("public-receive-address"));
        if condition == Some(FundingCondition::InsufficientFunds) {
            assert!(
                logged.contains("synchronization and funding retry automatically"),
                "{logged}"
            );
            if !logged.contains("public-receive-address") {
                assert!(logged.contains("wallet status --config FILE"), "{logged}");
            }
            let repeated = anyhow::Error::new(FundingCondition::InsufficientFunds).context(
                crate::rotation::error::TreasuryDepositAddress("public-receive-address".into()),
            );
            tracing::dispatcher::with_default(&subscriber, || {
                report_observation_failure(
                    &job,
                    "small",
                    Kind::Source,
                    &job.phase,
                    &repeated,
                    Some(&category),
                )
            });
            let repeated_logs = std::fs::read_to_string(log.path()).unwrap();
            let repeated_logs = &repeated_logs[logged.len()..];
            assert!(!repeated_logs.contains("INFO"), "{repeated_logs}");
            assert!(
                !repeated_logs.contains("public-receive-address"),
                "{repeated_logs}"
            );
        }
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
