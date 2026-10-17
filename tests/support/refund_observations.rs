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
                deadline: Some(1000),
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
    let pending = collect_refunds(&wallet, &accounting, 3, 2).unwrap();
    assert!(!pending.canonical.contains_key("op"));
    assert!(pending.issues.iter().any(|issue|matches!(issue,ReturnAccountingIssue::PendingObservedReturn { operation_id } if operation_id=="op")));
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
    assert_eq!(
        shielded.canonical.get("op"),
        Some(&alloy_primitives::U256::from(300))
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
    let disputed = collect_refunds(&wallet, &accounting, 21, 2).unwrap();
    assert!(disputed.credits.is_empty());
    assert!(disputed.issues.iter().any(|issue| matches!(issue, ReturnAccountingIssue::NoncanonicalCreditedOutput { operation_id } if operation_id == "op")));
    assert!(require_refund_shielding_evidence(&wallet, &accounting, 21, 2, "op").is_err());
    assert!(require_refund_shielding_evidence(&wallet, &accounting, 21, 2, "independent").is_ok());
    let removed = wallet.wallet_transactions.remove(&first).unwrap();
    let missing = collect_refunds(&wallet, &accounting, 21, 2).unwrap();
    assert!(missing.credits.is_empty());
    assert!(missing.issues.iter().any(|issue| matches!(
        issue,
        ReturnAccountingIssue::MissingCreditedTransaction { .. }
    )));
    wallet.wallet_transactions.insert(first, removed);
    wallet
        .wallet_transactions
        .get_mut(&first)
        .unwrap()
        .update_status(ConfirmationStatus::Confirmed(2.into()), 0, true);
    assert!(
        collect_refunds(&wallet, &accounting, 21, 2)
            .unwrap()
            .credits
            .is_empty(),
        "reappearance cannot credit again"
    );
    assert_eq!(store.return_accounting().unwrap().credited.len(), 2);
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
                        deadline: Some(1000),
                    }),
                )?;
                s.confirm_spend(&job.operation_id, 100, 1)?;
            }
            Ok(jobs[2].operation_id.clone())
        })
        .await
        .unwrap();
    let session = SyncSession {
        accounting_test_hook: None,
        source_pass: None,
        observation_start: None,
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
    let missing = session.reconcile_refunds(20, 2).await.unwrap();
    assert!(missing.issues.iter().any(|issue| matches!(issue, ReturnAccountingIssue::MissingCreditedTransaction { operation_id } if operation_id == &independent_operation)));
    assert_eq!(
        session
            .store
            .call(|s| Ok(s.status()?.refunds.len()))
            .await
            .unwrap(),
        1
    );
    let affected = first.issues[0].operation_id().to_owned();
    let before = session
        .store
        .call(move |s| {
            s.record_refund(RefundStatus {
                txid: "unidentifiable".into(),
                output_index: 0,
                operation_id: affected,
                amount: 1,
                height: 2,
            })?;
            Ok(serde_json::to_value(s.status()?.refunds)?)
        })
        .await
        .unwrap();
    let mut fresh = SyntheticWalletBuilder::new(
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    )
    .transparent_coin(1)
    .transparent_coin(70)
    .transparent_coin(55)
    .build();
    let new_txid = zcash_primitives::transaction::TxId::from_bytes([0xC2; 32]);
    let fresh_record = fresh.wallet_transactions.remove(&new_txid).unwrap();
    session
        .client
        .wallet()
        .write()
        .await
        .wallet_transactions
        .insert(new_txid, fresh_record);
    let incomplete = session.reconcile_refunds(20, 2).await.unwrap();
    assert_eq!(incomplete.attribution, ReturnAttribution::Unidentifiable);
    assert!(incomplete.observed_outputs > 0);
    let after = session
        .store
        .call(|s| Ok(serde_json::to_value(s.status()?.refunds)?))
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "incomplete attribution cannot publish new credits or rewrite old rows"
    );
}

