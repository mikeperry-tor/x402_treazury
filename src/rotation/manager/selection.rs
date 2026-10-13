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

/// Fixed priority identities plus one monotonic page bound traversal memory.
struct CandidateTraversal {
    through: i64,
    priority_ids: Vec<String>,
    priority: VecDeque<String>,
    page: VecDeque<(i64, String, String)>,
    cursor: i64,
    exhausted: bool,
}
impl CandidateTraversal {
    fn new(through: i64, priority: Vec<String>) -> Self {
        Self {
            through,
            priority_ids: priority.clone(),
            priority: priority.into(),
            page: VecDeque::new(),
            cursor: -1,
            exhausted: false,
        }
    }
    async fn next(&mut self, owner: &ManagedPool) -> Result<Option<String>> {
        if let Some(wallet) = self.priority.pop_front() {
            return Ok(Some(wallet));
        }
        loop {
            if let Some((sequence, wallet, _)) = self.page.pop_front() {
                self.cursor = sequence;
                if !self.priority_ids.contains(&wallet) {
                    return Ok(Some(wallet));
                }
                continue;
            }
            if self.exhausted {
                return Ok(None);
            }
            let pool = owner.pool.clone();
            let (cursor, through) = (self.cursor, self.through);
            let rows = owner
                .store
                .call(move |s| s.wallet_page(&pool, cursor, through, PAGE_SIZE))
                .await?;
            if rows.is_empty() {
                self.exhausted = true;
                return Ok(None);
            }
            self.page = rows.into();
        }
    }
    fn is_exhausted(&self) -> bool {
        self.exhausted && self.priority.is_empty() && self.page.is_empty()
    }
}

/// Retain only this traversal batch plus the read lanes. Gated selection
/// consumes completed proofs; local failures remain available at exhaustion.
#[derive(Default)]
struct CandidateCompletions {
    completed: Vec<ChainView>,
    incomplete: bool,
    wallet_failure: Option<anyhow::Error>,
}
impl CandidateCompletions {
    fn record(
        &mut self,
        result: Result<(CandidateEvidenceState, Option<ChainView>)>,
    ) -> Result<()> {
        self.incomplete |=
            completion_is_incomplete(result, &mut self.completed, &mut self.wallet_failure)?;
        Ok(())
    }
    fn drain_ready(
        &mut self,
        reads: &mut tokio::task::JoinSet<Result<(CandidateEvidenceState, Option<ChainView>)>>,
    ) -> Result<bool> {
        let mut read_completed = false;
        while let Some(result) = reads.try_join_next() {
            read_completed = true;
            self.record(result.context("candidate acquisition task failed")?)?;
        }
        Ok(read_completed)
    }
    fn finish(self) -> Result<CandidatePassResult> {
        if let Some(error) = self.wallet_failure {
            return Err(error);
        }
        Ok(if self.incomplete {
            CandidatePassResult::Incomplete
        } else {
            CandidatePassResult::CompleteNegative
        })
    }
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
        let mut traversal = CandidateTraversal::new(through, priority);
        let mut completions = CandidateCompletions::default();
        let mut reads =
            tokio::task::JoinSet::<Result<(CandidateEvidenceState, Option<ChainView>)>>::new();

