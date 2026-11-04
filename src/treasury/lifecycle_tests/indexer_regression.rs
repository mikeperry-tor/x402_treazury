//! Shorter heights preserve established observations unless divergence is demonstrated.
use super::*;

async fn empty_owner(net: &MockNet, birthday: u32) -> (tempfile::TempDir, Treasury) {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = Treasury::create_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        birthday,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(net.indexer_uri().to_string(), 1).unwrap());
    (dir, owner)
}

#[tokio::test]
async fn shorter_prefix_preserves_established_history_and_later_healthy_sync_resumes() {
    for (old_tip, shorter_tip) in [(100, 50), (1250, 150), (1250, 1200)] {
        let original = MockNet::launch().await;
        original.chain.write().await.mine_empty_blocks(old_tip);
        let (_dir, mut owner) = empty_owner(&original, 100).await;
        let stop = CancellationToken::new();
        owner.sync_once(&stop).await.unwrap();
        let expected = {
            let wallet = owner.client.as_ref().unwrap().wallet().read().await;
            assert_eq!(wallet.sync_state.wallet_birthday(), Some(100.into()));
            assert!(wallet.wallet_blocks.contains_key(&100.into()));
            assert!(wallet.wallet_blocks.contains_key(&old_tip.into()));
            let mut scan = Vec::new();
            wallet.sync_state.clone().write(&mut scan).unwrap();
            (
                scan,
                wallet
                    .wallet_blocks
                    .iter()
                    .map(|(height, block)| (*height, block.block_hash()))
                    .collect::<Vec<_>>(),
            )
        };
        let shorter = MockNet::launch().await;
        shorter.chain.write().await.mine_empty_blocks(shorter_tip);
        shorter.chain.write().await.faults.inject(
            Rpc::BlockRange,
            Fault::Fail(
                tonic::Code::InvalidArgument,
                "stop after destructive startup reset".into(),
            ),
        );
        owner.configure_sync(SyncSettings::new(shorter.indexer_uri().to_string(), 1).unwrap());
        assert!(owner.sync_once(&stop).await.is_err());
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        assert_eq!(u32::from(wallet.birthday()), 100);
        assert_eq!(wallet.sync_state.wallet_birthday(), Some(100.into()));
        let mut scan = Vec::new();
        wallet.sync_state.clone().write(&mut scan).unwrap();
        assert_eq!(scan, expected.0);
        assert_eq!(
            wallet
                .wallet_blocks
                .iter()
                .map(|(height, block)| (*height, block.block_hash()))
                .collect::<Vec<_>>(),
            expected.1
        );
        assert_eq!(
            shorter.chain.read().await.faults.pending(Rpc::BlockRange),
            1,
            "ambiguous regression must not begin a rescan"
        );
        drop(wallet);
        owner.configure_sync(SyncSettings::new(original.indexer_uri().to_string(), 1).unwrap());
        owner.sync_once(&stop).await.unwrap();
        assert_eq!(
            owner.status().await.unwrap().sync.unwrap().phase,
            SyncPhase::Ready
        );
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn interrupted_unsigned_target_shrinks_only_unscanned_tail_and_keeps_completed_prefix() {
    let original = MockNet::launch().await;
    original.chain.write().await.mine_empty_blocks(100);
    let (_dir, mut owner) = empty_owner(&original, 100).await;
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    original.chain.write().await.mine_empty_blocks(1150);
    original.chain.write().await.faults.inject(
        Rpc::BlockRange,
        Fault::Fail(
            tonic::Code::InvalidArgument,
            "interrupt new target before scanning".into(),
        ),
    );
    assert!(owner.sync_once(&stop).await.is_err());
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(
        wallet.sync_state.last_known_chain_height(),
        Some(1250.into())
    );
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(100.into()));
    assert!(wallet.wallet_blocks.contains_key(&100.into()));
    drop(wallet);
    let shorter = MockNet::launch().await;
    shorter.chain.write().await.mine_empty_blocks(150);
    owner.configure_sync(SyncSettings::new(shorter.indexer_uri().to_string(), 1).unwrap());
    owner.sync_once(&stop).await.unwrap();
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(100.into()));
    assert_eq!(
        wallet.sync_state.last_known_chain_height(),
        Some(150.into())
    );
    assert!(wallet.wallet_blocks.contains_key(&100.into()));
    drop(wallet);
    owner.close().await.unwrap();
}

#[tokio::test]
async fn fresh_future_birthday_has_no_established_history_and_scans_supported_activation() {
    let net = MockNet::launch().await;
    net.chain.write().await.mine_empty_blocks(50);
    let (_dir, mut owner) = empty_owner(&net, 100).await;
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(u32::from(wallet.birthday()), 100);
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
    drop(wallet);
    owner.close().await.unwrap();
}

