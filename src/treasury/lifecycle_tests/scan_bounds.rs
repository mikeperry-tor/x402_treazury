//! A pre-fix interrupted snapshot must rebuild its missing scan-bound metadata.
use super::*;

#[tokio::test]
async fn saved_missing_scanned_boundary_rescans_after_restart_without_losing_original_owner() {
    let (dir, net, mut owner, endpoint) = funded_fixture(1).await;
    let stop = CancellationToken::new();
    let pool = owner
        .ensure_capacity_group(
            "missing_bound".into(),
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
    let operation = job.operation_id.clone();
    let inputs = store
        .call(move |s| s.prepared_inputs(&operation))
        .await
        .unwrap();
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint.clone(),
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
    let addresses = owner.addresses().await.unwrap();
    let identity = owner.status().await.unwrap().treasury_id;
    {
        let mut wallet = owner.client.as_ref().unwrap().wallet().write().await;
        let bound = wallet.sync_state.highest_scanned_height().unwrap();
        assert!(wallet.wallet_blocks.remove(&bound).is_some());
        assert!(
            wallet
                .sync_state
                .scan_ranges()
                .iter()
                .any(|range| range.priority().is_scanned() && range.block_range().end - 1 == bound),
            "the saved old range must still require its removed boundary"
        );
    }
    let mut observation = owner.status().await.unwrap().sync.unwrap();
    observation.phase = SyncPhase::Failed;
    owner.checkpoint(observation).await.unwrap();
    drop(store);
    owner.close().await.unwrap();
    owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint, 1).unwrap());
    net.chain.write().await.faults.inject(
        Rpc::BlockRange,
        Fault::Fail(
            tonic::Code::InvalidArgument,
            "synthetic interrupted missing-bound rescan".into(),
        ),
    );
    assert!(owner.sync_once(&stop).await.is_err());
    let operation = job.operation_id.clone();
    owner
        .store
        .call(move |s| {
            assert!(!s.operation_pending(&operation)?);
            assert_eq!(s.operation(&operation)?.submission, "CONFIRMED");
            assert_eq!(s.operation(&operation)?.attempts, 1);
            assert!(!s.source_previously_closed(&operation)?);
            Ok(())
        })
        .await
        .unwrap();
    owner.sync_once(&stop).await.unwrap();
    assert_eq!(owner.addresses().await.unwrap(), addresses);
    let sync = owner.status().await.unwrap().sync.unwrap();
    assert_eq!(sync.phase, SyncPhase::Ready);
    assert_eq!(
        sync.confirmed_shielded_zatoshis,
        1_000_000
            - prepared.facts().unwrap().amount_zatoshis
            - prepared.facts().unwrap().fee_zatoshis,
        "the full rescan must reconstruct the canonical change from the exact original transaction"
    );
    let operation = job.operation_id.clone();
    let raw = prepared.bytes().to_vec();
    owner
        .store
        .call(move |s| {
            assert!(!s.operation_pending(&operation)?);
            assert_eq!(s.operation(&operation)?.submission, "CONFIRMED");
            assert_eq!(s.operation(&operation)?.attempts, 1);
            assert_eq!(s.prepared_bytes(&operation)?.as_slice(), raw);
            assert_eq!(s.prepared_inputs(&operation)?, inputs);
            assert_eq!(s.funding_jobs()?.len(), 1);
            assert_eq!(s.funding_recovery_count(&job.id)?, 0);
            Ok(())
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn pruned_checkpoint_leaf_rebuilds_canonical_sdk_state() {
    use incrementalmerkletree::{Address, Hashable, Level, Position};
    use orchard::tree::MerkleHashOrchard;
    use pepper_sync::shardtree_ext::ShardTreeExt;
    use shardtree::store::{Checkpoint, ShardStore as _};
    use shardtree::{LocatedPrunableTree, RetentionFlags, Tree};
    let (_dir, net, mut owner, _endpoint) = funded_fixture(1).await;
    let addresses = owner.addresses().await.unwrap();
    {
        let mut wallet = owner.client.as_ref().unwrap().wallet().write().await;
        let mut tree = pepper_sync::wallet::ShardTrees::new().orchard;
        let level = Level::from(16);
        // As in shardtree's pinned interior-pruning regression, only a
        // merged subtree hash remains while the old checkpoint is indexed.
        // This is damaged derived SDK cache data, never new chain evidence.
        tree.store_mut()
            .put_shard(
                LocatedPrunableTree::from_parts(
                    Address::from_parts(level, 0),
                    Tree::leaf((
                        MerkleHashOrchard::empty_root(level),
                        RetentionFlags::EPHEMERAL,
                    )),
                )
                .unwrap(),
            )
            .unwrap();
        tree.store_mut()
            .add_checkpoint(22.into(), Checkpoint::at_position(Position::from(0)))
            .unwrap();
        tree.store_mut()
            .add_checkpoint(
                23.into(),
                Checkpoint::at_position(Position::from((1u64 << 16) - 1)),
            )
            .unwrap();
        assert!(matches!(
            tree.rollback_to_checkpoint(22.into()),
            Err(shardtree::error::ShardTreeError::Query(
                shardtree::error::QueryError::CheckpointPruned
            ))
        ));
        wallet.shard_trees.orchard = tree;
    }
    net.chain.write().await.reorg_to(22);
    let stop = CancellationToken::new();
    assert!(owner.sync_once(&stop).await.is_err());
    {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        assert!(
            wallet.wallet_blocks.is_empty(),
            "the typed pruned-checkpoint rollback must use the full rescan fallback"
        );
        assert!(
            wallet
                .sync_state
                .scan_ranges()
                .iter()
                .all(|range| !range.priority().is_scanned())
        );
    }
    owner.sync_once(&stop).await.unwrap();
    assert_eq!(owner.addresses().await.unwrap(), addresses);
    let status = owner.status().await.unwrap();
    assert_eq!(status.sync.as_ref().unwrap().phase, SyncPhase::Ready);
    assert_eq!(status.sync.unwrap().confirmed_shielded_zatoshis, 1_000_000);
    assert!(!status.outgoing_pending);
    assert!(status.funding_jobs.is_empty());
    owner.close().await.unwrap();
}
