//! Recovery scenarios formerly exercised by automatic shielding, now using
//! accepted Ironwood native payments. Transparent shielding is explicitly rejected.
use super::*;
use pepper_sync::wallet::OutputInterface;
use serde_json::json;

struct NoSubmission;
impl TransactionSubmission for NoSubmission {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("forbidden submission")
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("forbidden lookup")
    }
}
struct Undelivered {
    sender: submission::GrpcSubmission,
    attempts: usize,
}
impl TransactionSubmission for Undelivered {
    async fn preflight(&mut self, tx: &PreparedTransaction, retry: bool) -> Result<()> {
        self.sender.preflight(tx, retry).await
    }
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        self.attempts += 1;
        Ok(SubmissionOutcome::Unknown)
    }
    async fn lookup(&mut self, tx: &PreparedTransaction) -> Result<TransactionPresence> {
        self.sender.lookup(tx).await
    }
}
fn sender(endpoint: &str) -> submission::GrpcSubmission {
    submission::GrpcSubmission::with_network(
        endpoint.into(),
        endpoint.into(),
        TreasuryNetwork::Synthetic,
    )
    .unwrap()
}
async fn native(
    owner: &mut Treasury,
    request_id: &'static str,
    amount: u64,
) -> PreparedTransaction {
    let recipient = owner.preview_address().await.unwrap();
    let destination = recipient.clone();
    let id=owner.store.call(move |s| {
        let operation=s.reserve_native("owner","send",request_id,json!({"asset_id":crate::direct_swap::route::ZEC,"chain":"zec","amount":amount.to_string(),"recipient":destination}))?.0;
        Ok(s.accept_direct_send(&operation.id,None,crate::rotation::store::DirectSendAuthority{max_debit_zatoshis:None})?.id)
    }).await.unwrap();
    owner
        .prepare(PrepareRequest {
            managed_funding: None,
            operation_id: id,
            pool_id: None,
            deadline: None,
            recipient,
            amount_zatoshis: amount,
        })
        .await
        .unwrap()
}
async fn load_pending(owner: &Treasury) -> PreparedTransaction {
    owner
        .store
        .call(|s| PreparedTransaction::load(s, &s.jobless_pending_operation()?.unwrap()))
        .await
        .unwrap()
}
async fn ironwood_inputs(
    owner: &Treasury,
    prepared: &PreparedTransaction,
) -> Vec<crate::rotation::store::preparation::OwnedInput> {
    let id = prepared.operation_id().to_owned();
    let inputs = owner
        .store
        .call(move |s| s.prepared_inputs(&id))
        .await
        .unwrap();
    assert!(!inputs.is_empty());
    assert!(
        inputs
            .iter()
            .all(|input| input.pool == crate::rotation::store::preparation::InputPool::Ironwood)
    );
    let tx = submission::decode(prepared.bytes()).unwrap();
    assert!(
        tx.transparent_bundle()
            .is_none_or(|bundle| bundle.vin.is_empty() && bundle.vout.is_empty())
    );
    assert!(tx.sapling_bundle().is_none() && tx.orchard_bundle().is_none());
    assert!(tx.ironwood_bundle().is_some());
    inputs
}

#[tokio::test]
async fn ironwood_expiry_closes_work_despite_orphaned_selected_input() {
    let (_dir, net, mut owner, endpoint) = funded_fixture_amount(2, 20_000).await;
    let address = owner.preview_address().await.unwrap();
    let large = net
        .funding_transaction(vec![(&address, 1_000_000, None)])
        .await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![large.clone()]);
        chain.mine_empty_blocks(2);
    }
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    let original = native(&mut owner, "original", 1_000_000).await;
    let selected = ironwood_inputs(&owner, &original).await;
    assert_eq!(
        selected.len(),
        2,
        "both Ironwood notes are required by the actual proposal"
    );
    let mut sender = Undelivered {
        sender: sender(&endpoint),
        attempts: 0,
    };
    owner
        .submit_prepared(original.operation_id().into(), &mut sender, false, &stop)
        .await
        .unwrap();
    owner
        .maintain_pending_sources(&mut sender, &stop)
        .await
        .unwrap();
    assert_eq!(load_pending(&owner).await.bytes(), original.bytes());
    assert_eq!(
        sender.attempts, 1,
        "absence before signed expiry never authorizes replay"
    );
    let expiry = original.facts().unwrap().expiry_height;
    {
        let mut chain = net.chain.write().await;
        chain.reorg_to(20);
        chain.mine_empty_blocks(3);
        chain.mine_block(vec![large]);
        let remaining = expiry + 2 - chain.tip();
        chain.mine_empty_blocks(remaining);
    }
    if owner.sync_once(&stop).await.is_err() {
        owner.sync_once(&stop).await.unwrap();
    }
    {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        let notes = wallet
            .wallet_transactions
            .values()
            .filter(|tx| tx.status().is_confirmed())
            .flat_map(|tx| tx.ironwood_notes())
            .collect::<Vec<_>>();
        assert_eq!(
            notes.iter().map(|note| note.value()).sum::<u64>(),
            1_000_000
        );
    }
    owner
        .maintain_pending_sources(&mut NoSubmission, &stop)
        .await
        .unwrap();
    let id = original.operation_id().to_owned();
    let bytes = original.bytes().to_vec();
    owner
        .store
        .call(move |s| {
            assert_eq!(s.operation(&id)?.submission, "EXPIRED");
            assert_eq!(s.operation(&id)?.attempts, 1);
            assert_eq!(s.prepared_bytes(&id)?.as_slice(), bytes);
            assert_eq!(s.prepared_inputs(&id)?, selected);
            assert!(s.jobless_pending_operation()?.is_none());
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        sender.attempts, 1,
        "maintenance closes the original without creating replacement authority"
    );
    // Independent explicit authority can use surviving SDK funds after expiry.
    let next = native(&mut owner, "separate-request", 50_000).await;
    assert_ne!(next.operation_id(), original.operation_id());
    assert_ne!(next.bytes(), original.bytes());
    owner
        .submit_prepared(next.operation_id().into(), &mut sender, false, &stop)
        .await
        .unwrap();
    assert_eq!(sender.attempts, 2);
    owner.close().await.unwrap();
}

