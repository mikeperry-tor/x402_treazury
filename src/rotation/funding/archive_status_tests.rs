//! Original bridge obligations survive source expiry and operation rollover.
use super::*;
use crate::rotation::store::{SyncObservation, SyncPhase};

fn fixture() -> (tempfile::TempDir, Store, FundingJob) {
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
        "archive",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    (dir, store, job)
}

fn original_quote(job: &FundingJob) -> Quote {
    let mut original = quote(job);
    original.request["fixture_job"] = serde_json::json!(job.operation_id);
    original.deposit = Some(format!("deposit-{}", job.operation_id));
    original
}

fn prepare_original(store: &mut Store, job: &FundingJob, disclose: bool) {
    let height = store
        .treasury_sync_state()
        .unwrap()
        .sync
        .and_then(|sync| sync.height)
        .unwrap_or(1);
    store
        .save_funding_quote(&job.id, &serde_json::to_vec(&original_quote(job)).unwrap())
        .unwrap();
    store
        .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
        .unwrap();
    let revision = store.snapshot().unwrap().0;
    store
        .prepare_with_facts(
            &job.operation_id,
            revision,
            b"calculated",
            crate::rotation::store::test_support::transaction_raw(
                u32::try_from(height).unwrap() + 100,
            )
            .as_slice(),
            Some(TransactionFacts {
                txid: crate::rotation::store::test_support::transaction_txid(
                    u32::try_from(height).unwrap() + 100,
                ),
                expiry_height: u32::try_from(height).unwrap() + 100,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: Some(u64::MAX),
            }),
        )
        .unwrap();
    if disclose {
        store
            .request_broadcast(&job.operation_id, 1, height, false)
            .unwrap();
    }
}

fn expire_and_rotate(store: &mut Store, original: &FundingJob, max_attempts: u32) -> FundingJob {
    prepare_original(store, original, true);
    let height = u64::from(
        store
            .operation(&original.operation_id)
            .unwrap()
            .facts
            .expiry_height,
    ) + 2;
    let revision = store.snapshot().unwrap().0;
    let revision = store
        .save_sync_snapshot(
            revision,
            b"canonical SDK fixture",
            Some(SyncObservation {
                phase: SyncPhase::Ready,
                last_error: None,
                snapshot_revision: revision,
                checked_at: Some(1),
                checkpoint_at: 1,
                scanned_blocks: u32::try_from(height).unwrap(),
                target_height: Some(height),
                observed_tip_height: Some(height),
                height: Some(height),
                confirmations: 2,
                confirmed_pool_balances_zatoshis: None,
                confirmed_shielded_zatoshis: 1000,
                spendable_shielded_zatoshis: 1000,
            }),
        )
        .unwrap();
    store
        .resolve_expired(&original.operation_id, revision)
        .unwrap();
    store
        .reauthorize_closed_funding_for_operation(
            &original.id,
            &original.operation_id,
            max_attempts,
        )
        .unwrap();
    store.funding_job(&original.id).unwrap().unwrap()
}

