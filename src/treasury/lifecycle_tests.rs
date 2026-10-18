//! Local synthetic-chain coverage of the real treasury owner and signed journal.
//! Real proofs/bytes and scans, but no consensus validation or live bridge settlement.
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
use zingolib::testutils::mock_indexer::{LostSendDestination, MockNet, faucet_funding_transaction};

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
    let mut net = MockNet::launch().await;
    // Obtain the fixture endpoint without adding any production egress constructor.
    let endpoint_client = net.client(SEED).await;
    let endpoint = endpoint_client.indexer_uri().unwrap().to_string();
    drop(endpoint_client);
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
        let tip = chain.tip();
        assert_eq!(chain.reorg_to(23).len(), 2);
        chain.mine_empty_blocks(tip - 23);
        assert_eq!(chain.tip(), tip);
    }
    assert_eq!(
        handle
            .submit(pending_job.operation_id.clone(), false)
            .await
            .unwrap(),
        SubmissionOutcome::Accepted
    );
    let after = store.call(|s| s.treasury_sync_state()).await.unwrap();
    assert!(
        after.snapshot_revision > before.snapshot_revision,
        "reorg must force a scan"
    );
    assert_eq!(after.sync.unwrap().phase, SyncPhase::Ready);
    // Historical source facts do not gate a best-effort send. The exact pending
    // transaction still owns its inputs and blocks another calculation.
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
}

#[tokio::test]
async fn same_height_tip_only_reorg_updates_sdk_without_historical_source_gate() {
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
    store
        .call(move |s| {
            assert_eq!(s.operation(&id)?.attempts, 1);
            assert!(s.require_preparation_available().is_ok());
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
    drop(store);
    owner.close().await.unwrap();
}
