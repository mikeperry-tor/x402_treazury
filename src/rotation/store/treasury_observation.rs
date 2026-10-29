//! Observation demand derives from existing claims and transaction journals.
use super::*;

// Keep this lookup rooted in current slots, followed by the unique wallet/job
// binding. Historical allocations must not grow the serialized demand check.
// CROSS JOIN preserves that access order even as the funding journal grows.
const CURRENT_DEMAND: &str = "SELECT f.id,j.operation_id FROM pools p CROSS JOIN reserve_claims c ON c.pool_id=p.id CROSS JOIN funding_jobs f ON f.wallet_id=c.wallet_id AND f.pool_id=c.pool_id AND f.allocation_kind=c.slot AND f.claim_generation=c.generation CROSS JOIN funding_progress j ON j.job_id=f.id WHERE p.name=?1 AND p.enabled=1 AND c.enabled=1 AND f.state!='COMPLETE' AND f.readiness_requirement IS NULL";

impl StoreHandle {
    pub(crate) fn treasury_changes(&self) -> std::sync::Arc<tokio::sync::Notify> {
        self.treasury_changes.clone()
    }
}
impl Store {
    pub(crate) fn treasury_observation_needed(&self, configured: &[String]) -> Result<bool> {
        #[cfg(test)]
        self.inventory_read_fault(inventory::Read::TreasuryDemand)?;
        // A validated transaction can require confirmation/expiry evidence even
        // after destination credit, stale phases, or removed configuration.
        let accepted: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing x JOIN treasury_operations t ON t.id=x.id WHERE x.state='PREPARED')",
            [], |r| r.get(0),
        )?;
        if accepted {
            return Ok(true);
        }
        // Invalid published bytes keep their owner. Unpublished intent history
        // cannot hide an unrelated current claim after SDK reconstruction.
        if self.preparation_pending()?
            || self.funding_restriction != super::super::restriction::FundingRestriction::Ordinary
        {
            return Ok(false);
        }
        for name in configured {
            let mut query = self.db.prepare(CURRENT_DEMAND)?;
            for entry in query.query_map([name], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })? {
                let (id, operation) = entry?;
                match self.funding_facts(&id, &operation) {
                    Ok(facts) if facts.new_funding_allowed => return Ok(true),
                    Err(error) if error.is::<scheduling::FundingDecodeFailure>() => {
                        // The funding owner reports this row's scoped payload
                        // failure. It cannot hide independent valid claims.
                        self.scope_funding_decode_failure(&id, error)?;
                    }
                    Err(error) => return Err(error),
                    Ok(_) => {}
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        (dir, store)
    }
    fn allocate(store: &mut Store) -> funding::FundingJob {
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "pool",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        store.funding_jobs().unwrap().remove(0)
    }
    fn prepared(store: &mut Store, job: &funding::FundingJob, valid: bool) {
        store
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
            .unwrap();
        let revision = store.snapshot().unwrap().0;
        store
            .prepare_with_facts(
                &job.operation_id,
                revision,
                b"next",
                crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                valid.then(|| TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(100),
                    expiry_height: 100,
                    amount_zatoshis: 50,
                    fee_zatoshis: 10,
                    deadline: Some(10000),
                }),
            )
            .unwrap();
    }
    #[test]
    fn current_claims_need_configured_authority_but_destination_waiting_does_not_scan_zec() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        let groups = vec!["pool".to_owned()];
        assert!(store.treasury_observation_needed(&groups).unwrap());
        assert!(!store.treasury_observation_needed(&[]).unwrap());
        prepared(&mut store, &job, true);
        // Source submission consumes new-deposit eligibility, independently of
        // destination credit/provider observations still needed after confirmation.
        store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&job.operation_id],
            )
            .unwrap();
        store.confirm_spend(&job.operation_id).unwrap();
        let facts = store.funding_facts(&job.id, &job.operation_id).unwrap();
        assert!(facts.credit_needed() && !facts.new_funding_allowed);
        assert!(!store.treasury_observation_needed(&groups).unwrap());
    }
    #[test]
    fn completed_uncredited_claims_do_not_scan_but_signed_work_and_new_claims_do() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        let groups = vec!["pool".to_owned()];
        assert!(store.treasury_observation_needed(&groups).unwrap());
        // A historical completion can retain an uncredited current slot. It
        // must not request scans for work the funding inventory excludes.
        store
            .db
            .execute(
                "UPDATE funding_jobs SET state='COMPLETE' WHERE id=?1",
                [&job.id],
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE funding_progress SET phase='\"COMPLETE\"' WHERE job_id=?1",
                [&job.id],
            )
            .unwrap();
        assert!(!store.treasury_observation_needed(&groups).unwrap());
        assert!(
            store
                .funding_work_page("", store.funding_work_boundary().unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .initial_credit_recorded
        );
        assert!(store.prepared_bytes(&job.operation_id).is_err());

        // Completion labels cannot abandon validated signed liabilities.
        prepared(&mut store, &job, true);
        assert!(store.treasury_observation_needed(&[]).unwrap());
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
        store.confirm_spend(&job.operation_id).unwrap();
        assert!(!store.treasury_observation_needed(&groups).unwrap());

        // A replacement claim independently requests discovery again.
        store
            .db
            .execute_batch("UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1")
            .unwrap();
        assert!(
            store
                .allocate_capacity_kind(&job.pool_id, super::super::AllocationKind::Small, true)
                .unwrap()
                .is_some()
        );
        assert!(store.treasury_observation_needed(&groups).unwrap());
    }
    #[test]
    fn demand_reads_current_bindings_without_decoding_historical_jobs() {
        for transition in [
            "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1",
            "UPDATE reserve_claims SET generation=generation+1",
            "UPDATE reserve_claims SET enabled=0",
            "UPDATE reserve_claims SET slot='large'",
        ] {
            let (_dir, mut store) = fixture();
            let old = allocate(&mut store);
            store.db.execute_batch(transition).unwrap();
            store
                .db
                .execute(
                    "UPDATE wallets SET role='RETIRED' WHERE id=?1",
                    [&old.wallet_id],
                )
                .unwrap();
            // A decoder sentinel: even extracting the operation identity fails
            // if demand enumerates this obsolete allocation before filtering it.
            store
                .db
                .execute(
                    "UPDATE funding_progress SET operation_id=X'FF' WHERE job_id=?1",
                    [&old.id],
                )
                .unwrap();
            assert!(!store.treasury_observation_needed(&["pool".into()]).unwrap());

            // Restore a vacant enabled slot and bind a replacement allocation;
            // the obsolete journal must not hide the new claim's demand.
            store.db.execute_batch("UPDATE reserve_claims SET enabled=1,slot='small',wallet_id=NULL,generation=generation+1").unwrap();
            let replacement = store
                .allocate_capacity_kind(&old.pool_id, super::super::AllocationKind::Small, true)
                .unwrap();
            assert!(replacement.is_some());
            assert!(store.treasury_observation_needed(&["pool".into()]).unwrap());
        }
    }
    #[test]
    fn current_demand_uses_point_lookups_instead_of_funding_history() {
        let (_dir, mut store) = fixture();
        allocate(&mut store);
        let mut query = store
            .db
            .prepare(&format!("EXPLAIN QUERY PLAN {CURRENT_DEMAND}"))
            .unwrap();
        let plan = query
            .query_map(["pool"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for (table, key) in [
            ("p", "name=?"),
            ("c", "pool_id=?"),
            ("f", "wallet_id=?"),
            ("j", "job_id=?"),
        ] {
            assert!(plan.iter().any(|step| step.starts_with(&format!("SEARCH {table} ")) && step.contains(key)), "{plan:?}");
        }
    }
    #[test]
    fn validated_pending_work_survives_credit_stale_phase_and_removed_configuration() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        prepared(&mut store, &job, true);
        store
            .record_funding_balance(
                &job.id,
                &job.operation_id,
                &job.wallet_id,
                &job.recipient,
                U256::from(1),
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE funding_progress SET phase='\"COMPLETE\"' WHERE job_id=?1",
                [&job.id],
            )
            .unwrap();
        store.disable_pool("pool").unwrap();
        store.deny_new_funding();
        assert!(store.treasury_observation_needed(&[]).unwrap());
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
    }
    #[test]
    fn unpublished_intent_does_not_hide_claims_but_invalid_published_bytes_keep_their_fence() {
        for publish_invalid in [false, true] {
            let (_dir, mut store) = fixture();
            let job = allocate(&mut store);
            if publish_invalid {
                prepared(&mut store, &job, false);
            } else {
                store
                    .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                    .unwrap();
            }
            assert_eq!(
                store
                    .treasury_observation_needed(&["pool".to_owned()])
                    .unwrap(),
                !publish_invalid
            );
            assert!(
                store
                    .require_unstarted_preparation(&job.operation_id)
                    .is_err()
            );
            assert!(store.abandon_unprepared(&job.operation_id).is_err());
            assert_eq!(
                store.require_preparation_available().is_err(),
                publish_invalid
            );
        }
    }
    #[test]
    fn malformed_provider_row_cannot_hide_valid_siblings_or_force_discovery_alone() {
        let (_dir, mut store) = fixture();
        let malformed = allocate(&mut store);
        store
            .db
            .execute(
                "UPDATE funding_progress SET provider_success='invalid' WHERE job_id=?1",
                [&malformed.id],
            )
            .unwrap();
        assert!(
            store
                .treasury_observation_needed(&["pool".to_owned()])
                .unwrap()
        );
        // An unknown optional provider hint cannot suppress a current enabled
        // funding claim, or invent demand outside configured groups.
        assert!(!store.treasury_observation_needed(&[]).unwrap());
        assert!(
            !store
                .funding_facts(&malformed.id, &malformed.operation_id)
                .unwrap()
                .provider_success
        );
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "sibling",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        assert!(
            store
                .treasury_observation_needed(&["pool".to_owned(), "sibling".to_owned()])
                .unwrap()
        );
        // Valid signed ownership is observed independently of malformed provider flags.
        prepared(&mut store, &malformed, true);
        assert!(store.treasury_observation_needed(&[]).unwrap());
    }
    #[tokio::test]
    async fn capacity_notification_retains_a_wakeup_before_the_waiter_registers() {
        let (_dir, mut store) = fixture();
        let changed = store.treasury_changes.clone();
        allocate(&mut store);
        use futures_util::FutureExt;
        assert!(changed.notified().now_or_never().is_some());
        assert!(changed.notified().now_or_never().is_none());
    }
}
