//! Expiry must recover every immutable shielding input, not just its fee.
use super::*;
use crate::rotation::store::shielding::Input;

pub(super) fn check(
    current: &LightWallet,
    network: TreasuryNetwork,
    address: &str,
    inputs: &[Input],
    height: u64,
    depth: u32,
) -> Result<()> {
    let Some(zcash_keys::address::Address::Transparent(receiver)) =
        zcash_keys::address::Address::decode(&network.chain(), address)
    else {
        anyhow::bail!("shielding expiry address malformed");
    };
    ensure!(!inputs.is_empty(), "shielding expiry inputs missing");
    let owned = current.wallet_outputs::<TransparentCoin>();
    let mut seen = std::collections::BTreeSet::new();
    for input in inputs {
        ensure!(
            input.amount > 0 && seen.insert((input.txid.as_str(), input.output_index)),
            "shielding expiry input mismatch"
        );
        let coin = owned
            .iter()
            .find(|coin| {
                coin.output_id().txid().to_string() == input.txid
                    && coin.output_id().output_index() == input.output_index
            })
            .context(crate::rotation::transaction::RecoveryEvidenceError::InputMissing)?;
        ensure!(
            coin.address() == address
                && coin.value() == input.amount
                && coin.script().0.0 == input.script
                && *coin.script() == receiver.script().into(),
            crate::rotation::transaction::RecoveryEvidenceError::InputMismatch
        );
        let confirmed = current
            .output_transaction(*coin)
            .status()
            .get_confirmed_height()
            .map(|h| u64::from(u32::from(h)));
        ensure!(
            confirmed.is_some_and(|h| h > 0 && height >= h && height - h + 1 >= u64::from(depth)),
            "shielding expiry input not confirmed"
        );
        ensure!(
            current.output_spend_status(*coin).is_unspent(),
            crate::rotation::transaction::RecoveryEvidenceError::InputSpent
        );
    }
    Ok(())
}

#[cfg(all(test, feature = "zcash-testutils"))]
mod tests {
    use super::*;
    use pepper_sync::wallet::WalletTransaction;
    use zingo_status::confirmation_status::ConfirmationStatus;
    use zingolib::testutils::synthetic_wallet::SyntheticWalletBuilder;

    fn wallet() -> LightWallet {
        SyntheticWalletBuilder::new(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .transparent_coin(100_000)
        .transparent_coin(200_000)
        .build()
    }

    #[test]
    fn expiry_requires_every_exact_shielding_input_even_when_one_covers_fee() {
        let original = wallet();
        let coins = original.wallet_outputs::<TransparentCoin>();
        let address = coins[0].address();
        let inputs: Vec<_> = coins
            .iter()
            .map(|coin| Input {
                txid: coin.output_id().txid().to_string(),
                output_index: coin.output_id().output_index(),
                amount: coin.value(),
                script: coin.script().0.0.clone(),
            })
            .collect();
        let verify = |wallet: &LightWallet, inputs: &[Input], height| {
            check(
                wallet,
                TreasuryNetwork::Synthetic,
                address,
                inputs,
                height,
                2,
            )
        };
        verify(&original, &inputs, 20).unwrap();
        assert!(verify(&original, &inputs, 3).is_err());
        assert!(verify(&original, &[], 20).is_err());
        let mut duplicate = inputs.clone();
        duplicate.push(inputs[0].clone());
        assert!(verify(&original, &duplicate, 20).is_err());
        for change in 0..4 {
            let mut altered = inputs.clone();
            match change {
                0 => altered[0].amount += 1,
                1 => altered[0].script.push(0),
                2 => altered[0].output_index += 1,
                _ => altered[0].txid = "00".repeat(32),
            }
            assert!(verify(&original, &altered, 20).is_err());
        }
        let mut missing = wallet();
        missing
            .wallet_transactions
            .remove(&coins[0].output_id().txid());
        assert!(verify(&missing, &inputs, 20).is_err());
        let mut spent = wallet();
        let spender = zcash_primitives::transaction::TxId::from_bytes([99; 32]);
        let record = spent
            .wallet_transactions
            .get_mut(&coins[0].output_id().txid())
            .unwrap();
        record.transparent_coins_mut()[0].set_spending_transaction(Some(spender));
        spent.wallet_transactions.insert(
            spender,
            WalletTransaction::new_for_test(spender, ConfirmationStatus::Mempool(10.into())),
        );
        assert!(verify(&spent, &inputs, 20).is_err());
        assert!(check(&original, TreasuryNetwork::Mainnet, address, &inputs, 20, 2).is_err());
    }
}
