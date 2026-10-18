//! Advisory selection over existing, authorized wallet identities. No network I/O.
use super::capacity::payment_ownership;
use super::*;
use crate::rotation::manager::PaymentCandidateHandle;

impl Store {
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
        self.advisory_candidate(pool, cost, bound, through, &[])
    }

    pub(crate) fn advisory_candidate(
        &self,
        pool: &str,
        cost: U256,
        bound: Option<&str>,
        through: i64,
        attempted: &[String],
    ) -> Result<Option<PaymentCandidateHandle>> {
        let mut query = self.db.prepare("SELECT id,address,balance FROM wallets WHERE pool_id=?1 AND sequence<=?2 AND role IN ('ACTIVE','RETAINED','READY') ORDER BY CASE role WHEN 'ACTIVE' THEN 0 WHEN 'RETAINED' THEN 1 ELSE 2 END,sequence")?;
        let mut best = None;
        let mut best_rank = u8::MAX;
        let mut rows = query.query(params![pool, through])?;
        while let Some(row) = rows.next()? {
            let wallet: String = row.get(0)?;
            if attempted.contains(&wallet) {
                continue;
            }
            let Some(owner) = payment_ownership(&self.db, pool, &wallet)? else {
                continue;
            };
            if cost > owner.payment_limit {
                continue;
            }
            let estimate: String = row.get(2)?;
            let affordable = amount(&estimate)
                .ok()
                .is_some_and(|balance| balance >= cost);
            let rank = match (affordable, bound == Some(wallet.as_str())) {
                (true, true) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            };
            if rank < best_rank {
                best_rank = rank;
                best = Some(PaymentCandidateHandle {
                    wallet,
                    address: row.get(1)?,
                });
            }
        }
        Ok(best)
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
