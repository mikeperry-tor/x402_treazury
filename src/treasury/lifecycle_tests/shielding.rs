//! Actual pinned SDK transparent selection, proof, journal and confirmation.
use super::*;
use pepper_sync::wallet::{KeyIdInterface, OutputInterface};

struct NoSubmission;
impl TransactionSubmission for NoSubmission {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("forbidden submission")
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("forbidden lookup")
    }
}

// Model loss before delivery after durable broadcast intent. Lookup and
// preflight still use the real local indexer; no replacement bytes are sent.
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

#[tokio::test]
async fn automatic_shielding_expiry_requires_every_selected_input_before_recovery() {
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let addresses = transparent_addresses(&mut owner, 1).await;
    let small = faucet_funding_transaction(vec![(&addresses[0], 20_000, None)]).await;
    let large = faucet_funding_transaction(vec![(&addresses[0], 1_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![small]);
        chain.mine_block(vec![large]);
        chain.mine_empty_blocks(2);
    }
    let stop = CancellationToken::new();
    owner.configure_automatic_shielding(30_000);
    owner.sync_once(&stop).await.unwrap();
    let mut sender = Undelivered {
        sender: submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        attempts: 0,
    };
    owner
        .maintain_owned_transparent(&mut sender, &stop)
        .await
        .unwrap();
    let prepared = load_pending(&owner).await;
    let operation = prepared.operation_id().to_owned();
    let expiry = prepared.facts().unwrap().expiry_height;
    assert!(expiry > 0);
    let selected = owner
        .store
        .call({
            let operation = operation.clone();
            move |s| s.prepared_inputs(&operation)
        })
        .await
        .unwrap();
    assert_eq!(
        selected.len(),
        2,
        "both actual transparent outputs selected"
    );
    assert!(
        selected
            .iter()
            .all(|i| i.pool == crate::rotation::store::preparation::InputPool::Transparent)
    );
    owner
        .maintain_owned_transparent(&mut sender, &stop)
        .await
        .unwrap();
    assert_eq!(load_pending(&owner).await.bytes(), prepared.bytes());
    assert_eq!(
        sender.attempts, 1,
        "absence before expiry never permits replay"
    );

    // Expire an intent that never reached the node. The SDK must actually
    // invalidate its calculated transaction and make the inputs unspent again.
    {
        let mut chain = net.chain.write().await;
        let blocks = expiry + 2 - chain.tip();
        chain.mine_empty_blocks(blocks);
    }
    owner.sync_once(&stop).await.unwrap();

    // Exercise the manifest-to-transaction validator with actual SDK snapshots,
    // not just its value helper. A missing small input cannot be replaced by a
    // sufficient aggregate remainder. Use an isolated copy, leaving the owner
    // unchanged for the full automatic expiry path below.
    let original_bytes = owner
        .store
        .call({
            let operation = operation.clone();
            move |s| s.prepared_snapshot(&operation)
        })
        .await
        .unwrap();
    let original = LightClient::from_reader(
        original_bytes.as_slice(),
        config(owner._scratch.path(), WalletConfig::Read, owner.network).unwrap(),
    )
    .await
    .unwrap();
    let current_bytes = snapshot(owner.client.as_ref().unwrap()).await.unwrap();
    let missing = LightClient::from_reader(
        current_bytes.as_slice(),
        config(owner._scratch.path(), WalletConfig::Read, owner.network).unwrap(),
    )
    .await
    .unwrap();
    let original = original.wallet().read().await;
    let mut current = missing.wallet().write().await;
    let small_id = current
        .wallet_outputs::<pepper_sync::wallet::TransparentCoin>()
        .into_iter()
        .find(|output| output.value() == 20_000)
        .unwrap()
        .output_id()
        .txid();
    current.wallet_transactions.remove(&small_id);
    assert!(
        current
            .wallet_outputs::<pepper_sync::wallet::TransparentCoin>()
            .iter()
            .map(|o| o.value())
            .sum::<u64>()
            > prepared.facts().unwrap().fee_zatoshis
    );
    let txid =
        zcash_primitives::transaction::TxId::from_hex(&prepared.facts().unwrap().txid).unwrap();
    let error = super::super::expiry::check_inputs::<pepper_sync::wallet::TransparentCoin>(
        &original,
        &current,
        &txid,
        &selected,
        crate::rotation::store::preparation::InputPool::Transparent,
        u64::from(expiry) + 2,
        2,
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<crate::rotation::transaction::RecoveryEvidenceError>(),
        Some(&crate::rotation::transaction::RecoveryEvidenceError::InputMissing),
        "{error:#}"
    );
    drop(current);
    drop(original);
    assert_eq!(load_pending(&owner).await.bytes(), prepared.bytes());
    assert_eq!(sender.attempts, 1);

    // The real owner still has every original outpoint. Exercise automatic
    // manifest-backed two-scan release rather than calling Store directly.
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE fixture_completed_scans (observation TEXT NOT NULL);
        CREATE TRIGGER record_completed_scan AFTER INSERT ON treasury_sync
        WHEN json_extract(NEW.observation,'$.phase')='ready'
        BEGIN INSERT INTO fixture_completed_scans VALUES(NEW.observation); END;",
    )
    .unwrap();
    owner
        .maintain_owned_transparent(&mut sender, &stop)
        .await
        .unwrap();
    let completed: Vec<SyncObservation> = db
        .prepare("SELECT observation FROM fixture_completed_scans")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|value| serde_json::from_str(&value.unwrap()).unwrap())
        .collect();
    assert_eq!(
        completed.len(),
        2,
        "expiry performs two new completed scans, not just checkpoints"
    );
    assert!(
        completed
            .iter()
            .all(|scan| scan.at_observed_tip() && scan.height.unwrap() >= u64::from(expiry) + 2)
    );
    assert!(!owner.status().await.unwrap().outgoing_pending);
    owner
        .store
        .call({
            let operation = operation.clone();
            let bytes = prepared.bytes().to_vec();
            move |s| {
                assert_eq!(s.operation(&operation)?.submission, "EXPIRED");
                assert_eq!(s.operation(&operation)?.attempts, 1);
                assert_eq!(s.prepared_bytes(&operation)?.as_slice(), bytes);
                Ok(())
            }
        })
        .await
        .unwrap();
    // Only the next pass may select newly spendable capital under current
    // authority. It calculates new bytes after proven expiry, never rebroadcasts.
    owner
        .maintain_owned_transparent(&mut sender, &stop)
        .await
        .unwrap();
    let next = load_pending(&owner).await;
    assert_ne!(next.operation_id(), operation);
    assert_ne!(next.bytes(), prepared.bytes());
    assert_eq!(sender.attempts, 2);
    owner.close().await.unwrap();
}
async fn transparent_addresses(owner: &mut Treasury, count: usize) -> Vec<String> {
    let mut addresses = Vec::new();
    for _ in 0..count {
        let (_, receiver) = owner
            .client
            .as_mut()
            .unwrap()
            .generate_transparent_address(zip32::AccountId::ZERO, false)
            .await
            .unwrap();
        addresses.push(
            zcash_keys::address::Address::Transparent(receiver).encode(&owner.network.chain()),
        );
    }
    let bytes = snapshot(owner.client.as_ref().unwrap()).await.unwrap();
    let revision = owner.revision;
    owner.revision = owner
        .store
        .call(move |s| s.save_snapshot(revision, &bytes))
        .await
        .unwrap();
    addresses
}
async fn load_pending(owner: &Treasury) -> PreparedTransaction {
    owner
        .store
        .call(|s| PreparedTransaction::load(s, &s.jobless_pending_operation()?.unwrap()))
        .await
        .unwrap()
}
fn selected_and_owned(tx: &PreparedTransaction, principal: u64) {
    let raw = submission::decode(tx.bytes()).unwrap();
    let transparent = raw.transparent_bundle().unwrap();
    assert!(transparent.vout.is_empty());
    assert_eq!(transparent.vin.len(), 1);
    assert_eq!(tx.facts().unwrap().amount_zatoshis, 0);
    assert!(tx.facts().unwrap().deadline.is_none());
    assert!(principal > tx.facts().unwrap().fee_zatoshis);
}

