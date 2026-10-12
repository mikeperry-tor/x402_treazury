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
        let (through, priority, changes) = self
            .store
            .call_with_guard(guard, move |s| {
                let through = s.wallet_boundary(&pool)?;
                Ok((
                    through,
                    s.candidate_priority(&pool, &wallet, through)?,
                    s.capacity_changes(&pool),
                ))
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
        let mut reads =
            tokio::task::JoinSet::<Result<(CandidateEvidenceState, Option<ChainView>)>>::new();

        // Retain only the current traversal batch plus the four read lanes.
        // Selection consumes this batch even when it has no sufficient proof.
        let mut completed = Vec::new();
        loop {
            // Register before inspecting evidence to avoid a publication between
            // the last gated scan and the wait being lost. Wake all group waiters.
            let changed = changes.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            while let Some(result) = reads.try_join_next() {
                incomplete |= completion_is_incomplete(
                    result.context("candidate acquisition task failed")?,
                    &mut completed,
                )?;
            }
            // This is the actual gated selection, not a provisional winner.
            // Recompute all completed cached candidates, liabilities, and roles.
            if let Some(selected) = self
                .select_completed_candidate(&bound.wallet, cost, through, &mut completed)
                .await?
            {
                // `reads` is dropped now, without waiting for unrelated EOF.
                return Ok(CandidatePassResult::Selected(Box::new(selected)));
            }
            let mut evaluated_since_selection = 0;
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
                    .call_with_guard(guard, move |s| {
                        Ok((
                            s.candidate_evidence_state(&pool, &id)?,
                            s.cached_payment_view(&pool, &id)?,
                        ))
                    })
                    .await?;
                if let Some(view) = state.1 {
                    completed.push(view);
                }
                let state = state.0;
                if state == CandidateEvidenceState::Missing {
                    let owner = self.clone();
                    let context = crate::qualification::capture_case();
                    reads.spawn(crate::qualification::inherit_case(context, async move {
                        owner.acquire_candidate_wallet(wallet).await
                    }));
                }
                #[cfg(test)]
                self.admission_test_stage(AdmissionTestStage::CandidateEvaluated)
                    .await;
                // Poll independently ready acquisitions while walking cached or
                // ineligible rows, so a long inventory cannot hide a completion.
                let mut read_completed = false;
                while let Some(result) = reads.try_join_next() {
                    read_completed = true;
                    incomplete |= completion_is_incomplete(
                        result.context("candidate acquisition task failed")?,
                        &mut completed,
                    )?;
                }
                evaluated_since_selection += 1;
                // Recheck immediately after our reads complete. Otherwise batch
                // the full cache scan at the existing inventory page size: this
                // also notices independent evidence/liability changes without
                // rescanning the same cache for every unchanged inventory row.
                if read_completed || evaluated_since_selection == PAGE_SIZE {
                    if let Some(selected) = self
                        .select_completed_candidate(&bound.wallet, cost, through, &mut completed)
                        .await?
                    {
                        return Ok(CandidatePassResult::Selected(Box::new(selected)));
                    }
                    evaluated_since_selection = 0;
                }
            }
            // Independent publication during this batch must be considered
            // before waiting on potentially stalled acquisitions.
            if evaluated_since_selection > 0
                && !reads.is_empty()
                && let Some(selected) = self
                    .select_completed_candidate(&bound.wallet, cost, through, &mut completed)
                    .await?
            {
                return Ok(CandidatePassResult::Selected(Box::new(selected)));
            }
            #[cfg(test)]
            self.admission_test_stage(AdmissionTestStage::CandidateWaiting)
                .await;
            let completion = tokio::select! {
                result = reads.join_next() => result,
                _ = &mut changed, if !reads.is_empty() => continue,
            };
            if let Some(result) = completion {
                incomplete |= completion_is_incomplete(
                    result.context("candidate acquisition task failed")?,
                    &mut completed,
                )?;
                continue;
            }
            if exhausted && priority.is_empty() && page.is_empty() {
                // A final gated recheck may see independently completed evidence;
                // this does not reopen the captured pass or acquire another row.
                if let Some(selected) = self
                    .select_completed_candidate(&bound.wallet, cost, through, &mut completed)
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
        completed: &mut Vec<ChainView>,
    ) -> Result<Option<SelectedCandidate>> {
        #[cfg(test)]
        self.admission_test_stage(AdmissionTestStage::CandidateScan)
            .await;
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let (pool, bound) = (self.pool.clone(), bound.to_owned());
        debug_assert!(completed.len() <= PAGE_SIZE + READS);
        let completed = std::mem::take(completed);
        let selected = self
            .store
            .call_with_guard(guard.clone(), move |s| {
                s.payment_candidate_with_evidence(&pool, cost, Some(&bound), through, &completed)
            })
            .await?;
        Ok(selected.map(|(candidate, view)| SelectedCandidate {
            candidate,
            view,
            guard,
        }))
    }

    async fn acquire_candidate_wallet(
        &self,
        wallet: String,
    ) -> Result<(CandidateEvidenceState, Option<ChainView>)> {
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let (pool, id) = (self.pool.clone(), wallet.clone());
        let query = self
            .store
            .call_with_guard(guard, move |s| {
                s.scoped_chain_query(&pool, vec![id], vec![])
            })
            .await?;
        let view = self.acquire_view(query, true).await?;
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let pool = self.pool.clone();
        self.store
            .call_with_guard(guard, move |s| {
                let state = s.publish_candidate_evidence(&pool, &wallet, view)?;
                Ok((state, s.cached_payment_view(&pool, &wallet)?))
            })
            .await
    }
}

// Integrity rejection is not a new acquisition allowance or permission to shop
// around a rejected proof. Acquisition failures leave a negative pass incomplete.
fn completion_is_incomplete(
    result: Result<(CandidateEvidenceState, Option<ChainView>)>,
    completed: &mut Vec<ChainView>,
) -> Result<bool> {
    use crate::rotation::base::{EndpointMismatch, EvidenceConflict, evidence::EvidenceFailure};
    match result {
        Ok((state, view)) => {
            if let Some(view) = view {
                completed.push(view);
            }
            Ok(state == CandidateEvidenceState::Missing)
        }
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