#[test]
fn credited_refund_issues_preserve_dedup_and_scope_shielding_to_both_claimed_owners() {
    let wallet = SyntheticWalletBuilder::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").transparent_coin(100).build();
    let coin = wallet.wallet_outputs::<pepper_sync::wallet::TransparentCoin>()[0];
    let address = coin.address().to_owned();
    let mut accounting = crate::rotation::store::refunds::ReturnAccounting::default();
    accounting.bindings.insert(
        address.clone(),
        std::collections::BTreeSet::from(["old".into()]),
    );
    accounting.confirmed.extend(["old".into(), "new".into()]);
    accounting.credited = collect_refunds(&wallet, &accounting, 20, 2)
        .unwrap()
        .credits;
    assert_eq!(accounting.credited.len(), 1);
    let original = accounting.credited[0].clone();
    for (reason, amount, index) in [
        ("amount", 99, original.output_index),
        ("output", 100, original.output_index + 1),
    ] {
        accounting.credited[0].amount = amount;
        accounting.credited[0].output_index = index;
        let observed = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
        assert!(
            observed.credits.is_empty(),
            "disputed original must not be rediscovered"
        );
        assert!(observed.issues.iter().any(|issue| match reason {
            "amount" => matches!(issue, ReturnAccountingIssue::CreditedAmountConflict { .. }),
            _ => matches!(issue, ReturnAccountingIssue::MissingCreditedOutput { .. }),
        }));
        assert!(require_refund_shielding_evidence(&wallet, &accounting, 20, 2, "old").is_err());
        assert!(require_refund_shielding_evidence(&wallet, &accounting, 20, 2, "healthy").is_ok());
    }
    accounting.credited[0] = original.clone();
    accounting.credited[0].txid =
        zcash_primitives::transaction::TxId::from_bytes([0xEE; 32]).to_string();
    let missing = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert!(missing.credits.is_empty());
    assert!(missing.issues.iter().any(|issue| matches!(
        issue,
        ReturnAccountingIssue::MissingCreditedTransaction { .. }
    )));
    accounting.credited[0] = original.clone();
    accounting.bindings.insert(
        address.clone(),
        std::collections::BTreeSet::from(["new".into()]),
    );
    let conflict = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert!(conflict.credits.is_empty());
    for owner in ["old", "new"] {
        assert!(
            conflict
                .issues
                .contains(&ReturnAccountingIssue::CreditedOwnershipConflict {
                    operation_id: owner.into()
                })
        );
        assert!(require_refund_shielding_evidence(&wallet, &accounting, 20, 2, owner).is_err());
    }
    accounting
        .bindings
        .insert(address, std::collections::BTreeSet::from(["old".into()]));
    accounting.credited[0].txid = "malformed".into();
    let unknown = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert!(unknown.credits.is_empty());
    assert!(
        unknown
            .issues
            .contains(&ReturnAccountingIssue::InvalidCreditedIdentity {
                operation_id: "old".into()
            })
    );
    accounting.credited[0] = original;
    accounting.confirmed.remove("old");
    let unconfirmed = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert!(unconfirmed.credits.is_empty());
    assert!(
        unconfirmed
            .issues
            .contains(&ReturnAccountingIssue::UnconfirmedCreditedSource {
                operation_id: "old".into()
            })
    );
}

