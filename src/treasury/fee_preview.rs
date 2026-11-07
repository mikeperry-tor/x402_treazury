//! Display-only SDK proposals; no input ownership or executable terms escape.
use super::{Treasury, native_recipient, send::source_proposal};
use anyhow::{Result, ensure};
use zcash_client_backend::zip321::{Payment, TransactionRequest};
use zcash_keys::address::Address;
use zcash_protocol::{PoolType, value::Zatoshis};
use zingolib::wallet::{LightWallet, error::ProposeSendError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FeeTarget {
    Native(String),
    NearDeposit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FeePreviewUnavailable {
    ScanRequired,
    InsufficientFunds,
    Unsupported,
    Unavailable,
}

impl std::fmt::Display for FeePreviewUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ScanRequired => "scan_required",
            Self::InsufficientFunds => "insufficient_funds",
            Self::Unsupported => "unsupported_transaction",
            Self::Unavailable => "unavailable",
        })
    }
}
impl std::error::Error for FeePreviewUnavailable {}

impl Treasury {
    pub(super) async fn preview_fee(&self, target: FeeTarget, amount: u64) -> Result<u64> {
        let client = self
            .client
            .as_ref()
            .ok_or(FeePreviewUnavailable::Unavailable)?;
        wallet_fee(&mut *client.wallet().write().await, target, amount)
    }
}

fn wallet_fee(wallet: &mut LightWallet, target: FeeTarget, amount: u64) -> Result<u64> {
    let amount = Zatoshis::from_u64(amount)?;
    ensure!(amount > Zatoshis::ZERO, "preview amount must be positive");
    let chain = wallet.chain_type();
    let native = matches!(&target, FeeTarget::Native(_));
    let recipient = match target {
        FeeTarget::Native(encoded) => {
            native_recipient(&chain, &encoded).map_err(|_| FeePreviewUnavailable::Unsupported)?
        }
        FeeTarget::NearDeposit => {
            // Zero public-key hash, used only to model one P2PKH deposit output.
            // Decode through an existing dependency, then encode for this chain.
            // This receiver and the proposal never leave this display-only path.
            Address::decode(
                &zingolib::config::ChainType::Mainnet,
                "t1Hsc1LR8yKnbbe3twRp88p6vFfC5t7DLbs",
            )
            .ok_or(FeePreviewUnavailable::Unavailable)?
        }
    };
    let payment = Payment::new(
        recipient.encode(&chain).parse()?,
        Some(amount),
        None,
        None,
        None,
        vec![],
    )?;
    let proposal =
        source_proposal(wallet, TransactionRequest::new(vec![payment])?).map_err(proposal_error)?;
    // Use the same supported source shape as preparation, without accepting it.
    let step = proposal.steps().first();
    if proposal.steps().len() != 1
        || !step.transparent_inputs().is_empty()
        || step.shielded_inputs().is_none()
        || (native
            && (step.payment_pools().len() != 1
                || step.payment_pools().get(&0) != Some(&PoolType::IRONWOOD)))
        || step
            .balance()
            .proposed_change()
            .iter()
            .any(|change| change.output_pool() != PoolType::IRONWOOD)
        || super::inputs::selected_inputs(&proposal).is_err()
    {
        return Err(FeePreviewUnavailable::Unsupported.into());
    }
    Ok(zingolib::data::proposal::total_fee(&proposal)?.into_u64())
}

fn proposal_error(error: ProposeSendError) -> FeePreviewUnavailable {
    use zcash_client_backend::data_api::error::Error;
    use zingolib::wallet::error::WalletError;
    match error {
        ProposeSendError::Proposal(Error::InsufficientFunds { .. }) => {
            FeePreviewUnavailable::InsufficientFunds
        }
        ProposeSendError::Proposal(
            Error::ScanRequired
            | Error::DataSource(
                WalletError::NoSyncData
                | WalletError::SyncIncomplete
                | WalletError::CheckpointNotFound { .. },
            ),
        ) => FeePreviewUnavailable::ScanRequired,
        _ => FeePreviewUnavailable::Unavailable,
    }
}

