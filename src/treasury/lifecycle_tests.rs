//! Local synthetic-chain coverage of the real treasury owner and signed journal.
//! Real proofs/bytes and scans, but no consensus validation or live bridge settlement.
mod birthday;
mod discovery;
mod sapling;
mod scan_bounds;
mod shielding;
mod terminal;
mod worker;
use super::*;
use crate::rotation::{
    near::Quote,
    store::{AllocationKind, funding::FundingJob},
    transaction::{
        BroadcastTransaction, ManagedFundingAuthority, PreparationDeferred, PrepareRequest,
        PreparedTransaction, SubmissionOutcome, TransactionPreparer, TransactionPresence,
        TransactionSubmission,
    },
};
use zingolib::testutils::mock_indexer::{
    Fault, LostSendDestination, MockNet, Rejection, Rpc, faucet_funding_transaction,
};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const DEPOSIT: &str = "t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8";

fn request(job: &FundingJob, deadline: u64) -> PrepareRequest {
    PrepareRequest {
        managed_funding: Some(ManagedFundingAuthority {
            job_id: job.id.clone(),
            auto_fund: true,
            max_attempts: 3,
        }),
        operation_id: job.operation_id.clone(),
        pool_id: Some(job.pool_id.clone()),
        deadline,
        recipient: DEPOSIT.into(),
        amount_zatoshis: 50_000,
        max_fee_zatoshis: 50_000,
        max_input_zatoshis: 100_000,
    }
}

async fn save_quotes(store: &StoreHandle, jobs: Vec<FundingJob>, deadline: u64) {
    store
        .call(move |s| {
            for job in jobs {
                // Bridge evidence is supplied by this fixture; production quote validation
                // remains independently covered. The owner checks this exact binding.
                let quote = Quote {
                    request: serde_json::json!({"amount":job.target,"recipient":job.recipient}),
                    response: serde_json::json!({}),
                    input: 50_000,
                    deadline,
                    deposit: Some(DEPOSIT.into()),
                };
                s.save_funding_quote(&job.id, &serde_json::to_vec(&quote)?)?;
            }
            Ok(())
        })
        .await
        .unwrap();
}

async fn pending(store: &StoreHandle, job: &FundingJob, raw: &[u8]) {
    let operation = job.operation_id.clone();
    let raw = raw.to_vec();
    store
        .call(move |s| {
            assert!(s.status()?.outgoing_pending);
            assert_eq!(PreparedTransaction::load(s, &operation)?.bytes(), raw);
            assert!(s.abandon_unprepared(&operation).is_err());
            assert!(s.require_preparation_available().is_err());
            Ok(())
        })
        .await
        .unwrap();
}