#[test]
fn archived_status_pages_keep_each_original_under_one_job_across_restart() {
    let (dir, mut store, mut current) = fixture();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let count = crate::rotation::store::scheduling::PAGE_SIZE + 1;
    let mut originals = Vec::new();
    for _ in 0..count {
        let original = current.clone();
        current = expire_and_rotate(&mut store, &original, u32::try_from(count).unwrap());
        originals.push(original);
    }
    let identity = store.id().to_owned();
    drop(store);
    let mut store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &identity,
    )
    .unwrap();
    let through = store.funding_work_boundary().unwrap();
    let mut cursor = (String::new(), String::new());
    let mut seen = std::collections::BTreeSet::new();
    let mut pages = 0;
    loop {
        let page = store.funding_status_page(&cursor, through).unwrap();
        if page.is_empty() {
            break;
        }
        pages += 1;
        for (key, entry) in page {
            assert!(key > cursor);
            cursor = key;
            let Some(original) = entry.unwrap() else {
                continue;
            };
            assert_eq!(original.id, current.id);
            assert!(seen.insert(original.operation_id.clone()));
            let expected = originals
                .iter()
                .find(|job| job.operation_id == original.operation_id)
                .unwrap();
            assert_eq!(
                store
                    .funding_status_quote(&original.id, &original.operation_id)
                    .unwrap()
                    .as_slice(),
                serde_json::to_vec(&original_quote(expected)).unwrap()
            );
            assert_eq!(store.operation(&original.operation_id).unwrap().attempts, 1);
            assert_eq!(
                store
                    .prepared_bytes(&original.operation_id)
                    .unwrap()
                    .as_slice(),
                crate::rotation::store::test_support::transaction_raw(
                    101 + 102
                        * u32::try_from(
                            originals
                                .iter()
                                .position(|job| job.operation_id == original.operation_id)
                                .unwrap()
                        )
                        .unwrap()
                )
                .as_slice()
            );
        }
    }
    assert_eq!(pages, 2);
    assert_eq!(seen.len(), count);
    assert!(!seen.contains(&current.operation_id));
    store
        .record_provider_outcome(
            &originals[0].id,
            &originals[0].operation_id,
            &SwapStatus::Failed,
            2,
        )
        .unwrap();
    let remaining = store
        .funding_status_page(&(String::new(), String::new()), through)
        .unwrap();
    assert!(remaining.iter().all(|(_, row)| {
        row.as_ref()
            .unwrap()
            .as_ref()
            .is_none_or(|row| row.operation_id != originals[0].operation_id)
    }));
    assert_eq!(
        store
            .funding_job(&current.id)
            .unwrap()
            .unwrap()
            .operation_id,
        current.operation_id
    );
    assert!(store.funding_quote(&current.id).is_err());
    assert_eq!(
        store.funding_recovery_count(&current.id).unwrap(),
        u64::try_from(count).unwrap()
    );
    store
        .record_destination_readiness(&current.id, "1", "initial credit fixture", 1)
        .unwrap();
    assert!(
        store
            .finish_funding_if_proven(&current.id, &current.operation_id)
            .unwrap()
    );
    assert!(
        crate::rotation::store::funding::destination_obligation_pending(&db, &current.wallet_id)
            .unwrap()
    );
    for original in &originals {
        store
            .record_provider_outcome(&original.id, &original.operation_id, &SwapStatus::Failed, 3)
            .unwrap();
    }
    assert!(
        !crate::rotation::store::funding::destination_obligation_pending(&db, &current.wallet_id)
            .unwrap(),
        "terminal original bridge outcomes remove delivery uncertainty without another deposit"
    );
    drop(store);
    let reopened = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &identity,
    )
    .unwrap();
    assert!(
        reopened
            .funding_status_page(
                &(String::new(), String::new()),
                reopened.funding_work_boundary().unwrap()
            )
            .unwrap()
            .is_empty()
    );
    assert!(
        !crate::rotation::store::funding::destination_obligation_pending(&db, &current.wallet_id)
            .unwrap()
    );
}

#[derive(Clone)]
struct ArchiveReads {
    inner: Reads,
    started: mpsc::UnboundedSender<String>,
    success_operation: String,
}
impl FundingObservations for ArchiveReads {
    type QuoteRequest = Quote;
    async fn quote(&self, request: Quote) -> Result<Quote> {
        self.inner.quote(request).await
    }
    async fn credit(&self, job: FundingJob) -> Result<()> {
        self.inner.credit(job).await
    }
    async fn status(&self, original: Quote) -> Result<SwapStatus> {
        let operation = original.request["fixture_job"].as_str().unwrap().to_owned();
        self.started.send(operation.clone()).unwrap();
        self.inner.status(original).await.map(|_| {
            if operation == self.success_operation {
                SwapStatus::Success
            } else {
                SwapStatus::Processing
            }
        })
    }
}
struct ArchiveSource {
    inner: Source,
    started: mpsc::UnboundedSender<String>,
    success_operation: String,
}
impl FundingBackend for ArchiveSource {
    type Observations = ArchiveReads;
    fn observations(&self) -> ArchiveReads {
        ArchiveReads {
            inner: self.inner.observations(),
            started: self.started.clone(),
            success_operation: self.success_operation.clone(),
        }
    }
    fn max_attempts(&self, job: &FundingJob) -> u32 {
        self.inner.max_attempts(job)
    }
    async fn quote_prerequisite(&mut self, job: &FundingJob) -> Result<Quote> {
        self.inner.quote_prerequisite(job).await
    }
    async fn prepare(&mut self, job: &FundingJob, original: &Quote) -> Result<()> {
        self.inner.prepare(job, original).await
    }
    async fn submit(&mut self, job: &FundingJob) -> Result<()> {
        self.inner.submit(job).await
    }
    async fn reconcile(&mut self, job: &FundingJob) -> Result<bool> {
        self.inner.reconcile(job).await
    }
}