#[tokio::test]
async fn automatic_shielding_is_per_address_keeps_uncertain_bytes_and_recovers_unattributed_funds()
{
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let addresses = transparent_addresses(&mut owner, 4).await;
    let funding = faucet_funding_transaction(vec![
        (&addresses[0], 5_000, None),
        (&addresses[1], 10_000, None),
        (&addresses[2], 1_000_000, None),
        (&addresses[3], 2_000_000, None),
    ])
    .await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![funding]);
        chain.mine_empty_blocks(2);
    }
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    assert!(
        owner.status().await.unwrap().funding_jobs.is_empty(),
        "no refund/job attribution"
    );
    owner
        .maintain_owned_transparent(&mut NoSubmission, &stop)
        .await
        .unwrap();
    owner.configure_automatic_shielding(1); // Actual fee rejection cannot accept an input owner.
    owner
        .maintain_owned_transparent(&mut NoSubmission, &stop)
        .await
        .unwrap();
    assert!(owner.status().await.unwrap().treasury_operations.is_empty());
    // The SDK permits a proposal consuming exactly its standard fee. Reject it
    // before accepting inputs, so it cannot block useful recovery elsewhere.
    assert!(
        owner
            .shield_address(addresses[1].clone(), None, 30_000, &stop)
            .await
            .is_err()
    );
    assert!(owner.healthy);
    assert!(owner.status().await.unwrap().treasury_operations.is_empty());
    owner.configure_automatic_shielding(30_000);
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint.clone(),
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    net.chain.write().await.lose_next_send_response = Some(LostSendDestination::Mempool);
    owner
        .maintain_owned_transparent(&mut sender, &stop)
        .await
        .unwrap();
    let first = load_pending(&owner).await;
    selected_and_owned(&first, 1_000_000);
    let first_identity = first.network_identity().unwrap();
    let first_bytes = first.bytes().to_vec();
    let first_id = first.operation_id().to_owned();
    owner
        .store
        .call({
            let id = first_id.clone();
            move |s| {
                assert_eq!(s.operation(&id)?.attempts, 1);
                assert_eq!(s.operation(&id)?.submission, "UNKNOWN");
                assert_eq!(s.prepared_inputs(&id)?.len(), 1);
                assert!(s.accept_preparation("duplicate", None, 1, None).is_err());
                Ok(())
            }
        })
        .await
        .unwrap();
    let identity = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    owner.sync_once(&stop).await.unwrap();
    for enabled in [false, true] {
        if enabled {
            owner.configure_automatic_shielding(30_000);
        }
        owner
            .maintain_owned_transparent(&mut sender, &stop)
            .await
            .unwrap();
        let retained = load_pending(&owner).await;
        assert_eq!(retained.bytes(), first_bytes);
        assert_eq!(retained.network_identity().unwrap(), first_identity);
        assert_eq!(owner.status().await.unwrap().treasury_operations.len(), 1);
    }
    {
        let mut chain = net.chain.write().await;
        chain.mine_mempool();
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&stop).await.unwrap();
    owner
        .maintain_owned_transparent(&mut sender, &stop)
        .await
        .unwrap();
    assert!(!owner.status().await.unwrap().outgoing_pending);
    // A later maintenance pass handles the independent address, never linking it.
    owner
        .maintain_owned_transparent(&mut sender, &stop)
        .await
        .unwrap();
    let second = load_pending(&owner).await;
    selected_and_owned(&second, 2_000_000);
    assert_ne!(second.network_identity().unwrap(), first_identity);
    assert_ne!(second.bytes(), first_bytes);
    {
        let mut chain = net.chain.write().await;
        chain.mine_mempool();
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&stop).await.unwrap();
    owner
        .maintain_owned_transparent(&mut sender, &stop)
        .await
        .unwrap();
    let wallet = owner.client.as_ref().unwrap().wallet().read().await;
    let tx = submission::decode(second.bytes()).unwrap();
    let record = wallet.wallet_transactions.get(&tx.txid()).unwrap();
    let owned: u64 = record
        .orchard_notes()
        .iter()
        .map(|n| (n.key_id().account_id(), n.value()))
        .chain(
            record
                .ironwood_notes()
                .iter()
                .map(|n| (n.key_id().account_id(), n.value())),
        )
        .chain(
            record
                .sapling_notes()
                .iter()
                .map(|n| (n.key_id().account_id(), n.value())),
        )
        .filter(|(account, _)| *account == zip32::AccountId::ZERO)
        .map(|(_, value)| value)
        .sum();
    assert_eq!(owned, 2_000_000 - second.facts().unwrap().fee_zatoshis);
    drop(wallet);
    let status = owner.status().await.unwrap();
    assert!(
        status.sync.unwrap().spendable_shielded_zatoshis
            >= 4_000_000
                - first.facts().unwrap().fee_zatoshis
                - second.facts().unwrap().fee_zatoshis
    );
    assert_eq!(status.treasury_operations.len(), 2);
    owner.close().await.unwrap();
}