#[tokio::test]
async fn ironwood_pending_bytes_survive_lost_response_restart_and_independent_work() {
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let stop = CancellationToken::new();
    let first = native(&mut owner, "first", 50_000).await;
    ironwood_inputs(&owner, &first).await;
    let identity = first.network_identity().unwrap();
    let mut sender = sender(&endpoint);
    net.chain.write().await.lose_next_send_response = Some(LostSendDestination::Mempool);
    assert_eq!(
        owner
            .submit_prepared(first.operation_id().into(), &mut sender, false, &stop)
            .await
            .unwrap(),
        SubmissionOutcome::Unknown
    );
    let first_id = first.operation_id().to_owned();
    owner
        .store
        .call(move |s| {
            assert_eq!(s.operation(&first_id)?.attempts, 1);
            assert_eq!(s.operation(&first_id)?.submission, "UNKNOWN");
            assert!(s.accept_preparation("duplicate", None, 1, None).is_err());
            Ok(())
        })
        .await
        .unwrap();
    let treasury = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        treasury,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    owner.sync_once(&stop).await.unwrap();
    owner
        .maintain_pending_sources(&mut sender, &stop)
        .await
        .unwrap();
    let retained = load_pending(&owner).await;
    assert_eq!(retained.bytes(), first.bytes());
    assert_eq!(retained.network_identity().unwrap(), identity);
    assert_eq!(owner.status().await.unwrap().treasury_operations.len(), 1);
    {
        let mut chain = net.chain.write().await;
        chain.mine_mempool();
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&stop).await.unwrap();
    owner
        .maintain_pending_sources(&mut sender, &stop)
        .await
        .unwrap();
    assert!(
        !owner
            .store
            .call(move |s| s.operation_pending(first.operation_id()))
            .await
            .unwrap()
    );
    let second = native(&mut owner, "second", 50_000).await;
    ironwood_inputs(&owner, &second).await;
    assert_ne!(second.network_identity().unwrap(), identity);
    owner
        .submit_prepared(second.operation_id().into(), &mut sender, false, &stop)
        .await
        .unwrap();
    {
        let mut chain = net.chain.write().await;
        chain.mine_mempool();
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&stop).await.unwrap();
    owner
        .maintain_pending_sources(&mut sender, &stop)
        .await
        .unwrap();
    let status = owner.status().await.unwrap();
    assert_eq!(status.treasury_operations.len(), 2);
    assert!(
        status
            .treasury_operations
            .iter()
            .all(|operation| operation.submission == "CONFIRMED" && operation.attempts == 1)
    );
    assert!(status.funding_jobs.is_empty());
    owner.close().await.unwrap();
}

