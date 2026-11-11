//! Durable first-use demand; consumed initial claims own the separate credit fact.
use super::*;

pub(super) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS bootstrap_demand(pool_id TEXT PRIMARY KEY REFERENCES pools(id)); CREATE INDEX IF NOT EXISTS funding_initial_credit ON funding_jobs(pool_id) WHERE readiness_requirement IS NOT NULL;")?;
    Ok(())
}

impl Store {
    /// The serialized Store command remains accepted if its waiter disconnects.
    /// Recording demand never allocates, changes group authority or signs.
    pub(crate) fn activate_bootstrap_demand(&mut self, pool_id: &str) -> Result<bool> {
        ensure!(
            self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM pools WHERE id=?1)",
                [pool_id],
                |r| r.get::<_, bool>(0)
            )?,
            "bootstrap demand group does not exist"
        );
        let inserted = self.db.execute(
            "INSERT OR IGNORE INTO bootstrap_demand(pool_id) VALUES (?1)",
            [pool_id],
        )? != 0;
        if inserted {
            self.funding_changes.notify_one();
            self.capacity_changes(pool_id).notify_waiters();
        }
        Ok(inserted)
    }

    pub(crate) fn has_bootstrap_demand(&self, pool_id: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM bootstrap_demand WHERE pool_id=?1)",
            [pool_id],
            |r| r.get(0),
        )?)
    }

    /// Includes retired wallets: depletion and later role changes cannot undo credit.
    pub(crate) fn has_initial_credit(&self, pool_id: &str) -> Result<bool> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM funding_jobs WHERE pool_id=?1 AND readiness_requirement IS NOT NULL)", [pool_id], |r| r.get(0))?)
    }

    /// Current configuration selects participation; durable facts only determine
    /// eligibility and the initial opportunity cadence. Disabled groups retain work.
    pub(crate) fn bootstrap_allocation_intervals(
        &self,
        participants: &[String],
    ) -> Result<std::collections::BTreeMap<String, bool>> {
        #[cfg(test)]
        self.inventory_read_fault(inventory::Read::BootstrapEligibility)?;
        let mut eligible = std::collections::BTreeMap::new();
        for name in participants {
            let pool: Option<String> = self
                .db
                .query_row(
                    "SELECT id FROM pools WHERE name=?1 AND enabled=1",
                    [name],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(pool) = pool else { continue };
            let initial = !self.has_initial_credit(&pool)?;
            if !initial || self.has_bootstrap_demand(&pool)? {
                eligible.insert(name.clone(), initial);
            }
        }
        Ok(eligible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concurrent_first_use_coalesces_and_cancelled_waiter_keeps_demand() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &temp.path().join("wallet"),
            &temp.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = store
            .ensure_capacity_group(
                "participant",
                &test_support::reserves("2"),
                Some(U256::from(1)),
                0,
            )
            .unwrap();
        let other = store
            .ensure_capacity_group(
                "cancelled",
                &test_support::reserves("2"),
                Some(U256::from(1)),
                0,
            )
            .unwrap();
        let changes = store.capacity_changes(&pool);
        let changed = changes.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let (store, worker) = StoreHandle::spawn(store);
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let store = store.clone();
            let pool = pool.clone();
            calls.spawn(async move {
                store
                    .call(move |s| s.activate_bootstrap_demand(&pool))
                    .await
                    .unwrap()
            });
        }
        let mut insertions = 0;
        while let Some(result) = calls.join_next().await {
            insertions += usize::from(result.unwrap());
        }
        assert_eq!(insertions, 1);
        assert!(futures_util::poll!(&mut changed).is_ready());
        let repeated = changes.notified();
        tokio::pin!(repeated);
        repeated.as_mut().enable();
        let already_active = pool.clone();
        assert!(
            !store
                .call(move |s| s.activate_bootstrap_demand(&already_active))
                .await
                .unwrap()
        );
        assert!(futures_util::poll!(&mut repeated).is_pending());
        let (entered, accepted) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let caller = store.clone();
        let cancelled_pool = other.clone();
        let waiter = tokio::spawn(async move {
            caller
                .call(move |s| {
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                    s.activate_bootstrap_demand(&cancelled_pool)
                })
                .await
        });
        accepted.await.unwrap();
        waiter.abort();
        release.send(()).unwrap();
        assert!(waiter.await.unwrap_err().is_cancelled());
        store
            .call(move |s| {
                assert!(s.has_bootstrap_demand(&other)?);
                assert!(s.funding_jobs()?.is_empty());
                Ok(())
            })
            .await
            .unwrap();
        drop(store);
        worker.await.unwrap();
    }

    #[test]
    fn bootstrap_demand_preserves_uuid_history_and_never_allocates() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("wallet");
        let key = temp.path().join("key");
        let mut store = Store::create(&dir, &key, 1, b"fixture").unwrap();
        let treasury = store.id().to_owned();
        let pool = store
            .ensure_capacity_group(
                "participant",
                &test_support::reserves("2"),
                Some(U256::from(1)),
                0,
            )
            .unwrap();
        assert!(!store.has_bootstrap_demand(&pool).unwrap());
        assert!(!store.has_initial_credit(&pool).unwrap());
        let names = vec!["participant".to_owned()];
        assert!(
            store
                .bootstrap_allocation_intervals(&names)
                .unwrap()
                .is_empty()
        );
        assert!(store.activate_bootstrap_demand(&pool).unwrap());
        assert_eq!(
            store.bootstrap_allocation_intervals(&names).unwrap(),
            [("participant".into(), true)].into()
        );
        assert!(!store.activate_bootstrap_demand(&pool).unwrap());
        assert!(store.activate_bootstrap_demand("missing").is_err());
        assert!(store.funding_jobs().unwrap().is_empty());
        assert_eq!(store.status().unwrap().pools[0].addresses.len(), 0);
        store
            .rename_capacity_group("participant", "renamed")
            .unwrap();
        store.disable_pool("renamed").unwrap();
        drop(store);
        let mut store = Store::open(&dir, &key, &treasury).unwrap();
        assert!(store.has_bootstrap_demand(&pool).unwrap());
        assert!(!store.has_initial_credit(&pool).unwrap());
        store
            .ensure_capacity_group(
                "renamed",
                &test_support::reserves("2"),
                Some(U256::from(1)),
                0,
            )
            .unwrap();
        store
            .allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        store
            .record_credit(&job.wallet_id, "1", "saved", 2)
            .unwrap();
        assert!(store.has_initial_credit(&pool).unwrap());
        assert_eq!(
            store
                .bootstrap_allocation_intervals(&["renamed".into()])
                .unwrap(),
            [("renamed".into(), false)].into()
        );
        store
            .db
            .execute(
                "UPDATE wallets SET role='RETIRED', balance='0' WHERE id=?1",
                [&job.wallet_id],
            )
            .unwrap();
        store.disable_pool("renamed").unwrap();
        assert!(store.has_initial_credit(&pool).unwrap());
        let backup = temp.path().join("backup");
        store.backup(&backup).unwrap();
        drop(store);
        let store = Store::open(&backup, &backup.join("key"), &treasury).unwrap();
        assert!(store.has_bootstrap_demand(&pool).unwrap());
        assert!(store.has_initial_credit(&pool).unwrap());
    }

    #[test]
    fn old_current_schema_additively_initializes_empty_demand() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("wallet");
        let key = temp.path().join("key");
        let mut store = Store::create(&dir, &key, 1, b"fixture").unwrap();
        let treasury = store.id().to_owned();
        let pool =
            test_support::declare_two_reserves(&mut store, "existing", "2", Some(U256::from(1)), 0)
                .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        store
            .record_credit(&job.wallet_id, "1", "saved", 2)
            .unwrap();
        store
            .db
            .execute_batch("DROP TABLE bootstrap_demand; DROP INDEX funding_initial_credit;")
            .unwrap();
        drop(store);
        let store = Store::open(&dir, &key, &treasury).unwrap();
        assert_eq!(store.status().unwrap().state_schema_version, 23);
        assert_eq!(store.funding_jobs().unwrap().len(), 2);
        assert!(!store.has_bootstrap_demand(&pool).unwrap());
        assert!(store.has_initial_credit(&pool).unwrap());
    }
}
