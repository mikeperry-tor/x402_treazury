//! Completed SDK results stay with the existing owner until local publication.
use super::*;
use crate::rotation::transaction::{PreparedTransaction, TransactionFacts};

pub(super) const MAINTENANCE_SECONDS: u64 = 60;

pub(super) struct CompletedPreparation {
    pub(super) operation: String,
    expected_revision: i64,
    raw: Zeroizing<Vec<u8>>,
    snapshot: Option<Zeroizing<Vec<u8>>>,
    // Invalid completed bytes are retained without broadcast-ready facts.
    facts: Option<TransactionFacts>,
}
impl Treasury {
    pub(super) async fn retain_calculation(
        &mut self,
        operation: String,
        raw: Zeroizing<Vec<u8>>,
        facts: Result<TransactionFacts>,
    ) -> Result<PreparedTransaction> {
        let (facts, invalid) = match facts {
            Ok(facts) => (Some(facts), None),
            Err(error) => (None, Some(error)),
        };
        self.completed = Some(CompletedPreparation {
            operation,
            expected_revision: self.revision,
            raw,
            snapshot: None,
            facts,
        });
        let prepared = match self.publish_completed().await {
            Ok(value) => value,
            Err(error) => {
                if self.completed.is_some() {
                    diagnostics::report_local_retry(&error, "transaction publication");
                }
                return Err(error);
            }
        };
        match invalid {
            Some(error) => {
                Err(error.context("completed transaction retained without submission authority"))
            }
            None => prepared
                .ok_or_else(|| crate::rotation::transaction::PreparationAlreadyFulfilled.into()),
        }
    }