#[tokio::test]
async fn held_archived_success_stops_private_replacement_but_preserves_disclosed_source() {
    for stage in ["untouched", "private", "disclosed"] {
        let (dir, mut raw, original) = fixture();
        let current = expire_and_rotate(&mut raw, &original, 3);
        let disclosed = stage == "disclosed";
        if stage == "untouched" {
            raw.save_funding_quote(
                &current.id,
                &serde_json::to_vec(&original_quote(&current)).unwrap(),
            )
            .unwrap();
        } else {
            prepare_original(&mut raw, &current, disclosed);
        }
        raw.disable_pool("archive").unwrap();
        let original_id = original.operation_id.clone();
        let gate = Arc::new(Semaphore::new(0));
        let (started, mut entered) = mpsc::unbounded_channel();
        let (statuses, mut completed) = mpsc::unbounded_channel();
        let (source_started, mut source_events) = mpsc::unbounded_channel();
        let (store, actor) = StoreHandle::spawn(raw);
        let worker = FundingWorker::new(
            store.clone(),
            ArchiveSource {
                started,
                success_operation: original_id.clone(),
                inner: Source {
                    store: store.clone(),
                    gate: Arc::new(Semaphore::new(100)),
                    started: source_started,
                    reads: Reads {
                        credits: Arc::new(Semaphore::new(100)),
                        credit_calls: Default::default(),
                        statuses,
                        status_gates: Arc::new(std::collections::BTreeMap::from([(
                            original_id.clone(),
                            gate.clone(),
                        )])),
                        failed_job: Default::default(),
                        processing: false,
                        quotes: None,
                    },
                },
            },
            60,
        );
        let running = tokio::spawn({
            let worker = worker.clone();
            async move { worker.tick(1000).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while entered.recv().await.unwrap() != original_id {}
        })
        .await
        .unwrap();
        while let Ok(operation) = completed.try_recv() {
            assert_ne!(operation, original_id);
        }
        assert!(
            !running.is_finished(),
            "the exact old status request remains held"
        );
        let current_before = current.clone();
        store
            .call(move |s| {
                assert_eq!(
                    s.funding_job(&current_before.id)?.unwrap().operation_id,
                    current_before.operation_id
                );
                if stage == "untouched" {
                    assert!(s.prepared_bytes(&current_before.operation_id).is_err());
                } else {
                    assert_eq!(
                        s.prepared_bytes(&current_before.operation_id)?.as_slice(),
                        crate::rotation::store::test_support::transaction_raw(203).as_slice()
                    );
                }
                Ok(())
            })
            .await
            .unwrap();
        gate.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(5), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            std::iter::from_fn(|| completed.try_recv().ok())
                .any(|operation| operation == original_id)
        );
        let original_check = original.clone();
        let current_check = current.clone();
        store
            .call(move |s| {
                s.ensure_capacity_group(
                    "archive",
                    &crate::rotation::store::test_support::reserves("5"),
                    Some(alloy_primitives::U256::from(1_000_000)),
                    0,
                )?;
                let facts = s.funding_facts(&current_check.id, &current_check.operation_id)?;
                assert!(facts.claim_fulfilled);
                assert!(
                    !facts.provider_success,
                    "an old result cannot fabricate the new swap's status"
                );
                assert!(!facts.new_funding_allowed);
                if stage == "untouched" {
                    assert!(
                        s.accept_preparation(
                            &current_check.operation_id,
                            Some(&current_check.pool_id),
                            60,
                            None
                        )
                        .is_err()
                    );
                    assert!(s.prepared_bytes(&current_check.operation_id).is_err());
                } else {
                    assert!(
                        s.request_broadcast(&current_check.operation_id, 2, 1, false)
                            .is_err()
                    );
                    assert_eq!(
                        s.operation(&current_check.operation_id)?.attempts,
                        i64::from(disclosed)
                    );
                    assert_eq!(
                        s.prepared_bytes(&current_check.operation_id)?.as_slice(),
                        crate::rotation::store::test_support::transaction_raw(203).as_slice()
                    );
                    assert_eq!(s.operation_pending(&current_check.operation_id)?, disclosed);
                }
                assert_eq!(
                    s.prepared_bytes(&original_check.operation_id)?.as_slice(),
                    crate::rotation::store::test_support::transaction_raw(101).as_slice()
                );
                assert_eq!(s.operation(&original_check.operation_id)?.attempts, 1);
                if disclosed {
                    s.record_destination_readiness(
                        &current_check.id,
                        "1",
                        "initial credit fixture",
                        1,
                    )?;
                    assert!(s.finish_funding_if_proven(
                        &current_check.id,
                        &current_check.operation_id
                    )?);
                    assert_eq!(
                        s.funding_job(&current_check.id)?.unwrap().phase,
                        FundingPhase::Complete
                    );
                    assert!(s.operation_pending(&current_check.operation_id)?);
                }
                Ok(())
            })
            .await
            .unwrap();
        if disclosed {
            while source_events.try_recv().is_ok() {}
            // Job completion is separate from this different disclosed original.
            // This mock proves scheduler demand; the SDK lifecycle case owns the
            // canonical closure assertion for the real recovery implementation.
            let due = {
                let owner = store.funding_owner().lock_owned().await;
                let state = owner.state.lock().unwrap();
                state
                    .records
                    .get(&Key {
                        operation: current.operation_id.clone(),
                        kind: Kind::Source,
                    })
                    .expect("disclosed source retains its observation record")
                    .due
            };
            // The existing UUID-based jitter can put this poll after 1100.
            worker.tick(due.max(1100)).await.unwrap();
            assert_eq!(source_events.try_recv().unwrap(), current.id);
            let current_check = current.clone();
            store
                .call(move |s| {
                    assert!(s.operation_pending(&current_check.operation_id)?);
                    assert_eq!(s.operation(&current_check.operation_id)?.attempts, 1);
                    assert_eq!(
                        s.prepared_bytes(&current_check.operation_id)?.as_slice(),
                        crate::rotation::store::test_support::transaction_raw(203).as_slice()
                    );
                    Ok(())
                })
                .await
                .unwrap();
        }
        drop(worker);
        drop(store);
        actor.await.unwrap();
        let identity = crate::rotation::store::status(&dir.path().join("state"))
            .unwrap()
            .treasury_id;
        let reopened = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &identity,
        )
        .unwrap();
        assert!(
            reopened
                .funding_facts(&current.id, &current.operation_id)
                .unwrap()
                .claim_fulfilled
        );
        assert_eq!(
            reopened
                .prepared_bytes(&original.operation_id)
                .unwrap()
                .as_slice(),
            crate::rotation::store::test_support::transaction_raw(101).as_slice()
        );
    }
}

