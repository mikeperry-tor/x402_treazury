//! Real Ironwood proving against synthetic commitments, not consensus settlement.
//! All keys are public test material; the only endpoint is a localhost info stub.
use super::*;
use crate::{
    rotation::store::{SyncObservation, SyncPhase},
    treasury::SyncSettings,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zingolib::testutils::synthetic_wallet::SyntheticWalletBuilder;

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
// Post-NU6.3: all newly selected shielded funds and outputs use Ironwood.
const TIP: u32 = 3_500_000;

fn inject_ironwood_note(wallet: &mut zingolib::wallet::LightWallet) {
    use incrementalmerkletree::{Position, Retention};
    use pepper_sync::wallet::{IronwoodNote, OutputId, WalletTransaction};
    use shardtree::store::{Checkpoint, ShardStore};
    use zcash_protocol::{TxId, consensus::BlockHeight, memo::Memo};
    let viewing: orchard::keys::FullViewingKey = wallet
        .unified_key_store
        .get(&zip32::AccountId::ZERO)
        .unwrap()
        .try_into()
        .unwrap();
    let note = zingolib::mocks::orchard_note::OrchardCryptoNoteBuilder::default()
        .recipient(viewing.address_at(0u32, zip32::Scope::External))
        .value(orchard::value::NoteValue::from_raw(100_000))
        .build();
    wallet
        .shard_trees
        .ironwood
        .append(
            orchard::tree::MerkleHashOrchard::from_cmx(&note.commitment().into()),
            Retention::Marked,
        )
        .unwrap();
    let txid = TxId::from_bytes([0x41; 32]);
    let nullifier = note.nullifier(&viewing);
    let note = IronwoodNote::new_for_test(
        OutputId::new(txid, 0),
        zip32::AccountId::ZERO,
        zip32::Scope::External,
        note,
        Memo::Empty,
        Some(Position::from(0u64)),
    )
    .with_nullifier_for_test(nullifier);
    wallet.wallet_transactions.insert(
        txid,
        WalletTransaction::new_for_test_with_ironwood_notes(
            txid,
            zingo_status::confirmation_status::ConfirmationStatus::Confirmed(
                BlockHeight::from_u32(2),
            ),
            vec![note],
            vec![],
        ),
    );
    let store = wallet.shard_trees.ironwood.store_mut();
    store
        .remove_checkpoint(&BlockHeight::from_u32(TIP))
        .unwrap();
    store
        .add_checkpoint(
            BlockHeight::from_u32(TIP),
            Checkpoint::at_position(Position::from(0u64)),
        )
        .unwrap();
}

fn varint(mut n: u64) -> Vec<u8> {
    let mut out = vec![];
    while n >= 128 {
        out.push(n as u8 | 128);
        n >>= 7;
    }
    out.push(n as u8);
    out
}

#[tokio::test]
async fn funded_preparation_proves_and_restores_identical_pending_bytes() {
    qualify_preparation(PreparationCase::PublicationFailure).await;
    qualify_preparation(PreparationCase::Unfunded).await;
}

#[tokio::test]
async fn calculation_failure_clears_proposal_but_retains_accepted_inputs() {
    qualify_preparation(PreparationCase::CalculationFailure).await;
}

#[derive(Clone, Copy)]
enum PreparationCase {
    PublicationFailure,
    Unfunded,
    CalculationFailure,
}

async fn qualify_preparation(case: PreparationCase) {
    let funded = !matches!(case, PreparationCase::Unfunded);
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut treasury = Treasury::create(
        state.clone(),
        key.clone(),
        TIP,
        Some(zeroize::Zeroizing::new(SEED.into())),
    )
    .await
    .unwrap();
    let addresses = treasury.addresses().await.unwrap();
    let treasury_id = treasury.status().await.unwrap().treasury_id;
    {
        let client = treasury.client.as_ref().unwrap();
        let mut wallet = client.wallet().write().await;
        // Only take empty tree checkpoints and scanned ranges from the regtest
        // builder. Preserve the real mainnet wallet, mnemonic, addresses and keys.
        let empty = SyntheticWalletBuilder::new(SEED).tip(TIP).build();
        wallet.sync_state = empty.sync_state;
        wallet.shard_trees = empty.shard_trees;
        if funded {
            inject_ironwood_note(&mut wallet);
        }
        wallet.wallet_settings.min_confirmations = 1.try_into().unwrap();
        assert_eq!(
            wallet
                .shielded_spendable_balance(zip32::AccountId::ZERO, false)
                .unwrap()
                .into_u64(),
            if funded { 100_000 } else { 0 }
        );
    }
    treasury
        .checkpoint(SyncObservation {
            phase: SyncPhase::Ready,
            last_error: None,
            snapshot_revision: treasury.revision,
            started_snapshot_revision: None,
            checked_at: Some(now().unwrap()),
            checkpoint_at: 0,
            scanned_blocks: 1,
            target_height: Some(TIP.into()),
            observed_tip_height: Some(TIP.into()),
            height: Some(TIP.into()),
            confirmations: 1,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 100_000,
            spendable_shielded_zatoshis: 100_000,
        })
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let fail_calculation = matches!(case, PreparationCase::CalculationFailure)
        .then(|| treasury.client.as_ref().unwrap().wallet().clone());
    let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
        let count = count.clone();
        let fail_calculation = fail_calculation.clone();
        async move {
            assert!(
                uri.path().ends_with("GetLightdInfo"),
                "unexpected RPC: {uri}"
            );
            count.fetch_add(1, Ordering::SeqCst);
            if let Some(wallet) = fail_calculation {
                // Production selection has finished; fail calculation after
                // acceptance without adding a hook to the financial path.
                assert!(
                    wallet
                        .write()
                        .await
                        .unified_key_store
                        .remove(&zip32::AccountId::ZERO)
                        .is_some()
                );
            }
            let message = [
                vec![0x22, 4, b'm', b'a', b'i', b'n', 0x38],
                varint(TIP.into()),
            ]
            .concat();
            let body = [
                vec![0],
                (message.len() as u32).to_be_bytes().to_vec(),
                message,
            ]
            .concat();
            (
                [("content-type", "application/grpc"), ("grpc-status", "0")],
                body,
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    treasury.configure_sync(
        SyncSettings::new(format!("http://{}", listener.local_addr().unwrap()), 1).unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let pool = treasury
        .ensure_capacity_group(
            "proof".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    let job = treasury
        .allocate_capacity_kind(pool, crate::rotation::store::AllocationKind::Small, true)
        .await
        .unwrap()
        .unwrap();
    let job = treasury
        .store
        .call(move |s| Ok(s.funding_job(&job)?.unwrap()))
        .await
        .unwrap();
    let operation = job.operation_id.clone();
    let deadline = now().unwrap() + 600;
    let quote_job = job.clone();
    treasury.store.call(move |s| {
        let quote=crate::rotation::near::Quote {sizing_preview:None,
            request:serde_json::json!({"amount":quote_job.target,"recipient":quote_job.recipient,
                "refundTo":crate::rotation::near::test_shielded_refund(&s.network().chain())}),
            response:serde_json::json!({}),input:50_000,deadline,
            deposit:Some("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into())};
        s.save_funding_quote(&quote_job.id,&serde_json::to_vec(&quote)?)
    }).await.unwrap();
    let request = || PrepareRequest {
        managed_funding: Some(crate::rotation::transaction::ManagedFundingAuthority {
            job_id: job.id.clone(),
            auto_fund: true,
            max_attempts: 3,
        }),
        operation_id: operation.clone(),
        pool_id: Some(job.pool_id.clone()),
        deadline: Some(deadline),
        recipient: "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into(),
        amount_zatoshis: 50_000,
        max_fee_zatoshis: 120_000,
        max_input_zatoshis: 180_000,
    };
    if !funded {
        let error = treasury.prepare(request()).await.err().unwrap();
        assert!(
            error.is::<crate::rotation::transaction::PreparationDeferred>(),
            "{error:#}"
        );
        assert_eq!(
            error.downcast_ref::<crate::rotation::error::FundingCondition>(),
            Some(&crate::rotation::error::FundingCondition::InsufficientFunds),
            "{error:#}"
        );
        let address = error
            .downcast_ref::<crate::rotation::error::TreasuryDepositAddress>()
            .unwrap();
        assert_eq!(
            Some(address.0.as_str()),
            treasury.addresses().await.unwrap()[0]["encoded_address"].as_str()
        );
        assert!(treasury.healthy && treasury.running.is_none());
        assert!(!treasury.status().await.unwrap().outgoing_pending);
        treasury.close().await.unwrap();
        server.abort();
        return;
    }
    if matches!(case, PreparationCase::CalculationFailure) {
        let revision = treasury.revision;
        let error = treasury.prepare(request()).await.err().unwrap();
        assert_eq!(error.to_string(), "missing treasury account");
        assert!(!error.is::<crate::rotation::transaction::PreparationDeferred>());
        assert!(!treasury.healthy && treasury.running.is_none());
        assert_eq!(treasury.revision, revision);
        assert!(matches!(
            treasury
                .client
                .as_mut()
                .unwrap()
                .calculate_stored_proposal()
                .await,
            Err(zingolib::lightclient::error::LightClientError::SendError(
                zingolib::lightclient::error::SendError::NoStoredProposal
            ))
        ));
        let id = operation.clone();
        let selected = treasury
            .store
            .call(move |s| {
                assert!(s.require_unstarted_preparation(&id).is_err());
                assert!(s.prepared_bytes(&id).is_err());
                s.prepared_inputs(&id)
            })
            .await
            .unwrap();
        assert_eq!(selected.len(), 1);
        let status = treasury.status().await.unwrap();
        assert!(!status.outgoing_pending && status.treasury_operations.is_empty());
        assert!(
            treasury
                .prepare(request())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("requires reopen")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no replay or submission");
        treasury.close().await.unwrap();
        let restored = Treasury::open(state, key, treasury_id).await.unwrap();
        assert_eq!(restored.revision, revision);
        restored
            .store
            .call(move |s| {
                assert_eq!(s.prepared_inputs(&operation)?, selected);
                assert!(s.require_unstarted_preparation(&operation).is_err());
                assert!(s.abandon_unprepared(&operation).is_err());
                s.require_preparation_available()?;
                assert!(s.prepared_bytes(&operation).is_err());
                Ok(())
            })
            .await
            .unwrap();
        restored.close().await.unwrap();
        server.abort();
        return;
    }
    let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_prepare BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'fixture prepared commit failure'); END;").unwrap();
    let revision = treasury.revision;
    treasury.enable_publication_retry();
    let error = treasury.prepare(request()).await.err().unwrap();
    assert!(format!("{error:#}").contains("fixture prepared commit failure"));
    assert!(!treasury.healthy);
    assert_eq!(treasury.revision, revision);
    let status = treasury.status().await.unwrap();
    assert!(
        !status.outgoing_pending,
        "failed insert publishes no raw bytes"
    );
    assert!(
        treasury.running.is_some(),
        "the live owner retains its calculation"
    );
    assert!(status.treasury_operations.is_empty());
    let id = operation.clone();
    let accepted_inputs = treasury
        .store
        .call(move |s| {
            assert!(s.require_unstarted_preparation(&id).is_err());
            assert!(s.prepared_bytes(&id).is_err());
            s.prepared_inputs(&id)
        })
        .await
        .unwrap();
    assert_eq!(accepted_inputs.len(), 1);
    {
        let mut queued = Box::pin(treasury.prepare(request()));
        assert!(futures_util::poll!(queued.as_mut()).is_pending());
    }
    assert!(treasury.running.is_some());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "retry after failure must not reach the indexer"
    );
    db.execute_batch("DROP TRIGGER reject_prepare;").unwrap();
    drop(db);
    let prepared = treasury.publish_completed().await.unwrap().unwrap();
    assert!(treasury.healthy && treasury.running.is_none());
    assert_eq!(treasury.revision, revision + 1);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "local publication retry never recalculates or calls the indexer"
    );
    let facts = prepared.facts().unwrap().clone();
    assert_eq!(facts.amount_zatoshis, 50_000);
    assert!(facts.fee_zatoshis > 0 && facts.fee_zatoshis <= 20_000);
    assert!(facts.expiry_height > TIP && facts.expiry_height < TIP + 100);
    let tx = super::super::submission::decode(prepared.bytes()).unwrap();
    assert_eq!(tx.txid().to_string(), facts.txid);
    let id = operation.clone();
    let selected = treasury
        .store
        .call(move |s| s.prepared_inputs(&id))
        .await
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected, accepted_inputs,
        "publication retains original inputs"
    );
    assert!(
        tx.ironwood_bundle().unwrap().actions().len() > selected.len(),
        "dummy actions are excluded from selected input ownership"
    );
    tx.ironwood_bundle()
        .unwrap()
        .verify_proof(&orchard::circuit::VerifyingKey::build(
            orchard::circuit::OrchardCircuitVersion::PostNu6_3,
        ))
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "preparation must not broadcast"
    );
    assert!(treasury.healthy);
    let revision = treasury.revision;
    assert!(treasury.status().await.unwrap().outgoing_pending);
    treasury.close().await.unwrap();

    let restored = Treasury::open(state, key, treasury_id).await.unwrap();
    assert_eq!(restored.addresses().await.unwrap(), addresses);
    assert_eq!(restored.revision, revision);
    assert!(restored.status().await.unwrap().outgoing_pending);
    let id = operation.clone();
    let recovered = restored
        .store
        .call(move |s| PreparedTransaction::load(s, &id))
        .await
        .unwrap();
    assert_eq!(recovered.bytes(), prepared.bytes());
    assert_eq!(recovered.facts(), Some(&facts));
    let wallet = restored.client.as_ref().unwrap().wallet().read().await;
    let record = wallet.wallet_transactions.get(&tx.txid()).unwrap();
    let mut saved = vec![];
    record.transaction().write(&mut saved).unwrap();
    assert_eq!(saved, prepared.bytes());
    assert_eq!(
        wallet
            .shielded_spendable_balance(zip32::AccountId::ZERO, false)
            .unwrap()
            .into_u64(),
        0,
        "calculated inputs and pending change must not be spendable after reopen"
    );
    drop(wallet);
    restored
        .store
        .call(move |s| {
            assert_eq!(s.prepared_inputs(&operation)?, selected);
            assert!(s.require_unstarted_preparation(&operation).is_err());
            assert!(s.abandon_unprepared(&operation).is_err());
            assert!(s.require_preparation_available().is_err());
            Ok(())
        })
        .await
        .unwrap();
    restored.close().await.unwrap();
    server.abort();
}

async fn synthetic_deposit_client(
    tip: u32,
    nu7: u32,
    legacy: Option<crate::rotation::store::preparation::InputPool>,
) -> (tempfile::TempDir, zingolib::lightclient::LightClient) {
    let dir = tempfile::tempdir().unwrap();
    let heights = zingolib::testutils::mock_activation_heights_with(|era| era.set_nu7(Some(nu7)));
    let builder = SyntheticWalletBuilder::new(SEED)
        .tip(tip)
        .activation_heights(heights);
    let mut wallet = match legacy {
        Some(crate::rotation::store::preparation::InputPool::Sapling) => {
            builder.sapling_note(100_000)
        }
        Some(crate::rotation::store::preparation::InputPool::Orchard) => {
            builder.orchard_note(100_000)
        }
        None => builder.ironwood_note(100_000),
        _ => unreachable!("fixture only selects legacy shielded pool or Ironwood"),
    }
    .build();
    let chain = wallet.chain_type();
    wallet.wallet_settings.min_confirmations = 1.try_into().unwrap();
    wallet.mark_dirty();
    let bytes = wallet.save().unwrap().unwrap();
    let config = zingolib::config::ClientConfig::builder()
        .set_chain_type(chain)
        .set_wallet_dir(dir.path().to_path_buf())
        .set_wallet_config(zingolib::config::WalletConfig::Read)
        .build()
        .unwrap();
    let client = zingolib::lightclient::LightClient::from_reader(bytes.as_slice(), config)
        .await
        .unwrap();
    (dir, client)
}

async fn deposit_proposal(client: &mut zingolib::lightclient::LightClient) -> DepositProposal {
    let recipient = client
        .wallet()
        .read()
        .await
        .transparent_addresses()
        .values()
        .next()
        .unwrap()
        .clone();
    let payment = zcash_client_backend::zip321::Payment::new(
        recipient.parse().unwrap(),
        Some(Zatoshis::from_u64(50_000).unwrap()),
        None,
        None,
        None,
        vec![],
    )
    .unwrap();
    client
        .propose_send(
            zcash_client_backend::zip321::TransactionRequest::new(vec![payment]).unwrap(),
            zip32::AccountId::ZERO,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn backend_deposit_expiry_before_at_nu7_and_upgrade_boundary() {
    use zingolib::wallet::expiry::{tx_expiry_delta, tx_expiry_height};

    let chain = zingolib::config::ChainType::Regtest(
        zingolib::testutils::mock_activation_heights_with(|era| era.set_nu7(Some(100))),
    );
    // Additional rule positions need no wallet setup or transaction proof.
    for (target, delta) in [(81, 40), (99, 40), (100, 120), (101, 120)] {
        let target = zcash_protocol::consensus::BlockHeight::from_u32(target);
        assert_eq!(tx_expiry_delta(&chain, target), delta);
        assert_eq!(tx_expiry_height(&chain, target), target + delta);
    }
    // Real backend transactions target immediately before and at activation.
    // Explicit custom schedules exercise ordinary expiry without offline retargeting.
    for (tip, nu7) in [(98, 100), (99, 100)] {
        let (_dir, mut client) = synthetic_deposit_client(tip, nu7, None).await;
        let proposal = deposit_proposal(&mut client).await;
        let selected = super::super::inputs::selected_inputs(&proposal).unwrap();
        let target = u32::from(proposal.min_target_height());
        assert_eq!(target, tip + 1, "backend cases must straddle activation");
        let expected_delta = if target >= nu7 { 120 } else { 40 };
        let ids = {
            let mut wallet = client.wallet().write().await;
            calculate_deposit(&mut wallet, &proposal).unwrap()
        };
        client.clear_proposal().await;
        assert!(
            client.calculate_stored_proposal().await.is_err(),
            "public cleanup consumed stored proposal"
        );
        let wallet = client.wallet().read().await;
        let tx = wallet
            .wallet_transactions
            .get(&ids[0])
            .unwrap()
            .transaction();
        assert_eq!(u32::from(tx.expiry_height()), target + expected_delta);
        assert_eq!(
            selected,
            super::super::inputs::selected_inputs(&proposal).unwrap()
        );
        assert!(tx.transparent_bundle().unwrap().vin.is_empty());
        assert_eq!(tx.transparent_bundle().unwrap().vout.len(), 1);
    }
}

#[tokio::test]
async fn ironwood_source_selection_rejects_legacy_only_capital_and_preserves_public_failure_cleanup()
 {
    use crate::rotation::store::preparation::InputPool;
    for legacy in [InputPool::Sapling, InputPool::Orchard] {
        let (_dir, client) = synthetic_deposit_client(80, 100, Some(legacy)).await;
        // Existing SDK note decoding does not authorize new source inputs from
        // unsupported shielded pools, even when their value covers the payment.
        let payment = zcash_client_backend::zip321::Payment::new(
            "t2RnBRiqrN1nW4ecZs1Fj3WWjNdnSs4kiX8".parse().unwrap(),
            Some(Zatoshis::from_u64(50_000).unwrap()),
            None,
            None,
            None,
            vec![],
        )
        .unwrap();
        let result = source_proposal(
            &mut *client.wallet().write().await,
            zcash_client_backend::zip321::TransactionRequest::new(vec![payment]).unwrap(),
        );
        assert!(matches!(
            result,
            Err(zingolib::wallet::error::ProposeSendError::Proposal(
                zcash_client_backend::data_api::error::Error::InsufficientFunds { .. }
            ))
        ));
    }
    let (_dir, mut client) = synthetic_deposit_client(80, 100, None).await;
    let proposal = deposit_proposal(&mut client).await;
    let key = {
        let mut wallet = client.wallet().write().await;
        let key = wallet
            .unified_key_store
            .remove(&zip32::AccountId::ZERO)
            .unwrap();
        assert!(calculate_deposit(&mut wallet, &proposal).is_err());
        key
    };
    client.clear_proposal().await;
    client
        .wallet()
        .write()
        .await
        .unified_key_store
        .insert(zip32::AccountId::ZERO, key);
    assert!(client.calculate_stored_proposal().await.is_err());
    let proposal = deposit_proposal(&mut client).await;
    assert!(
        super::super::inputs::selected_inputs(&proposal)
            .unwrap()
            .iter()
            .all(|input| input.pool == InputPool::Ironwood)
    );
    client.clear_proposal().await;
}
