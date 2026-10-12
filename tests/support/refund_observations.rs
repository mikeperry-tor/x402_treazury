use super::*;
use crate::rotation::{store::Store, transaction::TransactionFacts};
use crate::test_funding_quote as quote_fixture;
use pepper_sync::wallet::WalletTransaction;
use zingo_status::confirmation_status::ConfirmationStatus;
use zingolib::testutils::synthetic_wallet::SyntheticWalletBuilder;
#[test]
fn refund_observations_require_confirmed_origins_even_after_shielding() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    store.reserve("op", None, 1, 200).unwrap();
    store
        .prepare_with_facts(
            "op",
            1,
            b"next",
            b"signed",
            Some(TransactionFacts {
                txid: "source".into(),
                expiry_height: 20,
                amount_zatoshis: 150,
                fee_zatoshis: 20,
                deadline: 1000,
            }),
        )
        .unwrap();
    let mut wallet=SyntheticWalletBuilder::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").transparent_coin(100).transparent_coin(200).build();
    let coin = wallet.wallet_outputs::<pepper_sync::wallet::TransparentCoin>()[0];
    let address = coin.address().to_owned();
    let mut accounting = store.return_accounting().unwrap();
    accounting
        .bindings
        .entry(address.clone())
        .or_default()
        .insert("op".into());
    let first = coin.output_id().txid();
    assert!(
        collect_refunds(&wallet, &accounting, 20, 2)
            .unwrap()
            .credits
            .is_empty()
    );
    store.confirm_spend("op", 170, 1).unwrap();
    accounting.confirmed.insert("op".into());
    assert!(
        collect_refunds(&wallet, &store.return_accounting().unwrap(), 20, 2)
            .unwrap()
            .credits
            .is_empty()
    );
    assert!(
        collect_refunds(&wallet, &accounting, 1, 2)
            .unwrap()
            .credits
            .is_empty()
    );
    assert_eq!(
        collect_refunds(&wallet, &accounting, 3, 2)
            .unwrap()
            .credits
            .len(),
        1
    );
    let refunds = collect_refunds(&wallet, &accounting, 4, 2).unwrap().credits;
    assert_eq!(refunds.len(), 2);
    store.record_refund(refunds[0].clone()).unwrap();
    accounting.credited = store.return_accounting().unwrap().credited;
    let mixed = collect_refunds(&wallet, &accounting, 4, 2).unwrap();
    assert_eq!(mixed.observed_outputs, 2);
    assert_eq!(mixed.credits.len(), 1);
    assert_eq!(
        (&mixed.credits[0].txid, mixed.credits[0].output_index),
        (&refunds[1].txid, refunds[1].output_index)
    );
    for refund in &refunds {
        store.record_refund(refund.clone()).unwrap();
        store.record_refund(refund.clone()).unwrap();
    }
    accounting.credited = store.return_accounting().unwrap().credited;
    let spending = zcash_primitives::transaction::TxId::from_bytes([99; 32]);
    for tx in wallet.wallet_transactions.values_mut() {
        for coin in tx.transparent_coins_mut() {
            coin.set_spending_transaction(Some(spending));
        }
    }
    wallet.wallet_transactions.insert(
        spending,
        WalletTransaction::new_for_test(spending, ConfirmationStatus::Confirmed(10.into())),
    );
    let shielded = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert_eq!(
        shielded.observed_outputs, 2,
        "shielding cannot erase origin evidence"
    );
    assert!(
        shielded.credits.is_empty(),
        "known outputs need no duplicate publication"
    );
    accounting
        .bindings
        .get_mut(&address)
        .unwrap()
        .insert("ambiguous-original".into());
    assert!(
        collect_refunds(&wallet, &accounting, 20, 2)
            .unwrap()
            .credits
            .is_empty(),
        "ambiguous attribution preserves known credits without blocking canonical readiness"
    );
    accounting
        .bindings
        .get_mut(&address)
        .unwrap()
        .remove("ambiguous-original");
    let original_bindings = std::mem::take(&mut accounting.bindings);
    let missing = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert!(missing.credits.is_empty());
    assert_eq!(
        missing.issues,
        vec![ReturnAccountingIssue::MissingOriginalBinding {
            operation_id: "op".into()
        }]
    );
    accounting.bindings = original_bindings;
    wallet
        .wallet_transactions
        .get_mut(&first)
        .unwrap()
        .update_status(ConfirmationStatus::Failed(21.into()), 0, true);
    assert!(
        collect_refunds(&wallet, &accounting, 21, 2)
            .unwrap_err()
            .to_string()
            .contains("treasury_refund_reorg")
    );
    wallet.wallet_transactions.remove(&first);
    assert!(collect_refunds(&wallet, &accounting, 21, 2).is_err());
}

