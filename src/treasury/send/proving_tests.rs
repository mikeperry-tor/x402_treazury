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
        inject_confirmed_orchard_notes(&mut wallet, 1, 100_000, TIP);
        wallet.wallet_settings.min_confirmations = 1.try_into().unwrap();
        assert_eq!(
            wallet
                .shielded_spendable_balance(zip32::AccountId::ZERO, false)
                .unwrap()
                .into_u64(),
            100_000
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
            height: Some(TIP.into()),
            confirmations: 1,
            max_age_seconds: 300,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 100_000,
            spendable_shielded_zatoshis: 100_000,
        })
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
        let count = count.clone();
        async move {
            assert!(
                uri.path().ends_with("GetLightdInfo"),
                "unexpected RPC: {uri}"
            );
            count.fetch_add(1, Ordering::SeqCst);
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
        SyncSettings::new(format!("http://{}", listener.local_addr().unwrap()), 1, 300).unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let operation = uuid::Uuid::new_v4().to_string();
    let prepared = treasury
        .prepare(PrepareRequest {
            operation_id: operation.clone(),
            pool_id: None,
            daily_limit_zatoshis: 100_000,
            deadline: now().unwrap() + 600,
            recipient: "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into(),
            amount_zatoshis: 50_000,
            max_fee_zatoshis: 20_000,
            max_input_zatoshis: 80_000,
        })
        .await
        .unwrap();
    let facts = prepared.facts().unwrap().clone();
    assert_eq!(facts.amount_zatoshis, 50_000);
    assert!(facts.fee_zatoshis > 0 && facts.fee_zatoshis <= 20_000);
    assert!(facts.expiry_height > TIP && facts.expiry_height < TIP + 100);
    let tx = super::super::submission::decode(prepared.bytes()).unwrap();
    assert_eq!(tx.txid().to_string(), facts.txid);
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
            assert!(s.require_spend_ready(now()?, 1).is_err());
            Ok(())
        })
        .await
        .unwrap();
    restored.close().await.unwrap();
    server.abort();
}