#[cfg(all(test, feature = "zcash-testutils"))]
mod tests {
    use super::*;
    use zingolib::testutils::synthetic_wallet::SyntheticWalletBuilder;

    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn native_address(wallet: &LightWallet) -> String {
        let spending: zcash_keys::keys::UnifiedSpendingKey = wallet
            .unified_key_store
            .get(&zip32::AccountId::ZERO)
            .unwrap()
            .try_into()
            .unwrap();
        let receiver = spending
            .to_unified_full_viewing_key()
            .orchard()
            .unwrap()
            .address_at(0u32, zip32::Scope::External);
        zcash_keys::address::UnifiedAddress::from_receivers(Some(receiver), None, None, None, None)
            .unwrap()
            .encode(&wallet.chain_type())
    }

    #[tokio::test]
    async fn sdk_fees_leave_wallet_and_existing_proposal_unchanged() {
        let wallet = SyntheticWalletBuilder::new(SEED)
            .ironwood_note(500_000)
            .build();
        let recipient = native_address(&wallet);
        let mut client = zingolib::lightclient::LightClient::new_for_test(wallet).await;
        let payment = Payment::new(
            recipient.parse().unwrap(),
            Some(Zatoshis::from_u64(50_000).unwrap()),
            None,
            None,
            None,
            vec![],
        )
        .unwrap();
        // Retain an unrelated cached proposal; fee display must not replace or clear it.
        let cached = client
            .propose_send(
                TransactionRequest::new(vec![payment]).unwrap(),
                zip32::AccountId::ZERO,
            )
            .await
            .unwrap();
        let expected_native_fee = zingolib::data::proposal::total_fee(&cached)
            .unwrap()
            .into_u64();
        let mut wallet = client.wallet().write().await;
        let before = super::super::snapshot_wallet(&mut wallet).unwrap().0;
        // Debug also covers unsaved proposal and advisory-lock state. Never print this
        // representation; all wallet keys here are the public deterministic test seed.
        let before_ephemeral = format!("{wallet:?}");
        assert_eq!(
            wallet_fee(&mut wallet, FeeTarget::Native(recipient), 50_000).unwrap(),
            expected_native_fee
        );
        assert!(wallet_fee(&mut wallet, FeeTarget::NearDeposit, 50_000).unwrap() > 0);
        assert!(
            before_ephemeral == format!("{wallet:?}"),
            "fee preview changed wallet or cached proposal state"
        );
        assert!(
            before == super::super::snapshot_wallet(&mut wallet).unwrap().0,
            "fee preview changed persisted wallet bytes"
        );
    }

    #[test]
    fn unavailable_fees_are_typed_and_do_not_fall_back_to_legacy_funds() {
        let mut wallet = SyntheticWalletBuilder::new(SEED).build();
        let recipient = native_address(&wallet);
        let error = wallet_fee(&mut wallet, FeeTarget::Native(recipient), 50_000).unwrap_err();
        assert_eq!(
            error.downcast_ref::<FeePreviewUnavailable>(),
            Some(&FeePreviewUnavailable::InsufficientFunds)
        );
        wallet.sync_state = pepper_sync::wallet::SyncState::new();
        let error = wallet_fee(&mut wallet, FeeTarget::NearDeposit, 50_000).unwrap_err();
        assert_eq!(
            error.downcast_ref::<FeePreviewUnavailable>(),
            Some(&FeePreviewUnavailable::ScanRequired)
        );

        let mut legacy = SyntheticWalletBuilder::new(SEED)
            .orchard_note(500_000)
            .build();
        assert!(wallet_fee(&mut legacy, FeeTarget::NearDeposit, 50_000).is_err());
    }

    #[test]
    fn transparent_shape_stays_private_and_native_rejects_it() {
        let mut wallet = SyntheticWalletBuilder::new(SEED)
            .ironwood_note(500_000)
            .build();
        let chain = wallet.chain_type();
        let address = Address::decode(
            &zingolib::config::ChainType::Mainnet,
            "t1Hsc1LR8yKnbbe3twRp88p6vFfC5t7DLbs",
        )
        .unwrap();
        assert!(matches!(address, Address::Transparent(_)));
        let encoded = address.encode(&chain);
        assert!(matches!(
            Address::decode(&chain, &encoded),
            Some(Address::Transparent(_))
        ));
        let before = super::super::snapshot_wallet(&mut wallet).unwrap().0;
        let error = wallet_fee(&mut wallet, FeeTarget::Native(encoded), 50_000).unwrap_err();
        assert_eq!(
            error.downcast_ref::<FeePreviewUnavailable>(),
            Some(&FeePreviewUnavailable::Unsupported)
        );
        assert!(before == super::super::snapshot_wallet(&mut wallet).unwrap().0);
    }
}