        loop {
            // Register before inspecting evidence to avoid a publication between
            // the last gated scan and the wait being lost. Wake all group waiters.
            let changed = changes.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            completions.drain_ready(&mut reads)?;
            // This is the actual gated selection, not a provisional winner.
            // Recompute all completed cached candidates, liabilities, and roles.
            if let Some(selected) = self
                .select_completed_candidate(
                    &bound.wallet,
                    cost,
                    through,
                    &mut completions.completed,
                )
                .await?
            {
                // `reads` is dropped now, without waiting for unrelated EOF.
                return Ok(CandidatePassResult::Selected(Box::new(selected)));
            }
            let mut evaluated_since_selection = 0;
            while reads.len() < READS {
                let Some(wallet) = traversal.next(self).await? else {
                    break;
                };
                self.evaluate_candidate(wallet, &mut completions, &mut reads)
                    .await?;
                #[cfg(test)]
                self.admission_test_stage(AdmissionTestStage::CandidateEvaluated)
                    .await;
                // Poll independently ready acquisitions while walking cached or
                // ineligible rows, so a long inventory cannot hide a completion.
                let read_completed = completions.drain_ready(&mut reads)?;
                evaluated_since_selection += 1;
                // Recheck immediately after our reads complete. Otherwise batch
                // the full cache scan at the existing inventory page size: this
                // also notices independent evidence/liability changes without
                // rescanning the same cache for every unchanged inventory row.
                if read_completed || evaluated_since_selection == PAGE_SIZE {
                    if let Some(selected) = self
                        .select_completed_candidate(
                            &bound.wallet,
                            cost,
                            through,
                            &mut completions.completed,
                        )
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
                    .select_completed_candidate(
                        &bound.wallet,
                        cost,
                        through,
                        &mut completions.completed,
                    )
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
                completions.record(result.context("candidate acquisition task failed")?)?;
                continue;
            }
            if traversal.is_exhausted() {
                // A final gated recheck may see independently completed evidence;
                // this does not reopen the captured pass or acquire another row.
                if let Some(selected) = self
                    .select_completed_candidate(
                        &bound.wallet,
                        cost,
                        through,
                        &mut completions.completed,
                    )
                    .await?
                {
                    return Ok(CandidatePassResult::Selected(Box::new(selected)));
                }
                return completions.finish();
            }
        }
    }

    async fn evaluate_candidate(
        &self,
        wallet: String,
        completions: &mut CandidateCompletions,
        reads: &mut tokio::task::JoinSet<Result<(CandidateEvidenceState, Option<ChainView>)>>,
    ) -> Result<()> {
        // A cached insufficient balance still completes an evaluation;
        // never reread it merely to seek a more favorable answer.
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let (pool, id) = (self.pool.clone(), wallet.clone());
        let state = self
            .store
            .call_with_guard(guard, move |s| s.candidate_evidence(&pool, &id))
            .await;
        let state = match state {
            Ok(state) => state,
            Err(error)
                if error.downcast_ref::<super::super::base::evidence::EvidenceFailure>()
                    == Some(&super::super::base::evidence::EvidenceFailure::MalformedSupport) =>
            {
                completions.record(Err(error))?;
                (CandidateEvidenceState::Ineligible, None)
            }
            // Store-wide failures are not failed unsigned acquisitions.
            Err(error) => return Err(error),
        };
        if let Some(view) = state.1 {
            completions.completed.push(view);
        }
        let state = state.0;
        if state == CandidateEvidenceState::Missing {
            let owner = self.clone();
            let context = crate::qualification::capture_case();
            reads.spawn(crate::qualification::inherit_case(context, async move {
                let result = owner.acquire_candidate_wallet(wallet.clone()).await;
                #[cfg(test)]
                owner
                    .admission_test_stage(AdmissionTestStage::CandidateReadCompleted { wallet })
                    .await;
                result
            }));
        }
        Ok(())
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
                s.publish_candidate_evidence(&pool, &wallet, view)
            })
            .await
    }
}

// Wallet-local rejection preserves its diagnostic without vetoing independently
// verified wallets. It grants no new acquisition or fallback for the failed proof.
// Endpoint trust failures still stop the pass after shared-store invalidation.
fn completion_is_incomplete(
    result: Result<(CandidateEvidenceState, Option<ChainView>)>,
    completed: &mut Vec<ChainView>,
    wallet_failure: &mut Option<anyhow::Error>,
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
            if error.is::<EndpointMismatch>() {
                return Err(error);
            }
            if error.is::<EvidenceConflict>()
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
                tracing::warn!(category = %super::super::base::safe_diagnostic(&error), "candidate wallet evidence rejected; checking independent capacity");
                wallet_failure.get_or_insert(error);
            }
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests;