#[tokio::test]
async fn archived_processing_status_keeps_its_own_completion_floor_after_rollover() {
    let (dir, mut raw, original) = fixture();
    let current = expire_and_rotate(&mut raw, &original, 3);
    raw.disable_pool("archive").unwrap();
    let identity = raw.id().to_owned();
    let (started, mut entered) = mpsc::unbounded_channel();
    let (statuses, mut completed) = mpsc::unbounded_channel();
    let (source_started, _source_events) = mpsc::unbounded_channel();
    let (store, actor) = StoreHandle::spawn(raw);
    let worker = FundingWorker::new(
        store.clone(),
        ArchiveSource {
            started,
            success_operation: String::new(),
            inner: Source {
                store: store.clone(),
                gate: Arc::new(Semaphore::new(100)),
                started: source_started,
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
        },
        60,
    );
    worker.tick(1000).await.unwrap();
    assert_eq!(entered.try_recv().unwrap(), original.operation_id);
    assert_eq!(completed.try_recv().unwrap(), original.operation_id);
    let lease = store.funding_owner().lock_owned().await;
    let due = lease
        .state
        .lock()
        .unwrap()
        .records
        .get(&Key {
            operation: original.operation_id.clone(),
            kind: Kind::Status,
        })
        .unwrap()
        .due;
    drop(lease);
    assert!(due >= 1060);
    worker.tick(due - 1).await.unwrap();
    assert!(entered.try_recv().is_err());
    assert!(completed.try_recv().is_err());
    worker.tick(due).await.unwrap();
    assert_eq!(entered.try_recv().unwrap(), original.operation_id);
    assert_eq!(completed.try_recv().unwrap(), original.operation_id);
    assert!(entered.try_recv().is_err());
    drop(worker);
    drop(store);
    actor.await.unwrap();
    let reopened = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &identity,
    )
    .unwrap();
    let page = reopened
        .funding_status_page(
            &(String::new(), String::new()),
            reopened.funding_work_boundary().unwrap(),
        )
        .unwrap();
    assert_eq!(
        page.iter()
            .filter(|(_, entry)| entry.as_ref().unwrap().is_some())
            .count(),
        1
    );
    assert_eq!(
        reopened
            .funding_job(&current.id)
            .unwrap()
            .unwrap()
            .operation_id,
        current.operation_id
    );
    assert_eq!(
        reopened.operation(&original.operation_id).unwrap().attempts,
        1
    );
    assert_eq!(
        reopened
            .prepared_bytes(&original.operation_id)
            .unwrap()
            .as_slice(),
        crate::rotation::store::test_support::transaction_raw(101).as_slice()
    );
}

