//! One caller-owned candidate pass. Bounded pages/futures do not bound inventory.
use super::*;
use crate::rotation::{
    base::ChainView,
    store::{history::PAGE_SIZE, selection::CandidateEvidenceState},
};
use std::collections::VecDeque;

// At most three priority identities leave the fourth lane for keyset traversal.
const READS: usize = 4;

pub(super) struct SelectedCandidate {
    pub candidate: PaymentCandidateHandle,
    pub view: ChainView,
    /// Keep through local signing/journal acknowledgement for the bound payer;
    /// drop before the single unsigned provider handoff for a replacement.
    pub guard: Arc<tokio::sync::OwnedMutexGuard<()>>,
}

pub(super) enum CandidatePassResult {
    Selected(Box<SelectedCandidate>),
    /// Every eligible captured candidate had usable evidence at its evaluation.
    CompleteNegative,
    /// Failed or already-stale acquisition is not global insufficient funds.
    Incomplete,
}

impl ManagedPool {
    /// Caller already owns the foreground admission permit. This function does
    /// not allocate, mutate roles, reserve funds, or sign. Dropping its futures
    /// cancels only this caller's unsigned acquisitions, never maintenance.
    pub(super) async fn acquire_candidate(
        &self,
        bound: &PaymentCandidateHandle,
        cost: U256,
    ) -> Result<CandidatePassResult> {
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let (pool, wallet) = (self.pool.clone(), bound.wallet.clone());
        let (through, priority) = self
            .store
            .call_with_guard(guard, move |s| {
                let through = s.wallet_boundary(&pool)?;
                Ok((through, s.candidate_priority(&pool, &wallet, through)?))
            })
            .await?;
        // Fixed small priority list is also the traversal dedup set. Keyset
        // monotonicity deduplicates every other candidate without history growth.
        let priority_ids = priority.clone();
        let mut priority: VecDeque<_> = priority.into();
        let mut page = VecDeque::new();
        let mut cursor = -1;
        let mut exhausted = false;
        let mut incomplete = false;
        let mut reads = tokio::task::JoinSet::<Result<CandidateEvidenceState>>::new();

        loop {
            while let Some(result) = reads.try_join_next() {
                incomplete |=
                    completion_is_incomplete(result.context("candidate acquisition task failed")?)?;
            }
            // This is the actual gated selection, not a provisional winner.
            // Recompute all completed cached candidates, liabilities, and roles.
            if let Some(selected) = self
                .select_completed_candidate(&bound.wallet, cost, through)
                .await?
            {
                // `reads` is dropped now, without waiting for unrelated EOF.
                return Ok(CandidatePassResult::Selected(Box::new(selected)));
            }
            while reads.len() < READS {
                let next = if let Some(wallet) = priority.pop_front() {
                    Some(wallet)
                } else {
                    loop {
                        if let Some((sequence, wallet, _)) = page.pop_front() {
                            cursor = sequence;
                            if !priority_ids.contains(&wallet) {
                                break Some(wallet);
                            }
                            continue;
                        }
                        if exhausted {
                            break None;
                        }
                        let pool = self.pool.clone();
                        let rows = self
                            .store
                            .call(move |s| s.wallet_page(&pool, cursor, through, PAGE_SIZE))
                            .await?;
                        if rows.is_empty() {
                            exhausted = true;
                            break None;
                        }
                        page = rows.into();
                    }
                };
                let Some(wallet) = next else { break };
                // A cached insufficient balance still completes an evaluation;
                // never reread it merely to seek a more favorable answer.
                let guard = Arc::new(self.gate.clone().lock_owned().await);
                let (pool, id) = (self.pool.clone(), wallet.clone());
                let state = self
                    .store
                    .call_with_guard(guard, move |s| s.candidate_evidence_state(&pool, &id))
                    .await?;
                if state == CandidateEvidenceState::Missing {
                    let owner = self.clone();
                    let context = crate::qualification::capture_case();
                    reads.spawn(crate::qualification::inherit_case(context, async move {
                        owner.acquire_candidate_wallet(wallet).await
                    }));
                }
                // Poll independently ready acquisitions while walking cached or
                // ineligible rows, so a long inventory cannot hide a completion.
                while let Some(result) = reads.try_join_next() {
                    incomplete |= completion_is_incomplete(
                        result.context("candidate acquisition task failed")?,
                    )?;
                }
                if let Some(selected) = self
                    .select_completed_candidate(&bound.wallet, cost, through)
                    .await?
                {
                    return Ok(CandidatePassResult::Selected(Box::new(selected)));
                }
            }
            if let Some(result) = reads.join_next().await {
                incomplete |=
                    completion_is_incomplete(result.context("candidate acquisition task failed")?)?;
                continue;
            }
            if exhausted && priority.is_empty() && page.is_empty() {
                // A final gated recheck may see independently completed evidence;
                // this does not reopen the captured pass or acquire another row.
                if let Some(selected) = self
                    .select_completed_candidate(&bound.wallet, cost, through)
                    .await?
                {
                    return Ok(CandidatePassResult::Selected(Box::new(selected)));
                }
                return Ok(if incomplete {
                    CandidatePassResult::Incomplete
                } else {
                    CandidatePassResult::CompleteNegative
                });
            }
        }
    }

