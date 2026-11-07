//! Seed-only shielded refund discovery through the actual pinned SDK.
use super::*;

// Each family covers both unary/stream or current/deprecated methods. A marker
// is consumed before processing, so failed or swallowed requests cannot hide.
async fn forbid_transparent_address_rpcs(net: &MockNet) {
    let mut chain = net.chain.write().await;
    for rpc in [Rpc::TaddressTxids, Rpc::TaddressBalance, Rpc::AddressUtxos] {
        chain.faults.inject(
            rpc,
            Fault::Fail(
                tonic::Code::PermissionDenied,
                "forbidden transparent address RPC".into(),
            ),
        );
    }
}
async fn assert_no_transparent_address_rpcs(net: &MockNet) {
    let chain = net.chain.read().await;
    assert!(chain.taddr_request_log().is_empty());
    for rpc in [Rpc::TaddressTxids, Rpc::TaddressBalance, Rpc::AddressUtxos] {
        assert_eq!(
            chain.faults.pending(rpc),
            1,
            "transparent address RPC reached indexer"
        );
    }
}

#[tokio::test]
async fn distant_shielded_refund_is_spendable_after_seed_only_restore() {
    let (_original_dir, net, mut original, endpoint) = funded_fixture(2).await;
    // Leave a gap wider than either transparent discovery setting. These UA
    // ordinals are never copied into the restored wallet.
    for _ in 0..64 {
        original
            .client
            .as_mut()
            .unwrap()
            .generate_unified_address(ReceiverSelection::orchard_only(), zip32::AccountId::ZERO)
            .await
            .unwrap();
    }
    crate::rotation::test_capacity::treasury_pair(&original, "refund", "5", None, 0)
        .await
        .unwrap();
    let job = original
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let refund = original.refund_address(job.id.clone()).await.unwrap();
    let zcash_keys::address::Address::Unified(receivers) =
        zcash_keys::address::Address::decode(&original.network.chain(), &refund).unwrap()
    else {
        panic!("new refund must be a unified address");
    };
    assert!(receivers.orchard().is_some() && receivers.sapling().is_none());
    assert!(receivers.transparent().is_none());
    assert_eq!(original.refund_address(job.id).await.unwrap(), refund);
    let receipt = net.funding_transaction(vec![(&refund, 40_000, None)]).await;
    let receipt_id = submission::decode(&receipt).unwrap().txid();
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![receipt]);
        chain.mine_empty_blocks(2);
    }
    original.close().await.unwrap();

    let restored_dir = tempfile::tempdir().unwrap();
    let mut restored = Treasury::create_with_network(
        restored_dir.path().join("state"),
        restored_dir.path().join("key"),
        1,
        Some(Zeroizing::new(SEED.into())),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let imported_settings = restored
        .client
        .as_ref()
        .unwrap()
        .wallet()
        .read()
        .await
        .wallet_settings
        .sync_config
        .transparent_address_discovery
        .clone();
    assert!(
        !imported_settings.scopes.external
            && !imported_settings.scopes.internal
            && !imported_settings.scopes.refund
    );
    let initial_addresses = restored.addresses().await.unwrap();
    assert!(!initial_addresses.to_string().contains(&refund));
    let initial = restored.status().await.unwrap();
    assert!(initial.funding_jobs.is_empty() && initial.treasury_operations.is_empty());
    forbid_transparent_address_rpcs(&net).await;
    restored.configure_sync(SyncSettings::new(endpoint, 2).unwrap());
    restored.sync_once(&CancellationToken::new()).await.unwrap();
    assert_no_transparent_address_rpcs(&net).await;
    let recovered = restored.status().await.unwrap();
    let sync = recovered.sync.unwrap();
    assert_eq!(sync.confirmed_shielded_zatoshis, 1_040_000);
    assert_eq!(sync.spendable_shielded_zatoshis, 1_040_000);
    assert!(recovered.funding_jobs.is_empty() && recovered.treasury_operations.is_empty());
    let wallet = restored.client.as_ref().unwrap().wallet().read().await;
    assert!(
        wallet
            .wallet_transactions
            .get(&receipt_id)
            .unwrap()
            .status()
            .get_confirmed_height()
            .is_some()
    );
    drop(wallet);
    restored
        .store
        .call(|s| {
            assert!(!s.status()?.outgoing_pending);
            assert!(s.funding_jobs()?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    restored.close().await.unwrap();
}

#[tokio::test]
async fn reopened_discovery_enabled_snapshot_preserves_binding_without_transparent_recovery() {
    let (dir, mut owner) = unfunded_fixture().await;
    crate::rotation::test_capacity::treasury_pair(&owner, "legacy_refund", "5", None, 0)
        .await
        .unwrap();
    let job = owner
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let (_, receiver) = owner
        .client
        .as_mut()
        .unwrap()
        .generate_transparent_address(zip32::AccountId::ZERO, false)
        .await
        .unwrap();
    let legacy = zcash_keys::address::Address::Transparent(receiver).encode(&owner.network.chain());
    // Model a current seed snapshot saved before the operator disabled recovery.
    owner
        .client
        .as_ref()
        .unwrap()
        .wallet()
        .write()
        .await
        .wallet_settings
        .sync_config
        .transparent_address_discovery = Default::default();
    let bytes = snapshot(owner.client.as_ref().unwrap()).await.unwrap();
    let revision = owner.revision;
    let (job_id, saved) = (job.id.clone(), legacy.clone());
    owner.revision = owner
        .store
        .call(move |s| s.save_refund_address(&job_id, &saved, revision, &bytes))
        .await
        .unwrap();
    let expected_revision = owner.revision;
    let identity = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut restored = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    assert!(
        restored
            .refund_address(job.id.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("shielded-only")
    );
    let saved_job = job.id.clone();
    let saved_binding = restored
        .store
        .call(move |s| s.refund_address(&saved_job))
        .await
        .unwrap();
    assert_eq!(saved_binding.as_deref(), Some(legacy.as_str()));
    let opened = restored
        .client
        .as_ref()
        .unwrap()
        .wallet()
        .read()
        .await
        .wallet_settings
        .sync_config
        .transparent_address_discovery
        .clone();
    assert!(!opened.scopes.external && !opened.scopes.internal && !opened.scopes.refund);
    // Reconstruction must also override the enabled policy still in committed bytes.
    restored
        .reconstruct_sdk(&CancellationToken::new())
        .await
        .unwrap();
    let discovery = restored
        .client
        .as_ref()
        .unwrap()
        .wallet()
        .read()
        .await
        .wallet_settings
        .sync_config
        .transparent_address_discovery
        .clone();
    assert!(!discovery.scopes.external && !discovery.scopes.internal && !discovery.scopes.refund);
    // A T-only late receipt does not authorize address-specific recovery.
    let net = MockNet::launch().await;
    forbid_transparent_address_rpcs(&net).await;
    let receipt = net
        .funding_transaction(vec![(&legacy, 100_000, None)])
        .await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![receipt]);
        chain.mine_empty_blocks(2);
    }
    restored.configure_sync(SyncSettings::new(net.indexer_uri().to_string(), 2).unwrap());
    restored.sync_once(&CancellationToken::new()).await.unwrap();
    assert_no_transparent_address_rpcs(&net).await;
    assert_eq!(
        restored
            .snapshot_balances()
            .await
            .unwrap()
            .confirmed_transparent,
        None
    );
    let status = restored.status().await.unwrap();
    assert!(status.snapshot_revision > expected_revision);
    assert!(status.treasury_operations.is_empty() && !status.outgoing_pending);
    restored.close().await.unwrap();
}

#[tokio::test]
async fn refund_binding_failure_returns_no_address_and_reopens_original_snapshot() {
    let (dir, mut owner) = unfunded_fixture().await;
    crate::rotation::test_capacity::treasury_pair(&owner, "refund_failure", "5", None, 0)
        .await
        .unwrap();
    let job = owner
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let before = owner.status().await.unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_refund_binding BEFORE INSERT ON funding_refunds BEGIN SELECT RAISE(ABORT,'fixture refund commit failure'); END;").unwrap();
    assert!(owner.refund_address(job.id.clone()).await.is_err());
    assert!(!owner.healthy);
    assert!(
        owner
            .refund_address(job.id.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("reopen")
    );
    let after = owner.status().await.unwrap();
    assert_eq!(after.snapshot_revision, before.snapshot_revision);
    assert!(after.treasury_operations.is_empty() && !after.outgoing_pending);
    let job_id = job.id.clone();
    owner
        .store
        .call(move |s| {
            assert!(s.refund_address(&job_id)?.is_none());
            assert_eq!(
                s.funding_job(&job_id)?.unwrap().phase,
                crate::rotation::store::funding::FundingPhase::Allocated
            );
            Ok(())
        })
        .await
        .unwrap();
    let quotes: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM funding_progress WHERE quote IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        quotes, 0,
        "no quote or source work may precede the durable refund binding"
    );
    db.execute_batch("DROP TRIGGER reject_refund_binding")
        .unwrap();
    owner.close().await.unwrap();
    let mut restored = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        before.treasury_id,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let address = restored.refund_address(job.id.clone()).await.unwrap();
    let zcash_keys::address::Address::Unified(receivers) =
        zcash_keys::address::Address::decode(&restored.network.chain(), &address).unwrap()
    else {
        panic!("new refund must be a unified address");
    };
    assert!(receivers.transparent().is_none());
    assert_eq!(restored.refund_address(job.id).await.unwrap(), address);
    let status = restored.status().await.unwrap();
    assert_eq!(status.snapshot_revision, before.snapshot_revision + 1);
    assert!(status.treasury_operations.is_empty() && !status.outgoing_pending);
    restored.close().await.unwrap();
}

#[tokio::test]
async fn saved_mixed_refund_rejects_new_transparent_shielding_without_side_effects() {
    use zcash_keys::address::UnifiedAddress;
    let (_dir, mut owner) = unfunded_fixture().await;
    let net = MockNet::launch().await;
    forbid_transparent_address_rpcs(&net).await;
    owner.configure_sync(SyncSettings::new(net.indexer_uri().to_string(), 2).unwrap());
    crate::rotation::test_capacity::treasury_pair(&owner, "mixed_refund", "5", None, 0)
        .await
        .unwrap();
    let job = owner
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let client = owner.client.as_mut().unwrap();
    let (_, shielded) = client
        .generate_unified_address(ReceiverSelection::orchard_only(), zip32::AccountId::ZERO)
        .await
        .unwrap();
    let (_, transparent) = client
        .generate_transparent_address(zip32::AccountId::ZERO, false)
        .await
        .unwrap();
    // Existing cached receiver facts are retained; they grant no new T-input work.
    let mixed = UnifiedAddress::from_receivers(
        Some(*shielded.orchard().unwrap()),
        None,
        Some(transparent),
        None,
        None,
    )
    .unwrap()
    .encode(&owner.network.chain());
    let bytes = snapshot(owner.client.as_ref().unwrap()).await.unwrap();
    let revision = owner.revision;
    let (job_id, saved) = (job.id.clone(), mixed.clone());
    owner.revision = owner
        .store
        .call(move |s| s.save_refund_address(&job_id, &saved, revision, &bytes))
        .await
        .unwrap();
    let before = owner.store.call(|s| Ok(s.snapshot()?.0)).await.unwrap();
    let error = owner
        .shield_refund(job.id.clone(), &CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("unsupported"), "{error:#}");
    owner
        .store
        .call(move |s| {
            assert_eq!(s.snapshot()?.0, before);
            assert_eq!(s.refund_address(&job.id)?.as_deref(), Some(mixed.as_str()));
            assert!(!s.status()?.outgoing_pending);
            assert!(s.status()?.treasury_operations.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    assert_no_transparent_address_rpcs(&net).await;
    owner.close().await.unwrap();
}

#[tokio::test]
async fn unsupported_refund_binding_is_rejected_before_sync_or_preparation() {
    for invalid in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut owner = Treasury::create_with_network(
            dir.path().join("state"),
            dir.path().join("key"),
            1,
            Some(Zeroizing::new(SEED.into())),
            TreasuryNetwork::Synthetic,
        )
        .await
        .unwrap();
        crate::rotation::test_capacity::treasury_pair(&owner, "unsupported_refund", "5", None, 0)
            .await
            .unwrap();
        let job = owner
            .store
            .call(|s| Ok(s.funding_jobs()?.remove(0)))
            .await
            .unwrap();
        let unsupported = if invalid {
            "invalid refund encoding".to_owned()
        } else {
            zcash_keys::address::Address::Tex([0; 20]).encode(&owner.network.chain())
        };
        let bytes = snapshot(owner.client.as_ref().unwrap()).await.unwrap();
        let revision = owner.revision;
        let (job_id, saved) = (job.id.clone(), unsupported.clone());
        owner.revision = owner
            .store
            .call(move |s| s.save_refund_address(&job_id, &saved, revision, &bytes))
            .await
            .unwrap();
        let before = owner.status().await.unwrap();
        // There is deliberately no indexer configuration. Rejection must happen
        // before a sync attempt, accepted input owner or operation UUID exists.
        let error = owner
            .shield_refund(job.id.clone(), &CancellationToken::new())
            .await
            .err()
            .expect("unsupported refund must be rejected");
        assert!(error.to_string().contains("unsupported"), "{error:#}");
        assert!(owner.healthy);
        let after = owner.status().await.unwrap();
        assert_eq!(after.snapshot_revision, before.snapshot_revision);
        assert!(after.treasury_operations.is_empty() && !after.outgoing_pending);
        let job_id = job.id;
        owner
            .store
            .call(move |s| {
                assert_eq!(
                    s.refund_address(&job_id)?.as_deref(),
                    Some(unsupported.as_str())
                );
                s.require_preparation_available()?;
                assert!(
                    s.qualification_state()?["pending_source_operations"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                Ok(())
            })
            .await
            .unwrap();
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn fresh_shielded_wallet_skips_transparent_rpc_and_reopens_receive_policy() {
    let dir = tempfile::tempdir().unwrap();
    let net = MockNet::launch().await;
    forbid_transparent_address_rpcs(&net).await;
    let mut owner = Treasury::create_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        1,
        None,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let addresses = owner.addresses().await.unwrap();
    let addresses = addresses.as_array().unwrap();
    for address in addresses {
        assert_eq!(address["receiver_capabilities"]["transparent"], false);
        assert_eq!(address["receiver_capabilities"]["ironwood"], true);
        assert!(address["receiver_capabilities"].get("sapling").is_none());
    }
    let receive = addresses
        .iter()
        .find(|address| address["receiver_capabilities"]["ironwood"] == true)
        .expect("fresh wallet must publish an Ironwood receive address");
    let recipient = receive["encoded_address"].as_str().unwrap().to_owned();
    let receipt = net
        .funding_transaction(vec![(&recipient, 100_000, None)])
        .await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![receipt]);
        chain.mine_empty_blocks(2);
    }
    owner.configure_sync(SyncSettings::new(net.indexer_uri().to_string(), 2).unwrap());
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        owner
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .spendable_shielded_zatoshis,
        100_000
    );
    assert_no_transparent_address_rpcs(&net).await;
    let identity = owner.status().await.unwrap().treasury_id;
    let before = owner.addresses().await.unwrap();
    owner.close().await.unwrap();
    let mut reopened = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    assert_eq!(reopened.addresses().await.unwrap(), before);
    reopened.configure_sync(SyncSettings::new(net.indexer_uri().to_string(), 2).unwrap());
    reopened.sync_once(&CancellationToken::new()).await.unwrap();
    assert_no_transparent_address_rpcs(&net).await;
    assert_eq!(
        reopened
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .spendable_shielded_zatoshis,
        100_000
    );
    reopened.close().await.unwrap();
}
