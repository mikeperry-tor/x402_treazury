//! Fast wallet-output validation; no proving, indexer or consensus node.
use super::*;
use pepper_sync::wallet::WalletTransaction;
use zingo_status::confirmation_status::ConfirmationStatus;
use zingolib::testutils::synthetic_wallet::SyntheticWalletBuilder;
const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

fn cases<T: OutputInterface>(
    build: impl Fn(u64) -> LightWallet,
    spend: impl Fn(&mut WalletTransaction, zcash_primitives::transaction::TxId),
) {
    let original = build(100);
    assert_eq!(
        check_owned_outputs(original.wallet_outputs::<T>(), &original, 3, 2).unwrap(),
        100
    );
    for height in [0, 1, 2] {
        assert!(check_owned_outputs(original.wallet_outputs::<T>(), &original, height, 2).is_err());
    }
    let changed = build(101);
    assert!(
        check_owned_outputs(original.wallet_outputs::<T>(), &changed, 20, 2)
            .unwrap_err()
            .to_string()
            .contains("input mismatch")
    );
    let mut missing = build(100);
    missing.wallet_transactions.clear();
    assert!(
        check_owned_outputs(original.wallet_outputs::<T>(), &missing, 20, 2)
            .unwrap_err()
            .to_string()
            .contains("input missing")
    );
    for status in [
        ConfirmationStatus::Confirmed(0.into()),
        ConfirmationStatus::Confirmed(21.into()),
        ConfirmationStatus::Failed(3.into()),
    ] {
        let mut current = build(100);
        for tx in current.wallet_transactions.values_mut() {
            tx.update_status(ConfirmationStatus::Failed(3.into()), 0, true);
            tx.update_status(ConfirmationStatus::Mempool(3.into()), 0, true);
            tx.update_status(status, 0, true);
        }
        assert!(check_owned_outputs(original.wallet_outputs::<T>(), &current, 20, 2).is_err());
    }
    for status in [
        ConfirmationStatus::Confirmed(10.into()),
        ConfirmationStatus::Mempool(10.into()),
        ConfirmationStatus::Calculated(10.into()),
    ] {
        let mut current = build(100);
        let spending = zcash_primitives::transaction::TxId::from_bytes([99; 32]);
        for tx in current.wallet_transactions.values_mut() {
            spend(tx, spending);
        }
        current
            .wallet_transactions
            .insert(spending, WalletTransaction::new_for_test(spending, status));
        assert!(
            check_owned_outputs(original.wallet_outputs::<T>(), &current, 20, 2)
                .unwrap_err()
                .to_string()
                .contains("spent or ambiguous")
        );
    }
    assert_eq!(
        check_owned_outputs(std::iter::empty::<&T>(), &original, 20, 2).unwrap(),
        0
    );
}
#[test]
fn expiry_inputs_all_pools_validate_confirmation_value_and_spend_status() {
    cases::<TransparentCoin>(
        |n| {
            SyntheticWalletBuilder::new(SEED)
                .transparent_coin(n)
                .build()
        },
        |tx, id| {
            for output in tx.transparent_coins_mut() {
                output.set_spending_transaction(Some(id));
            }
        },
    );
    cases::<SaplingNote>(
        |n| SyntheticWalletBuilder::new(SEED).sapling_note(n).build(),
        |tx, id| {
            for output in tx.sapling_notes_mut() {
                output.set_spending_transaction(Some(id));
            }
        },
    );
    cases::<OrchardNote>(
        |n| SyntheticWalletBuilder::new(SEED).orchard_note(n).build(),
        |tx, id| {
            for output in tx.orchard_notes_mut() {
                output.set_spending_transaction(Some(id));
            }
        },
    );
    cases::<IronwoodNote>(
        |n| SyntheticWalletBuilder::new(SEED).ironwood_note(n).build(),
        |tx, id| {
            for output in tx.ironwood_notes_mut() {
                output.set_spending_transaction(Some(id));
            }
        },
    );
    let wallet = SyntheticWalletBuilder::new(SEED)
        .orchard_note(u64::MAX)
        .orchard_note(1)
        .build();
    assert!(
        check_owned_outputs(wallet.wallet_outputs::<OrchardNote>(), &wallet, 20, 2)
            .unwrap_err()
            .to_string()
            .contains("input value overflow")
    );
}

#[test]
fn changed_nullifier_cannot_release_input() {
    let original = SyntheticWalletBuilder::new(SEED).orchard_note(100).build();
    let mut current = SyntheticWalletBuilder::new(SEED).orchard_note(100).build();
    let id = current.wallet_outputs::<OrchardNote>()[0]
        .output_id()
        .txid();
    let record = current.wallet_transactions.get_mut(&id).unwrap();
    let note = record.orchard_notes()[0]
        .clone()
        .with_nullifier_for_test(orchard::note::Nullifier::from_bytes(&[2; 32]).unwrap());
    *record =
        WalletTransaction::new_for_test_with_orchard_notes(id, record.status(), vec![note], vec![]);
    assert!(
        check_owned_outputs(original.wallet_outputs::<OrchardNote>(), &current, 20, 2)
            .unwrap_err()
            .to_string()
            .contains("input mismatch")
    );
}
