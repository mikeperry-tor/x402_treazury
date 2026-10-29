//! Log severity must not change retry health or persisted funding conditions.
use super::*;
use crate::rotation::{
    error::{FundingCondition, TreasuryFunds},
    transaction::PreparationDeferred,
};
use tracing::instrument::WithSubscriber;

#[tokio::test]
async fn retrying_observations_are_debug_and_operator_requirements_warn_without_changing_backoff() {
    if crate::test_log_capture::run_in_child(
        "rotation::funding::scheduler::tests::diagnostics::retrying_observations_are_debug_and_operator_requirements_warn_without_changing_backoff",
    ) {
        return;
    }
    let cases = [
        (
            anyhow::Error::new(FundingCondition::InsufficientFunds)
                .context(TreasuryFunds::Shortfall)
                .context(crate::rotation::error::TreasuryDepositAddress(
                    "public-receive-address".into(),
                ))
                .context("private upstream shortage"),
            "INFO",
            "add ZEC to public-receive-address and allow confirmations",
        ),
        (
            anyhow::Error::new(FundingCondition::InsufficientFunds)
                .context(TreasuryFunds::Waiting)
                .context("private upstream shortage"),
            "DEBUG",
            "waiting for existing ZEC to become spendable",
        ),
        (
            anyhow::Error::new(FundingCondition::InsufficientFunds)
                .context("private upstream shortage"),
            "INFO",
            "Treasury funds could not be assessed",
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
        let assessment = TreasuryFunds::from_error(&error);
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
                .admit(&key, Some(&job.phase), 1000, &BTreeSet::new()),
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
        if level == "DEBUG" {
            // Independent lifecycle progress can still be INFO. The waiting
            // diagnostic itself must remain DEBUG.
            assert!(
                !logged
                    .lines()
                    .any(|line| line.contains("INFO") && line.contains(message)),
                "{logged}"
            );
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
            if assessment != TreasuryFunds::Waiting && !logged.contains("public-receive-address") {
                assert!(logged.contains("wallet status --config FILE"), "{logged}");
            }
            if assessment != TreasuryFunds::Shortfall {
                assert!(!logged.contains("add ZEC"), "{logged}");
                assert!(!category.contains("add ZEC"), "{category}");
            }
            let repeated = anyhow::Error::new(FundingCondition::InsufficientFunds)
                .context(assessment)
                .context(crate::rotation::error::TreasuryDepositAddress(
                    "public-receive-address".into(),
                ));
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

#[test]
fn canonical_disagreement_reports_automatic_resolution_once_per_error_episode() {
    if crate::test_log_capture::run_in_child(
        "rotation::funding::scheduler::tests::diagnostics::canonical_disagreement_reports_automatic_resolution_once_per_error_episode",
    ) {
        return;
    }
    use crate::rotation::transaction::{ConfirmationObservation, RecoveryEvidenceError};
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
        "diagnostics",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    let phase = FundingPhase::DepositPending;
    for (automatic_resolution, error) in [
        (
            true,
            anyhow::Error::new(RecoveryEvidenceError::ConfirmationMismatch),
        ),
        (
            true,
            anyhow::Error::new(RecoveryEvidenceError::ChainChanged),
        ),
        // RecoveryObservation preserves these safe facts without the typed cause.
        (
            true,
            anyhow::Error::new(ConfirmationObservation {
                lookup_height: 100,
                wallet_height: None,
                scanned_height: Some(101),
            }),
        ),
        (
            false,
            anyhow::Error::new(RecoveryEvidenceError::ConfirmationPending),
        ),
    ] {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(log.reopen().unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let error = error
                .context("source_reconciliation_failed")
                .context("private upstream details");
            let category =
                report_observation_failure(&job, "small", Kind::Source, &phase, &error, None);
            report_observation_failure(
                &job,
                "small",
                Kind::Source,
                &phase,
                &error,
                Some(&category),
            );
        });
        let logged = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            logged.lines().filter(|line| line.contains("INFO")).count(),
            usize::from(automatic_resolution),
            "{logged}"
        );
        if automatic_resolution {
            assert!(
                logged.contains("canonical resolution will retry automatically"),
                "{logged}"
            );
        }
        assert!(logged.contains("DEBUG"));
        assert!(
            !logged.contains("WARN") && !logged.contains("private"),
            "{logged}"
        );
        assert!(logged.contains(&job.operation_id) && logged.contains(&job.id));
    }
}

#[tokio::test]
async fn retained_request_debug_matches_actual_status_acquisitions_without_shortening_cadence() {
    if crate::test_log_capture::run_in_child(
        "rotation::funding::scheduler::tests::diagnostics::retained_request_debug_matches_actual_status_acquisitions_without_shortening_cadence",
    ) {
        return;
    }
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
        "diagnostics",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = raw.funding_jobs().unwrap().remove(0);
    submitted(&mut raw, &job);
    raw.disable_pool("diagnostics").unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let (store, actor) = StoreHandle::spawn(raw);
    let (statuses, mut observed) = mpsc::unbounded_channel();
    let (started, _source_events) = mpsc::unbounded_channel();
    let worker = FundingWorker::new(
        store.clone(),
        Source {
            store: store.clone(),
            gate: Arc::new(Semaphore::new(100)),
            started,
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
        60,
    );
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(log.reopen().unwrap())
            .finish(),
    );
    // The real application installs a global dispatcher. This exact child owns
    // its only installation, including observations in the retained owner task.
    tracing::dispatcher::set_global_default(subscriber.clone()).unwrap();
    let message = "Checking an earlier funding request that still awaits a final provider outcome";
    // Ordinary current-swap acquisition stays routine.
    worker.tick(1000).await.unwrap();
    assert_eq!(observed.try_recv().unwrap(), job.id);
    assert!(
        !std::fs::read_to_string(log.path())
            .unwrap()
            .contains(message)
    );
    // A retained copy of this exact request is presentation context, not authority.
    db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote) SELECT operation_id,job_id,phase,quote FROM funding_progress WHERE job_id=?1",[&job.id]).unwrap();
    let due = {
        let lease = store.funding_owner().lock_owned().await;
        let state = lease.state.lock().unwrap();
        state.records[&Key {
            operation: job.operation_id.clone(),
            kind: Kind::Status,
        }]
            .due
    };
    assert!(due >= 1060);
    worker.tick(due - 1).await.unwrap();
    assert!(observed.try_recv().is_err());
    assert!(
        !std::fs::read_to_string(log.path())
            .unwrap()
            .contains(message)
    );
    worker.tick(due).await.unwrap();
    assert_eq!(observed.try_recv().unwrap(), job.id);
    let logged = std::fs::read_to_string(log.path()).unwrap();
    let checks: Vec<_> = logged
        .lines()
        .filter(|line| line.contains(message))
        .collect();
    assert_eq!(checks.len(), 1, "{logged}");
    assert!(
        checks[0].contains("DEBUG")
            && checks[0].contains(&job.operation_id)
            && checks[0].contains(&job.id)
    );
    let next_due = {
        let lease = store.funding_owner().lock_owned().await;
        let state = lease.state.lock().unwrap();
        state.records[&Key {
            operation: job.operation_id.clone(),
            kind: Kind::Status,
        }]
            .due
    };
    assert!(next_due >= due + 60);
    worker.tick(next_due - 1).await.unwrap();
    assert!(observed.try_recv().is_err());
    assert_eq!(
        std::fs::read_to_string(log.path())
            .unwrap()
            .matches(message)
            .count(),
        1
    );
    worker.tick(next_due).await.unwrap();
    assert_eq!(observed.try_recv().unwrap(), job.id);
    assert_eq!(
        std::fs::read_to_string(log.path())
            .unwrap()
            .matches(message)
            .count(),
        2
    );
    assert!(
        std::fs::read_to_string(log.path())
            .unwrap()
            .lines()
            .filter(|line| line.contains(message))
            .all(|line| line.contains("DEBUG") && !line.contains("INFO") && !line.contains("WARN"))
    );
    let terminal_due = {
        let lease = store.funding_owner().lock_owned().await;
        let state = lease.state.lock().unwrap();
        state.records[&Key {
            operation: job.operation_id.clone(),
            kind: Kind::Status,
        }]
            .due
    };
    assert!(terminal_due >= next_due + 60);
    let id = job.id.clone();
    let operation = job.operation_id.clone();
    store
        .call(move |s| {
            s.record_provider_outcome(&id, &operation, &SwapStatus::Failed, next_due + 1)
        })
        .await
        .unwrap();
    worker.tick(terminal_due).await.unwrap();
    assert!(observed.try_recv().is_err());
    assert_eq!(
        std::fs::read_to_string(log.path())
            .unwrap()
            .matches(message)
            .count(),
        2
    );
    let operation = job.operation_id.clone();
    store
        .call(move |s| {
            assert_eq!(s.operation(&operation)?.attempts, 1);
            assert_eq!(
                s.prepared_bytes(&operation)?.as_slice(),
                crate::rotation::store::test_support::transaction_raw(100).as_slice()
            );
            Ok(())
        })
        .await
        .unwrap();
    drop(worker);
    drop(store);
    actor.await.unwrap();
}