#[test]
fn archived_success_closes_legacy_raw_and_preserves_explicit_disclosed_retry() {
    for disclosed in [false, true] {
        let (dir, mut store, original) = fixture();
        let current = expire_and_rotate(&mut store, &original, 3);
        prepare_original(&mut store, &current, disclosed);
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        // A legacy binary could leave an already expired old row pending.
        db.execute(
            "UPDATE outgoing SET state='PREPARED' WHERE id=?1",
            [&original.operation_id],
        )
        .unwrap();
        store
            .record_provider_outcome(
                &original.id,
                &original.operation_id,
                &SwapStatus::Success,
                2,
            )
            .unwrap();
        assert!(!store.operation_pending(&original.operation_id).unwrap());
        assert_eq!(
            store.operation_pending(&current.operation_id).unwrap(),
            disclosed
        );
        assert_eq!(
            store
                .prepared_bytes(&original.operation_id)
                .unwrap()
                .as_slice(),
            crate::rotation::store::test_support::transaction_raw(101).as_slice()
        );
        assert_eq!(store.operation(&original.operation_id).unwrap().attempts, 1);
        assert_eq!(
            store.operation(&current.operation_id).unwrap().attempts,
            i64::from(disclosed)
        );
        assert!(
            store
                .request_broadcast(&current.operation_id, 2, 104, false)
                .is_err()
        );
        if disclosed {
            assert_eq!(
                store
                    .request_broadcast(&current.operation_id, 2, 104, true)
                    .unwrap(),
                2
            );
        } else {
            assert!(
                store
                    .request_broadcast(&current.operation_id, 2, 104, true)
                    .is_err()
            );
        }
        assert_eq!(
            store
                .prepared_bytes(&current.operation_id)
                .unwrap()
                .as_slice(),
            crate::rotation::store::test_support::transaction_raw(203).as_slice()
        );
    }
}