    async fn select_completed_candidate(
        &self,
        bound: &str,
        cost: U256,
        through: i64,
    ) -> Result<Option<SelectedCandidate>> {
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let (pool, bound) = (self.pool.clone(), bound.to_owned());
        let selected = self
            .store
            .call_with_guard(guard.clone(), move |s| {
                let Some(candidate) =
                    s.cached_payment_candidate_at_boundary(&pool, cost, Some(&bound), through)?
                else {
                    return Ok(None);
                };
                // A clock tick may expire the candidate between these local checks.
                // That loses this provisional result, never the original traversal.
                let Some(view) = s.cached_payment_view(&pool, &candidate.wallet)? else {
                    return Ok(None);
                };
                Ok(Some((candidate, view)))
            })
            .await?;
        Ok(selected.map(|(candidate, view)| SelectedCandidate {
            candidate,
            view,
            guard,
        }))
    }

    async fn acquire_candidate_wallet(&self, wallet: String) -> Result<CandidateEvidenceState> {
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let (pool, id) = (self.pool.clone(), wallet.clone());
        let query = self
            .store
            .call_with_guard(guard, move |s| {
                s.scoped_chain_query(&pool, vec![id], vec![])
            })
            .await?;
        let view = self.acquire_view(query, true, None).await?;
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let pool = self.pool.clone();
        self.store
            .call_with_guard(guard, move |s| {
                s.publish_candidate_evidence(&pool, &wallet, view)
            })
            .await
    }
}

// Integrity rejection is not a new acquisition allowance or permission to shop
// around a rejected proof. Acquisition failures leave a negative pass incomplete.
fn completion_is_incomplete(result: Result<CandidateEvidenceState>) -> Result<bool> {
    use crate::rotation::base::{EndpointMismatch, EvidenceConflict, evidence::EvidenceFailure};
    match result {
        Ok(CandidateEvidenceState::Usable | CandidateEvidenceState::Ineligible) => Ok(false),
        Ok(CandidateEvidenceState::Missing) => Ok(true),
        Err(error) => {
            if error.is::<EvidenceConflict>()
                || error.is::<EndpointMismatch>()
                || matches!(
                    error.downcast_ref::<EvidenceFailure>(),
                    Some(
                        EvidenceFailure::ScopeMismatch
                            | EvidenceFailure::InvalidTuple
                            | EvidenceFailure::ConfirmedReplacement
                            | EvidenceFailure::LatestReplacement
                            | EvidenceFailure::AmountContradiction
                            | EvidenceFailure::Invalidated
                            | EvidenceFailure::ChainRepairRequired
                            | EvidenceFailure::EvidenceRepairRequired
                            | EvidenceFailure::ResolutionConflict
                            | EvidenceFailure::MalformedSupport
                    )
                )
            {
                return Err(error);
            }
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests;
