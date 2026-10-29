//! SDK observations for reporting only; proposal selection owns spendability.
use crate::rotation::error::TreasuryFunds;
use pepper_sync::wallet::{IronwoodNote, OrchardNote, SaplingNote, TransparentCoin};
use zingolib::wallet::LightWallet;

/// Computed on demand from an existing SDK snapshot; never persisted separately.
#[derive(Default)]
pub(crate) struct SnapshotBalances {
    pub confirmed_shielded: Option<u64>,
    pub pending_shielded: Option<u64>,
    pub spendable_shielded: Option<u64>,
    pub confirmed_transparent: Option<u64>,
    pub pending_transparent: Option<u64>,
}
impl SnapshotBalances {
    pub(super) fn read(wallet: &LightWallet) -> Self {
        let account = zip32::AccountId::ZERO;
        let balance = wallet.account_balance(account).ok();
        Self {
            confirmed_shielded: balance.as_ref().and_then(|b| {
                [
                    b.confirmed_ironwood_balance,
                    b.confirmed_orchard_balance,
                    b.confirmed_sapling_balance,
                ]
                .into_iter()
                .try_fold(0u64, |sum, value| sum.checked_add(value?.into_u64()))
            }),
            pending_shielded: balance.as_ref().and_then(|b| {
                [
                    b.unconfirmed_ironwood_balance,
                    b.unconfirmed_orchard_balance,
                    b.unconfirmed_sapling_balance,
                ]
                .into_iter()
                .try_fold(0u64, |sum, value| sum.checked_add(value?.into_u64()))
            }),
            spendable_shielded: wallet
                .shielded_spendable_balance(account, false)
                .ok()
                .map(|v| v.into_u64()),
            // Unlike account_balance, include dust and immature coinbase funds.
            confirmed_transparent: wallet
                .get_filtered_balance::<TransparentCoin, _>(
                    |_, tx| tx.status().is_confirmed(),
                    account,
                )
                .ok()
                .map(|v| v.into_u64()),
            pending_transparent: wallet
                .unconfirmed_balance::<TransparentCoin>(account)
                .ok()
                .map(|v| v.into_u64()),
        }
    }
}

pub(super) fn funding_assessment(
    wallet: &LightWallet,
    complete_scan: bool,
    required: u64,
) -> TreasuryFunds {
    if !complete_scan {
        return TreasuryFunds::Unavailable;
    }
    // Calculated, transmitted and mempool transactions include incoming/change
    // and live reservations. Historical confirmed spenders are not pending work.
    if wallet
        .wallet_transactions
        .values()
        .any(|tx| tx.status().is_pending())
    {
        return TreasuryFunds::Waiting;
    }
    let account = zip32::AccountId::ZERO;
    // Include dust and transparent holdings when explaining a capital shortage.
    // These SDK totals are not an additional spending check or durable ledger.
    let confirmed = [
        wallet.confirmed_balance::<IronwoodNote>(account),
        wallet.confirmed_balance::<OrchardNote>(account),
        wallet.confirmed_balance::<SaplingNote>(account),
        wallet.get_filtered_balance::<TransparentCoin, _>(
            |_, tx| tx.status().is_confirmed(),
            account,
        ),
    ]
    .into_iter()
    .try_fold(0u64, |sum, balance| {
        sum.checked_add(balance.ok()?.into_u64())
    });
    match confirmed {
        Some(total) if total < required => TreasuryFunds::Shortfall,
        Some(_) => TreasuryFunds::Waiting,
        None => TreasuryFunds::Unavailable,
    }
}

