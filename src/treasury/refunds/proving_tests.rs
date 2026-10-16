//! Calculate-only real Orchard proving with unfunded public synthetic inputs.
use super::*;
use crate::rotation::store::{AllocationKind, SyncObservation, SyncObservationStart, SyncPhase};
use pepper_sync::wallet::{OutputId, TransparentCoin, WalletTransaction};
use zingo_status::confirmation_status::ConfirmationStatus;
use zingolib::testutils::synthetic_wallet::SyntheticWalletBuilder;

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const TIP: u32 = 3_400_000;

#[tokio::test]
async fn shielding_calculates_fee_only_and_restores_exact_owned_bytes() {
    for fail_commit in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut treasury = Treasury::create(
            state.clone(),
            key.clone(),
            TIP,
            Some(Zeroizing::new(SEED.into())),
        )
        .await
        .unwrap();
        let job = treasury
            .store
            .call(|s| {
                let pool = s.ensure_capacity_group(
                    "shield",
                    &crate::rotation::store::test_support::reserves("1"),
                    None,
                    0,
                )?;
                s.allocate_capacity_kind(&pool, AllocationKind::Small, true)?;
                Ok(s.funding_jobs()?.remove(0))
            })
            .await
            .unwrap();
        let address = treasury.refund_address(job.id.clone()).await.unwrap();
        let selected_input;
        {
            let mut wallet = treasury.client.as_ref().unwrap().wallet().write().await;
            let empty = SyntheticWalletBuilder::new(SEED).tip(TIP).build();
            wallet.sync_state = empty.sync_state;
            wallet.shard_trees = empty.shard_trees;
            wallet.wallet_settings.min_confirmations = 1.try_into().unwrap();
            let key_id = *wallet
                .transparent_addresses()
                .iter()
                .find(|(_, a)| **a == address)
                .unwrap()
                .0;
            let zcash_keys::address::Address::Transparent(receiver) =
                zcash_keys::address::Address::decode(&treasury.network.chain(), &address).unwrap()
            else {
                panic!("transparent address required")
            };
            let txid = zcash_primitives::transaction::TxId::from_bytes([0xC0; 32]);
            let coin = TransparentCoin::new_for_test(
                OutputId::new(txid, 0),
                key_id,
                address.clone(),
                receiver.script().into(),
                Zatoshis::const_from_u64(100_000),
            );
            selected_input = crate::rotation::store::shielding::Input {
                txid: txid.to_string(),
                output_index: 0,
                amount: 100_000,
                script: coin.script().0.0.clone(),
            };
            wallet.wallet_transactions.insert(
                txid,
                WalletTransaction::new_for_test(txid, ConfirmationStatus::Confirmed(2.into()))
                    .with_transparent_coins_for_test(vec![coin]),
            );
        }
        let start = SyncObservationStart::capture(300).unwrap();
        assert!(
            treasury
                .checkpoint_observed(
                    SyncObservation {
                        phase: SyncPhase::Ready,
                        last_error: None,
                        snapshot_revision: treasury.revision,
                        checked_at: Some(start.checked_at()),
                        checkpoint_at: 0,
                        scanned_blocks: 1,
                        target_height: Some(TIP.into()),
                        observed_tip_height: Some(TIP.into()),
                        height: Some(TIP.into()),
                        confirmations: 1,
                        max_age_seconds: 300,
                        confirmed_pool_balances_zatoshis: None,
                        confirmed_shielded_zatoshis: 0,
                        spendable_shielded_zatoshis: 0,
                    },
                    start
                )
                .await
                .unwrap()
        );
        let revision = treasury.revision;
        let selected = address.clone();
        let recipient = job.recipient.clone();
        let (owner, policy) = treasury
            .store
            .call(move |s| s.begin_manual_shielding("shield", revision, &job.id, &selected, 50_000))
            .await
            .unwrap();
        let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
        if fail_commit {
            db.execute_batch("CREATE TRIGGER reject_shield BEFORE INSERT ON shielding_inputs BEGIN SELECT RAISE(ABORT,'fixture shielding commit failure'); END;").unwrap();
        }
        let result = treasury
            .finish_owned_refund(address, recipient, 50_000, owner.clone(), policy.clone())
            .await;
        if fail_commit {
            let error = result.err().unwrap();
            assert!(
                format!("{error:#}").contains("fixture shielding commit failure"),
                "{error:#}"
            );
            assert!(treasury.client.is_none());
            assert!(!treasury.healthy);
            assert_eq!(treasury.revision, revision);
            for table in [
                "outgoing",
                "budget_entries",
                "shielding_operations",
                "shielding_inputs",
            ] {
                assert_eq!(
                    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
            }
            assert!(
                db.query_row(
                    "SELECT closed FROM treasury_preparations WHERE operation_id='shield'",
                    [],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
            );
        } else {
            let prepared = result.unwrap();
            assert_eq!(prepared.facts().unwrap().amount_zatoshis, 0);
            assert_eq!(prepared.facts().unwrap().deadline, None);
            prepared.check_submission_window(u64::MAX, true).unwrap();
            let fee = prepared.facts().unwrap().fee_zatoshis;
            assert!(fee > 0 && fee < 100_000);
            let binding =
                crate::rotation::store::shielding::Binding::new(policy, vec![selected_input], fee)
                    .unwrap();
            let repeated_facts = prepared.facts().unwrap().clone();
            let repeated_raw = prepared.bytes().to_vec();
            treasury
                .store
                .call(move |s| {
                    let (revision, snapshot) = s.snapshot()?;
                    s.pause_new_funding();
                    // A lost acknowledgement is a read of the exact accepted commit,
                    // not new preparation authority after policy is disabled.
                    assert_eq!(
                        s.prepare_owned_shielding(
                            &owner,
                            &snapshot,
                            &repeated_raw,
                            repeated_facts,
                            &binding
                        )?,
                        revision
                    );
                    assert_eq!(s.snapshot()?.0, revision);
                    Ok(())
                })
                .await
                .unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT reserved FROM budget_entries WHERE id='shield'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                i64::try_from(fee).unwrap()
            );
            assert_eq!(
                db.query_row(
                    "SELECT amount FROM shielding_inputs WHERE operation_id='shield'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                100_000
            );
            assert_eq!(
                db.query_row(
                    "SELECT attempts FROM treasury_operations WHERE id='shield'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            let id = treasury.status().await.unwrap().treasury_id;
            let raw = prepared.bytes().to_vec();
            treasury.close().await.unwrap();
            let reopened = Treasury::open(state, key, id).await.unwrap();
            let bytes = reopened
                .store
                .call(|s| s.prepared_bytes("shield"))
                .await
                .unwrap();
            assert_eq!(&*bytes, &raw);
            // Historical internal facts may still carry the old numeric timer;
            // the exact immutable header, inputs and bytes govern their lifetime.
            let expiry = prepared.facts().unwrap().expiry_height;
            let mut historical = prepared.facts().unwrap().clone();
            historical.deadline = Some(1);
            db.execute(
                "UPDATE treasury_operations SET facts=?1 WHERE id='shield'",
                [serde_json::to_string(&historical).unwrap()],
            )
            .unwrap();
            reopened
                .store
                .call(move |s| {
                    let restored = PreparedTransaction::load(s, "shield")?;
                    restored.check_submission_window(u64::MAX, true)?;
                    assert!(
                        s.request_broadcast("shield", u64::MAX, u64::from(expiry), false)
                            .is_err()
                    );
                    assert_eq!(s.operation("shield")?.attempts, 0);
                    // No RPC occurs: mint intent only, with a still-live block expiry.
                    assert_eq!(
                        s.request_broadcast("shield", 100_000, u64::from(expiry) - 1, false)?,
                        1
                    );
                    Ok(())
                })
                .await
                .unwrap();
        }
    }
}