#[tokio::test]
async fn automatic_shielding_never_accepts_unconfirmed_or_paused_inputs_and_actor_starts_after_sync()
 {
    let (_dir, net, mut owner, _endpoint) = funded_fixture(2).await;
    let addresses = transparent_addresses(&mut owner, 1).await;
    owner.configure_automatic_shielding(30_000);
    let funding = faucet_funding_transaction(vec![(&addresses[0], 1_000_000, None)]).await;
    net.chain.write().await.submit_transaction(funding);
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    owner
        .maintain_owned_transparent(&mut NoSubmission, &stop)
        .await
        .unwrap();
    assert!(owner.status().await.unwrap().treasury_operations.is_empty());
    {
        let mut chain = net.chain.write().await;
        chain.mine_mempool();
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&stop).await.unwrap();
    owner
        .store
        .call(|s| {
            s.pause_new_funding();
            Ok(())
        })
        .await
        .unwrap();
    owner
        .maintain_owned_transparent(&mut NoSubmission, &stop)
        .await
        .unwrap();
    assert!(owner.status().await.unwrap().treasury_operations.is_empty());
    let store = owner.store_handle();
    store
        .call(|s| {
            s.deny_new_funding();
            Ok(())
        })
        .await
        .unwrap();
    owner
        .maintain_owned_transparent(&mut NoSubmission, &stop)
        .await
        .unwrap();
    drop(store);
    owner.close().await.unwrap();
    // A separate funded fixture below observes the production actor trigger.
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let addresses = transparent_addresses(&mut owner, 1).await;
    let funding = faucet_funding_transaction(vec![(&addresses[0], 1_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![funding]);
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&stop).await.unwrap();
    let original = owner
        .shield_address(addresses[0].clone(), None, 30_000, &stop)
        .await
        .unwrap();
    let original_bytes = original.bytes().to_vec();
    let identity = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    owner.configure_automatic_shielding(30_000);
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
    let store = owner.store_handle();
    let (handle, commands) = actor::channel();
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    // A completed command sync triggers maintenance without waiting for cadence.
    tokio::time::timeout(Duration::from_secs(30), handle.sync())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), entered.cancelled())
        .await
        .unwrap();
    let prepared = store
        .call(|s| PreparedTransaction::load(s, &s.jobless_pending_operation()?.unwrap()))
        .await
        .unwrap();
    assert_eq!(prepared.bytes(), original_bytes);
    let id = prepared.operation_id().to_owned();
    store
        .call(move |s| {
            assert_eq!(s.operation(&id)?.attempts, 1);
            Ok(())
        })
        .await
        .unwrap();
    // Shutdown during accepted submission retains exact bytes, no second attempt.
    stop.cancel();
    release.cancel();
    drop(handle);
    drop(store);
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
