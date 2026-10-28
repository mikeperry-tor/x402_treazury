//! Real Sapling proofs and canonical SDK scans on a fabricated localhost chain.
//! The mock does not provide consensus or live settlement qualification.
use super::*;
use pepper_sync::wallet::{NoteInterface, OutputInterface, SaplingNote};

struct NoDisclosure;
impl TransactionSubmission for NoDisclosure {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("relocated-note recovery cannot submit original work");
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("canonical relocated-note recovery needs no inclusion hint");
    }
}
async fn sapling_address(owner: &mut Treasury) -> String {
    let client = owner.client.as_mut().unwrap();
    client
        .generate_unified_address(ReceiverSelection::sapling_only(), zip32::AccountId::ZERO)
        .await
        .unwrap();
    client
        .unified_addresses_json()
        .await
        .members()
        .find(|address| address["has_sapling"] == true && address["has_orchard"] == false)
        .unwrap()["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn prepare(owner: &mut Treasury, name: &str) -> (FundingJob, PreparedTransaction) {
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
    let job = owner
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let deadline = now().unwrap() + 7200;
    save_quotes(&owner.store, vec![job.clone()], deadline).await;
    let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
    (job, prepared)
}
async fn finish_scan(owner: &mut Treasury, stop: &CancellationToken) {
    // The SDK may checkpoint its conservative rollback before returning its
    // recoverable discontinuity error. The existing next scan resumes it.
    if owner.sync_once(stop).await.is_err() {
        owner.sync_once(stop).await.unwrap();
    }
}

#[tokio::test]
async fn relocated_sapling_note_closes_expired_original_when_unspent_or_spent_canonically() {
    for competing_spend in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let net = MockNet::launch().await;
        let endpoint = net.indexer_uri().to_string();
        let mut owner = Treasury::create_with_network(
            dir.path().join("state"),
            dir.path().join("key"),
            1,
            Some(Zeroizing::new(SEED.into())),
            TreasuryNetwork::Synthetic,
        )
        .await
        .unwrap();
        let address = sapling_address(&mut owner).await;
        let funding = faucet_funding_transaction(vec![(&address, 1_000_000, None)]).await;
        let funding_id = submission::decode(&funding).unwrap().txid();
        {
            let mut chain = net.chain.write().await;
            chain.mine_empty_blocks(20);
            chain.mine_block(vec![funding.clone()]);
            chain.mine_empty_blocks(2);
        }
        let stop = CancellationToken::new();
        owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
        owner.sync_once(&stop).await.unwrap();
        let (job, prepared) = prepare(&mut owner, "original").await;
        let (old_id, old_value, old_position, old_nullifier) = {
            let wallet = owner.client.as_ref().unwrap().wallet().read().await;
            let note = wallet
                .wallet_outputs::<SaplingNote>()
                .into_iter()
                .find(|note| note.output_id().txid() == funding_id)
                .unwrap();
            (
                note.output_id(),
                note.value(),
                note.position(),
                note.nullifier(),
            )
        };
        assert!(old_position.is_some() && old_nullifier.is_some());
        let operation = job.operation_id.clone();
        let original_inputs = owner
            .store
            .call(move |s| s.prepared_inputs(&operation))
            .await
            .unwrap();
        assert!(original_inputs.iter().any(|input| input.pool
            == crate::rotation::store::preparation::InputPool::Sapling
            && input.txid == funding_id.to_string()
            && input.output_index == old_id.output_index()));
        let padding = faucet_funding_transaction(vec![(&address, 1, None)]).await;
        {
            let mut chain = net.chain.write().await;
            chain.reorg_to(20);
            // The exact same received transaction now follows new commitments.
            chain.mine_block(vec![padding]);
            chain.mine_block(vec![funding]);
            chain.mine_empty_blocks(1);
        }
        finish_scan(&mut owner, &stop).await;
        let current_nullifier = {
            let wallet = owner.client.as_ref().unwrap().wallet().read().await;
            let note = wallet
                .wallet_outputs::<SaplingNote>()
                .into_iter()
                .find(|note| note.output_id() == old_id)
                .unwrap();
            assert_eq!(note.value(), old_value);
            assert_ne!(
                note.position(),
                old_position,
                "actual canonical commitment position must change"
            );
            assert_ne!(
                note.nullifier(),
                old_nullifier,
                "Sapling nullifier derives from the canonical note position"
            );
            assert!(wallet.output_spend_status(note).is_unspent());
            note.nullifier().unwrap()
        };
        let premature = owner
            .recover_expired(job.operation_id.clone(), &mut NoDisclosure, &stop)
            .await
            .unwrap_err();
        assert_eq!(
            premature.downcast_ref::<crate::rotation::transaction::RecoveryEvidenceError>(),
            Some(&crate::rotation::transaction::RecoveryEvidenceError::ExpiryNotBuried)
        );
        pending(&owner.store, &job, prepared.bytes()).await;
        let operation = job.operation_id.clone();
        let expected_inputs = original_inputs.clone();
        let original_job = job.id.clone();
        owner
            .store
            .call(move |s| {
                assert_eq!(s.operation(&operation)?.attempts, 0);
                assert_eq!(s.prepared_inputs(&operation)?, expected_inputs);
                assert_eq!(s.funding_recovery_count(&original_job)?, 0);
                Ok(())
            })
            .await
            .unwrap();
        let mut other = None;
        let mut competing_id = None;
        if competing_spend {
            // A second authorized test owner scans the relocated note and signs
            // its current nullifier. The original owner never recalculates.
            let other_dir = tempfile::tempdir().unwrap();
            let mut spender = Treasury::create_with_network(
                other_dir.path().join("state"),
                other_dir.path().join("key"),
                1,
                Some(Zeroizing::new(SEED.into())),
                TreasuryNetwork::Synthetic,
            )
            .await
            .unwrap();
            assert_eq!(sapling_address(&mut spender).await, address);
            spender.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
            spender.sync_once(&stop).await.unwrap();
            let (other_job, competing) = prepare(&mut spender, "competing").await;
            let signed = submission::decode(competing.bytes()).unwrap();
            assert!(
                signed
                    .sapling_bundle()
                    .unwrap()
                    .shielded_spends()
                    .iter()
                    .any(|spend| spend.nullifier() == &current_nullifier)
            );
            assert_ne!(
                signed.txid(),
                submission::decode(prepared.bytes()).unwrap().txid()
            );
            competing_id = Some(signed.txid());
            let mut sender = submission::GrpcSubmission::with_network(
                endpoint.clone(),
                endpoint.clone(),
                TreasuryNetwork::Synthetic,
            )
            .unwrap();
            spender
                .submit_prepared(other_job.operation_id, &mut sender, false, &stop)
                .await
                .unwrap();
            net.chain.write().await.mine_mempool();
            other = Some((other_dir, spender));
        }
        {
            let mut chain = net.chain.write().await;
            let remaining = prepared.facts().unwrap().expiry_height + 2 - chain.tip();
            chain.mine_empty_blocks(remaining);
        }
        owner
            .recover_expired(job.operation_id.clone(), &mut NoDisclosure, &stop)
            .await
            .unwrap();
        {
            let wallet = owner.client.as_ref().unwrap().wallet().read().await;
            let note = wallet
                .wallet_outputs::<SaplingNote>()
                .into_iter()
                .find(|note| note.output_id() == old_id)
                .unwrap();
            assert_eq!(note.nullifier(), Some(current_nullifier));
            if let Some(spender) = competing_id {
                assert_eq!(note.spending_transaction(), Some(spender));
                assert!(!wallet.output_spend_status(note).is_unspent());
                assert!(
                    wallet
                        .wallet_transactions
                        .get(&spender)
                        .unwrap()
                        .status()
                        .get_confirmed_height()
                        .is_some()
                );
            } else {
                assert!(wallet.output_spend_status(note).is_unspent());
            }
        }
        let operation = job.operation_id;
        let raw = prepared.bytes().to_vec();
        owner
            .store
            .call(move |s| {
                assert_eq!(s.operation(&operation)?.submission, "EXPIRED");
                assert_eq!(s.operation(&operation)?.attempts, 0);
                assert_eq!(s.prepared_bytes(&operation)?.as_slice(), raw);
                assert_eq!(s.prepared_inputs(&operation)?, original_inputs);
                assert!(!s.operation_pending(&operation)?);
                assert_eq!(s.funding_recovery_count(&job.id)?, 1);
                assert_eq!(s.funding_jobs()?.len(), 1);
                Ok(())
            })
            .await
            .unwrap();
        if let Some((_dir, spender)) = other {
            spender.close().await.unwrap();
        }
        owner.close().await.unwrap();
    }
}