#[test]
fn conflicting_credit_suppresses_both_owners_but_independent_discoveries_continue() {
    let mut wallet = SyntheticWalletBuilder::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").transparent_coin(100).transparent_coin(200).build();
    let coins = wallet.wallet_outputs::<pepper_sync::wallet::TransparentCoin>();
    let address = coins[0].address().to_owned();
    let original = RefundStatus {
        txid: coins[0].output_id().txid().to_string(),
        output_index: coins[0].output_id().output_index(),
        operation_id: "A".into(),
        amount: i64::try_from(coins[0].value()).unwrap(),
        height: 2,
    };
    let mut other = SyntheticWalletBuilder::new(
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    )
    .transparent_coin(1)
    .transparent_coin(1)
    .transparent_coin(300)
    .build();
    let independent = zcash_primitives::transaction::TxId::from_bytes([0xC2; 32]);
    let record = other.wallet_transactions.remove(&independent).unwrap();
    let independent_address = record.transparent_coins()[0].address().to_owned();
    wallet.wallet_transactions.insert(independent, record);
    let mut accounting = crate::rotation::store::refunds::ReturnAccounting::default();
    accounting.credited.push(original);
    accounting
        .confirmed
        .extend(["A".into(), "B".into(), "C".into()]);
    accounting
        .bindings
        .insert(address, std::collections::BTreeSet::from(["B".into()]));
    accounting.bindings.insert(
        independent_address,
        std::collections::BTreeSet::from(["C".into()]),
    );
    let observed = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert_eq!(observed.observed_outputs, 3);
    assert_eq!(observed.credits.len(), 1);
    assert_eq!(observed.credits[0].operation_id, "C");
    assert_eq!(observed.credits[0].amount, 300);
    for owner in ["A", "B"] {
        assert!(require_refund_shielding_evidence(&wallet, &accounting, 20, 2, owner).is_err());
    }
    assert!(require_refund_shielding_evidence(&wallet, &accounting, 20, 2, "C").is_ok());
}

#[test]
fn unidentified_historical_credit_stops_attribution_and_shielding_without_failing_observation() {
    let wallet = SyntheticWalletBuilder::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").transparent_coin(100).build();
    let coin = wallet.wallet_outputs::<pepper_sync::wallet::TransparentCoin>()[0];
    let mut accounting = crate::rotation::store::refunds::ReturnAccounting::default();
    accounting.bindings.insert(
        coin.address().to_owned(),
        std::collections::BTreeSet::from(["healthy".into()]),
    );
    accounting.confirmed.insert("healthy".into());
    let original = RefundStatus {
        txid: "not-a-txid".into(),
        output_index: 0,
        operation_id: "missing-owner".into(),
        amount: 99,
        height: 1,
    };
    accounting.credited.push(original.clone());
    let observed = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert_eq!(observed.attribution, ReturnAttribution::Unidentifiable);
    assert_eq!(observed.observed_outputs, 1);
    assert!(observed.credits.is_empty());
    assert!(
        observed
            .issues
            .contains(&ReturnAccountingIssue::InvalidCreditedIdentity {
                operation_id: "missing-owner".into()
            })
    );
    for operation in ["healthy", "missing-owner"] {
        assert!(
            require_refund_shielding_evidence(&wallet, &accounting, 20, 2, operation)
                .unwrap_err()
                .to_string()
                .contains("unidentifiable_return_attribution")
        );
    }
    assert_eq!(
        serde_json::to_value(&accounting.credited[0]).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}

#[test]
fn failed_uncredited_output_does_not_permanently_veto_return_accounting() {
    let mut wallet=SyntheticWalletBuilder::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").transparent_coin(100).build();
    let coin = wallet.wallet_outputs::<pepper_sync::wallet::TransparentCoin>()[0];
    let txid = coin.output_id().txid();
    let mut accounting = crate::rotation::store::refunds::ReturnAccounting::default();
    accounting.confirmed.insert("original".into());
    accounting
        .bindings
        .entry(coin.address().to_owned())
        .or_default()
        .insert("original".into());
    wallet
        .wallet_transactions
        .get_mut(&txid)
        .unwrap()
        .update_status(ConfirmationStatus::Failed(20.into()), 0, true);
    let observation = collect_refunds(&wallet, &accounting, 20, 2).unwrap();
    assert!(observation.credits.is_empty());
    assert!(observation.issues.is_empty());
    assert_eq!(
        observation.canonical.get("original"),
        Some(&alloy_primitives::U256::ZERO)
    );
}
