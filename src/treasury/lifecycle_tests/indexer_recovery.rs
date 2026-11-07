//! Real disclosed originals keep their liabilities through ambiguous and proven recovery.
use super::*;
use crate::rotation::store::preparation::OwnedInput;

#[derive(Clone)]
struct Original {
    job: FundingJob,
    raw: Vec<u8>,
    inputs: Vec<OwnedInput>,
    snapshot: Vec<u8>,
    quote: Vec<u8>,
    isolation: crate::network::IsolationId,
    txid: String,
}
impl Original {
    async fn assert_retained(&self, owner: &Treasury, pending: bool) {
        let original = self.clone();
        owner
            .store
            .call(move |s| {
                let job = s.funding_job(&original.job.id)?.unwrap();
                assert_eq!(
                    (
                        &job.id,
                        &job.pool_id,
                        &job.wallet_id,
                        &job.operation_id,
                        &job.recipient,
                        &job.target,
                        &job.selected_output,
                        &job.accepted_output
                    ),
                    (
                        &original.job.id,
                        &original.job.pool_id,
                        &original.job.wallet_id,
                        &original.job.operation_id,
                        &original.job.recipient,
                        &original.job.target,
                        &original.job.selected_output,
                        &original.job.accepted_output
                    )
                );
                let operation = s.operation(&job.operation_id)?;
                assert_eq!(operation.attempts, 1);
                assert_eq!(operation.facts.txid, original.txid);
                assert_eq!(s.operation_pending(&job.operation_id)?, pending);
                assert!(!s.source_previously_closed(&job.operation_id)?);
                assert!(s.prepared_bytes(&job.operation_id)?.as_slice() == original.raw);
                assert_eq!(s.prepared_inputs(&job.operation_id)?, original.inputs);
                assert!(s.prepared_snapshot(&job.operation_id)?.as_slice() == original.snapshot);
                assert!(
                    s.funding_status_quote(&job.id, &job.operation_id)?
                        .as_slice()
                        == original.quote
                );
                assert!(!s.funding_facts(&job.id, &job.operation_id)?.archived);
                assert_eq!(s.funding_jobs()?.len(), 1);
                let facts = s.funding_facts(&job.id, &job.operation_id)?;
                assert_eq!(facts.source_pending, pending);
                assert!(
                    !facts.source_closed
                        && !facts.provider_success
                        && !facts.initial_credit_recorded
                );
                if pending {
                    assert!(s.require_preparation_available().is_err());
                    assert!(s.abandon_unprepared(&job.operation_id).is_err());
                    let prepared = PreparedTransaction::load(s, &job.operation_id)?;
                    assert_eq!(prepared.network_identity()?, original.isolation);
                } else {
                    assert_eq!(operation.submission, "CONFIRMED");
                    s.require_preparation_available()?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }
}

async fn disclose_once(owner: &mut Treasury, endpoint: &str) -> (Original, PreparedTransaction) {
    let pool = owner
        .ensure_capacity_group(
            "indexer_recovery".into(),
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
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.into(),
        endpoint.into(),
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    assert_eq!(
        owner
            .submit_prepared(
                job.operation_id.clone(),
                &mut sender,
                false,
                &CancellationToken::new()
            )
            .await
            .unwrap(),
        SubmissionOutcome::Accepted
    );
    let operation = job.operation_id;
    let id = job.id;
    let raw = prepared.bytes().to_vec();
    let isolation = prepared.network_identity().unwrap();
    let original = owner
        .store
        .call(move |s| {
            Ok(Original {
                job: s.funding_job(&id)?.unwrap(),
                raw,
                inputs: s.prepared_inputs(&operation)?,
                snapshot: s.prepared_snapshot(&operation)?.to_vec(),
                quote: s.funding_status_quote(&id, &operation)?.to_vec(),
                isolation,
                txid: s.operation(&operation)?.facts.txid,
            })
        })
        .await
        .unwrap();
    original.assert_retained(owner, true).await;
    (original, prepared)
}

async fn mark_no_send(net: &MockNet) {
    net.chain
        .write()
        .await
        .faults
        .inject(Rpc::SendTransaction, Fault::EndStream { after: 0 });
}
async fn assert_no_send(net: &MockNet) {
    assert_eq!(
        net.chain.read().await.faults.pending(Rpc::SendTransaction),
        1,
        "sync/recovery must not disclose another transaction"
    );
}

#[tokio::test]
async fn stale_prefix_keeps_disclosed_original_across_cancel_reopen_and_healthy_confirmation() {
    let (dir, original_net, mut owner, endpoint) = funded_fixture(1).await;
    original_net
        .chain
        .write()
        .await
        .mine_empty_blocks(1250 - 23);
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    let (original, _) = disclose_once(&mut owner, &endpoint).await;
    assert_eq!(original_net.chain.read().await.mempool_len(), 1);
    let (funding, blocks) = {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        let funding = wallet
            .wallet_transactions
            .values()
            .find(|tx| tx.status().get_confirmed_height() == Some(21.into()))
            .unwrap();
        assert_eq!(original.inputs.len(), 1);
        assert_eq!(original.inputs[0].txid, funding.txid().to_string());
        let mut bytes = Vec::new();
        funding.transaction().write(&mut bytes).unwrap();
        let blocks = wallet
            .wallet_blocks
            .iter()
            .map(|(height, block)| (*height, block.block_hash()))
            .collect::<Vec<_>>();
        (bytes, blocks)
    };
    let shorter = MockNet::launch().await;
    {
        let mut chain = shorter.chain.write().await;
        chain.mine_empty_blocks(20);
        chain.mine_block(vec![funding]);
        chain.mine_empty_blocks(150 - 21);
        chain
            .faults
            .inject(Rpc::BlockRange, Fault::EndStream { after: 0 });
    }
    mark_no_send(&original_net).await;
    mark_no_send(&shorter).await;
    let short_settings = SyncSettings::new(shorter.indexer_uri().to_string(), 1).unwrap();
    owner.configure_sync(short_settings.clone());
    assert!(owner.sync_once(&stop).await.is_err());
    let observation = owner.status().await.unwrap().sync.unwrap();
    assert_eq!(observation.height, Some(1250));
    assert!(
        observation
            .last_error
            .unwrap()
            .contains("indexer_tip_regressed_without_divergence")
    );
    {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
        assert_eq!(
            wallet
                .wallet_blocks
                .iter()
                .map(|(height, block)| (*height, block.block_hash()))
                .collect::<Vec<_>>(),
            blocks
        );
    }
    original.assert_retained(&owner, true).await;
    let identity = owner.status().await.unwrap().treasury_id;
    let addresses = owner.addresses().await.unwrap();
    owner.close().await.unwrap();
    owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(short_settings);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(owner.sync_once(&cancelled).await.is_err());
    assert_eq!(
        owner.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Offline
    );
    original.assert_retained(&owner, true).await;
    assert!(owner.sync_once(&stop).await.is_err());
    original.assert_retained(&owner, true).await;
    assert_eq!(
        shorter.chain.read().await.faults.pending(Rpc::BlockRange),
        1
    );
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 1).unwrap());
    owner.sync_once(&stop).await.unwrap();
    assert_eq!(
        owner.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Ready
    );
    assert_eq!(owner.addresses().await.unwrap(), addresses);
    original.assert_retained(&owner, true).await;
    assert_no_send(&original_net).await;
    assert_no_send(&shorter).await;
    original_net.chain.write().await.mine_mempool();
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    assert_eq!(
        owner
            .reconcile_prepared(original.job.operation_id.clone(), &mut sender, &stop)
            .await
            .unwrap(),
        TransactionPresence::Confirmed { height: 1251 }
    );
    original.assert_retained(&owner, false).await;
    assert_no_send(&original_net).await;
    assert_no_send(&shorter).await;
    owner.close().await.unwrap();
}

struct ObserveUnknown {
    lookups: usize,
}
impl TransactionSubmission for ObserveUnknown {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("recovery must not replay or replace the accepted original");
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        self.lookups += 1;
        Ok(TransactionPresence::Unknown)
    }
}

#[tokio::test]
async fn proven_deep_fork_finds_prebirthday_credit_but_keeps_orphaned_original_liability() {
    use pepper_sync::wallet::{NoteInterface as _, OutputInterface as _};
    use zingolib::testutils::mock_indexer::faucet_funding_transaction_on;
    let dir = tempfile::tempdir().unwrap();
    let net = MockNet::launch().await;
    let endpoint = net.indexer_uri().to_string();
    let mut owner = Treasury::create_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        100,
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
    let activation = net.chain.read().await.activation_heights();
    let funding =
        faucet_funding_transaction_on(activation, 121.into(), vec![(&address, 1_000_000, None)])
            .await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_empty_blocks(120);
        chain.mine_block(vec![funding]);
        chain.mine_empty_blocks(1250 - 121);
    }
    let settings = SyncSettings::new(endpoint.clone(), 1).unwrap();
    owner.configure_sync(settings.clone());
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    let (original, prepared) = disclose_once(&mut owner, &endpoint).await;
    assert_eq!(original.inputs.len(), 1);
    {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        let funding_id =
            zcash_primitives::transaction::TxId::from_hex(&original.inputs[0].txid).unwrap();
        assert_eq!(
            wallet.wallet_transactions[&funding_id]
                .status()
                .get_confirmed_height(),
            Some(121.into())
        );
    }
    net.chain.write().await.mine_mempool();
    assert_eq!(net.chain.read().await.mempool_len(), 0);
    let new_credit =
        faucet_funding_transaction_on(activation, 90.into(), vec![(&address, 2_000_000, None)])
            .await;
    {
        let mut chain = net.chain.write().await;
        let evicted = chain.reorg_to(80);
        assert!(evicted.iter().any(|raw| raw.as_slice() == original.raw));
        chain.mine_empty_blocks(9);
        chain.mine_block(vec![new_credit]);
        chain.mine_empty_blocks(60);
        chain.faults.inject(
            Rpc::BlockRange,
            Fault::Fail(
                tonic::Code::InvalidArgument,
                "interrupt proven replacement scan".into(),
            ),
        );
    }
    mark_no_send(&net).await;
    assert!(owner.sync_once(&stop).await.is_err());
    {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        assert_eq!(u32::from(wallet.birthday()), 100);
        assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
    }
    assert_eq!(
        net.chain.read().await.faults.pending(Rpc::BlockRange),
        0,
        "bound divergence must reach the deliberately interrupted rescan"
    );
    original.assert_retained(&owner, true).await;
    assert_no_send(&net).await;
    let identity = owner.status().await.unwrap().treasury_id;
    let addresses = owner.addresses().await.unwrap();
    owner.close().await.unwrap();
    owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity.clone(),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(settings.clone());
    owner.sync_once(&stop).await.unwrap();
    let observation = owner.status().await.unwrap().sync.unwrap();
    assert_eq!(observation.phase, SyncPhase::Ready);
    assert_eq!(observation.confirmed_shielded_zatoshis, 2_000_000);
    assert_eq!(observation.spendable_shielded_zatoshis, 2_000_000);
    assert_eq!(owner.addresses().await.unwrap(), addresses);
    {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        assert_eq!(u32::from(wallet.birthday()), 100);
        assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
        let credit = wallet
            .wallet_transactions
            .values()
            .find(|tx| tx.status().get_confirmed_height() == Some(90.into()))
            .unwrap();
        let note = credit
            .ironwood_notes()
            .iter()
            .find(|note| note.value() == 2_000_000)
            .unwrap();
        assert!(
            wallet
                .shard_trees
                .ironwood
                .witness_at_checkpoint_id(note.position().unwrap(), &150.into())
                .unwrap()
                .is_some()
        );
        let original_input = &original.inputs[0];
        assert!(
            !wallet.wallet_transactions.contains_key(
                &zcash_primitives::transaction::TxId::from_hex(&original_input.txid).unwrap()
            ),
            "the current SDK must not fabricate the orphaned old funding input"
        );
    }
    original.assert_retained(&owner, true).await;
    assert!(150 < prepared.facts().unwrap().expiry_height);
    let mut observer = ObserveUnknown { lookups: 0 };
    assert_eq!(
        owner
            .recover_funding_job(original.job.id.clone(), true, &mut observer, &stop)
            .await
            .unwrap(),
        recovery::RecoveryOutcome::Waiting("waiting_for_transaction_expiry")
    );
    assert_eq!(
        observer.lookups, 1,
        "zero allowance does not suppress the existing original observation"
    );
    original.assert_retained(&owner, true).await;
    assert_no_send(&net).await;
    owner.close().await.unwrap();
    owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(settings);
    original.assert_retained(&owner, true).await;
    assert_eq!(owner.addresses().await.unwrap(), addresses);
    assert_no_send(&net).await;
    owner.close().await.unwrap();
}