#[tokio::test]
async fn matching_old_anchor_cannot_distinguish_a_fork_above_that_anchor() {
    let original = MockNet::launch().await;
    original.chain.write().await.mine_empty_blocks(1250);
    let (_dir, mut owner) = empty_owner(&original, 100).await;
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    let replacement = MockNet::launch().await;
    {
        let mut chain = replacement.chain.write().await;
        chain.mine_empty_blocks(119);
        chain.reorg_to(119);
        chain.mine_empty_blocks(31);
    }
    owner.configure_sync(SyncSettings::new(replacement.indexer_uri().to_string(), 1).unwrap());
    assert!(owner.sync_once(&stop).await.is_err());
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(100.into()));
    assert_eq!(
        wallet.sync_state.last_known_chain_height(),
        Some(1250.into())
    );
    drop(wallet);
    replacement.chain.write().await.mine_empty_blocks(1100);
    owner.sync_once(&stop).await.unwrap();
    assert_eq!(
        owner.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Ready
    );
    owner.close().await.unwrap();
}

// Author a small interrupted SDK snapshot through its public versioned reader.
// These fixtures have no pending scan targets; each count is a one-byte CompactSize.
fn interrupted_state(
    ranges: &[(u32, u32, pepper_sync::sync::ScanPriority)],
    ironwood_shards: &[(u32, u32)],
    transparent_floor: u32,
) -> pepper_sync::wallet::SyncState {
    let mut encoded = vec![5, u8::try_from(ranges.len()).unwrap()];
    for (start, end, priority) in ranges {
        encoded.extend(start.to_le_bytes());
        encoded.extend(end.to_le_bytes());
        encoded.push(*priority as u8);
    }
    encoded.extend([0, 0]); // Sapling and Orchard shard ranges.
    encoded.push(u8::try_from(ironwood_shards.len()).unwrap());
    for (start, end) in ironwood_shards {
        encoded.extend(start.to_le_bytes());
        encoded.extend(end.to_le_bytes());
    }
    encoded.extend([0, 1]); // Targets and present transparent floor.
    encoded.extend(transparent_floor.to_le_bytes());
    pepper_sync::wallet::SyncState::read(encoded.as_slice()).unwrap()
}

