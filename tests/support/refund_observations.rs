use super::*;
use crate::rotation::{store::Store, transaction::TransactionFacts};
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
    store.reserve("op", None, 1, 200, 1000).unwrap();
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
    let bindings = vec![("op".into(), coin.address().to_owned())];
    let first = coin.output_id().txid();
    assert!(
        collect_refunds(&wallet, &bindings, &store.status().unwrap(), 20, 2)
            .unwrap()
            .is_empty()
    );
    store.confirm_spend("op", 170, 1).unwrap();
    let mut status = store.status().unwrap();
    assert!(
        collect_refunds(
            &wallet,
            &[("op".into(), "unrelated".into())],
            &status,
            20,
            2
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        collect_refunds(&wallet, &bindings, &status, 1, 2)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        collect_refunds(&wallet, &bindings, &status, 3, 2)
            .unwrap()
            .len(),
        1
    );
    let refunds = collect_refunds(&wallet, &bindings, &status, 4, 2).unwrap();
    assert_eq!(refunds.len(), 2);
    for refund in &refunds {
        store.record_refund(refund.clone()).unwrap();
        store.record_refund(refund.clone()).unwrap();
    }
    status = store.status().unwrap();
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
    assert_eq!(
        collect_refunds(&wallet, &bindings, &status, 20, 2)
            .unwrap()
            .len(),
        2,
        "shielding cannot erase origin evidence"
    );
    wallet
        .wallet_transactions
        .get_mut(&first)
        .unwrap()
        .update_status(ConfirmationStatus::Failed(21.into()), 0, true);
    assert!(
        collect_refunds(&wallet, &bindings, &status, 21, 2)
            .unwrap_err()
            .to_string()
            .contains("treasury_refund_reorg")
    );
    wallet.wallet_transactions.remove(&first);
    assert!(collect_refunds(&wallet, &bindings, &status, 21, 2).is_err());
}
