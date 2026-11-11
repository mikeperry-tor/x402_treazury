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
    /// eligibility for the uniform opportunity cadence. Disabled groups retain work.
    pub(crate) fn bootstrap_allocation_groups(
        &self,
        participants: &[String],
    ) -> Result<std::collections::BTreeSet<String>> {
        #[cfg(test)]
        self.inventory_read_fault(inventory::Read::BootstrapEligibility)?;
        let mut eligible = std::collections::BTreeSet::new();
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
            if self.has_initial_credit(&pool)? || self.has_bootstrap_demand(&pool)? {
                eligible.insert(name.clone());
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
                .bootstrap_allocation_groups(&names)
                .unwrap()
                .is_empty()
        );
        assert!(store.activate_bootstrap_demand(&pool).unwrap());
        assert_eq!(
            store.bootstrap_allocation_groups(&names).unwrap(),
            ["participant".into()].into()
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
                .bootstrap_allocation_groups(&["renamed".into()])
                .unwrap(),
            ["renamed".into()].into()
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
        assert_eq!(
            store
                .bootstrap_allocation_groups(&["existing".into()])
                .unwrap(),
            ["existing".into()].into(),
            "historical credit admits a participant without first-use demand"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retired_participant_restart_keeps_destination_and_schedules_replacement() {
        use crate::{
            payment::{PaidClient, SpendPolicy},
            rotation::{
                base::BaseRpc,
                funding::{FundingBackend, FundingObservations, FundingWorker},
                manager::ManagedPool,
                near::{Quote, SwapStatus},
                store::funding::FundingJob,
            },
        };
        use std::{sync::Arc, time::Duration};
        use tokio::sync::mpsc;
        use tokio_util::sync::CancellationToken;

        // Keep the paused clock fixed while the blocking Store owner progresses.
        async fn local<T>(future: impl std::future::Future<Output = T>) -> T {
            tokio::pin!(future);
            loop {
                tokio::select! {
                    biased;
                    value = &mut future => return value,
                    _ = tokio::task::yield_now() => {},
                }
            }
        }

        #[derive(Clone)]
        struct Unfunded(mpsc::UnboundedSender<FundingJob>);
        impl FundingObservations for Unfunded {
            type QuoteRequest = ();
            async fn quote(&self, _: ()) -> Result<Quote> {
                panic!("unfunded fixture must not request quotes")
            }
            async fn status(&self, _: Quote) -> Result<SwapStatus> {
                panic!("unfunded fixture has no swaps to observe")
            }
            async fn credit(&self, _: FundingJob) -> Result<()> {
                Ok(())
            }
        }
        impl FundingBackend for Unfunded {
            type Observations = Self;
            fn observations(&self) -> Self {
                self.clone()
            }
            fn allocation_groups(&self) -> Vec<String> {
                ["bootstrap", "participant", "dormant"]
                    .map(str::to_owned)
                    .into()
            }
            async fn quote_prerequisite(&mut self, job: &FundingJob) -> Result<()> {
                self.0.send(job.clone()).unwrap();
                anyhow::bail!("fixture treasury is unfunded")
            }
            async fn prepare(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
                panic!("unfunded fixture must not prepare transactions")
            }
            async fn submit(&mut self, _: &FundingJob) -> Result<()> {
                panic!("unfunded fixture must not submit transactions")
            }
            async fn reconcile(&mut self, _: &FundingJob) -> Result<bool> {
                panic!("fixture has no accepted source transactions")
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("wallet");
        let key = temp.path().join("key");
        let mut state = Store::create(&dir, &key, 1, b"fixture").unwrap();
        let treasury = state.id().to_owned();
        let mut pools = Vec::new();
        for name in ["bootstrap", "participant", "dormant"] {
            pools.push(
                state
                    .ensure_capacity_group(
                        name,
                        &test_support::reserves("2"),
                        Some(U256::from(1_000_000)),
                        0,
                    )
                    .unwrap(),
            );
        }
        let bootstrap = pools[0].clone();
        let participant = pools[1].clone();
        state
            .allocate_capacity_kind(&bootstrap, AllocationKind::Small, true)
            .unwrap();
        let id = state
            .allocate_capacity_kind(&participant, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let historical = state.funding_job(&id).unwrap().unwrap();
        state
            .record_credit(&historical.wallet_id, "1", "saved", 2)
            .unwrap();
        assert!(
            state
                .finish_funding_if_proven(&id, &historical.operation_id)
                .unwrap()
        );
        // Saved post-retirement inventory, not a test of retirement eligibility:
        // the consumed reserve is vacant and the only old signer is retired.
        let tx = state.db.transaction().unwrap();
        tx.execute(
            "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1 WHERE wallet_id=?1",
            [&historical.wallet_id],
        )
        .unwrap();
        tx.execute(
            "UPDATE wallets SET role='RETIRED',balance='0' WHERE id=?1",
            [&historical.wallet_id],
        )
        .unwrap();
        tx.commit().unwrap();
        drop(state);

        let state = Store::open(&dir, &key, &treasury).unwrap();
        assert!(state.has_initial_credit(&participant).unwrap());
        assert!(!state.has_bootstrap_demand(&participant).unwrap());
        assert!(
            state
                .payment_identity_requires_creation(&participant)
                .unwrap()
        );
        assert_eq!(state.funding_jobs().unwrap().len(), 2);
        let (store, actor) = StoreHandle::spawn(state);
        let manager = |pool| {
            Arc::new(
                ManagedPool::new(
                    store.clone(),
                    pool,
                    BaseRpc::new("http://127.0.0.1:1").unwrap(),
                    SpendPolicy::dollars("1").unwrap(),
                )
                .unwrap(),
            )
        };
        let destination = manager(participant.clone());
        let client = PaidClient::managed(destination.clone()).with_bootstrap(manager(bootstrap));
        // A nonactivating capture cannot mask broken historical-credit discovery
        // by creating fresh first-use demand before the owner starts.
        let selected = local(client.capture_effective(false)).await.unwrap();
        assert!(Arc::ptr_eq(&selected.managed_pool().unwrap(), &destination));
        drop(selected);
        let (requested, mut requests) = mpsc::unbounded_channel();
        let worker = FundingWorker::new(store.clone(), Unfunded(requested), 5)
            .with_bootstrap_participants(["participant".into(), "dormant".into()].into());
        let stop = CancellationToken::new();
        let task = tokio::spawn(worker.run(stop.clone()));
        // Existing bootstrap work dispatches after startup eligibility discovery,
        // so the participant's opportunity is armed before advancing the clock.
        let bootstrap_job = local(requests.recv()).await.unwrap();
        assert_eq!(bootstrap_job.pool_name, "bootstrap");
        // Finish publishing that attempt before advancing time, so its ordinary
        // retry can independently witness a later scheduler iteration.
        local(async {
            loop {
                let id = bootstrap_job.id.clone();
                if store
                    .call(move |s| Ok(s.funding_job(&id)?.unwrap().next_poll > 0))
                    .await
                    .unwrap()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        local(store.call(|s| {
            assert_eq!(s.funding_jobs()?.len(), 2, "no eager replacement");
            Ok(())
        }))
        .await
        .unwrap();
        tokio::time::advance(Duration::from_secs(120)).await;
        // Either new work or the independent bootstrap retry follows due
        // opportunities. Assert inventory directly so broken participant
        // eligibility fails here instead of waiting forever for its callback.
        let progressed = local(requests.recv()).await.unwrap();
        assert!(matches!(
            progressed.pool_name.as_str(),
            "bootstrap" | "participant"
        ));
        let participant_id = participant.clone();
        let old_job = historical.id.clone();
        let replacement = local(store.call(move |s| {
            Ok(s.funding_jobs()?
                .into_iter()
                .find(|job| job.pool_id == participant_id && job.id != old_job))
        }))
        .await
        .unwrap()
        .expect("historically credited participant must receive a replacement");
        assert_ne!(replacement.id, historical.id);
        assert_ne!(replacement.wallet_id, historical.wallet_id);
        assert_eq!(replacement.pool_id, participant);
        let selected = local(client.capture_effective(false)).await.unwrap();
        assert!(Arc::ptr_eq(&selected.managed_pool().unwrap(), &destination));
        drop(selected);
        stop.cancel();
        local(task).await.unwrap().unwrap();
        local(store.call(move |s| {
            assert!(s.has_initial_credit(&participant)?);
            assert!(!s.has_bootstrap_demand(&participant)?);
            assert_eq!(s.funding_jobs()?.len(), 3, "dormant group stays unallocated");
            assert_eq!(s.funding_wallet_type(&replacement.id)?, "small");
            let old: (String, String, String) = s.db.query_row(
                "SELECT w.role,w.balance,f.readiness_requirement FROM wallets w JOIN funding_jobs f ON f.wallet_id=w.id WHERE w.id=?1", [&historical.wallet_id],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            )?;
            assert_eq!(old, ("RETIRED".into(), "0".into(), "1".into()));
            assert_eq!(s.db.query_row("SELECT count(*) FROM outgoing", [], |r| r.get::<_, i64>(0))?, 0);
            assert_eq!(s.db.query_row("SELECT count(*) FROM payment_attempts", [], |r| r.get::<_, i64>(0))?, 0);
            Ok(())
        })).await.unwrap();
        drop(client);
        drop(destination);
        drop(store);
        local(actor).await.unwrap();
    }
}
