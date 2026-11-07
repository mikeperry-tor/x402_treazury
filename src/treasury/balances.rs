//! SDK observations for reporting only; proposal selection owns spendability.
use crate::rotation::error::TreasuryFunds;
use pepper_sync::wallet::IronwoodNote;
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
        Self {
            confirmed_shielded: wallet
                .confirmed_balance::<IronwoodNote>(account)
                .ok()
                .map(|v| v.into_u64()),
            pending_shielded: wallet
                .unconfirmed_balance::<IronwoodNote>(account)
                .ok()
                .map(|v| v.into_u64()),
            spendable_shielded: wallet
                .spendable_balance::<IronwoodNote>(account, false)
                .ok()
                .map(|v| v.into_u64()),
            // Stable report fields do not advertise unsupported input pools.
            confirmed_transparent: None,
            pending_transparent: None,
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
    let account = zip32::AccountId::ZERO;
    // SDK balances exclude outputs already linked to a spender. Only actual
    // owned Ironwood capital can resolve a supported payment shortage.
    let owned = [
        wallet.confirmed_balance::<IronwoodNote>(account),
        wallet.unconfirmed_balance::<IronwoodNote>(account),
    ]
    .into_iter()
    .try_fold(0u64, |sum, balance| {
        sum.checked_add(balance.ok()?.into_u64())
    });
    match owned {
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
            .ironwood_note(1_000)
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
            .ironwood_notes_mut()[0]
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
                TreasuryFunds::Shortfall,
                "spent original inputs without owned change cannot resolve a shortage"
            );
        }
        let wallet = SyntheticWalletBuilder::new(SEED)
            .transparent_coin(100_000)
            .build();
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Shortfall,
            "unsupported transparent holdings do not authorize shielding or hide an Ironwood shortage"
        );
        assert_eq!(SnapshotBalances::read(&wallet).confirmed_transparent, None);
        let wallet = SyntheticWalletBuilder::new(SEED)
            .ironwood_note(100_000)
            .build();
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Waiting,
            "sufficient confirmed holdings may still need confirmations or witnesses"
        );
    }

    #[test]
    fn unsupported_history_cannot_inflate_ironwood_balances_or_hide_shortfalls() {
        let mut wallet = SyntheticWalletBuilder::new(SEED)
            .tip(30)
            .ironwood_note(20_000)
            .sapling_note(80_000)
            .orchard_note(100_000)
            .transparent_coin(100_000)
            .build();
        let balances = SnapshotBalances::read(&wallet);
        assert_eq!(balances.confirmed_shielded, Some(20_000));
        assert_eq!(balances.spendable_shielded, Some(20_000));
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Shortfall
        );
        let unsupported = wallet
            .wallet_transactions
            .values_mut()
            .find(|tx| !tx.orchard_notes().is_empty())
            .unwrap();
        *unsupported = WalletTransaction::new_for_test_with_orchard_notes(
            unsupported.txid(),
            ConfirmationStatus::Mempool(31.into()),
            unsupported.orchard_notes().to_vec(),
            vec![],
        );
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Shortfall,
            "unsupported pending funds are not a supported capital wait"
        );
        assert_eq!(SnapshotBalances::read(&wallet).pending_shielded, Some(0));
    }

    #[test]
    fn actual_supported_pending_capital_can_resolve_a_shortfall() {
        let mut wallet = SyntheticWalletBuilder::new(SEED)
            .tip(30)
            .ironwood_note(70_000)
            .build();
        let transaction = wallet.wallet_transactions.values_mut().next().unwrap();
        *transaction = WalletTransaction::new_for_test_with_ironwood_notes(
            transaction.txid(),
            ConfirmationStatus::Mempool(31.into()),
            transaction.ironwood_notes().to_vec(),
            vec![],
        );
        assert_eq!(
            SnapshotBalances::read(&wallet).pending_shielded,
            Some(70_000)
        );
        assert_eq!(
            funding_assessment(&wallet, true, 50_000),
            TreasuryFunds::Waiting
        );
        assert_eq!(
            funding_assessment(&wallet, true, 80_000),
            TreasuryFunds::Shortfall
        );
    }
}
