//! Replacement-chain credits may predate the old chain's wallet birthday.
use super::*;

#[tokio::test]
async fn reorg_before_declared_birthday_keeps_earlier_coverage_through_failure_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let net = MockNet::launch().await;
    net.chain.write().await.mine_empty_blocks(100);
    let mut owner = Treasury::create_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        100,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let settings = SyncSettings::new(net.indexer_uri().to_string(), 1).unwrap();
    owner.configure_sync(settings.clone());
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
    let funding = faucet_funding_transaction(vec![(&address, 1_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.reorg_to(30);
        chain.mine_empty_blocks(9);
        chain.mine_block(vec![funding]);
        chain.mine_empty_blocks(10);
        // Save the lowered coverage before its first block can finish scanning.
        chain.faults.inject(
            Rpc::BlockRange,
            Fault::Fail(
                tonic::Code::InvalidArgument,
                "synthetic interrupted rescan".into(),
            ),
        );
    }
    // No old block overlaps tip50, so a true shorter fork is still ambiguous.
    // Preserve history until the endpoint can demonstrate a divergent block100.
    assert!(owner.sync_once(&CancellationToken::new()).await.is_err());
    {
        let wallet = owner.client.as_ref().unwrap().wallet().read().await;
        assert_eq!(wallet.sync_state.wallet_birthday(), Some(100.into()));
    }
    net.chain.write().await.mine_empty_blocks(50);
    let error = owner
        .sync_once(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        diagnostics::sync_detail(&error).as_deref(),
        Some("sync_scan_failed: reason=chain_verification_failed")
    );
    assert_eq!(net.chain.read().await.faults.pending(Rpc::BlockRange), 1);
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert!(wallet.wallet_blocks.is_empty());
    assert!(wallet.wallet_transactions.is_empty());
    assert!(
        wallet
            .sync_state
            .scan_ranges()
            .iter()
            .all(|range| !range.priority().is_scanned())
    );
    assert_eq!(
        u32::from(wallet.birthday()),
        100,
        "declared metadata remains intact"
    );
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
    drop(wallet);
    // The proven-clear cycle ends before BlockRange. Exercise the intended
    // interrupted rescan separately, then reopen its saved earlier coverage.
    let error = owner
        .sync_once(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        diagnostics::sync_detail(&error).as_deref(),
        Some(
            "sync_scan_failed: reason=indexer_request_failed grpc_code=3 grpc_status=InvalidArgument"
        )
    );
    assert_eq!(net.chain.read().await.faults.pending(Rpc::BlockRange), 0);
    assert_eq!(
        owner
            .client
            .as_ref()
            .unwrap()
            .wallet()
            .read()
            .await
            .sync_state
            .wallet_birthday(),
        Some(1.into())
    );
    let id = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        id.clone(),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(settings.clone());
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        owner
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .confirmed_shielded_zatoshis,
        1_000_000,
        "credit at replacement height40 must not be skipped by old birthday100 or observed tip50"
    );
    // A later full clear must retain this earlier floor, rather than silently
    // reverting to the immutable birthday and dropping that owned credit.
    net.chain
        .write()
        .await
        .mine_empty_blocks(pepper_sync::sync::MAX_REORG_ALLOWANCE + 1);
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    let old_tip = net.chain.read().await.tip();
    net.chain.write().await.reorg_to(50);
    let error = owner
        .sync_once(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        diagnostics::sync_detail(&error).as_deref(),
        Some("sync_scan_failed: reason=indexer_tip_regressed_without_divergence")
    );
    net.chain.write().await.mine_empty_blocks(old_tip - 50);
    let error = owner
        .sync_once(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        diagnostics::sync_detail(&error).as_deref(),
        Some("sync_scan_failed: reason=chain_verification_failed")
    );
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(u32::from(wallet.birthday()), 100);
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
    assert!(wallet.wallet_blocks.is_empty());
    assert!(wallet.wallet_transactions.is_empty());
    assert!(
        wallet
            .sync_state
            .scan_ranges()
            .iter()
            .all(|range| !range.priority().is_scanned())
    );
    drop(wallet);
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        id,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(settings);
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        owner
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .confirmed_shielded_zatoshis,
        1_000_000
    );
    owner.close().await.unwrap();
}

#[tokio::test]
async fn unknown_deep_fork_rescans_before_birthday_even_when_tip_remains_above_it() {
    let dir = tempfile::tempdir().unwrap();
    let net = MockNet::launch().await;
    net.chain.write().await.mine_empty_blocks(1_250);
    let mut owner = Treasury::create_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        100,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let settings = SyncSettings::new(net.indexer_uri().to_string(), 1).unwrap();
    owner.configure_sync(settings.clone());
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
    let credit = faucet_funding_transaction(vec![(&address, 1_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.reorg_to(80);
        chain.mine_empty_blocks(9);
        chain.mine_block(vec![credit]);
        chain.mine_empty_blocks(60);
    }
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(u32::from(wallet.birthday()), 100);
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
    drop(wallet);
    let id = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        id,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(settings);
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        owner
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .confirmed_shielded_zatoshis,
        1_000_000,
        "credit90 before declared birthday100 remains visible at replacement tip150"
    );
    owner.close().await.unwrap();
}

#[tokio::test]
async fn same_height_fork_before_birthday_restarts_complete_ranges_after_full_clear() {
    let dir = tempfile::tempdir().unwrap();
    let net = MockNet::launch().await;
    net.chain.write().await.mine_empty_blocks(500);
    let mut owner = Treasury::create_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        100,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let settings = SyncSettings::new(net.indexer_uri().to_string(), 1).unwrap();
    owner.configure_sync(settings.clone());
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
    let credit = faucet_funding_transaction(vec![(&address, 1_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.reorg_to(80);
        chain.mine_empty_blocks(9);
        chain.mine_block(vec![credit]);
        chain.mine_empty_blocks(410);
    }
    assert!(owner.sync_once(&CancellationToken::new()).await.is_err());
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    assert_eq!(u32::from(wallet.birthday()), 100);
    assert_eq!(wallet.sync_state.wallet_birthday(), Some(1.into()));
    drop(wallet);
    let id = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        id,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(settings);
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        owner
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .confirmed_shielded_zatoshis,
        1_000_000,
        "credit90 before declared birthday100 remains visible at replacement tip500"
    );
    owner.close().await.unwrap();
}