fn accounting(state: &Path, prepared: &PreparedTransaction, confirmed: bool) {
    let db = rusqlite::Connection::open_with_flags(
        state.join("state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (submission, raw, owner): (String,Vec<u8>,Vec<u8>) = db.query_row(
        "SELECT t.submission,o.raw,o.preparation FROM treasury_operations t JOIN outgoing o ON o.id=t.id WHERE t.id=?1",
        [prepared.operation_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(!raw.is_empty() && !owner.is_empty());
    assert_eq!(submission == "CONFIRMED", confirmed);
    assert!(prepared.facts().unwrap().fee_zatoshis > 0);
}

// Pause after durable intent, while retaining the real production sender.
struct GatedSubmission {
    sender: submission::GrpcSubmission,
    entered: CancellationToken,
    release: CancellationToken,
}
impl TransactionSubmission for GatedSubmission {
    async fn preflight(&mut self, tx: &PreparedTransaction, retry: bool) -> Result<()> {
        self.sender.preflight(tx, retry).await
    }
    async fn submit(&mut self, tx: BroadcastTransaction) -> Result<SubmissionOutcome> {
        self.entered.cancel();
        self.release.cancelled().await;
        self.sender.submit(tx).await
    }
    async fn lookup(&mut self, tx: &PreparedTransaction) -> Result<TransactionPresence> {
        self.sender.lookup(tx).await
    }
}

async fn funded_fixture(confirmations: u64) -> (tempfile::TempDir, MockNet, Treasury, String) {
    funded_fixture_amount(confirmations, 1_000_000).await
}

async fn funded_fixture_amount(
    confirmations: u64,
    amount: u64,
) -> (tempfile::TempDir, MockNet, Treasury, String) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let net = MockNet::launch().await;
    let endpoint = net.indexer_uri().to_string();
    let mut owner = Treasury::create_with_network(
        state.clone(),
        key.clone(),
        1,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let address = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let funding = faucet_funding_transaction(vec![(&address, amount, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_empty_blocks(20);
        chain.mine_block(vec![funding]);
        chain.mine_empty_blocks(2);
    }
    let settings = SyncSettings::new(endpoint.clone(), confirmations).unwrap();
    owner.configure_sync(settings.clone());
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    (dir, net, owner, endpoint)
}

#[tokio::test]
async fn inclusion_after_wallet_scan_waits_without_resubmitting_or_releasing_inputs() {
    struct MineDuringLookup {
        net: MockNet,
        sender: submission::GrpcSubmission,
        mined: bool,
        sends: usize,
    }
    impl TransactionSubmission for MineDuringLookup {
        async fn preflight(&mut self, tx: &PreparedTransaction, retry: bool) -> Result<()> {
            self.sender.preflight(tx, retry).await
        }
        async fn submit(&mut self, tx: BroadcastTransaction) -> Result<SubmissionOutcome> {
            self.sends += 1;
            self.sender.submit(tx).await
        }
        async fn lookup(&mut self, tx: &PreparedTransaction) -> Result<TransactionPresence> {
            if !self.mined {
                self.net.chain.write().await.mine_mempool();
                self.mined = true;
            }
            self.sender.lookup(tx).await
        }
    }
    let (_dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let pool = owner
        .ensure_capacity_group(
            "scan_race".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    owner
        .allocate_capacity_kind(pool, AllocationKind::Small, true)
        .await
        .unwrap();
    let job = owner
        .store
        .call(|s| s.funding_jobs())
        .await
        .unwrap()
        .remove(0);
    let deadline = now().unwrap() + 7200;
    save_quotes(&owner.store, vec![job.clone()], deadline).await;
    let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
    let raw = prepared.bytes().to_vec();
    let mut sender = MineDuringLookup {
        net,
        sender: submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        mined: false,
        sends: 0,
    };
    let stop = CancellationToken::new();
    owner
        .submit_prepared(job.operation_id.clone(), &mut sender, false, &stop)
        .await
        .unwrap();
    let error = owner
        .reconcile_for_observation(job.operation_id.clone(), &mut sender, &stop)
        .await
        .unwrap_err();
    assert!(
        error.is::<crate::rotation::transaction::RecoveryEvidenceError>(),
        "{error:#}"
    );
    assert_eq!(
        recovery::evidence_outcome(&error),
        Some(recovery::RecoveryOutcome::Waiting(
            "zcash_confirmations_pending"
        ))
    );
    pending(&owner.store, &job, &raw).await;
    let before = owner.status().await.unwrap();
    assert_eq!(before.treasury_operations[0].attempts, 1);
    assert_eq!(sender.sends, 1);
    assert_eq!(before.funding_jobs[0].operation_id, job.operation_id);
    sender.net.chain.write().await.mine_empty_blocks(1);
    owner.sync_once(&stop).await.unwrap();
    assert!(matches!(
        owner
            .reconcile_for_observation(job.operation_id, &mut sender, &stop)
            .await
            .unwrap(),
        TransactionPresence::Confirmed { .. }
    ));
    let after = owner.status().await.unwrap();
    assert!(!after.outgoing_pending);
    assert_eq!(after.treasury_operations[0].submission, "CONFIRMED");
    assert_eq!(after.treasury_operations[0].attempts, 1);
    assert_eq!(sender.sends, 1);
    assert_eq!(after.funding_jobs.len(), before.funding_jobs.len());
    owner.close().await.unwrap();
}

#[tokio::test]
async fn unsubmitted_funding_expiry_uses_only_its_one_proof_scan() {
    struct ReadOnly;
    impl TransactionSubmission for ReadOnly {
        async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
            panic!("recovery must not submit");
        }
        async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
            panic!("canonical expiry proof needs no external inclusion corroboration");
        }
    }
    for (malformed_sequencing, warm_cached) in [(false, false), (true, false), (true, true)] {
        let (dir, net, mut owner, _endpoint) = funded_fixture(2).await;
        let pool = owner
            .ensure_capacity_group(
                "expiry".into(),
                crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )
            .await
            .unwrap();
        owner
            .allocate_capacity_kind(pool, AllocationKind::Small, true)
            .await
            .unwrap();
        let jobs = owner.store.call(|s| s.funding_jobs()).await.unwrap();
        let job = jobs[0].clone();
        let deadline = now().unwrap() + 7200;
        save_quotes(&owner.store, jobs, deadline).await;
        let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
        let expiry = prepared.facts().unwrap().expiry_height;
        {
            let mut chain = net.chain.write().await;
            let blocks = expiry + 2 - chain.tip();
            chain.mine_empty_blocks(blocks);
        }
        if warm_cached {
            // A successful cached observation is not a new proof scan in this
            // invocation. Recovery must still acquire one actual canonical scan.
            owner.sync_once(&CancellationToken::new()).await.unwrap();
        }
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE fixture_completed_scans(observation TEXT NOT NULL);
            CREATE TRIGGER record_completed_scan AFTER INSERT ON treasury_sync
            WHEN json_extract(NEW.observation,'$.phase')='ready'
            BEGIN INSERT INTO fixture_completed_scans VALUES(NEW.observation); END;",
        )
        .unwrap();
        if malformed_sequencing {
            db.execute_batch("PRAGMA ignore_check_constraints=ON")
                .unwrap();
            db.execute(
                "UPDATE treasury_operations SET attempts='invalid',submission=x'00' WHERE id=?1",
                [&job.operation_id],
            )
            .unwrap();
            db.execute("UPDATE funding_progress SET attempts='bad',next_poll='bad',last_error=x'00',phase='invalid' WHERE job_id=?1",[&job.id]).unwrap();
            db.execute("INSERT INTO funding_health(job_id,started_at,error_streak,timed_out) VALUES(?1,'bad','bad','bad') ON CONFLICT(job_id) DO UPDATE SET started_at='bad',error_streak='bad',timed_out='bad'",[&job.id]).unwrap();
            db.execute_batch("PRAGMA ignore_check_constraints=OFF")
                .unwrap();
        }
        let original_operation = job.operation_id.clone();
        let mut sender = ReadOnly;
        assert_eq!(
            owner
                .recover_funding_job(
                    job.id.clone(),
                    3,
                    false,
                    &mut sender,
                    &CancellationToken::new()
                )
                .await
                .unwrap(),
            recovery::RecoveryOutcome::Waiting("funding_disabled")
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM fixture_completed_scans", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1,
            "one completed canonical proof scan suffices; do not scan just to decide whether to scan"
        );
        owner
            .store
            .call(move |s| {
                assert_eq!(
                    s.funding_job(&job.id)?.unwrap().operation_id,
                    job.operation_id
                );
                assert_eq!(
                    s.observation_operation(&job.operation_id)?.submission,
                    "EXPIRED"
                );
                if malformed_sequencing {
                    assert!(s.operation(&job.operation_id).is_err());
                } else {
                    assert_eq!(s.operation(&job.operation_id)?.attempts, 0);
                }
                assert_eq!(
                    s.prepared_bytes(&job.operation_id)?.as_slice(),
                    prepared.bytes()
                );
                assert!(!s.operation_pending(&job.operation_id)?);
                Ok(())
            })
            .await
            .unwrap();
        if malformed_sequencing {
            let diagnostic: rusqlite::types::Value = db
                .query_row(
                    "SELECT attempts FROM funding_progress WHERE operation_id=?1",
                    [&original_operation],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                diagnostic,
                rusqlite::types::Value::Text("bad".into()),
                "canonical recovery does not rewrite diagnostic counters"
            );
        }
        let sequencing: (rusqlite::types::Value, rusqlite::types::Value) = db
            .query_row(
                "SELECT attempts,submission FROM treasury_operations WHERE id=?1",
                [&original_operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            sequencing,
            if malformed_sequencing {
                (
                    rusqlite::types::Value::Text("invalid".into()),
                    rusqlite::types::Value::Blob(vec![0]),
                )
            } else {
                (
                    rusqlite::types::Value::Integer(0),
                    rusqlite::types::Value::Text("PREPARED".into()),
                )
            },
            "canonical expiry closes ownership without rewriting sequencing reports"
        );
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn concurrent_funding_reuses_scans_preserves_pending_bytes_and_resumes_after_restart() {
    let (dir, net, owner, endpoint) = funded_fixture(2).await;
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let settings = SyncSettings::new(endpoint.clone(), 2).unwrap();
    assert!(
        owner
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .spendable_shielded_zatoshis
            >= 1_000_000
    );
    for name in ["alpha", "beta"] {
        let pool = owner
            .ensure_capacity_group(
                name.into(),
                crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )
            .await
            .unwrap();
        owner
            .allocate_capacity_kind(pool, AllocationKind::Small, true)
            .await
            .unwrap();
    }
    let id = owner.status().await.unwrap().treasury_id;
    let store = owner.store_handle();
    let jobs = store.call(|s| s.funding_jobs()).await.unwrap();
    assert_eq!(jobs.len(), 2);
    let deadline = now().unwrap() + 7200;
    save_quotes(&store, jobs.clone(), deadline).await;
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    let sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint.clone(),
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    // Keep paused time stable while SDK scans use their private runtime and
    // Store publications finish on blocking workers.
    let active = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    for wrong_binding in [false, true] {
        let mut rejected = request(&jobs[0], deadline);
        if wrong_binding {
            rejected.pool_id = Some(jobs[1].pool_id.clone());
        } else {
            rejected.managed_funding.as_mut().unwrap().auto_fund = false;
        }
        let error = handle.prepare(rejected).await.err().unwrap();
        assert!(error.is::<PreparationDeferred>(), "{error:#}");
        assert!(format!("{error:#}").contains(if wrong_binding {
            "funding source pool binding mismatch"
        } else {
            "funding_disabled"
        }));
        let operation = jobs[0].operation_id.clone();
        store
            .call(move |s| s.require_unstarted_preparation(&operation))
            .await
            .unwrap();
    }
    // Two independently bound recipients compete for the same source wallet.
    let (a, b) = tokio::join!(
        handle.prepare(request(&jobs[0], deadline)),
        handle.prepare(request(&jobs[1], deadline))
    );
    let (first, second, prepared, deferred) = match (a, b) {
        (Ok(p), Err(e)) => (&jobs[0], &jobs[1], p, e),
        (Err(e), Ok(p)) => (&jobs[1], &jobs[0], p, e),
        _ => panic!("exactly one preparation must win the source gate"),
    };
    assert!(deferred.is::<PreparationDeferred>(), "{deferred:#}");
    assert_eq!(
        prepared.network_identity().unwrap(),
        crate::network::IsolationId::evm(&first.recipient).unwrap()
    );
    accounting(&state, &prepared, false);
    let unstarted = second.operation_id.clone();
    store
        .call(move |s| s.require_unstarted_preparation(&unstarted))
        .await
        .unwrap();
    assert_eq!(
        handle
            .submit(first.operation_id.clone(), false)
            .await
            .unwrap(),
        SubmissionOutcome::Accepted
    );
    assert!(
        handle
            .submit(first.operation_id.clone(), false)
            .await
            .unwrap_err()
            .to_string()
            .contains("explicit_rebroadcast_required")
    );
    let before = store.call(|s| s.treasury_sync_state()).await.unwrap();
    // This indexer exposes mined transactions only; absence cannot release a send.
    for _ in 0..3 {
        assert_eq!(
            handle.reconcile(first.operation_id.clone()).await.unwrap(),
            TransactionPresence::Absent
        );
        pending(&store, first, prepared.bytes()).await;
    }
    let after = store.call(|s| s.treasury_sync_state()).await.unwrap();
    assert_eq!(before.snapshot_revision, after.snapshot_revision);
    assert_eq!(
        before.sync.unwrap().checked_at,
        after.sync.unwrap().checked_at
    );
    net.chain.write().await.mine_mempool();
    tokio::time::advance(Duration::from_secs(90)).await;
    assert!(
        handle
            .reconcile(first.operation_id.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("treasury_confirmation_pending")
    );
    pending(&store, first, prepared.bytes()).await;
    net.chain.write().await.mine_empty_blocks(1);
    tokio::time::advance(Duration::from_secs(90)).await;
    assert!(matches!(
        handle.reconcile(first.operation_id.clone()).await.unwrap(),
        TransactionPresence::Confirmed { .. }
    ));
    assert!(
        !store
            .call(|s| Ok(s.status()?.outgoing_pending))
            .await
            .unwrap()
    );
    accounting(&state, &prepared, true);
    let second_prepared = handle.prepare(request(second, deadline)).await.unwrap();
    assert_eq!(
        second_prepared.network_identity().unwrap(),
        crate::network::IsolationId::evm(&second.recipient).unwrap()
    );
    assert_ne!(
        prepared.network_identity().unwrap(),
        second_prepared.network_identity().unwrap()
    );
    accounting(&state, &second_prepared, false);
    let first_id = first.operation_id.clone();
    let second_id = second.operation_id.clone();
    stop.cancel();
    drop(store);
    task.await.unwrap().unwrap();
    // Restart must retain ownership, exact signed bytes and recipient identity.
    assert!(
        Treasury::open(state.clone(), key.clone(), id.clone())
            .await
            .is_err()
    );
    let mut owner = Treasury::open_with_network(state.clone(), key, id, TreasuryNetwork::Synthetic)
        .await
        .unwrap();
    owner.configure_sync(settings);
    let store = owner.store_handle();
    pending(&store, second, second_prepared.bytes()).await;
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    let sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    let entered = CancellationToken::new();
    let release = CancellationToken::new();
    let sender = GatedSubmission {
        sender,
        entered: entered.clone(),
        release: release.clone(),
    };
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    net.chain.write().await.lose_next_send_response = Some(LostSendDestination::Mempool);
    let caller = handle.clone();
    let operation = second_id.clone();
    let waiter = tokio::spawn(async move { caller.submit(operation, false).await });
    tokio::time::timeout(Duration::from_secs(30), entered.cancelled())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let operation = second_id.clone();
    store
        .call(move |s| {
            assert_eq!(s.operation(&operation)?.submission, "BROADCAST_REQUESTED");
            Ok(())
        })
        .await
        .unwrap();
    pending(&store, second, second_prepared.bytes()).await;
    release.cancel();
    for _ in 0..2 {
        assert_eq!(
            handle.reconcile(second_id.clone()).await.unwrap(),
            TransactionPresence::Absent
        );
        pending(&store, second, second_prepared.bytes()).await;
    }
    accounting(&state, &second_prepared, false);
    let operation = second_id.clone();
    store
        .call(move |s| {
            assert_eq!(s.operation(&operation)?.submission, "UNKNOWN");
            assert_eq!(s.operation(&operation)?.attempts, 1);
            Ok(())
        })
        .await
        .unwrap();
    net.chain.write().await.mine_mempool();
    net.chain.write().await.mine_empty_blocks(1);
    tokio::time::advance(Duration::from_secs(90)).await;
    assert!(matches!(
        handle.reconcile(second_id.clone()).await.unwrap(),
        TransactionPresence::Confirmed { .. }
    ));
    store
        .call(move |s| {
            for operation in [first_id, second_id] {
                let operation = s.operation(&operation)?;
                assert_eq!(operation.submission, "CONFIRMED");
                assert_eq!(
                    operation.attempts, 1,
                    "observation must never replay a send"
                );
            }
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    accounting(&state, &second_prepared, true);
    let pending_job = store
        .call(|s| {
            let pool = s.ensure_capacity_group(
                "pending",
                &crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )?;
            let job = s
                .allocate_capacity_kind(&pool, AllocationKind::Small, true)?
                .unwrap();
            Ok(s.funding_job(&job)?.unwrap())
        })
        .await
        .unwrap();
    save_quotes(&store, vec![pending_job.clone()], deadline).await;
    let pending_prepared = handle
        .prepare(request(&pending_job, deadline))
        .await
        .unwrap();
    // Preparation invalidates reuse; establish an unchanged, fresh scan before
    // the fork so submission must detect it through the reuse probe itself.
    handle.sync().await.unwrap();
    accounting(&state, &pending_prepared, false);
    // A same-height fork removes both confirmed source payments. The reuse
    // probe must notice the changed hash even though height and age still match.
    let before = store.call(|s| s.treasury_sync_state()).await.unwrap();
    {
        let mut chain = net.chain.write().await;
        assert!(chain.validate(pending_prepared.bytes()).is_ok());
        let tip = chain.tip();
        assert_eq!(chain.reorg_to(23).len(), 2);
        chain.mine_empty_blocks(tip - 23);
        assert_eq!(chain.tip(), tip);
        assert!(matches!(
            chain.validate(pending_prepared.bytes()),
            Err(Rejection::UnknownAnchor(_))
        ));
        // This stream-only marker is inert for unary SendTransaction. Its
        // consumption proves the RPC reached the mock's strict validator.
        chain
            .faults
            .inject(Rpc::SendTransaction, Fault::EndStream { after: 0 });
        assert_eq!(chain.faults.pending(Rpc::SendTransaction), 1);
    }
    assert_eq!(
        handle
            .submit(pending_job.operation_id.clone(), false)
            .await
            .unwrap(),
        SubmissionOutcome::Unknown
    );
    assert_eq!(
        net.chain.read().await.faults.pending(Rpc::SendTransaction),
        0
    );
    let after = store.call(|s| s.treasury_sync_state()).await.unwrap();
    assert!(
        after.snapshot_revision > before.snapshot_revision,
        "reorg must force a scan"
    );
    assert_eq!(after.sync.unwrap().phase, SyncPhase::Ready);
    // Historical source facts do not gate a best-effort send. The fork removed
    // its anchor, so the validator rejected it; the exact pending transaction
    // still owns its inputs and blocks another calculation.
    let third = store
        .call(|s| {
            let pool = s.ensure_capacity_group(
                "gamma",
                &crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )?;
            let job = s
                .allocate_capacity_kind(&pool, AllocationKind::Small, true)?
                .unwrap();
            Ok(s.funding_job(&job)?.unwrap())
        })
        .await
        .unwrap();
    save_quotes(&store, vec![third.clone()], deadline).await;
    let error = handle
        .prepare(request(&third, deadline))
        .await
        .err()
        .unwrap();
    assert!(error.is::<PreparationDeferred>(), "{error:#}");
    let unstarted = third.operation_id;
    let originals = [first.operation_id.clone(), second.operation_id.clone()];
    store
        .call(move |s| {
            s.require_unstarted_preparation(&unstarted)?;
            for id in originals {
                let operation = s.operation(&id)?;
                assert_eq!(operation.submission, "CONFIRMED");
                assert!(!s.operation_pending(&id)?);
                assert!(!s.source_previously_closed(&id)?);
                assert_eq!(operation.attempts, 1, "reorg detection cannot rebroadcast");
            }
            Ok(())
        })
        .await
        .unwrap();
    accounting(&state, &prepared, true);
    accounting(&state, &second_prepared, true);
    accounting(&state, &pending_prepared, false);
    pending(&store, &pending_job, pending_prepared.bytes()).await;
    let operation = pending_job.operation_id;
    store
        .call(move |s| {
            assert_eq!(
                s.operation(&operation)?.attempts,
                1,
                "the unchanged pending bytes were submitted once"
            );
            Ok(())
        })
        .await
        .unwrap();
    stop.cancel();
    drop(store);
    task.await.unwrap().unwrap();
    tokio::time::resume();
    active.abort();
}

#[tokio::test]
async fn same_height_tip_reorg_rebuilds_sdk_without_reopening_historical_source_work() {
    let (dir, net, mut owner, endpoint) = funded_fixture(1).await;
    let pool = owner
        .ensure_capacity_group(
            "tip".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    owner
        .allocate_capacity_kind(pool, AllocationKind::Small, true)
        .await
        .unwrap();
    let store = owner.store_handle();
    let job = store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let deadline = now().unwrap() + 7200;
    save_quotes(&store, vec![job.clone()], deadline).await;
    let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
    let stop = CancellationToken::new();
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    assert_eq!(
        owner
            .submit_prepared(job.operation_id.clone(), &mut sender, false, &stop)
            .await
            .unwrap(),
        SubmissionOutcome::Accepted
    );
    net.chain.write().await.mine_mempool();
    assert_eq!(
        owner
            .reconcile_prepared(job.operation_id.clone(), &mut sender, &stop)
            .await
            .unwrap(),
        TransactionPresence::Confirmed { height: 24 }
    );
    accounting(&dir.path().join("state"), &prepared, true);
    let id = job.operation_id.clone();
    let original_inputs = store.call(move |s| s.prepared_inputs(&id)).await.unwrap();
    {
        let mut chain = net.chain.write().await;
        assert_eq!(chain.reorg_to(23), vec![prepared.bytes().to_vec()]);
        chain.mine_empty_blocks(1);
    }
    owner.sync_for_command(&stop).await.unwrap();
    assert_eq!(
        owner.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Ready
    );
    let id = job.operation_id.clone();
    let expected_inputs = original_inputs.clone();
    let original_bytes = prepared.bytes().to_vec();
    store
        .call(move |s| {
            assert_eq!(s.operation(&id)?.attempts, 1);
            assert_eq!(s.operation(&id)?.submission, "CONFIRMED");
            assert!(!s.operation_pending(&id)?);
            assert!(!s.source_previously_closed(&id)?);
            assert_eq!(s.prepared_inputs(&id)?, expected_inputs);
            assert_eq!(s.prepared_bytes(&id)?.as_slice(), original_bytes);
            s.require_preparation_available()?;
            Ok(())
        })
        .await
        .unwrap();
    accounting(&dir.path().join("state"), &prepared, true);
    // Canonical reappearance updates SDK state without rebroadcasting or
    // rewriting the original immutable transaction facts.
    {
        let mut chain = net.chain.write().await;
        chain.reorg_to(23);
        chain.mine_block(vec![prepared.bytes().to_vec()]);
    }
    owner.sync_for_command(&stop).await.unwrap();
    assert_eq!(
        owner.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Ready
    );
    accounting(&dir.path().join("state"), &prepared, true);
    let id = job.operation_id;
    let original_bytes = prepared.bytes().to_vec();
    store
        .call(move |s| {
            assert_eq!(
                s.operation(&id)?.attempts,
                1,
                "canonical return cannot rebroadcast"
            );
            assert_eq!(s.prepared_inputs(&id)?, original_inputs);
            assert_eq!(s.prepared_bytes(&id)?.as_slice(), original_bytes);
            assert!(!s.operation_pending(&id)?);
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    owner.close().await.unwrap();
}

#[tokio::test]
async fn sdk_reorg_rescan_preserves_original_signed_work() {
    use crate::rotation::store::preparation::{InputPool, OwnedInput};
    use pepper_sync::wallet::{
        IronwoodNote, OrchardNote, OutputInterface, SaplingNote, TransparentCoin,
    };
    use shardtree::store::ShardStore as _;
    fn canonical_input<T: OutputInterface>(
        wallet: &zingolib::wallet::LightWallet,
        input: &OwnedInput,
    ) -> (u64, Option<zcash_primitives::transaction::TxId>) {
        let id = zcash_primitives::transaction::TxId::from_hex(&input.txid).unwrap();
        let output = wallet
            .wallet_outputs::<T>()
            .into_iter()
            .find(|output| {
                output.output_id().txid() == id
                    && output.output_id().output_index() == input.output_index
            })
            .expect("the exact original funding output must be reconstructed");
        assert_eq!(
            wallet
                .output_transaction(output)
                .status()
                .get_confirmed_height()
                .map(u32::from),
            Some(21),
            "the original funding transaction must remain canonically confirmed"
        );
        (output.value(), output.spending_transaction())
    }
    for (rollback, old_checkpoint_window, same_height_fork) in [
        (pepper_sync::sync::MAX_REORG_ALLOWANCE - 1, false, false),
        (pepper_sync::sync::MAX_REORG_ALLOWANCE - 1, true, false),
        (pepper_sync::sync::MAX_REORG_ALLOWANCE + 1, false, false),
        (pepper_sync::sync::MAX_REORG_ALLOWANCE + 1, false, true),
    ] {
        let (dir, net, mut owner, _endpoint) = funded_fixture(1).await;
        net.chain.write().await.mine_empty_blocks(rollback);
        owner.sync_once(&CancellationToken::new()).await.unwrap();
        let pool = owner
            .ensure_capacity_group(
                "rollback".into(),
                crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )
            .await
            .unwrap();
        owner
            .allocate_capacity_kind(pool, AllocationKind::Small, true)
            .await
            .unwrap();
        let mut store = owner.store_handle();
        let job = store
            .call(|s| Ok(s.funding_jobs()?.remove(0)))
            .await
            .unwrap();
        let deadline = now().unwrap() + 7200;
        save_quotes(&store, vec![job.clone()], deadline).await;
        let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
        let addresses = owner.addresses().await.unwrap();
        let original_inputs = {
            let operation = job.operation_id.clone();
            store
                .call(move |s| s.prepared_inputs(&operation))
                .await
                .unwrap()
        };
        let settings = owner.sync_settings.clone().unwrap();
        let treasury_id = owner.status().await.unwrap().treasury_id;
        if old_checkpoint_window {
            // A retained rollback boundary may be absent from an older saved
            // tree even while newer checkpoints and the actual note data exist.
            // Stay inside the current cache horizon so this exercises the exact
            // missing-checkpoint fallback rather than the known-deep-tip reset.
            {
                let mut wallet = owner.client.as_ref().unwrap().wallet().write().await;
                let tree = &mut wallet.shard_trees.orchard;
                let tip = tree.store().max_checkpoint_id().unwrap().unwrap();
                let rollback_height = 23.into();
                assert!(
                    tree.store()
                        .get_checkpoint(&rollback_height)
                        .unwrap()
                        .is_some()
                );
                tree.store_mut()
                    .remove_checkpoint(&rollback_height)
                    .unwrap();
                assert!(
                    tree.store()
                        .get_checkpoint(&rollback_height)
                        .unwrap()
                        .is_none()
                );
                assert_eq!(tree.store().max_checkpoint_id().unwrap(), Some(tip));
            }
            let observation = store
                .call(|s| Ok(s.treasury_sync_state()?.sync.unwrap()))
                .await
                .unwrap();
            owner.checkpoint(observation).await.unwrap();
            drop(store);
            owner.close().await.unwrap();
            owner = Treasury::open_with_network(
                dir.path().join("state"),
                dir.path().join("key"),
                treasury_id.clone(),
                TreasuryNetwork::Synthetic,
            )
            .await
            .unwrap();
            owner.configure_sync(settings.clone());
            store = owner.store_handle();
        }
        {
            let mut chain = net.chain.write().await;
            let tip = chain.tip();
            chain.reorg_to(23);
            if same_height_fork {
                chain.mine_empty_blocks(tip - 23);
            }
        }
        if old_checkpoint_window || same_height_fork {
            // Existing SDK truncation/fork recovery checkpoints its cleared
            // derived state even when this scan reports the recovery error.
            assert!(owner.sync_once(&CancellationToken::new()).await.is_err());
            pending(&store, &job, prepared.bytes()).await;
            drop(store);
            owner.close().await.unwrap();
            owner = Treasury::open_with_network(
                dir.path().join("state"),
                dir.path().join("key"),
                treasury_id,
                TreasuryNetwork::Synthetic,
            )
            .await
            .unwrap();
            owner.configure_sync(settings);
            store = owner.store_handle();
        }
        owner.sync_once(&CancellationToken::new()).await.unwrap();
        assert_eq!(owner.addresses().await.unwrap(), addresses);
        let synced = owner.status().await.unwrap().sync.unwrap();
        assert_eq!(synced.phase, SyncPhase::Ready);
        let (funding_value, reservation) = {
            let wallet = owner.client.as_ref().unwrap().wallet().read().await;
            assert_eq!(
                original_inputs.len(),
                1,
                "this fixture selects its one original funding note"
            );
            let input = &original_inputs[0];
            match &input.pool {
                InputPool::Transparent => canonical_input::<TransparentCoin>(&wallet, input),
                InputPool::Sapling => canonical_input::<SaplingNote>(&wallet, input),
                InputPool::Orchard => canonical_input::<OrchardNote>(&wallet, input),
                InputPool::Ironwood => canonical_input::<IronwoodNote>(&wallet, input),
            }
        };
        assert_eq!(
            funding_value, 1_000_000,
            "a Ready rescan must reconstruct the exact canonical owned value"
        );
        // The SDK advisory confirmed balance excludes notes reserved by a
        // Calculated transaction. A shallow rollback can retain that original
        // SDK reservation; a full clear reconstructs the note without it.
        let expected_balance = if let Some(spender) = reservation {
            assert_eq!(
                spender.to_string(),
                prepared.facts().unwrap().txid,
                "only the exact accepted original may reserve this funding note"
            );
            0
        } else {
            funding_value
        };
        assert_eq!(synced.confirmed_shielded_zatoshis, expected_balance);
        pending(&store, &job, prepared.bytes()).await;
        let operation = job.operation_id.clone();
        store
            .call(move |s| {
                assert_eq!(s.prepared_inputs(&operation)?, original_inputs);
                assert_eq!(s.operation(&operation)?.attempts, 0);
                assert_eq!(s.funding_jobs()?.len(), 1);
                Ok(())
            })
            .await
            .unwrap();
        // A second scan must finish from the repaired snapshot rather than
        // restoring the stale tip or abandoning the original input owner.
        owner.sync_once(&CancellationToken::new()).await.unwrap();
        pending(&store, &job, prepared.bytes()).await;
        drop(store);
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn sdk_confirmation_finishes_original_source_without_indexer_agreement_or_replay() {
    struct StaleLookup {
        presence: TransactionPresence,
        lookups: usize,
    }
    impl TransactionSubmission for StaleLookup {
        async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
            panic!("recovery cannot resubmit signed work");
        }
        async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
            self.lookups += 1;
            Ok(match &self.presence {
                TransactionPresence::Confirmed { height } => {
                    TransactionPresence::Confirmed { height: *height }
                }
                _ => TransactionPresence::Unknown,
            })
        }
    }
    for (presence, journaled_submission, alternate_authorization, malformed_sequencing) in [
        (TransactionPresence::Unknown, false, false, false),
        (
            TransactionPresence::Confirmed { height: 23 },
            true,
            false,
            false,
        ),
        (TransactionPresence::Unknown, false, true, false),
        (TransactionPresence::Unknown, false, false, true),
    ] {
        let (dir, net, mut owner, endpoint) = funded_fixture(1).await;
        let pool = owner
            .ensure_capacity_group(
                "confirmation".into(),
                crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )
            .await
            .unwrap();
        owner
            .allocate_capacity_kind(pool, AllocationKind::Small, true)
            .await
            .unwrap();
        let store = owner.store_handle();
        let job = store
            .call(|s| Ok(s.funding_jobs()?.remove(0)))
            .await
            .unwrap();
        let deadline = now().unwrap() + 7200;
        save_quotes(&store, vec![job.clone()], deadline).await;
        let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
        let stop = CancellationToken::new();
        let mut actual = submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap();
        if journaled_submission {
            owner
                .submit_prepared(job.operation_id.clone(), &mut actual, false, &stop)
                .await
                .unwrap();
            net.chain.write().await.mine_mempool();
        } else {
            // This matching fixture does not validate consensus signatures.
            // ZIP 244 permits authorizing-data variants with the same effects.
            let mut canonical = prepared.bytes().to_vec();
            if alternate_authorization {
                let scalar_start = canonical.len() - 32;
                canonical[scalar_start] ^= 1;
                assert_ne!(canonical.as_slice(), prepared.bytes());
                assert_eq!(
                    submission::decode(&canonical).unwrap().txid(),
                    submission::decode(prepared.bytes()).unwrap().txid()
                );
            }
            // The local submission-attempt label can still be zero.
            net.chain.write().await.mine_block(vec![canonical]);
        }
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        if malformed_sequencing {
            db.execute_batch("PRAGMA ignore_check_constraints=ON")
                .unwrap();
            db.execute(
                "UPDATE treasury_operations SET attempts='invalid',submission=x'00' WHERE id=?1",
                [&job.operation_id],
            )
            .unwrap();
            db.execute("UPDATE funding_progress SET attempts='bad',next_poll='bad',last_error=x'00',phase='invalid' WHERE job_id=?1",[&job.id]).unwrap();
            db.execute("INSERT INTO funding_health(job_id,started_at,error_streak,timed_out) VALUES(?1,'bad','bad','bad') ON CONFLICT(job_id) DO UPDATE SET started_at='bad',error_streak='bad',timed_out='bad'",[&job.id]).unwrap();
            db.execute_batch("PRAGMA ignore_check_constraints=OFF")
                .unwrap();
        }
        let mut sender = StaleLookup {
            presence,
            lookups: 0,
        };
        assert_eq!(
            owner
                .recover_funding_job(job.id.clone(), 3, false, &mut sender, &stop)
                .await
                .unwrap(),
            recovery::RecoveryOutcome::Waiting("swap_outcome_pending")
        );
        assert_eq!(
            sender.lookups, 0,
            "established SDK confirmation needs no indexer corroboration"
        );
        if alternate_authorization {
            let txid = submission::decode(prepared.bytes()).unwrap().txid();
            let wallet = owner.client.as_ref().unwrap().wallet().read().await;
            let record = wallet.wallet_transactions.get(&txid).unwrap();
            let mut current = Vec::new();
            record.transaction().write(&mut current).unwrap();
            assert_ne!(
                current.as_slice(),
                prepared.bytes(),
                "the SDK record must exercise distinct authorizing bytes"
            );
            assert_eq!(record.transaction().txid(), txid);
        }
        let original_operation = job.operation_id.clone();
        let operation = job.operation_id;
        store
            .call(move |s| {
                assert_eq!(s.observation_operation(&operation)?.submission, "CONFIRMED");
                if malformed_sequencing {
                    assert!(s.operation(&operation).is_err());
                } else {
                    assert_eq!(
                        s.operation(&operation)?.attempts,
                        i64::from(journaled_submission)
                    );
                }
                assert_eq!(s.prepared_bytes(&operation)?.as_slice(), prepared.bytes());
                assert_eq!(s.funding_recovery_count(&job.id)?, 0);
                Ok(())
            })
            .await
            .unwrap();
        if malformed_sequencing {
            let diagnostic: rusqlite::types::Value = db
                .query_row(
                    "SELECT attempts FROM funding_progress WHERE operation_id=?1",
                    [&original_operation],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                diagnostic,
                rusqlite::types::Value::Text("bad".into()),
                "canonical recovery does not rewrite diagnostic counters"
            );
        }
        let count: rusqlite::types::Value = db
            .query_row(
                "SELECT attempts FROM treasury_operations WHERE id=?1",
                [&original_operation],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count,
            if malformed_sequencing {
                rusqlite::types::Value::Text("invalid".into())
            } else {
                rusqlite::types::Value::Integer(i64::from(journaled_submission))
            },
            "canonical confirmation never repairs or resets disclosure counts"
        );
        drop(store);
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn expired_original_closes_after_another_canonical_transaction_spends_its_inputs() {
    struct NoLookup;
    impl TransactionSubmission for NoLookup {
        async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
            panic!("expiry recovery cannot submit");
        }
        async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
            panic!("canonical input proof needs no indexer corroboration");
        }
    }
    async fn prepare(owner: &mut Treasury, name: &str) -> (FundingJob, PreparedTransaction) {
        let pool = owner
            .ensure_capacity_group(
                name.into(),
                crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )
            .await
            .unwrap();
        owner
            .allocate_capacity_kind(pool, AllocationKind::Small, true)
            .await
            .unwrap();
        let job = owner
            .store
            .call(|s| Ok(s.funding_jobs()?.remove(0)))
            .await
            .unwrap();
        let deadline = now().unwrap() + 7200;
        save_quotes(&owner.store, vec![job.clone()], deadline).await;
        let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
        (job, prepared)
    }
    let (_dir, net, mut owner, endpoint) = funded_fixture(2).await;
    // A second deterministic test wallet owns the same funded note. It represents
    // another authorized spender, never another calculation by the original owner.
    let other_dir = tempfile::tempdir().unwrap();
    let mut other = Treasury::create_with_network(
        other_dir.path().join("state"),
        other_dir.path().join("key"),
        1,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    other.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    let stop = CancellationToken::new();
    other.sync_once(&stop).await.unwrap();
    let (job, original) = prepare(&mut owner, "original").await;
    let (other_job, competing) = prepare(&mut other, "other").await;
    assert_ne!(
        original.facts().unwrap().txid,
        competing.facts().unwrap().txid
    );
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    other
        .submit_prepared(other_job.operation_id, &mut sender, false, &stop)
        .await
        .unwrap();
    {
        let mut chain = net.chain.write().await;
        chain.mine_mempool();
        let expiry = original.facts().unwrap().expiry_height;
        let remaining = expiry + 2 - chain.tip();
        chain.mine_empty_blocks(remaining);
    }
    let store = owner.store_handle();
    let operation = job.operation_id.clone();
    let initial = original.bytes().to_vec();
    owner
        .recover_expired(operation.clone(), &mut NoLookup, &stop)
        .await
        .unwrap();
    store
        .call(move |s| {
            assert_eq!(s.operation(&operation)?.submission, "EXPIRED");
            assert_eq!(s.operation(&operation)?.attempts, 0);
            assert!(!s.operation_pending(&operation)?);
            assert_eq!(s.prepared_bytes(&operation)?.as_slice(), initial);
            assert_eq!(s.funding_jobs()?.len(), 1);
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    owner.close().await.unwrap();
    other.close().await.unwrap();
}

#[tokio::test]
async fn buried_funding_expiry_ignores_stale_inclusion_hints_without_replay_or_new_deposit() {
    struct StaleHint {
        confirmed: bool,
        lookups: usize,
    }
    impl TransactionSubmission for StaleHint {
        async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
            panic!("recovery cannot submit");
        }
        async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
            self.lookups += 1;
            Ok(if self.confirmed {
                TransactionPresence::Confirmed { height: 24 }
            } else {
                TransactionPresence::Mempool
            })
        }
    }
    for confirmed in [false, true] {
        let (_dir, net, mut owner, endpoint) = funded_fixture(2).await;
        let pool = owner
            .ensure_capacity_group(
                "expiry_hint".into(),
                crate::rotation::test_capacity::small_reserve("5"),
                None,
                0,
            )
            .await
            .unwrap();
        owner
            .allocate_capacity_kind(pool, AllocationKind::Small, true)
            .await
            .unwrap();
        let store = owner.store_handle();
        let job = store
            .call(|s| Ok(s.funding_jobs()?.remove(0)))
            .await
            .unwrap();
        let deadline = now().unwrap() + 7200;
        save_quotes(&store, vec![job.clone()], deadline).await;
        let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
        let stop = CancellationToken::new();
        let mut actual = submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap();
        owner
            .submit_prepared(job.operation_id.clone(), &mut actual, false, &stop)
            .await
            .unwrap();
        {
            let mut chain = net.chain.write().await;
            let remaining = prepared.facts().unwrap().expiry_height + 2 - chain.tip();
            chain.mine_empty_blocks(remaining);
        }
        // Scheduled source observation reuses this completed current-owner scan;
        // only the actual expiry input proof requests another completed scan.
        owner.sync_once(&stop).await.unwrap();
        let mut sender = StaleHint {
            confirmed,
            lookups: 0,
        };
        let outcome = owner
            .recover_funding_observed(job.id.clone(), 3, false, &mut sender, &stop, true)
            .await
            .unwrap();
        assert_eq!(
            outcome.outcome,
            recovery::RecoveryOutcome::Waiting("funding_disabled")
        );
        assert_eq!(
            sender.lookups, 0,
            "a stale hint cannot veto canonical expiry input proof"
        );
        let operation = job.operation_id;
        store
            .call(move |s| {
                assert_eq!(s.operation(&operation)?.submission, "EXPIRED");
                assert_eq!(s.operation(&operation)?.attempts, 1);
                assert_eq!(s.prepared_bytes(&operation)?.as_slice(), prepared.bytes());
                assert!(!s.operation_pending(&operation)?);
                assert_eq!(s.funding_jobs()?.len(), 1);
                assert_eq!(s.funding_recovery_count(&job.id)?, 1);
                Ok(())
            })
            .await
            .unwrap();
        drop(store);
        owner.close().await.unwrap();
    }
}
