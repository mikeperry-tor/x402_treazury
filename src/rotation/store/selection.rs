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

struct EligiblePaymentEvidence {
    ownership: super::capacity::PaymentOwnership,
    observation: super::super::base::evidence::BalanceObservation,
    capacity: U256,
}

impl Store {
    /// Initial hint selection passes no bound payer. Challenge-time selection
    /// keeps a sufficient bound payer and otherwise uses the identical ranking.
    /// Call under the group gate when selecting for actual signing or handoff.
    pub(crate) fn cached_payment_candidate(
        &mut self,
        pool: &str,
        cost: U256,
        bound: Option<&str>,
    ) -> Result<Option<PaymentCandidateHandle>> {
        self.cached_payment_candidate_at_boundary(pool, cost, bound, i64::MAX)
    }

    pub(crate) fn cached_payment_candidate_at_boundary(
        &mut self,
        pool: &str,
        cost: U256,
        bound: Option<&str>,
        through: i64,
    ) -> Result<Option<PaymentCandidateHandle>> {
        Ok(self
            .payment_candidate_with_evidence(pool, cost, bound, through, &[])?
            .map(|(candidate, _)| candidate))
    }

    /// Completed caller-owned proofs survive retention-cache churn. Current
    /// observations are compared by chain version; all proofs still require fresh
    /// ownership, lineage, expiry and liability checks under the group gate.
    pub(crate) fn payment_candidate_with_evidence(
        &mut self,
        pool: &str,
        cost: U256,
        bound: Option<&str>,
        through: i64,
        completed: &[super::super::base::ChainView],
    ) -> Result<Option<(PaymentCandidateHandle, super::super::base::ChainView)>> {
        let now = super::super::base::now()?;
        let mut best: Option<(Rank, PaymentCandidateHandle, super::super::base::ChainView)> = None;
        let (certificates, mut failure) =
            self.merge_payment_certificates(pool, through, completed, now)?;
        for (wallet, certificate) in certificates {
            let eligible = match self.eligible_payment_evidence(
                pool,
                &wallet,
                cost,
                through,
                &certificate,
                now,
            ) {
                Ok(eligible) => eligible,
                Err(error) if error.is::<super::super::base::evidence::EvidenceFailure>() => {
                    failure.get_or_insert(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            let Some(EligiblePaymentEvidence {
                ownership,
                observation,
                capacity,
            }) = eligible
            else {
                continue;
            };
            let candidate = PaymentCandidateHandle {
                wallet: wallet.clone(),
                address: observation.address.clone(),
            };
            let view = payment_view(&wallet, &certificate, observation);
            if bound == Some(wallet.as_str()) {
                return Ok(Some((candidate, view)));
            }
            let rank = Rank::new(
                capacity,
                &ownership.role,
                ownership.slot.as_deref(),
                &wallet,
            );
            if best.as_ref().is_none_or(|(current, _, _)| rank < *current) {
                best = Some((rank, candidate, view));
            }
        }
        if best.is_none()
            && let Some(error) = failure
        {
            return Err(error);
        }
        Ok(best.map(|(_, candidate, view)| (candidate, view)))
    }

    fn merge_payment_certificates(
        &mut self,
        pool: &str,
        through: i64,
        completed: &[super::super::base::ChainView],
        now: u64,
    ) -> Result<(
        std::collections::BTreeMap<String, super::super::base::BalanceCertificate>,
        Option<anyhow::Error>,
    )> {
        let mut certificates: std::collections::BTreeMap<_, _> = self
            .evidence_cache
            .iter()
            .map(|(wallet, (_, cert))| (wallet.clone(), cert.clone()))
            .collect();
        // Scope before merging so a foreign group's conflict cannot change this
        // caller's negative result or invalidate evidence outside its group gate.
        let mut retained = std::collections::BTreeMap::new();
        for (wallet, certificate) in certificates {
            if self.candidate_within(pool, &wallet, through)? {
                retained.insert(wallet, certificate);
            }
        }
        certificates = retained;
        let mut failure = None;
        let mut rejected = std::collections::BTreeSet::new();
        for (wallet, certificate) in completed.iter().flat_map(|view| &view.balance_certificates) {
            if rejected.contains(wallet) || !self.candidate_within(pool, wallet, through)? {
                continue;
            }
            if let Some(saved) = certificates.get(wallet) {
                match evidence::check_payment_balance(
                    &self.db,
                    self.evidence_namespace,
                    certificate,
                ) {
                    Ok(support) if now <= support.observation.valid_until => {}
                    Ok(_) => continue,
                    Err(error) if error.is::<super::super::base::evidence::EvidenceFailure>() => {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
                match evidence::check_payment_balance(&self.db, self.evidence_namespace, saved) {
                    Ok(support) if now <= support.observation.valid_until => {
                        match self.compare_cached_balance_or_invalidate(
                            wallet,
                            certificate.observation(),
                            saved.observation(),
                        ) {
                            Ok(std::cmp::Ordering::Greater) => {}
                            Ok(_) => continue,
                            Err(error)
                                if error.is::<super::super::base::evidence::EvidenceFailure>() =>
                            {
                                tracing::warn!(category = %super::super::base::safe_diagnostic(&error), "candidate certificate conflict; checking independent capacity");
                                certificates.remove(wallet);
                                failure.get_or_insert(error);
                                rejected.insert(wallet.clone());
                                continue;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    Ok(_) => {}
                    Err(error) if error.is::<super::super::base::evidence::EvidenceFailure>() => {}
                    Err(error) => return Err(error),
                }
            }
            certificates.insert(wallet.clone(), certificate.clone());
        }
        Ok((certificates, failure))
    }

    fn candidate_within(&self, pool: &str, wallet: &str, through: i64) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM wallets WHERE id=?1 AND pool_id=?2 AND sequence<=?3)",
            params![wallet, pool, through],
            |row| row.get(0),
        )?)
    }

    fn eligible_payment_evidence(
        &self,
        pool: &str,
        wallet: &str,
        cost: U256,
        through: i64,
        certificate: &super::super::base::BalanceCertificate,
        now: u64,
    ) -> Result<Option<EligiblePaymentEvidence>> {
        if !self.candidate_within(pool, wallet, through)? {
            return Ok(None);
        }
        let ownership = match payment_ownership(&self.db, pool, wallet) {
            Ok(Some(ownership)) => ownership,
            Ok(None) => return Ok(None),
            // The captured inventory pass retains this wallet's diagnostic.
            // A bad historical binding cannot veto an independent cached payer.
            Err(error)
                if error.downcast_ref::<super::super::base::evidence::EvidenceFailure>()
                    == Some(&super::super::base::evidence::EvidenceFailure::MalformedSupport) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if cost > ownership.payment_limit {
            return Ok(None);
        }
        let support =
            match evidence::check_payment_balance(&self.db, self.evidence_namespace, certificate) {
                Ok(support) => support,
                Err(error) if error.is::<super::super::base::evidence::EvidenceFailure>() => {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
        if now > support.observation.valid_until {
            return Ok(None);
        }
        let exposure = evidence::exposure(&self.db, wallet)?;
        let capacity = available(support.observation.available(), exposure);
        if capacity < cost {
            return Ok(None);
        }
        Ok(Some(EligiblePaymentEvidence {
            ownership,
            observation: support.observation,
            capacity,
        }))
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
    /// Capture eligibility and its exact cached view once in this store job.
    pub(crate) fn candidate_evidence(
        &self,
        pool: &str,
        wallet: &str,
    ) -> Result<(
        CandidateEvidenceState,
        Option<super::super::base::ChainView>,
    )> {
        if payment_ownership(&self.db, pool, wallet)?.is_none() {
            return Ok((CandidateEvidenceState::Ineligible, None));
        }
        // Preserve the precise wallet-local failure for traversal diagnostics;
        // an unreadable exposure can never become a usable zero-liability view.
        evidence::exposure(&self.db, wallet)?;
        let missing = (CandidateEvidenceState::Missing, None);
        let Some((_, certificate)) = self.evidence_cache.get(wallet) else {
            return Ok(missing);
        };
        let support =
            match evidence::check_payment_balance(&self.db, self.evidence_namespace, certificate) {
                Ok(support) => support,
                Err(error) if error.is::<super::super::base::evidence::EvidenceFailure>() => {
                    return Ok(missing);
                }
                Err(error) => return Err(error),
            };
        let observation = support.observation;
        if super::super::base::now()? > observation.valid_until {
            return Ok(missing);
        }
        Ok((
            CandidateEvidenceState::Usable,
            Some(payment_view(wallet, certificate, observation)),
        ))
    }

    pub(crate) fn cached_payment_view(
        &self,
        pool: &str,
        wallet: &str,
    ) -> Result<Option<super::super::base::ChainView>> {
        Ok(self.candidate_evidence(pool, wallet)?.1)
    }

    /// Runtime evidence publication alone cannot consume a reserve or retire a wallet.
    pub(crate) fn publish_candidate_evidence(
        &mut self,
        pool: &str,
        wallet: &str,
        view: super::super::base::ChainView,
    ) -> Result<(
        CandidateEvidenceState,
        Option<super::super::base::ChainView>,
    )> {
        ensure!(
            view.balance_certificates.len() == 1
                && view.balance_certificates.contains_key(wallet)
                && view.resolutions.is_empty()
                && view.resolution_certificates.is_empty()
                && view.released.is_empty(),
            "candidate evidence scope mismatch"
        );
        self.prepare_payment_evidence_view(view)?;
        self.candidate_evidence(pool, wallet)
    }

    /// Bound payer and at most two enabled ready reserves leave a read lane for traversal.
    pub(crate) fn candidate_priority(
        &self,
        pool: &str,
        bound: &str,
        through: i64,
    ) -> Result<Vec<String>> {
        let mut priority = Vec::with_capacity(3);
        let bound: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM wallets WHERE id=?1 AND pool_id=?2 AND sequence<=?3",
                params![bound, pool, through],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(bound) = bound {
            priority.push(bound);
        }
        let mut stmt = self.db.prepare("SELECT w.id FROM reserve_claims c JOIN wallets w ON w.id=c.wallet_id WHERE c.pool_id=?1 AND c.enabled=1 AND w.pool_id=?1 AND w.role='READY' AND w.sequence<=?2 ORDER BY w.sequence")?;
        for wallet in stmt.query_map(params![pool, through], |r| r.get::<_, String>(0))? {
            let wallet = wallet?;
            if !priority.contains(&wallet) {
                priority.push(wallet);
            }
        }
        Ok(priority)
    }
}

impl Store {
    pub(crate) fn capacity_changes(&self, pool: &str) -> std::sync::Arc<tokio::sync::Notify> {
        self.group_gates
            .lock()
            .expect("group ownership lock")
            .entry(pool.to_owned())
            .or_default()
            .capacity_changed
            .clone()
    }

    pub(super) fn notify_capacity_changed(&self, pool: &str) {
        if let Some(owner) = self
            .group_gates
            .lock()
            .expect("group ownership lock")
            .get(pool)
        {
            owner.capacity_changed.notify_waiters();
        }
    }
}

#[cfg(test)]
mod notification_tests {
    use super::*;
    use futures_util::FutureExt;

    #[tokio::test]
    async fn capacity_notifications_are_group_scoped_and_cancel_safe() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let changes = store.capacity_changes("one");
        let other = store.capacity_changes("two");
        let first = changes.notified();
        let second = changes.notified();
        tokio::pin!(first, second);
        first.as_mut().enable();
        second.as_mut().enable();
        store.notify_capacity_changed("two");
        assert!(first.as_mut().now_or_never().is_none());
        assert!(second.as_mut().now_or_never().is_none());
        store.notify_capacity_changed("one");
        assert!(first.as_mut().now_or_never().is_some());
        assert!(second.as_mut().now_or_never().is_some());
        // A dropped subscription owns neither evidence nor a future notification.
        let mut cancelled = Box::pin(changes.notified());
        cancelled.as_mut().enable();
        drop(cancelled);
        let next = changes.notified();
        tokio::pin!(next);
        next.as_mut().enable();
        store.notify_capacity_changed("one");
        next.await;
        assert!(!std::sync::Arc::ptr_eq(&changes, &other));
    }
}

// Construct from the exact validated observation without renewing its authority.
fn payment_view(
    wallet: &str,
    certificate: &super::super::base::BalanceCertificate,
    observation: super::super::base::evidence::BalanceObservation,
) -> super::super::base::ChainView {
    super::super::base::ChainView {
        admission_valid_until: observation.valid_until,
        anchor: observation.confirmed.anchor.clone(),
        balances: [(wallet.to_owned(), observation.available())].into(),
        observations: [(wallet.to_owned(), observation)].into(),
        balance_certificates: [(wallet.to_owned(), certificate.clone())].into(),
        resolution_certificates: Default::default(),
        released: vec![],
        resolutions: Default::default(),
    }
}