#[cfg(all(test, feature = "zcash-testutils"))]
mod tests {
    use super::*;
    use pepper_sync::wallet::{OutputInterface, WalletTransaction};
    use zcash_protocol::TxId;
    use zingo_status::confirmation_status::ConfirmationStatus;
    use zingolib::testutils::synthetic_wallet::SyntheticWalletBuilder;

    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn sdk_capital_assessment_distinguishes_shortage_from_waits_and_unknown() {
        let mut wallet = SyntheticWalletBuilder::new(SEED)
            .orchard_note(1_000)
            .build();
        assert_eq!(
            funding_assessment(&wallet, false, 50_000),
            TreasuryFunds::Unavailable
        );
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Shortfall,
            "dust alone cannot become sufficient by waiting"
        );
        assert_eq!(
            funding_assessment(&wallet, true, 500),
            TreasuryFunds::Waiting,
            "dust must not be treated as absent capital"
        );
        let spender = TxId::from_bytes([42; 32]);
        wallet
            .wallet_transactions
            .values_mut()
            .next()
            .unwrap()
            .orchard_notes_mut()[0]
            .set_spending_transaction(Some(spender));
        for status in [
            ConfirmationStatus::Calculated(10.into()),
            ConfirmationStatus::Mempool(10.into()),
            ConfirmationStatus::Confirmed(10.into()),
        ] {
            wallet
                .wallet_transactions
                .insert(spender, WalletTransaction::new_for_test(spender, status));
            assert_eq!(
                funding_assessment(&wallet, true, 50_000),
                if status.is_pending() {
                    TreasuryFunds::Waiting
                } else {
                    TreasuryFunds::Shortfall
                },
                "historical spent inputs must not hide a new shortage"
            );
        }
        let wallet = SyntheticWalletBuilder::new(SEED)
            .transparent_coin(100_000)
            .build();
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Waiting,
            "owned transparent funds need shielding, not another deposit"
        );
        let wallet = SyntheticWalletBuilder::new(SEED)
            .orchard_note(100_000)
            .build();
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Waiting,
            "sufficient confirmed holdings may still need confirmations or witnesses"
        );
    }

    #[test]
    fn immature_coinbase_is_owned_capital_waiting_for_maturity() {
        use std::io::{Cursor, Read};
        use zcash_primitives::transaction::Transaction;
        use zcash_protocol::consensus::BranchId;

        let mut wallet = SyntheticWalletBuilder::new(SEED)
            .tip(10)
            .transparent_coin(100_000)
            .build();
        let (txid, original) = wallet.wallet_transactions.iter().next().unwrap();
        let txid = *txid;
        let coin = &original.transparent_coins()[0];
        // A small V1 transparent coinbase with the fixture's owned output.
        // Parsing and maturity are exercised here, not consensus validity.
        let mut raw_coinbase = 1u32.to_le_bytes().to_vec();
        raw_coinbase.push(1); // one input
        raw_coinbase.extend_from_slice(&[0; 32]);
        raw_coinbase.extend_from_slice(&u32::MAX.to_le_bytes());
        raw_coinbase.extend_from_slice(&[2, 1, 1]); // coinbase script
        raw_coinbase.extend_from_slice(&u32::MAX.to_le_bytes());
        raw_coinbase.push(1); // one output
        raw_coinbase.extend_from_slice(&coin.value().to_le_bytes());
        coin.script().write(&mut raw_coinbase).unwrap();
        raw_coinbase.extend_from_slice(&0u32.to_le_bytes()); // lock time
        let transaction = Transaction::read(raw_coinbase.as_slice(), BranchId::Sprout).unwrap();
        // Public serialization preserves SDK-owned received-coin metadata while
        // making this synthetic transaction a real transparent coinbase.
        let mut bytes = Vec::new();
        original.write(&mut bytes, &wallet.chain_type()).unwrap();
        let mut reader = Cursor::new(bytes.as_slice());
        reader.read_exact(&mut [0; 1]).unwrap();
        TxId::read(&mut reader).unwrap();
        ConfirmationStatus::read(&mut reader).unwrap();
        let start = reader.position() as usize;
        Transaction::read(&mut reader, BranchId::Nu6_3).unwrap();
        let end = reader.position() as usize;
        let mut replacement = bytes[..start].to_vec();
        transaction.write(&mut replacement).unwrap();
        replacement.extend_from_slice(&bytes[end..]);
        let record = WalletTransaction::read(replacement.as_slice(), &wallet.chain_type()).unwrap();
        assert!(
            record
                .transaction()
                .transparent_bundle()
                .unwrap()
                .is_coinbase()
        );
        wallet.wallet_transactions.insert(txid, record);
        assert_eq!(
            wallet
                .confirmed_balance::<TransparentCoin>(zip32::AccountId::ZERO)
                .unwrap()
                .into_u64(),
            0
        );
        assert_eq!(
            SnapshotBalances::read(&wallet).confirmed_transparent,
            Some(100_000)
        );
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Waiting
        );
    }
}
