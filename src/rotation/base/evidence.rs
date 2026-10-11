//! Complete balance values and canonical comparison, independent of payment policy.
use super::Anchor;
use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

/// Fixed failure categories. Acquisition failures alone never invalidate accepted facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceFailure {
    ScopeMismatch,
    InvalidTuple,
    ConfirmedReplacement,
    LatestReplacement,
    AmountContradiction,
}
impl std::fmt::Display for EvidenceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ScopeMismatch => "evidence_scope_mismatch",
            Self::InvalidTuple => "invalid_evidence_tuple",
            Self::ConfirmedReplacement => "confirmed_anchor_replacement",
            Self::LatestReplacement => "latest_anchor_replacement",
            Self::AmountContradiction => "evidence_amount_contradiction",
        })
    }
}
impl std::error::Error for EvidenceFailure {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancePoint {
    pub anchor: Anchor,
    pub amount: U256,
}

/// Exact observations; neither conservative value may be relabelled as an exact read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceObservation {
    pub wallet: String,
    pub address: String,
    pub network: String,
    pub asset: String,
    /// SHA-256 identity of the configured endpoint; contains no URL or credential.
    pub endpoint: String,
    pub confirmations: u64,
    pub confirmed: BalancePoint,
    pub latest: BalancePoint,
    pub valid_until: u64,
}
impl BalanceObservation {
    pub fn available(&self) -> U256 {
        self.confirmed.amount.min(self.latest.amount)
    }
    pub fn remainder(&self) -> U256 {
        self.confirmed.amount.max(self.latest.amount)
    }
    pub fn validate(&self) -> Result<(), EvidenceFailure> {
        if self.network != "eip155:8453" || !self.asset.eq_ignore_ascii_case(crate::payment::USDC) {
            return Err(EvidenceFailure::ScopeMismatch);
        }
        if self.wallet.is_empty()
            || self.address.parse::<Address>().is_err()
            || self.confirmed.anchor.hash.parse::<B256>().is_err()
            || self.latest.anchor.hash.parse::<B256>().is_err()
            || self.endpoint.len() != 64
            || !self.endpoint.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(EvidenceFailure::InvalidTuple);
        }
        if self.confirmations == 0
            || self.latest.anchor.height.checked_sub(self.confirmations)
                != Some(self.confirmed.anchor.height)
        {
            return Err(EvidenceFailure::InvalidTuple);
        }
        Ok(())
    }
    /// Compare complete observations before considering their age/order. Conflicts
    /// at equal hashes are integrity failures even when the incoming tuple is older.
    pub fn compare(&self, previous: &Self) -> Result<std::cmp::Ordering, EvidenceFailure> {
        self.validate()?;
        previous.validate()?;
        if self.wallet != previous.wallet
            || self.address != previous.address
            || self.network != previous.network
            || self.asset != previous.asset
            || self.confirmations != previous.confirmations
        {
            return Err(EvidenceFailure::ScopeMismatch);
        }
        for new in [&self.confirmed, &self.latest] {
            for old in [&previous.confirmed, &previous.latest] {
                if new.anchor.hash == old.anchor.hash
                    && (new.anchor.height != old.anchor.height || new.amount != old.amount)
                {
                    return Err(EvidenceFailure::AmountContradiction);
                }
            }
        }
        if self.confirmed.anchor.height == previous.confirmed.anchor.height
            && self.confirmed.anchor.hash != previous.confirmed.anchor.hash
        {
            return Err(EvidenceFailure::ConfirmedReplacement);
        }
        if self.latest.anchor.height == previous.latest.anchor.height
            && self.latest.anchor.hash != previous.latest.anchor.hash
        {
            return Err(EvidenceFailure::LatestReplacement);
        }
        Ok(self
            .confirmed
            .anchor
            .height
            .cmp(&previous.confirmed.anchor.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(height: u64, confirmed: u64, latest: u64) -> BalanceObservation {
        BalanceObservation {
            wallet: "wallet".into(),
            address: format!("0x{:040x}", 1),
            network: "eip155:8453".into(),
            asset: crate::payment::USDC.into(),
            endpoint: "a".repeat(64),
            confirmations: 12,
            confirmed: BalancePoint {
                anchor: Anchor {
                    height,
                    hash: format!("0x{height:064x}"),
                },
                amount: U256::from(confirmed),
            },
            latest: BalancePoint {
                anchor: Anchor {
                    height: height + 12,
                    hash: format!("0x{:064x}", height + 12),
                },
                amount: U256::from(latest),
            },
            valid_until: 120,
        }
    }
    #[test]
    fn exact_values_have_separate_admission_and_retirement_predicates() {
        let observation = observation(20, 8, 3);
        assert_eq!(observation.available(), U256::from(3));
        assert_eq!(observation.remainder(), U256::from(8));
        assert_eq!(observation.confirmed.amount, U256::from(8));
    }
    #[test]
    fn conflict_precedes_supersession_and_checks_cross_position_hashes() {
        let previous = observation(32, 8, 3);
        let old = observation(20, 8, 7);
        assert_eq!(
            old.compare(&previous),
            Err(EvidenceFailure::AmountContradiction)
        );
        let old = observation(20, 8, 8);
        assert_eq!(old.compare(&previous), Ok(std::cmp::Ordering::Less));
    }
    #[test]
    fn replacements_and_scope_changes_have_distinct_categories() {
        let original = observation(20, 8, 3);
        let mut changed = original.clone();
        changed.latest.anchor.hash = format!("0x{:064x}", 999);
        assert_eq!(
            changed.compare(&original),
            Err(EvidenceFailure::LatestReplacement)
        );
        changed.confirmed.anchor.hash = format!("0x{:064x}", 998);
        assert_eq!(
            changed.compare(&original),
            Err(EvidenceFailure::ConfirmedReplacement)
        );
        changed.confirmations = 1;
        assert_eq!(
            changed.compare(&original),
            Err(EvidenceFailure::InvalidTuple)
        );
    }
    #[test]
    fn lower_newer_balance_is_usable_and_never_extends_its_lifetime() {
        let original = observation(20, 8, 3);
        let newer = observation(21, 2, 1);
        assert_eq!(newer.compare(&original), Ok(std::cmp::Ordering::Greater));
        assert_eq!(newer.available(), U256::from(1));
        assert_eq!(newer.valid_until, 120);
    }
}