#[test]
fn archived_success_never_hides_orphan_facts_or_ambiguous_job_binding() {
    for corruption in ["orphan_facts", "conflicting_job"] {
        let (dir, mut store, original) = fixture();
        let current = expire_and_rotate(&mut store, &original, 3);
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        if corruption == "orphan_facts" {
            let facts = serde_json::to_string(&TransactionFacts {
                txid: "orphan".into(),
                expiry_height: 203,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: None,
            })
            .unwrap();
            db.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
            db.execute("INSERT INTO treasury_operations(id,facts,submission,attempts) VALUES (?1,?2,'PREPARED',0)", rusqlite::params![current.operation_id,facts]).unwrap();
            db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            store
                .record_provider_outcome(
                    &original.id,
                    &original.operation_id,
                    &SwapStatus::Success,
                    2,
                )
                .unwrap();
            assert!(
                store
                    .funding_facts(&current.id, &current.operation_id)
                    .unwrap()
                    .source_pending
            );
            assert!(
                !store
                    .funding_operation_fulfilled(&current.operation_id)
                    .unwrap()
            );
            assert!(store.prepared_bytes(&current.operation_id).is_err());
        } else {
            let sibling = store
                .funding_jobs()
                .unwrap()
                .into_iter()
                .find(|job| job.id != current.id)
                .unwrap();
            db.execute(
                "UPDATE funding_progress SET operation_id=?2 WHERE job_id=?1",
                rusqlite::params![sibling.id, original.operation_id],
            )
            .unwrap();
            assert!(
                store
                    .funding_operation_fulfilled(&original.operation_id)
                    .is_err()
            );
            assert!(!store.operation_pending(&original.operation_id).unwrap());
        }
        assert_eq!(
            store
                .prepared_bytes(&original.operation_id)
                .unwrap()
                .as_slice(),
            crate::rotation::store::test_support::transaction_raw(101).as_slice()
        );
        assert_eq!(store.operation(&original.operation_id).unwrap().attempts, 1);
    }
}

#[tokio::test]
async fn archived_status_ignores_current_phase_and_health_but_keeps_quote_integrity() {
    let (dir, mut store, original) = fixture();
    let current = expire_and_rotate(&mut store, &original, 3);
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute(
        "UPDATE funding_progress SET phase='invalid' WHERE job_id=?1",
        [&current.id],
    )
    .unwrap();
    db.execute("INSERT INTO funding_health(job_id,error_streak) VALUES (?1,'invalid') ON CONFLICT(job_id) DO UPDATE SET error_streak=excluded.error_streak", [&current.id]).unwrap();
    let routed = store.funding_job(&current.id).unwrap().unwrap();
    assert_eq!(routed.phase, FundingPhase::RecoveryRequired);
    assert_eq!(routed.error_streak, 0);
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT phase FROM funding_progress WHERE job_id=?1",
            [&current.id],
            |r| r.get(0)
        )
        .unwrap(),
        "invalid",
        "runtime routing does not rewrite saved diagnostic fields"
    );
    let page = store
        .funding_status_page(
            &(String::new(), String::new()),
            store.funding_work_boundary().unwrap(),
        )
        .unwrap();
    let captured = page
        .into_iter()
        .filter_map(|(_, entry)| entry.unwrap())
        .find(|job| job.operation_id == original.operation_id)
        .unwrap();
    let exact = store
        .funding_status_quote(&captured.id, &captured.operation_id)
        .unwrap();
    assert_eq!(
        exact.as_slice(),
        serde_json::to_vec(&original_quote(&original)).unwrap()
    );
    let (statuses, mut observed) = mpsc::unbounded_channel();
    let reader = Reads {
        credits: Arc::new(Semaphore::new(0)),
        credit_calls: Default::default(),
        statuses,
        status_gates: Default::default(),
        failed_job: Default::default(),
        processing: false,
        quotes: None,
    };
    let outcome = reader
        .status(serde_json::from_slice(&exact).unwrap())
        .await
        .unwrap();
    assert_eq!(observed.try_recv().unwrap(), original.operation_id);
    store
        .record_provider_outcome(&captured.id, &captured.operation_id, &outcome, 2)
        .unwrap();
    let (phase, success, health): (String, bool, String) = db.query_row("SELECT j.phase,j.provider_success,h.error_streak FROM funding_progress j JOIN funding_health h ON h.job_id=j.job_id WHERE j.job_id=?1", [&current.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        serde_json::from_str::<FundingPhase>(&phase).unwrap(),
        FundingPhase::VerifyingCredit
    );
    assert!(
        !success,
        "archive success is not the replacement's provider report"
    );
    assert_eq!(
        health, "invalid",
        "observational settlement does not invent health facts"
    );
    assert!(
        store
            .funding_facts(&current.id, &current.operation_id)
            .unwrap()
            .claim_fulfilled
    );
    db.execute(
        "UPDATE funding_recovery SET quote=x'00' WHERE operation_id=?1",
        [&original.operation_id],
    )
    .unwrap();
    assert!(
        store
            .funding_status_quote(&original.id, &original.operation_id)
            .is_err(),
        "authenticated quote binding remains mandatory"
    );
}
