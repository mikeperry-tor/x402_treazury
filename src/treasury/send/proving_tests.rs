//! Real Orchard proving against synthetic commitments, not consensus settlement.
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
use zingolib::testutils::synthetic_wallet::{
    SyntheticWalletBuilder, inject_confirmed_orchard_notes,
};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
// NU6.2: the injected V2 notes and the fixed Orchard circuit agree.
const TIP: u32 = 3_400_000;

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
    qualify_preparation(PreparationCase::Published).await;
    qualify_preparation(PreparationCase::Unfunded).await;
}

#[tokio::test]
async fn calculation_failure_clears_proposal_but_retains_accepted_inputs() {
    qualify_preparation(PreparationCase::CalculationFailure).await;
}

#[derive(Clone, Copy)]
enum PreparationCase {
    Published,
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
            inject_confirmed_orchard_notes(&mut wallet, 1, 100_000, TIP);
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
    let operation = uuid::Uuid::new_v4().to_string();
    let request = || PrepareRequest {
        managed_funding: None,

        operation_id: operation.clone(),
        pool_id: None,

        deadline: now().unwrap() + 600,
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
        assert!(treasury.healthy && treasury.completed.is_none());
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
        assert!(!treasury.healthy && treasury.completed.is_none());
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
        assert!(status.outgoing_pending && status.treasury_operations.is_empty());
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
                assert!(s.require_preparation_available().is_err());
                assert!(s.prepared_bytes(&operation).is_err());
                Ok(())
            })
            .await
            .unwrap();
        restored.close().await.unwrap();
        server.abort();
        return;
    }
    if matches!(case, PreparationCase::PublicationFailure) {
        let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_prepare BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'fixture prepared commit failure'); END;").unwrap();
        let revision = treasury.revision;
        let error = treasury.prepare(request()).await.err().unwrap();
        assert!(format!("{error:#}").contains("fixture prepared commit failure"));
        assert!(!treasury.healthy);
        assert_eq!(treasury.revision, revision);
        let status = treasury.status().await.unwrap();
        assert!(
            status.outgoing_pending,
            "accepted calculation survives failed byte publication"
        );
        assert!(status.treasury_operations.is_empty());
        assert!(
            treasury
                .prepare(request())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("requires reopen")
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "retry after failure must not reach the indexer"
        );
        db.execute_batch("DROP TRIGGER reject_prepare;").unwrap();
        drop(db);
        let saved = treasury.publish_completed().await.unwrap();
        let exact = saved.bytes().to_vec();
        assert!(treasury.healthy && treasury.completed.is_none());
        assert_eq!(treasury.revision, revision + 1);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "local publication retry never recalculates or calls the indexer"
        );
        treasury.close().await.unwrap();
        let restored = Treasury::open(state, key, treasury_id).await.unwrap();
        assert_eq!(restored.revision, revision + 1);
        let id = operation.clone();
        assert_eq!(
            restored
                .store
                .call(move |s| s.prepared_bytes(&id))
                .await
                .unwrap()
                .as_slice(),
            exact
        );
        assert!(restored.status().await.unwrap().outgoing_pending);
        restored.close().await.unwrap();
        server.abort();
        return;
    }
    let prepared = treasury.prepare(request()).await.unwrap();
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
    assert!(
        tx.orchard_bundle().unwrap().actions().len() > selected.len(),
        "dummy actions are excluded from selected input ownership"
    );
    tx.orchard_bundle()
        .unwrap()
        .verify_proof(&orchard::circuit::VerifyingKey::build(
            orchard::circuit::OrchardCircuitVersion::FixedPostNu6_2,
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
    sapling: bool,
) -> (tempfile::TempDir, zingolib::lightclient::LightClient) {
    let dir = tempfile::tempdir().unwrap();
    let heights = zingolib::testutils::mock_activation_heights_with(|era| era.set_nu7(Some(nu7)));
    let builder = SyntheticWalletBuilder::new(SEED)
        .tip(tip)
        .activation_heights(heights);
    let mut wallet = if sapling {
        builder.sapling_note(100_000)
    } else {
        builder.orchard_note(100_000)
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
    // Explicit custom schedules exercise ordinary expiry without offline retargeting.
    for (tip, nu7) in [(98, 100), (99, 100), (100, 100), (80, 100)] {
        let (_dir, mut client) = synthetic_deposit_client(tip, nu7, false).await;
        let proposal = deposit_proposal(&mut client).await;
        let selected = super::super::inputs::selected_inputs(&proposal).unwrap();
        let target = u32::from(proposal.min_target_height());
        let expected_delta = if target >= nu7 { 120 } else { 40 };
        let before = prover::parse_count();
        let ids = {
            let mut wallet = client.wallet().write().await;
            calculate_deposit(&mut wallet, &proposal).unwrap()
        };
        assert_eq!(
            prover::parse_count(),
            before,
            "Orchard-only work must not parse Sapling parameters"
        );
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
async fn backend_deposit_real_sapling_proving_and_public_failure_cleanup() {
    let (_dir, mut client) = synthetic_deposit_client(80, 100, true).await;
    let proposal = deposit_proposal(&mut client).await;
    let before = prover::parse_count();
    let ids = {
        let mut wallet = client.wallet().write().await;
        calculate_deposit(&mut wallet, &proposal).unwrap()
    };
    assert_eq!(prover::parse_count(), before + 1);
    client.clear_proposal().await;
    {
        let wallet = client.wallet().read().await;
        let transaction = wallet
            .wallet_transactions
            .get(&ids[0])
            .unwrap()
            .transaction();
        assert!(
            !transaction
                .sapling_bundle()
                .unwrap()
                .shielded_spends()
                .is_empty()
        );
        assert_eq!(
            u32::from(transaction.expiry_height()),
            u32::from(proposal.min_target_height()) + 40
        );
    }
    let (_dir, mut client) = synthetic_deposit_client(80, 100, false).await;
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
    // Declining the failed proposal leaves SDK selection usable.
    let _ = deposit_proposal(&mut client).await;
    client.clear_proposal().await;
}
