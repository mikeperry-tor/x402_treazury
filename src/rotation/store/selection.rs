//! Read-only selection from the bounded owner cache. Selection grants no authority.
use super::capacity::payment_ownership;
use super::*;
use crate::rotation::manager::PaymentCandidateHandle;

/// Shared admission arithmetic: liabilities can exceed the observed balance.
pub(super) fn available(balance: U256, exposure: U256) -> U256 {
    balance.saturating_sub(exposure)
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    available: U256,
    role: u8,
    slot: u8,
    wallet: String,
}
impl Rank {
    fn new(available: U256, role: &str, slot: Option<&str>, wallet: &str) -> Self {
        Self {
            available,
            role: match role {
                "ACTIVE" => 0,
                "RETAINED" => 1,
                _ => 2,
            },
            slot: u8::from(slot == Some("large")),
            wallet: wallet.to_owned(),
        }
    }
}

impl Store {
    /// Initial hint selection passes no bound payer. Challenge-time selection
    /// keeps a sufficient bound payer and otherwise uses the identical ranking.
    /// Call under the group gate when selecting for actual signing or handoff.
    pub(crate) fn cached_payment_candidate(
        &self,
        pool: &str,
        cost: U256,
        bound: Option<&str>,
    ) -> Result<Option<PaymentCandidateHandle>> {
        self.cached_payment_candidate_at_boundary(pool, cost, bound, i64::MAX)
    }

    pub(crate) fn cached_payment_candidate_at_boundary(
        &self,
        pool: &str,
        cost: U256,
        bound: Option<&str>,
        through: i64,
    ) -> Result<Option<PaymentCandidateHandle>> {
        let now = super::super::base::now()?;
        let mut best: Option<(Rank, PaymentCandidateHandle)> = None;
        for (wallet, (_, certificate)) in &self.evidence_cache {
            let within: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM wallets WHERE id=?1 AND pool_id=?2 AND sequence<=?3)",
                params![wallet, pool, through],
                |row| row.get(0),
            )?;
            if !within {
                continue;
            }
            let Some(ownership) = payment_ownership(&self.db, pool, wallet)? else {
                continue;
            };
            if cost > ownership.payment_limit {
                continue;
            }
            let support =
                match evidence::check_balance(&self.db, self.evidence_namespace, certificate) {
                    Ok(support) => support,
                    Err(error) if error.is::<super::super::base::evidence::EvidenceFailure>() => {
                        continue;
                    }
                    Err(error) => return Err(error),
                };
            if now > support.observation.valid_until {
                continue;
            }
            let capacity = available(
                support.observation.available(),
                evidence::exposure(&self.db, wallet)?,
            );
            if capacity < cost {
                continue;
            }
            let generation =
                self.db
                    .query_row("SELECT generation FROM pools WHERE id=?1", [pool], |row| {
                        row.get(0)
                    })?;
            let candidate = PaymentCandidateHandle {
                wallet: wallet.clone(),
                address: support.observation.address,
                generation,
            };
            if bound == Some(wallet.as_str()) {
                return Ok(Some(candidate));
            }
            let rank = Rank::new(capacity, &ownership.role, ownership.slot.as_deref(), wallet);
            if best.as_ref().is_none_or(|(current, _)| rank < *current) {
                best = Some((rank, candidate));
            }
        }
        Ok(best.map(|(_, candidate)| candidate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_precedes_role_and_slot_targets_do_not_rank() {
        let rank = |amount, role, slot, id| Rank::new(U256::from(amount), role, slot, id);
        assert!(rank(1u64, "READY", Some("large"), "z") < rank(2, "ACTIVE", None, "a"));
        assert!(rank(1, "ACTIVE", None, "z") < rank(1, "RETAINED", None, "a"));
        assert!(rank(1, "RETAINED", None, "z") < rank(1, "READY", Some("small"), "a"));
        assert!(rank(1, "READY", Some("small"), "z") < rank(1, "READY", Some("large"), "a"));
        assert!(rank(1, "RETAINED", None, "a") < rank(1, "RETAINED", None, "z"));
        assert_eq!(available(U256::from(3), U256::from(4)), U256::ZERO);
        assert_eq!(available(U256::from(8), U256::from(7)), U256::from(1));
    }
}

/// A usable insufficient observation still completes this candidate's evaluation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateEvidenceState {
    Ineligible,
    Usable,
    Missing,
}

impl Store {
    pub(crate) fn candidate_evidence_state(
        &self,
        pool: &str,
        wallet: &str,
    ) -> Result<CandidateEvidenceState> {
        if payment_ownership(&self.db, pool, wallet)?.is_none() {
            return Ok(CandidateEvidenceState::Ineligible);
        }
        Ok(if self.cached_payment_view(pool, wallet)?.is_some() {
            CandidateEvidenceState::Usable
        } else {
            CandidateEvidenceState::Missing
        })
    }

    pub(crate) fn cached_payment_view(
        &self,
        pool: &str,
        wallet: &str,
    ) -> Result<Option<super::super::base::ChainView>> {
        if payment_ownership(&self.db, pool, wallet)?.is_none() {
            return Ok(None);
        }
        let Some((_, certificate)) = self.evidence_cache.get(wallet) else {
            return Ok(None);
        };
        let support = match evidence::check_balance(&self.db, self.evidence_namespace, certificate)
        {
            Ok(support) => support,
            Err(error) if error.is::<super::super::base::evidence::EvidenceFailure>() => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let observation = support.observation;
        if super::super::base::now()? > observation.valid_until {
            return Ok(None);
        }
        Ok(Some(super::super::base::ChainView {
            admission_valid_until: observation.valid_until,
            anchor: observation.confirmed.anchor.clone(),
            balances: [(wallet.to_owned(), observation.available())].into(),
            observations: [(wallet.to_owned(), observation)].into(),
            balance_certificates: [(wallet.to_owned(), certificate.clone())].into(),
            resolution_certificates: Default::default(),
            released: vec![],
            resolutions: Default::default(),
        }))
    }

    /// Runtime evidence publication alone cannot consume a reserve or retire a wallet.
    pub(crate) fn publish_candidate_evidence(
        &mut self,
        pool: &str,
        wallet: &str,
        view: super::super::base::ChainView,
    ) -> Result<CandidateEvidenceState> {
        ensure!(
            view.balance_certificates.len() == 1
                && view.balance_certificates.contains_key(wallet)
                && view.resolutions.is_empty()
                && view.resolution_certificates.is_empty()
                && view.released.is_empty(),
            "candidate evidence scope mismatch"
        );
        self.prepare_evidence_view(view)?;
        self.candidate_evidence_state(pool, wallet)
    }

    /// Bound payer and at most two enabled ready reserves leave a read lane for traversal.
    pub(crate) fn candidate_priority(
        &self,
        pool: &str,
        bound: &str,
        through: i64,
    ) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare("SELECT id FROM wallets WHERE pool_id=?1 AND sequence<=?3 AND (id=?2 OR (role='READY' AND EXISTS(SELECT 1 FROM reserve_claims r WHERE r.pool_id=?1 AND r.wallet_id=wallets.id AND r.enabled=1))) ORDER BY CASE WHEN id=?2 THEN 0 ELSE 1 END,sequence LIMIT 3")?;
        Ok(stmt
            .query_map(params![pool, bound, through], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
}
