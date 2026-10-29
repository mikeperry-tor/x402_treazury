//! Seed-only shielded refund discovery through the actual pinned SDK.
use super::*;

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
            .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
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
    assert!(receivers.orchard().is_some() && receivers.sapling().is_some());
    assert!(receivers.transparent().is_none());
    assert_eq!(original.refund_address(job.id).await.unwrap(), refund);
    let receipt = faucet_funding_transaction(vec![(&refund, 40_000, None)]).await;
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
    let initial_addresses = restored.addresses().await.unwrap();
    assert!(!initial_addresses.to_string().contains(&refund));
    let initial = restored.status().await.unwrap();
    assert!(initial.funding_jobs.is_empty() && initial.treasury_operations.is_empty());
    restored.configure_sync(SyncSettings::new(endpoint, 2).unwrap());
    restored.sync_once(&CancellationToken::new()).await.unwrap();
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
async fn legacy_transparent_refund_binding_survives_encrypted_reopen() {
    let (dir, _net, mut owner, _endpoint) = funded_fixture(2).await;
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
    assert_eq!(restored.refund_address(job.id).await.unwrap(), legacy);
    let status = restored.status().await.unwrap();
    assert_eq!(status.snapshot_revision, expected_revision);
    assert!(status.treasury_operations.is_empty() && !status.outgoing_pending);
    restored.close().await.unwrap();
}

#[tokio::test]
async fn refund_binding_failure_returns_no_address_and_reopens_original_snapshot() {
    let (dir, _net, mut owner, _endpoint) = funded_fixture(2).await;
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
async fn mixed_ua_refund_shields_owned_transparent_receiver_without_rebinding() {
    use crate::rotation::store::preparation::InputPool;
    use zcash_keys::address::{Address, UnifiedAddress};
    for orchard in [true, false] {
        let (_dir, net, mut owner, _endpoint) = funded_fixture(2).await;
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
            .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
            .await
            .unwrap();
        let (_, transparent) = client
            .generate_transparent_address(zip32::AccountId::ZERO, false)
            .await
            .unwrap();
        // Current SDK generation keeps these separate. Compose a cached legacy
        // UA from genuinely owned receivers, retaining its exact encoding.
        let mixed = UnifiedAddress::from_receivers(
            orchard.then(|| shielded.orchard().cloned().unwrap()),
            (!orchard).then(|| shielded.sapling().cloned().unwrap()),
            Some(transparent),
            None,
            None,
        )
        .unwrap()
        .encode_receiver_preserving(&owner.network.chain());
        let Address::Unified(decoded) = Address::decode(&owner.network.chain(), &mixed).unwrap()
        else {
            panic!("mixed refund fixture must preserve its transparent receiver");
        };
        assert_eq!(decoded.transparent(), Some(&transparent));
        let bytes = snapshot(owner.client.as_ref().unwrap()).await.unwrap();
        let revision = owner.revision;
        let (job_id, saved) = (job.id.clone(), mixed.clone());
        owner.revision = owner
            .store
            .call(move |s| s.save_refund_address(&job_id, &saved, revision, &bytes))
            .await
            .unwrap();
        let receiver = Address::Transparent(transparent).encode(&owner.network.chain());
        let funding = faucet_funding_transaction(vec![(&receiver, 1_000_000, None)]).await;
        let funding_id = submission::decode(&funding).unwrap().txid().to_string();
        {
            let mut chain = net.chain.write().await;
            chain.mine_block(vec![funding]);
            chain.mine_empty_blocks(2);
        }
        let prepared = owner
            .shield_refund(job.id.clone(), 30_000, &CancellationToken::new())
            .await
            .unwrap();
        let transaction = submission::decode(prepared.bytes()).unwrap();
        let inputs = transaction.transparent_bundle().unwrap();
        assert_eq!(inputs.vin.len(), 1);
        assert!(inputs.vout.is_empty());
        assert_eq!(prepared.facts().unwrap().amount_zatoshis, 0);
        assert!(prepared.facts().unwrap().fee_zatoshis > 0);
        let (operation, job_id, saved) = (prepared.operation_id().to_owned(), job.id, mixed);
        let exact_raw = prepared.bytes().to_vec();
        owner
            .store
            .call(move |s| {
                assert_eq!(s.refund_address(&job_id)?.as_deref(), Some(saved.as_str()));
                assert_eq!(s.prepared_bytes(&operation)?.as_slice(), exact_raw);
                assert_eq!(
                    s.operation(&operation)?.attempts,
                    0,
                    "calculation must not broadcast"
                );
                assert_eq!(s.operation(&operation)?.submission, "PREPARED");
                let selected = s.prepared_inputs(&operation)?;
                assert_eq!(selected.len(), 1);
                assert_eq!(selected[0].pool, InputPool::Transparent);
                assert_eq!(selected[0].txid, funding_id);
                assert_eq!(selected[0].output_index, 0);
                Ok(())
            })
            .await
            .unwrap();
        owner.close().await.unwrap();
    }
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
            .shield_refund(job.id.clone(), 30_000, &CancellationToken::new())
            .await
            .err()
            .expect("unsupported refund must be rejected");
        assert!(!error.is::<crate::treasury::NoShieldingRequired>());
        assert!(
            error.to_string().contains(if invalid {
                "invalid refund address"
            } else {
                "TEX addresses are unsupported for refunds"
            }),
            "{error:#}"
        );
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