#[tokio::test]
async fn transparent_shielding_is_rejected_before_state_or_network_side_effects() {
    let (_dir, _net, mut owner, _endpoint) = funded_fixture(2).await;
    let before = owner.store.call(|s| s.snapshot()).await.unwrap();
    for stopped in [false, true] {
        let stop = CancellationToken::new();
        if stopped {
            stop.cancel();
        }
        let error = owner
            .shield_refund("not-a-funding-job".into(), &stop)
            .await
            .err()
            .expect("transparent shielding is unsupported");
        assert!(
            error.to_string().contains("unsupported") || error.to_string().contains("Ironwood"),
            "{error}"
        );
        assert!(owner.healthy && owner.running.is_none());
        assert!(owner.status().await.unwrap().treasury_operations.is_empty());
        assert_eq!(owner.store.call(|s| s.snapshot()).await.unwrap(), before);
    }
    owner
        .maintain_pending_sources(&mut NoSubmission, &CancellationToken::new())
        .await
        .unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn ironwood_actor_submission_cancellation_retains_exact_bytes_after_reopen() {
    let (dir, _net, mut owner, endpoint) = funded_fixture(2).await;
    let original = native(&mut owner, "cancel-submission", 50_000).await;
    let bytes = original.bytes().to_vec();
    let operation = original.operation_id().to_owned();
    let treasury = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        treasury,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    let entered = CancellationToken::new();
    let release = CancellationToken::new();
    let sender = GatedSubmission {
        sender: sender(&endpoint),
        entered: entered.clone(),
        release: release.clone(),
    };
    let store = owner.store_handle();
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    let caller = handle.clone();
    let id = operation.clone();
    let waiter = tokio::spawn(async move { caller.submit(id, false).await });
    entered.cancelled().await;
    store
        .call(move |s| {
            assert_eq!(s.operation(&operation)?.attempts, 1);
            assert_eq!(s.prepared_bytes(&operation)?.as_slice(), bytes);
            Ok(())
        })
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    stop.cancel();
    release.cancel();
    drop(handle);
    drop(store);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn ironwood_source_command_scan_is_consumed_once_before_unrelated_commands() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct CountLookup {
        sender: submission::GrpcSubmission,
        lookups: Arc<AtomicUsize>,
    }
    impl TransactionSubmission for CountLookup {
        async fn preflight(&mut self, tx: &PreparedTransaction, retry: bool) -> Result<()> {
            self.sender.preflight(tx, retry).await
        }
        async fn submit(&mut self, tx: BroadcastTransaction) -> Result<SubmissionOutcome> {
            self.sender.submit(tx).await
        }
        async fn lookup(&mut self, tx: &PreparedTransaction) -> Result<TransactionPresence> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            self.sender.lookup(tx).await
        }
    }
    let (_dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let original = native(&mut owner, "drain-command-scan", 50_000).await;
    // The source command must discover this new tip inside its own canonical scan.
    net.chain.write().await.mine_empty_blocks(1);
    let lookups = Arc::new(AtomicUsize::new(0));
    let sender = CountLookup {
        sender: sender(&endpoint),
        lookups: lookups.clone(),
    };
    let store = owner.store_handle();
    let (handle, commands) = actor::channel();
    let caller = handle.clone();
    let id = original.operation_id().to_owned();
    let submit = tokio::spawn(async move { caller.submit(id, false).await });
    while commands.test_is_empty() {
        tokio::task::yield_now().await;
    }
    let stop = CancellationToken::new();
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    assert_eq!(submit.await.unwrap().unwrap(), SubmissionOutcome::Accepted);
    // Serialized command acknowledgments follow maintenance for the preceding scan.
    for _ in 0..3 {
        assert!(
            handle
                .recover_funding("missing".into(), 1, false)
                .await
                .is_err()
        );
    }
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    store
        .call(move |s| {
            assert_eq!(
                s.prepared_bytes(original.operation_id())?.as_slice(),
                original.bytes()
            );
            assert!(s.operation_pending(original.operation_id())?);
            assert_eq!(s.operation(original.operation_id())?.attempts, 1);
            Ok(())
        })
        .await
        .unwrap();
    stop.cancel();
    drop(handle);
    drop(store);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn buried_ironwood_expiry_ignores_stale_inclusion_hints_without_another_calculation() {
    let (_dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let original = native(&mut owner, "buried-expiry", 50_000).await;
    let stop = CancellationToken::new();
    let mut actual = sender(&endpoint);
    owner
        .submit_prepared(original.operation_id().into(), &mut actual, false, &stop)
        .await
        .unwrap();
    {
        let mut chain = net.chain.write().await;
        let remaining = original.facts().unwrap().expiry_height + 2 - chain.tip();
        chain.mine_empty_blocks(remaining);
    }
    owner.sync_once(&stop).await.unwrap();
    net.chain.write().await.faults.inject(
        Rpc::LightdInfo,
        Fault::Fail(tonic::Code::Unavailable, "synthetic indexer offline".into()),
    );
    owner
        .maintain_pending_sources(&mut NoSubmission, &stop)
        .await
        .unwrap();
    assert_eq!(net.chain.read().await.faults.pending(Rpc::LightdInfo), 1);
    owner
        .store
        .call(move |s| {
            let id = original.operation_id();
            assert_eq!(s.operation(id)?.submission, "EXPIRED");
            assert_eq!(s.operation(id)?.attempts, 1);
            assert_eq!(s.prepared_bytes(id)?.as_slice(), original.bytes());
            assert!(!s.operation_pending(id)?);
            assert_eq!(s.status()?.treasury_operations.len(), 1);
            assert!(s.funding_jobs()?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
}