#[tokio::test]
async fn treasury_sync_return_stage_isolates_ambiguous_history_and_revalidates_credits() {
    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let dir = tempfile::tempdir().unwrap();
    let mut treasury = Treasury::create(
        dir.path().join("state"),
        dir.path().join("key"),
        2_000_000,
        Some(Zeroizing::new(SEED.into())),
    )
    .await
    .unwrap();
    crate::rotation::test_capacity::treasury_pair(
        &treasury,
        "ambiguous",
        "5",
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0,
    )
    .await
    .unwrap();
    crate::rotation::test_capacity::treasury_pair(
        &treasury,
        "independent",
        "5",
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0,
    )
    .await
    .unwrap();
    let mut wallet = SyntheticWalletBuilder::new(SEED)
        .transparent_coin(50)
        .build();
    let ambiguous_address = wallet.wallet_outputs::<pepper_sync::wallet::TransparentCoin>()[0]
        .address()
        .to_owned();
    let mut other = SyntheticWalletBuilder::new(
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    )
    .transparent_coin(1)
    .transparent_coin(70)
    .build();
    let independent_txid = zcash_primitives::transaction::TxId::from_bytes([0xC1; 32]);
    let record = other.wallet_transactions.remove(&independent_txid).unwrap();
    let independent_address = record.transparent_coins()[0].address().to_owned();
    wallet.wallet_transactions.insert(independent_txid, record);
    // Large unrelated resident history still has one discovery pass and cannot
    // renew, replace or contaminate the accounting evidence of either group.
    for n in 0..4096_u32 {
        let mut bytes = [0_u8; 32];
        bytes[..4].copy_from_slice(&n.to_le_bytes());
        let txid = zcash_primitives::transaction::TxId::from_bytes(bytes);
        wallet.wallet_transactions.insert(
            txid,
            WalletTransaction::new_for_test(txid, ConfirmationStatus::Confirmed(2.into())),
        );
    }
    let independent_operation = treasury
        .store
        .call(move |s| {
            let jobs = s.funding_jobs()?;
            for (job, address) in jobs.iter().take(3).zip([
                ambiguous_address.clone(),
                ambiguous_address,
                independent_address,
            ]) {
                let revision = s.status()?.snapshot_revision;
                s.save_refund_address(&job.id, &address, revision, b"return-fixture")?;
                s.save_funding_quote(
                    &job.id,
                    &quote_fixture::bound(&job.target, &job.recipient, "fixture-quote"),
                )?;
                s.advance_funding(
                    &job.id,
                    crate::rotation::store::funding::FundingPhase::Quoted,
                    crate::rotation::store::funding::FundingPhase::Preparing,
                )?;
                s.reserve(&job.operation_id, None, 1, 100)?;
                let revision = s.status()?.snapshot_revision;
                s.prepare_with_facts(
                    &job.operation_id,
                    revision,
                    b"source-fixture",
                    b"signed",
                    Some(TransactionFacts {
                        txid: job.operation_id.clone(),
                        expiry_height: 100,
                        amount_zatoshis: 80,
                        fee_zatoshis: 20,
                        deadline: 1000,
                    }),
                )?;
                s.confirm_spend(&job.operation_id, 100, 1)?;
            }
            Ok(jobs[2].operation_id.clone())
        })
        .await
        .unwrap();
    let session = SyncSession {
        identity: crate::network::IsolationId::treasury(
            &treasury.status().await.unwrap().treasury_id,
        ),
        network: treasury.network,
        client: treasury.client.take().unwrap(),
        store: treasury.store.clone(),
        revision: treasury.revision,
    };
    *session.client.wallet().write().await = wallet;
    // Exercise the actual asynchronous treasury-sync accounting stage and owned
    // store publication, not just the collector or a mock backend.
    let first = session.reconcile_refunds(20, 2).await.unwrap();
    assert_eq!(first.observed_outputs, 1);
    assert_eq!(first.issues.len(), 2);
    assert!(first.issues.iter().all(|issue| matches!(issue,
        ReturnAccountingIssue::AmbiguousOriginalBinding { operation_id } if operation_id != &independent_operation)));
    let second = session.reconcile_refunds(20, 2).await.unwrap();
    assert_eq!(first.issues, second.issues);
    assert_eq!(second.observed_outputs, first.observed_outputs);
    let status = session.store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.refunds.len(), 1);
    assert_eq!(status.refunds[0].operation_id, independent_operation);
    assert_eq!(status.refunds[0].amount, 70);
    assert_eq!(status.treasury_operations.len(), 3);
    assert!(
        status
            .treasury_operations
            .iter()
            .all(|op| op.submission == "CONFIRMED")
    );
    session
        .client
        .wallet()
        .write()
        .await
        .wallet_transactions
        .remove(&independent_txid);
    assert!(
        session
            .reconcile_refunds(20, 2)
            .await
            .unwrap_err()
            .to_string()
            .contains("treasury_refund_reorg")
    );
    assert_eq!(
        session
            .store
            .call(|s| Ok(s.status()?.refunds.len()))
            .await
            .unwrap(),
        1
    );
}
