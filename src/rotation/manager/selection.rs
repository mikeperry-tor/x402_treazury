//! Selection uses saved hints only; signing rechecks actual authority in the journal.
use super::*;
impl ManagedPool {
    pub(super) async fn acquire_candidate(
        &self,
        bound: &PaymentCandidateHandle,
        cost: U256,
    ) -> Result<Option<PaymentCandidateHandle>> {
        let (pool, wallet) = (self.pool.clone(), bound.wallet.clone());
        self.store
            .call(move |s| s.cached_payment_candidate(&pool, cost, Some(&wallet)))
            .await
    }
}
