//! Advisory selection over existing, authorized wallet identities. No network I/O.
use super::capacity::payment_ownership;
use super::*;
use crate::rotation::manager::PaymentCandidateHandle;

impl Store {
    /// Startup diagnostic only; never gates payment or requests an observation.
    pub(crate) fn payment_estimate_covers_limit(&self, pool: &str) -> Result<bool> {
        let mut query = self.db.prepare("SELECT id,address,balance FROM wallets WHERE pool_id=?1 AND role IN ('ACTIVE','RETAINED','READY') AND low_balance=0")?;
        let mut rows = query.query([pool])?;
        while let Some(row) = rows.next()? {
            let (Ok(wallet), Ok(address), Ok(balance)) = (
                row.get_ref(0)?.as_str(),
                row.get_ref(1)?.as_str(),
                row.get_ref(2)?.as_str(),
            ) else {
                continue;
            };
            if address.parse::<alloy_primitives::Address>().is_err() {
                continue;
            }
            if let Some(owner) = payment_ownership(&self.db, pool, wallet)?
                && amount(balance).is_ok_and(|balance| balance >= owner.payment_limit)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn advisory_candidate(
        &self,
        pool: &str,
        selection_amount: Option<U256>,
        bound: Option<&str>,
        through: i64,
        attempted: &[String],
    ) -> Result<Option<PaymentCandidateHandle>> {
        let attempted: std::collections::BTreeSet<&str> =
            attempted.iter().map(String::as_str).collect();
        let mut fallback: Option<(u8, PaymentCandidateHandle)> = None;
        let mut inspect = |wallet: &str,
                           is_bound: bool|
         -> Result<Option<PaymentCandidateHandle>> {
            if attempted.contains(wallet) {
                return Ok(None);
            }
            if payment_ownership(&self.db, pool, wallet)?.is_none() {
                return Ok(None);
            }
            // This amount may be a historical price hint. It ranks estimates; only the
            // current challenge and journal boundary enforce payment amount caps.
            let row:Option<(Option<String>,Option<String>,bool)>=self.db.query_row(
                "SELECT address,balance,low_balance FROM wallets WHERE pool_id=?1 AND id=?2 AND sequence<=?3",
                params![pool,wallet,through],|r|Ok((r.get_ref(0)?.as_str().ok().map(str::to_owned),r.get_ref(1)?.as_str().ok().map(str::to_owned),r.get(2)?))).optional()?;
            let Some((Some(address), estimate, low)) = row else {
                return Ok(None);
            };
            if address.parse::<alloy_primitives::Address>().is_err() {
                tracing::warn!(
                    category = "malformed_advisory_wallet_address",
                    "wallet identity unavailable; advancing to independent payment identity"
                );
                return Ok(None);
            }
            let candidate = PaymentCandidateHandle {
                wallet: wallet.to_owned(),
                address,
            };
            let affordable = !low
                && estimate
                    .as_deref()
                    .and_then(|v| amount(v).ok())
                    .is_some_and(|balance| match selection_amount {
                        Some(amount) => balance >= amount,
                        None => balance > U256::ZERO,
                    });
            if affordable {
                return Ok(Some(candidate));
            }
            let rank = u8::from(!is_bound) + 2 * u8::from(low);
            if fallback.as_ref().is_none_or(|(old, _)| rank < *old) {
                fallback = Some((rank, candidate));
            }
            Ok(None)
        };
        // A suitable bound payer requires only indexed identity reads, never a
        // whole-inventory sort or proof pass. Role queries use wallet_role_sequence.
        if let Some(wallet) = bound
            && let Some(candidate) = inspect(wallet, true)?
        {
            return Ok(Some(candidate));
        }
        let roles = if selection_amount.is_some() {
            ["RETAINED", "ACTIVE", "READY"]
        } else {
            ["ACTIVE", "RETAINED", "READY"]
        };
        for role in roles {
            let mut query=self.db.prepare("SELECT id FROM wallets WHERE pool_id=?1 AND role=?2 AND sequence<=?3 ORDER BY sequence")?;
            let mut rows = query.query(params![pool, role, through])?;
            while let Some(row) = rows.next()? {
                let Some(wallet) = row.get_ref(0)?.as_str().ok() else {
                    continue;
                };
                if bound == Some(wallet) {
                    continue;
                }
                if let Some(candidate) = inspect(wallet, false)? {
                    return Ok(Some(candidate));
                }
            }
        }
        Ok(fallback.map(|(_, candidate)| candidate))
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
        self.treasury_changes.notify_one();
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

#[cfg(test)]
mod startup_funding_tests {
    use super::*;
    #[test]
    fn startup_requires_one_eligible_estimate_covering_the_cap_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "pool",
            crate::rotation::test_capacity::two_reserves("2"),
            Some(U256::from(1_000_000)),
            0
        )
        .unwrap();
        let jobs = store.funding_jobs().unwrap();
        for job in &jobs {
            store
                .record_funding_balance(
                    &job.id,
                    &job.operation_id,
                    &job.wallet_id,
                    &job.recipient,
                    U256::from(600_000),
                )
                .unwrap();
        }
        assert!(
            !store.payment_estimate_covers_limit(&pool).unwrap(),
            "separate payer balances cannot be summed"
        );
        store
            .db
            .execute(
                "UPDATE wallets SET balance='1000000' WHERE id=?1",
                [&jobs[0].wallet_id],
            )
            .unwrap();
        assert!(store.payment_estimate_covers_limit(&pool).unwrap());
        for mutation in [
            "balance='999999'",
            "balance='unknown'",
            "balance=X'FF'",
            "low_balance=1",
            "address='invalid'",
            "role='RETIRED'",
            "role='ALLOCATED'",
        ] {
            store
                .db
                .execute(
                    &format!("UPDATE wallets SET {mutation} WHERE id=?1"),
                    [&jobs[0].wallet_id],
                )
                .unwrap();
            assert!(
                !store.payment_estimate_covers_limit(&pool).unwrap(),
                "{mutation}"
            );
            store.db.execute("UPDATE wallets SET balance='1000000',low_balance=0,address=?2,role='READY' WHERE id=?1", params![jobs[0].wallet_id,jobs[0].recipient]).unwrap();
        }
        store
            .db
            .execute("UPDATE reserve_claims SET enabled=0", [])
            .unwrap();
        assert!(!store.payment_estimate_covers_limit(&pool).unwrap());
        store
            .db
            .execute(
                "UPDATE wallets SET role='ACTIVE' WHERE id=?1",
                [&jobs[0].wallet_id],
            )
            .unwrap();
        assert!(
            store.payment_estimate_covers_limit(&pool).unwrap(),
            "active payer needs no reserve inventory"
        );
        store.disable_pool("pool").unwrap();
        assert!(!store.payment_estimate_covers_limit(&pool).unwrap());
        assert!(store.status().unwrap().treasury_operations.is_empty());
        assert_eq!(store.funding_jobs().unwrap().len(), 2);
    }
}

#[cfg(test)]
mod ranking_tests {
    use super::*;

    #[test]
    fn optional_hints_rank_roles_without_excluding_advisory_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "ranking",
            crate::rotation::test_capacity::two_reserves("2"),
            Some(U256::from(1_000_000)),
            0
        )
        .unwrap();
        for job in store.funding_jobs().unwrap() {
            store
                .record_credit(&job.wallet_id, "100", "fixture", 100)
                .unwrap();
        }
        let wallets: Vec<String> = store
            .db
            .prepare("SELECT id FROM wallets ORDER BY sequence")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let (older, newer) = (&wallets[0], &wallets[1]);
        let newer_address: String = store
            .db
            .query_row("SELECT address FROM wallets WHERE id=?1", [newer], |r| {
                r.get(0)
            })
            .unwrap();
        store
            .db
            .execute("UPDATE wallets SET role='RETAINED' WHERE id=?1", [older])
            .unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET role='ACTIVE',balance='1000' WHERE id=?1",
                [newer],
            )
            .unwrap();
        // Author an ordinary consumed-claim state for ACTIVE/RETAINED variants.
        store
            .db
            .execute(
                "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1",
                [],
            )
            .unwrap();
        let generation: i64 = store
            .db
            .query_row("SELECT max(generation) FROM reserve_claims", [], |r| {
                r.get(0)
            })
            .unwrap();
        let choose = |hint, bound, through, attempted: &[String]| {
            store
                .advisory_candidate(&pool, hint, bound, through, attempted)
                .unwrap()
                .map(|c| c.wallet)
        };
        assert_eq!(
            choose(Some(U256::from(50)), None, i64::MAX, &[]).as_ref(),
            Some(older)
        );
        assert_eq!(choose(None, None, i64::MAX, &[]).as_ref(), Some(newer));
        assert_eq!(
            choose(Some(U256::from(500)), None, i64::MAX, &[]).as_ref(),
            Some(newer)
        );
        // Actual-offer selection keeps an already suitable payer even while a
        // retained sibling could serve it. An unsuitable bound can hand off.
        assert_eq!(
            choose(Some(U256::from(50)), Some(newer.as_str()), i64::MAX, &[]).as_ref(),
            Some(newer)
        );
        assert_eq!(
            choose(Some(U256::from(500)), Some(older.as_str()), i64::MAX, &[]).as_ref(),
            Some(newer)
        );
        assert_eq!(
            choose(
                Some(U256::from(50)),
                None,
                i64::MAX,
                std::slice::from_ref(older)
            )
            .as_ref(),
            Some(newer)
        );
        assert!(choose(None, None, i64::MAX, &wallets).is_none());
        assert!(choose(None, None, -1, &[]).is_none());
        for mutation in [
            "balance='0'",
            "balance='unknown'",
            "balance=X'FF'",
            "low_balance=1",
            "address='invalid'",
        ] {
            store
                .db
                .execute(
                    &format!("UPDATE wallets SET {mutation} WHERE id=?1"),
                    [newer],
                )
                .unwrap();
            assert_eq!(
                choose(None, None, i64::MAX, &[]).as_ref(),
                Some(older),
                "{mutation}"
            );
            store
                .db
                .execute(
                    "UPDATE wallets SET balance='1000',low_balance=0,address=?2 WHERE id=?1",
                    params![newer, newer_address],
                )
                .unwrap();
        }
        // Present zero and absent price differ. A known zero retained estimate
        // covers zero; without a price, healthy positive ACTIVE is preferred.
        store
            .db
            .execute("UPDATE wallets SET balance='0' WHERE id=?1", [older])
            .unwrap();
        assert_eq!(
            choose(Some(U256::ZERO), None, i64::MAX, &[]).as_ref(),
            Some(older)
        );
        assert_eq!(choose(None, None, i64::MAX, &[]).as_ref(), Some(newer));
        // READY stays usable when ACTIVE has no healthy estimate; it needs no
        // invented minimum balance or active-role gate.
        store
            .db
            .execute(
                "UPDATE wallets SET role='READY',balance='100' WHERE id=?1",
                [older],
            )
            .unwrap();
        store
            .db
            .execute("UPDATE wallets SET balance='0' WHERE id=?1", [newer])
            .unwrap();
        // This is a separate authored READY variant, with its original exact
        // job binding, rather than a runtime reactivation of a consumed claim.
        store.db.execute("UPDATE reserve_claims SET wallet_id=?1,generation=(SELECT claim_generation FROM funding_jobs WHERE wallet_id=?1) WHERE pool_id=?2 AND slot=(SELECT allocation_kind FROM funding_jobs WHERE wallet_id=?1)", params![older,pool]).unwrap();
        assert_eq!(choose(None, None, i64::MAX, &[]).as_ref(), Some(older));
        store
            .db
            .execute("UPDATE wallets SET role='RETAINED',balance='100'", [])
            .unwrap();
        store
            .db
            .execute(
                "UPDATE reserve_claims SET wallet_id=NULL,generation=?1",
                [generation],
            )
            .unwrap();
        assert_eq!(
            choose(Some(U256::from(50)), None, i64::MAX, &[]).as_ref(),
            Some(older),
            "oldest retained wins independent of amount sorting"
        );
        store
            .db
            .execute("UPDATE wallets SET balance='unknown',low_balance=1", [])
            .unwrap();
        assert_eq!(
            choose(Some(U256::from(5000)), None, i64::MAX, &[]).as_ref(),
            Some(older),
            "low and unknown remain eligible"
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM payment_attempts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(store.funding_jobs().unwrap().len(), 2);
        assert_eq!(
            store
                .db
                .query_row("SELECT max(generation) FROM reserve_claims", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            generation
        );
    }
}
