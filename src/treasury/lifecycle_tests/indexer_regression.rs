//! Characterize unsigned shorter-tip resets before the reviewed recovery change.
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
async fn shorter_prefix_characterizes_current_destructive_reset() {
    for (old_tip, shorter_tip) in [(100, 50), (1250, 150)] {
        let original = MockNet::launch().await;
        original.chain.write().await.mine_empty_blocks(old_tip);
        let (_dir, mut owner) = empty_owner(&original, 100).await;
        let stop = CancellationToken::new();
        owner.sync_once(&stop).await.unwrap();
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
        // Current defect: a height alone clears valid derived observations and
        // persists the activation floor, before a divergent hash is established.
        assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
        assert!(wallet.wallet_blocks.is_empty());
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
async fn interrupted_unsigned_target_characterizes_current_reset_of_completed_prefix() {
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
    assert!(wallet.wallet_blocks.contains_key(&100.into()));
    drop(wallet);
    let shorter = MockNet::launch().await;
    shorter.chain.write().await.mine_empty_blocks(150);
    shorter.chain.write().await.faults.inject(
        Rpc::BlockRange,
        Fault::Fail(
            tonic::Code::InvalidArgument,
            "stop after destructive target reset".into(),
        ),
    );
    owner.configure_sync(SyncSettings::new(shorter.indexer_uri().to_string(), 1).unwrap());
    assert!(owner.sync_once(&stop).await.is_err());
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
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
async fn same_prefix_shallow_regression_characterizes_current_loss_of_scanned_tail() {
    let original = MockNet::launch().await;
    original.chain.write().await.mine_empty_blocks(1250);
    let (_dir, mut owner) = empty_owner(&original, 100).await;
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    let shorter = MockNet::launch().await;
    shorter.chain.write().await.mine_empty_blocks(1200);
    owner.configure_sync(SyncSettings::new(shorter.indexer_uri().to_string(), 1).unwrap());
    owner.sync_once(&stop).await.unwrap();
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(100.into()));
    assert_eq!(
        wallet.sync_state.last_known_chain_height(),
        Some(1200.into())
    );
    assert!(!wallet.wallet_blocks.contains_key(&1250.into()));
    drop(wallet);
    owner.close().await.unwrap();
}