    /// Only this idempotent local boundary repeats; SDK calculation never does.
    pub(super) async fn publish_completed(&mut self) -> Result<Option<PreparedTransaction>> {
        let completed = self
            .completed
            .as_mut()
            .context("completed result missing")?;
        if completed.snapshot.is_none() {
            completed.snapshot =
                Some(snapshot(self.client.as_ref().context("treasury unavailable")?).await?);
        }
        let id = completed.operation.clone();
        let expected = completed.expected_revision;
        let raw = completed.raw.clone();
        let bytes = completed
            .snapshot
            .as_ref()
            .expect("completed snapshot")
            .clone();
        let facts = completed.facts.clone();
        let validated = facts.is_some();
        let (revision, prepared) = self
            .store
            .call(move |store| {
                let revision = store.prepare_with_facts(&id, expected, &bytes, &raw, facts)?;
                // Acknowledging the durable commit is independent from projecting
                // a still-pending disclosure capability. Only the former repeats.
                let prepared: Result<Option<PreparedTransaction>> = (|| {
                    if store.funding_operation_fulfilled(&id)? {
                        Ok(None)
                    } else {
                        PreparedTransaction::load(store, &id).map(Some)
                    }
                })();
                Ok((revision, prepared))
            })
            .await?;
        self.revision = revision;
        self.completed = None;
        self.healthy = validated && prepared.is_ok();
        prepared
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    async fn fixture(valid: bool) -> (tempfile::TempDir, Treasury, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let mut treasury = Treasury::create(
            dir.path().join("state"),
            dir.path().join("key"),
            2_000_000,
            Some(Zeroizing::new(SEED.into())),
        )
        .await
        .unwrap();
        treasury
            .store
            .call(|s| s.accept_preparation("completed", None, 60, None))
            .await
            .unwrap();
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_completed BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'fixture publication failure'); END;").unwrap();
        treasury.healthy = false;
        let facts = TransactionFacts {
            txid: crate::rotation::store::test_support::transaction_txid(100),
            expiry_height: 100,
            amount_zatoshis: 50,
            fee_zatoshis: 10,
            deadline: None,
        };
        assert!(
            treasury
                .retain_calculation(
                    "completed".into(),
                    Zeroizing::new(crate::rotation::store::test_support::transaction_raw(100)),
                    if valid {
                        Ok(facts)
                    } else {
                        Err(anyhow::anyhow!("fixture validation failure"))
                    }
                )
                .await
                .is_err()
        );
        assert!(treasury.completed.is_some());
        (dir, treasury, db)
    }
    #[tokio::test]
    async fn completed_result_retries_only_identical_local_bytes_after_failure_or_lost_ack() {
        let (_dir, mut treasury, db) = fixture(true).await;
        assert!(!treasury.healthy);
        assert!(treasury.derive_address().await.is_err());
        let original = treasury.completed.as_ref().unwrap().raw.clone();
        let revision = treasury.revision;
        let error = treasury
            .reconstruct_sdk(&CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("completed publication still owned")
        );
        assert_eq!(treasury.revision, revision);
        assert_eq!(
            treasury.completed.as_ref().unwrap().raw.as_slice(),
            original.as_slice()
        );
        db.execute_batch("DROP TRIGGER reject_completed;").unwrap();
        let completion = treasury.completed.as_ref().unwrap();
        let (id, revision, bytes, raw, facts) = (
            completion.operation.clone(),
            completion.expected_revision,
            completion.snapshot.clone().unwrap(),
            completion.raw.clone(),
            completion.facts.clone(),
        );
        // The write commits but its result is lost. Current permission may pause;
        // neither event discards the accepted completion or mints another send.
        treasury
            .store
            .call(move |s| {
                s.prepare_with_facts(&id, revision, &bytes, &raw, facts)?;
                s.deny_new_funding();
                Err::<(), anyhow::Error>(anyhow::anyhow!("fixture lost acknowledgement"))
            })
            .await
            .unwrap_err();
        let saved = treasury.publish_completed().await.unwrap().unwrap();
        assert_eq!(
            saved.bytes(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
        assert_eq!(treasury.revision, 2);
        assert!(treasury.completed.is_none() && treasury.healthy);
        assert!(
            treasury
                .store
                .call(|s| s.request_broadcast("completed", 1, 1, false))
                .await
                .is_err()
        );
        treasury.close().await.unwrap();
    }
    #[tokio::test]
    async fn invalid_completed_bytes_are_durable_without_broadcast_authority() {
        let (_dir, mut treasury, db) = fixture(false).await;
        db.execute_batch("DROP TRIGGER reject_completed;").unwrap();
        let prepared = treasury.publish_completed().await.unwrap().unwrap();
        assert_eq!(
            prepared.bytes(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
        assert!(prepared.facts().is_none());
        assert!(treasury.completed.is_none() && !treasury.healthy);
        assert!(
            treasury
                .store
                .call(|s| s.request_broadcast("completed", 1, 1, false))
                .await
                .is_err()
        );
        treasury.close().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_close_waiter_does_not_discard_completed_result() {
        let (dir, treasury, db) = fixture(true).await;
        let store = treasury.store.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let blocked_store = store.clone();
        let blocker = tokio::spawn(async move {
            blocked_store
                .call(move |_| {
                    entered_tx.send(()).unwrap();
                    release_rx.blocking_recv().unwrap();
                    Ok(())
                })
                .await
                .unwrap();
        });
        entered_rx.await.unwrap();
        let mut close = Box::pin(treasury.close());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut close)
                .await
                .is_err()
        );
        drop(close);
        db.execute_batch("DROP TRIGGER reject_completed;").unwrap();
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if store
                    .call(|s| s.operation_pending("completed"))
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
        assert_eq!(
            store
                .call(|s| s.prepared_bytes("completed"))
                .await
                .unwrap()
                .as_slice(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
        let id = store.call(|s| Ok(s.id().to_owned())).await.unwrap();
        drop(store);
        drop(db);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(store) =
                    Store::open(&dir.path().join("state"), &dir.path().join("key"), &id)
                {
                    assert_eq!(
                        store.prepared_bytes("completed").unwrap().as_slice(),
                        crate::rotation::store::test_support::transaction_raw(100).as_slice()
                    );
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    struct LateFixture {
        dir: tempfile::TempDir,
        owner: Treasury,
        db: rusqlite::Connection,
        job: crate::rotation::store::funding::FundingJob,
        inputs: Vec<crate::rotation::store::preparation::OwnedInput>,
        snapshot: Vec<u8>,
    }
    fn late_facts() -> TransactionFacts {
        TransactionFacts {
            txid: crate::rotation::store::test_support::transaction_txid(100),
            expiry_height: 100,
            amount_zatoshis: 50,
            fee_zatoshis: 10,
            deadline: None,
        }
    }
    fn late_raw() -> Zeroizing<Vec<u8>> {
        Zeroizing::new(crate::rotation::store::test_support::transaction_raw(100))
    }
    async fn late_fixture() -> LateFixture {
        use crate::rotation::store::preparation::{CalculationInputs, InputPool, OwnedInput};
        let dir = tempfile::tempdir().unwrap();
        let owner = Treasury::create(
            dir.path().join("state"),
            dir.path().join("key"),
            2_000_000,
            Some(Zeroizing::new(SEED.into())),
        )
        .await
        .unwrap();
        let pool = owner
            .ensure_capacity_group(
                "late_publication".into(),
                crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )
            .await
            .unwrap();
        owner
            .allocate_capacity_kind(pool, crate::rotation::store::AllocationKind::Small, true)
            .await
            .unwrap();
        let job = owner
            .store
            .call(|s| Ok(s.funding_jobs()?.remove(0)))
            .await
            .unwrap();
        let inputs = vec![OwnedInput {
            pool: InputPool::Orchard,
            txid: crate::rotation::store::test_support::transaction_txid(90),
            output_index: 0,
        }];
        let saved_job = job.clone();
        let saved_inputs = inputs.clone();
        owner
            .store
            .call(move |s| {
                s.save_funding_quote(
                    &saved_job.id,
                    &serde_json::to_vec(&serde_json::json!({
                        "request":{"amount":saved_job.target,"recipient":saved_job.recipient}
                    }))?,
                )?;
                s.accept_preparation(
                    &saved_job.operation_id,
                    Some(&saved_job.pool_id),
                    60,
                    Some(CalculationInputs {
                        destination: "fixture deposit".into(),
                        inputs: saved_inputs,
                    }),
                )
            })
            .await
            .unwrap();
        let snapshot = snapshot(owner.client.as_ref().unwrap())
            .await
            .unwrap()
            .to_vec();
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        LateFixture {
            dir,
            owner,
            db,
            job,
            inputs,
            snapshot,
        }
    }
    async fn observe_delivery(fixture: &LateFixture, success: bool, credit: bool) {
        let job = fixture.job.clone();
        fixture
            .owner
            .store
            .call(move |s| {
                if success {
                    s.record_provider_outcome(
                        &job.id,
                        &job.operation_id,
                        &crate::rotation::near::SwapStatus::Success,
                        2,
                    )?;
                }
                if credit {
                    s.record_funding_balance(
                        &job.id,
                        &job.operation_id,
                        &job.wallet_id,
                        &job.recipient,
                        alloy_primitives::U256::ONE,
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }
    async fn assert_late_evidence(fixture: &LateFixture, pending: bool, valid: bool) {
        let job = fixture.job.clone();
        let inputs = fixture.inputs.clone();
        let expected = fixture.snapshot.clone();
        fixture
            .owner
            .store
            .call(move |s| {
                assert!(s.prepared_bytes(&job.operation_id)?.as_slice() == late_raw().as_slice());
                assert_eq!(s.prepared_inputs(&job.operation_id)?, inputs);
                assert!(s.prepared_snapshot(&job.operation_id)?.as_slice() == expected);
                assert_eq!(s.operation_pending(&job.operation_id)?, pending);
                let refusal = s
                    .request_broadcast(&job.operation_id, 1, 1, false)
                    .unwrap_err();
                assert!(
                    refusal.to_string().contains(if pending {
                        "initial funding claim consumed or disabled"
                    } else {
                        "transaction is not pending"
                    }),
                    "unexpected disclosure refusal: {refusal}"
                );
                if valid {
                    let operation = s.operation(&job.operation_id)?;
                    assert_eq!(operation.attempts, 0);
                    assert_eq!(
                        operation.submission, "PREPARED",
                        "completion cannot fabricate source confirmation"
                    );
                    assert_eq!(operation.facts, late_facts());
                } else {
                    assert!(s.status()?.treasury_operations.is_empty());
                }
                assert_eq!(s.funding_recovery_count(&job.id)?, 0);
                assert_eq!(
                    s.funding_job(&job.id)?.unwrap().operation_id,
                    job.operation_id
                );
                assert_eq!(s.funding_jobs()?.len(), 1);
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn late_bound_success_finishes_actual_publication_without_disclosure_or_retry() {
        if crate::test_log_capture::run_in_child(
            "treasury::publication::tests::late_bound_success_finishes_actual_publication_without_disclosure_or_retry",
        ) {
            return;
        }
        let log = tempfile::NamedTempFile::new().unwrap();
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(log.reopen().unwrap())
            .init();

        use crate::rotation::transaction::{PreparationAlreadyFulfilled, PreparationDeferred};
        for credit in [false, true] {
            let mut fixture = late_fixture().await;
            let revision = fixture.owner.revision;
            observe_delivery(&fixture, true, credit).await;
            let error = fixture
                .owner
                .retain_calculation(
                    fixture.job.operation_id.clone(),
                    late_raw(),
                    Ok(late_facts()),
                )
                .await
                .err()
                .unwrap();
            assert!(error.is::<PreparationAlreadyFulfilled>());
            assert!(!error.is::<PreparationDeferred>());
            assert!(fixture.owner.completed.is_none() && fixture.owner.healthy);
            assert_eq!(fixture.owner.revision, revision + 1);
            assert_late_evidence(&fixture, false, true).await;
            let job = fixture.job.clone();
            fixture
                .owner
                .store
                .call(move |s| {
                    let facts = s.funding_facts(&job.id, &job.operation_id)?;
                    assert!(facts.provider_success && !facts.source_pending);
                    assert_eq!(facts.initial_credit_recorded, credit);
                    assert_eq!(facts.credit_needed(), !credit);
                    assert!(PreparedTransaction::load(s, &job.operation_id).is_err());
                    s.require_preparation_available()
                })
                .await
                .unwrap();
            fixture
                .owner
                .reconstruct_sdk(&CancellationToken::new())
                .await
                .unwrap();
            assert_late_evidence(&fixture, false, true).await;
            fixture.owner.close().await.unwrap();
        }
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert!(!logs.contains("WARN") && !logs.contains("ERROR"), "{logs}");
        assert!(!logs.contains("local retry will continue"), "{logs}");
    }

    #[tokio::test]
    async fn positive_credit_only_publishes_pending_bytes_but_cannot_disclose_them() {
        let mut fixture = late_fixture().await;
        observe_delivery(&fixture, false, true).await;
        let prepared = fixture
            .owner
            .retain_calculation(
                fixture.job.operation_id.clone(),
                late_raw(),
                Ok(late_facts()),
            )
            .await
            .unwrap();
        assert!(prepared.bytes() == late_raw().as_slice());
        assert!(fixture.owner.completed.is_none() && fixture.owner.healthy);
        assert_late_evidence(&fixture, true, true).await;
        let job = fixture.job.clone();
        fixture
            .owner
            .store
            .call(move |s| {
                let facts = s.funding_facts(&job.id, &job.operation_id)?;
                assert!(
                    !facts.provider_success
                        && facts.initial_credit_recorded
                        && facts.source_pending
                );
                assert!(s.require_preparation_available().is_err());
                Ok(())
            })
            .await
            .unwrap();
        fixture.owner.close().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_completed_facts_are_not_masked_by_fulfilled_publication() {
        use crate::rotation::transaction::PreparationAlreadyFulfilled;
        let mut fixture = late_fixture().await;
        observe_delivery(&fixture, true, false).await;
        let error = fixture
            .owner
            .retain_calculation(
                fixture.job.operation_id.clone(),
                late_raw(),
                Err(anyhow::anyhow!("fixture original validation failure")),
            )
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("without submission authority"));
        assert!(format!("{error:#}").contains("fixture original validation failure"));
        assert!(!error.is::<PreparationAlreadyFulfilled>());
        assert!(fixture.owner.completed.is_none() && !fixture.owner.healthy);
        assert_late_evidence(&fixture, false, false).await;
        fixture.owner.close().await.unwrap();
    }

    #[tokio::test]
    async fn lost_publication_ack_then_success_reuses_original_committed_result() {
        let mut fixture = late_fixture().await;
        fixture.db.execute_batch("CREATE TRIGGER reject_late BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'fixture delayed publication'); END;").unwrap();
        assert!(
            fixture
                .owner
                .retain_calculation(
                    fixture.job.operation_id.clone(),
                    late_raw(),
                    Ok(late_facts())
                )
                .await
                .is_err()
        );
        let completion = fixture.owner.completed.as_ref().unwrap();
        let (id, revision, bytes, raw, facts) = (
            completion.operation.clone(),
            completion.expected_revision,
            completion.snapshot.clone().unwrap(),
            completion.raw.clone(),
            completion.facts.clone(),
        );
        fixture
            .db
            .execute_batch("DROP TRIGGER reject_late;")
            .unwrap();
        fixture
            .owner
            .store
            .call(move |s| {
                s.prepare_with_facts(&id, revision, &bytes, &raw, facts)?;
                Err::<(), _>(anyhow::anyhow!("fixture lost acknowledgment"))
            })
            .await
            .unwrap_err();
        observe_delivery(&fixture, true, false).await;
        assert!(fixture.owner.publish_completed().await.unwrap().is_none());
        assert_eq!(fixture.owner.revision, revision + 1);
        assert!(fixture.owner.completed.is_none() && fixture.owner.healthy);
        assert_late_evidence(&fixture, false, true).await;
        fixture.owner.close().await.unwrap();
    }

    fn corrupt_projection(db: &rusqlite::Connection) {
        db.execute_batch("CREATE TRIGGER corrupt_late_projection AFTER INSERT ON treasury_operations BEGIN UPDATE treasury_operations SET facts='not-json' WHERE id=NEW.id; END;").unwrap();
    }
    #[tokio::test]
    async fn postcommit_projection_error_clears_retry_owner_and_can_reconstruct() {
        if crate::test_log_capture::run_in_child(
            "treasury::publication::tests::postcommit_projection_error_clears_retry_owner_and_can_reconstruct",
        ) {
            return;
        }
        let log = tempfile::NamedTempFile::new().unwrap();
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(log.reopen().unwrap())
            .init();

        use crate::rotation::transaction::PreparationAlreadyFulfilled;
        let mut fixture = late_fixture().await;
        let revision = fixture.owner.revision;
        corrupt_projection(&fixture.db);
        let error = fixture
            .owner
            .retain_calculation(
                fixture.job.operation_id.clone(),
                late_raw(),
                Ok(late_facts()),
            )
            .await
            .err()
            .unwrap();
        assert!(!error.is::<PreparationAlreadyFulfilled>());
        assert!(fixture.owner.completed.is_none() && !fixture.owner.healthy);
        assert_eq!(fixture.owner.revision, revision + 1);
        fixture
            .db
            .execute_batch("DROP TRIGGER corrupt_late_projection;")
            .unwrap();
        fixture
            .db
            .execute(
                "UPDATE treasury_operations SET facts=?1 WHERE id=?2",
                rusqlite::params![
                    serde_json::to_string(&late_facts()).unwrap(),
                    fixture.job.operation_id
                ],
            )
            .unwrap();
        fixture
            .owner
            .reconstruct_sdk(&CancellationToken::new())
            .await
            .unwrap();
        let job = fixture.job.clone();
        fixture
            .owner
            .store
            .call(move |s| {
                assert!(s.prepared_bytes(&job.operation_id)?.as_slice() == late_raw().as_slice());
                assert_eq!(s.operation(&job.operation_id)?.attempts, 0);
                assert!(s.operation_pending(&job.operation_id)?);
                Ok(())
            })
            .await
            .unwrap();
        fixture.owner.close().await.unwrap();
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert!(!logs.contains("WARN") && !logs.contains("ERROR"), "{logs}");
        assert!(!logs.contains("local retry will continue"), "{logs}");
    }

    #[tokio::test]
    async fn close_after_committed_projection_error_still_releases_physical_owner() {
        let mut fixture = late_fixture().await;
        // Persist an actual observation so shutdown must transition it Offline;
        // set_sync_phase deliberately does not manufacture a missing observation.
        fixture
            .owner
            .checkpoint(SyncObservation {
                phase: SyncPhase::Preparing,
                last_error: None,
                snapshot_revision: fixture.owner.revision,
                checked_at: None,
                checkpoint_at: 0,
                scanned_blocks: 0,
                target_height: None,
                height: None,
                observed_tip_height: None,
                confirmations: 1,
                confirmed_pool_balances_zatoshis: None,
                confirmed_shielded_zatoshis: 0,
                spendable_shielded_zatoshis: 0,
            })
            .await
            .unwrap();
        fixture.db.execute_batch("CREATE TRIGGER reject_late BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'fixture delayed publication'); END;").unwrap();
        assert!(
            fixture
                .owner
                .retain_calculation(
                    fixture.job.operation_id.clone(),
                    late_raw(),
                    Ok(late_facts())
                )
                .await
                .is_err()
        );
        assert!(fixture.owner.completed.is_some());
        let identity = fixture.owner.status().await.unwrap().treasury_id;
        fixture
            .db
            .execute_batch("DROP TRIGGER reject_late;")
            .unwrap();
        corrupt_projection(&fixture.db);
        // This is the existing local-fixture watchdog, never financial authority.
        // Actual acceptance is the joined close plus successful exclusive reopen.
        assert!(
            tokio::time::timeout(Duration::from_secs(2), fixture.owner.close())
                .await
                .expect("a committed view failure must not enter the publication retry cadence")
                .is_err()
        );
        fixture
            .db
            .execute_batch("DROP TRIGGER corrupt_late_projection;")
            .unwrap();
        fixture
            .db
            .execute(
                "UPDATE treasury_operations SET facts=?1 WHERE id=?2",
                rusqlite::params![
                    serde_json::to_string(&late_facts()).unwrap(),
                    fixture.job.operation_id
                ],
            )
            .unwrap();
        let store = Store::open(
            &fixture.dir.path().join("state"),
            &fixture.dir.path().join("key"),
            &identity,
        )
        .unwrap();
        assert!(
            store
                .prepared_bytes(&fixture.job.operation_id)
                .unwrap()
                .as_slice()
                == late_raw().as_slice()
        );
        assert_eq!(
            store.prepared_inputs(&fixture.job.operation_id).unwrap(),
            fixture.inputs
        );
        assert!(
            store
                .prepared_snapshot(&fixture.job.operation_id)
                .unwrap()
                .as_slice()
                == fixture.snapshot
        );
        assert_eq!(
            store.operation(&fixture.job.operation_id).unwrap().attempts,
            0
        );
        assert!(store.operation_pending(&fixture.job.operation_id).unwrap());
        assert_eq!(
            store.treasury_sync_state().unwrap().sync.unwrap().phase,
            SyncPhase::Offline
        );
    }
}