#[tokio::test]
async fn interrupted_scanned_and_verifying_ranges_keep_established_observations() {
    use pepper_sync::sync::ScanPriority;
    for priority in [
        ScanPriority::ScannedWithoutMapping,
        ScanPriority::RefetchingNullifiers,
        ScanPriority::Verify,
    ] {
        let original = MockNet::launch().await;
        original.chain.write().await.mine_empty_blocks(100);
        let (_dir, mut owner) = empty_owner(&original, 100).await;
        let stop = CancellationToken::new();
        owner.sync_once(&stop).await.unwrap();
        let block_hash = {
            let mut wallet = owner.client.as_ref().unwrap().wallet().write().await;
            wallet.sync_state = interrupted_state(&[(100, 101, priority)], &[], 100);
            wallet.wallet_blocks[&100.into()].block_hash()
        };
        let shorter = MockNet::launch().await;
        shorter.chain.write().await.mine_empty_blocks(50);
        shorter.chain.write().await.faults.inject(
            Rpc::BlockRange,
            Fault::Fail(
                tonic::Code::InvalidArgument,
                "must not scan ambiguous history".into(),
            ),
        );
        owner.configure_sync(SyncSettings::new(shorter.indexer_uri().to_string(), 1).unwrap());
        assert!(owner.sync_once(&stop).await.is_err());
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        assert_eq!(wallet.sync_state.wallet_birthday(), Some(100.into()));
        assert_eq!(wallet.wallet_blocks[&100.into()].block_hash(), block_hash);
        let range = &wallet.sync_state.scan_ranges()[0];
        assert_eq!(range.block_range(), &(100.into()..101.into()));
        assert_eq!(
            range.priority(),
            if priority == ScanPriority::RefetchingNullifiers {
                ScanPriority::ScannedWithoutMapping
            } else {
                priority
            }
        );
        drop(wallet);
        assert_eq!(
            shorter.chain.read().await.faults.pending(Rpc::BlockRange),
            1
        );
        owner.configure_sync(SyncSettings::new(original.indexer_uri().to_string(), 1).unwrap());
        owner.sync_once(&stop).await.unwrap();
        assert_eq!(
            owner.status().await.unwrap().sync.unwrap().phase,
            SyncPhase::Ready
        );
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn confirmed_partial_merge_above_published_scan_bounds_prevents_tail_only_reset() {
    use pepper_sync::sync::ScanPriority;
    let (_dir, _net, mut owner, endpoint) = funded_fixture(1).await;
    let transactions = {
        let mut wallet = owner.client.as_ref().unwrap().wallet().write().await;
        // process_scan_results merges confirmed transactions before publishing its
        // scanned blocks/range. Model that interrupted boundary with the real
        // owned transaction at21 and only the preceding completed range at1..21.
        wallet.sync_state = interrupted_state(
            &[
                (1, 21, ScanPriority::Scanned),
                (21, 24, ScanPriority::Historic),
            ],
            &[],
            23,
        );
        wallet
            .wallet_blocks
            .retain(|height, _| u32::from(*height) <= 20);
        assert!(wallet.wallet_blocks.contains_key(&1.into()));
        assert!(wallet.wallet_blocks.contains_key(&20.into()));
        wallet
            .wallet_transactions
            .iter()
            .map(|(txid, transaction)| {
                let mut bytes = Vec::new();
                transaction.transaction().write(&mut bytes).unwrap();
                (*txid, transaction.status().get_confirmed_height(), bytes)
            })
            .collect::<Vec<_>>()
    };
    assert!(
        transactions
            .iter()
            .any(|(_, height, _)| *height == Some(21.into()))
    );
    let shorter = MockNet::launch().await;
    shorter.chain.write().await.mine_empty_blocks(20);
    owner.configure_sync(SyncSettings::new(shorter.indexer_uri().to_string(), 1).unwrap());
    assert!(owner.sync_once(&CancellationToken::new()).await.is_err());
    assert!(
        owner
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .last_error
            .unwrap()
            .contains("indexer_tip_regressed_without_divergence")
    );
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
    assert_eq!(wallet.sync_state.last_known_chain_height(), Some(23.into()));
    for (txid, height, bytes) in transactions {
        let transaction = &wallet.wallet_transactions[&txid];
        assert_eq!(transaction.status().get_confirmed_height(), height);
        let mut retained = Vec::new();
        transaction.transaction().write(&mut retained).unwrap();
        assert_eq!(retained, bytes);
    }
    drop(wallet);
    owner.configure_sync(SyncSettings::new(endpoint, 1).unwrap());
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    let sync = owner.status().await.unwrap().sync.unwrap();
    assert_eq!(sync.phase, SyncPhase::Ready);
    assert_eq!(sync.confirmed_shielded_zatoshis, 1_000_000);
    assert_eq!(sync.spendable_shielded_zatoshis, 1_000_000);
    owner.close().await.unwrap();
}

#[tokio::test]
async fn future_cached_roots_preserve_current_spendability_across_tail_shrink_and_reopen() {
    use incrementalmerkletree::{Address, Hashable, Level};
    use orchard::tree::MerkleHashOrchard;
    use pepper_sync::{sync::ScanPriority, wallet::NoteInterface as _};
    use shardtree::{LocatedPrunableTree, RetentionFlags, store::ShardStore as _};
    use std::sync::Arc;
    let (dir, net, mut owner, endpoint) = funded_fixture(1).await;
    let (position, expected_root, expected_path) = {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        let position = wallet
            .wallet_transactions
            .values()
            .flat_map(|tx| tx.ironwood_notes())
            .find_map(|note| note.position())
            .unwrap();
        let tree = &wallet.shard_trees.ironwood;
        let root = tree.root_at_checkpoint_id(&23.into()).unwrap().unwrap();
        let witness = tree
            .witness_at_checkpoint_id(position, &23.into())
            .unwrap()
            .unwrap();
        (position, root, witness.path_elems().to_vec())
    };
    {
        let mut wallet = owner.client.as_ref().unwrap().wallet().write().await;
        // A failed future sync fetched complete subtree metadata before it scanned
        // that target. Preserve the current shard's real leaves, adding precisely
        // the fetched whole-shard annotation and a future bare root.
        let level = Level::from(16);
        let future_root = MerkleHashOrchard::empty_root(level);
        let tree = &mut wallet.shard_trees.ironwood;
        let address = Address::from_parts(level, 0);
        let current = tree.store().get_shard(address).unwrap().unwrap();
        tree.store_mut()
            .put_shard(current.reannotate_root(Some(Arc::new(future_root))))
            .unwrap();
        tree.store_mut()
            .put_shard(LocatedPrunableTree::with_root_value(
                Address::from_parts(level, 1),
                (future_root, RetentionFlags::EPHEMERAL),
            ))
            .unwrap();
        wallet.sync_state = interrupted_state(
            &[
                (1, 24, ScanPriority::Scanned),
                (24, 1251, ScanPriority::Historic),
            ],
            &[(1, 1001), (1000, 1251)],
            1250,
        );
    }
    // Fail after safe tail normalization but before subtree-root ingestion. Local
    // note selection must already see coherent metadata even in this failed scan.
    net.chain.write().await.faults.inject(
        Rpc::Block,
        Fault::Fail(
            tonic::Code::InvalidArgument,
            "stop before root update".into(),
        ),
    );
    let failure = owner
        .sync_once(&CancellationToken::new())
        .await
        .unwrap_err();
    assert!(format!("{failure:#}").contains("indexer_request_failed grpc_code=3"));
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
    assert_eq!(wallet.sync_state.last_known_chain_height(), Some(23.into()));
    assert!(wallet.sync_state.ironwood_shard_ranges().is_empty());
    let tree = &wallet.shard_trees.ironwood;
    assert_eq!(
        tree.root_at_checkpoint_id(&23.into()).unwrap(),
        Some(expected_root)
    );
    assert_eq!(
        tree.witness_at_checkpoint_id(position, &23.into())
            .unwrap()
            .unwrap()
            .path_elems(),
        expected_path
    );
    assert!(
        tree.store()
            .get_shard(Address::from_parts(Level::from(16), 1))
            .unwrap()
            .is_some()
    );
    drop(wallet);
    let sync = owner.status().await.unwrap().sync.unwrap();
    assert_eq!(sync.phase, SyncPhase::Failed);
    assert_eq!(sync.confirmed_shielded_zatoshis, 1_000_000);
    assert_eq!(sync.spendable_shielded_zatoshis, 1_000_000);
    owner.sync_once(&CancellationToken::new()).await.unwrap();

    // A previous binary can have saved the shortened target with stale future
    // metadata. Reopening at the SAME tip must repair it before fallible I/O too.
    owner
        .client
        .as_ref()
        .unwrap()
        .wallet()
        .write()
        .await
        .sync_state = interrupted_state(
        &[(1, 24, ScanPriority::Scanned)],
        &[(1, 1001), (1000, 1251)],
        23,
    );
    let identity = owner.status().await.unwrap().treasury_id;
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
    {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        assert_eq!(wallet.sync_state.last_known_chain_height(), Some(23.into()));
        assert_eq!(wallet.sync_state.ironwood_shard_ranges().len(), 2);
        assert_eq!(
            wallet
                .shielded_spendable_balance(zip32::AccountId::ZERO, false)
                .unwrap()
                .into_u64(),
            0,
            "the persisted future completion metadata really blocks SDK selection"
        );
    }
    net.chain.write().await.faults.inject(
        Rpc::Block,
        Fault::Fail(
            tonic::Code::InvalidArgument,
            "stop before root update".into(),
        ),
    );
    let failure = owner
        .sync_once(&CancellationToken::new())
        .await
        .unwrap_err();
    assert!(format!("{failure:#}").contains("indexer_request_failed grpc_code=3"));
    let sync = owner.status().await.unwrap().sync.unwrap();
    assert_eq!(sync.phase, SyncPhase::Failed);
    assert_eq!(sync.spendable_shielded_zatoshis, 1_000_000);
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    let address = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let credit = faucet_funding_transaction(vec![(&address, 2_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![credit]);
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(wallet.sync_state.last_known_chain_height(), Some(26.into()));
    let tree = &wallet.shard_trees.ironwood;
    let current_root = tree.root_at_checkpoint_id(&26.into()).unwrap().unwrap();
    assert_ne!(
        current_root, expected_root,
        "healthy progress must insert new commitments"
    );
    assert!(
        tree.witness_at_checkpoint_id(position, &26.into())
            .unwrap()
            .is_some()
    );
    assert_eq!(
        tree.root_at_checkpoint_id(&23.into()).unwrap(),
        Some(expected_root)
    );
    assert_eq!(
        tree.witness_at_checkpoint_id(position, &23.into())
            .unwrap()
            .unwrap()
            .path_elems(),
        expected_path
    );
    let new_positions = wallet
        .wallet_transactions
        .values()
        .filter(|tx| tx.status().get_confirmed_height() == Some(24.into()))
        .flat_map(|tx| tx.ironwood_notes())
        .filter_map(|note| note.position())
        .collect::<Vec<_>>();
    assert!(!new_positions.is_empty());
    for new_position in new_positions {
        assert_ne!(new_position, position);
        assert!(
            tree.witness_at_checkpoint_id(new_position, &26.into())
                .unwrap()
                .is_some()
        );
    }
    drop(wallet);
    let sync = owner.status().await.unwrap().sync.unwrap();
    assert_eq!(sync.phase, SyncPhase::Ready);
    assert_eq!(sync.confirmed_shielded_zatoshis, 3_000_000);
    assert_eq!(sync.spendable_shielded_zatoshis, 3_000_000);
    owner.close().await.unwrap();
}
