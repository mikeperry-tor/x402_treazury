//! Selection uses saved hints only; signing rechecks actual authority in the journal.
use super::*;
impl ManagedPool {
    pub(super) async fn acquire_candidate(
        &self,
        bound: &PaymentCandidateHandle,
        cost: U256,
        through: i64,
        attempted: &[String],
    ) -> Result<Option<PaymentCandidateHandle>> {
        let (pool, wallet) = (self.pool.clone(), bound.wallet.clone());
        let attempted = attempted.to_vec();
        self.store
            .call(move |s| s.advisory_candidate(&pool, cost, Some(&wallet), through, &attempted))
            .await
    }
}
