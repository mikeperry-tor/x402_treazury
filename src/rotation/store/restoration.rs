//! Declared backup restoration cannot reconstruct missing off-chain authority.
//! This guard does not claim detection of an undeclared filesystem rollback.
use super::*;

pub(super) fn declared(db: &Connection) -> Result<bool> {
    Ok(
        db.query_row("SELECT EXISTS(SELECT 1 FROM restored_state)", [], |r| {
            r.get(0)
        })?,
    )
}

impl Store {
    /// Call offline, before using manually restored state. Exclusive Store
    /// ownership excludes in-process payment/treasury owners during declaration.
    /// There is deliberately no reset: scanning known chains cannot recover
    /// unknown still-valid authorizations or missing submission journals.
    pub fn declare_restored_backup(&mut self) -> Result<()> {
        self.db
            .execute("INSERT OR IGNORE INTO restored_state VALUES(1)", [])?;
        self.revoke_sync_certificate();
        Ok(())
    }

    pub(crate) fn require_complete_provenance(&self) -> Result<()> {
        ensure!(!declared(&self.db)?, AdmissionError::RestoredState);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied<T>(result: Result<T>) {
        assert_eq!(
            result.err().unwrap().downcast_ref::<AdmissionError>(),
            Some(&AdmissionError::RestoredState)
        );
    }

    #[test]
    fn declared_restoration_retains_known_bytes_and_blocks_new_authority_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"original snapshot").unwrap();
        let pool =
            test_support::declare_two_reserves(&mut store, "restore", "1", Some(U256::from(10)), 0)
                .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        let revision = store
            .save_refund_address(&job.id, "owned-refund", 1, b"snapshot with refund address")
            .unwrap();
        store.reserve("known", None, 1, 2).unwrap();
        store
            .prepare_with_facts(
                "known",
                revision,
                b"signed snapshot",
                b"exact saved bytes",
                Some(TransactionFacts {
                    txid: format!("{:064x}", 1),
                    expiry_height: 500,
                    amount_zatoshis: 1,
                    fee_zatoshis: 1,
                    deadline: Some(u64::MAX),
                }),
            )
            .unwrap();
        store.request_broadcast("known", 1, 1, false).unwrap();
        let backup = dir.path().join("old-backup");
        store.backup(&backup).unwrap();
        // The latest owner has later activity which is not in the saved copy.
        store.request_broadcast("known", 2, 1, true).unwrap();
        let id = store.id().to_owned();
        drop(store);
        let mut restored = Store::open(&backup, &backup.join("key"), &id).unwrap();
        assert_eq!(restored.operation("known").unwrap().attempts, 1);
        assert!(
            !restored
                .status()
                .unwrap()
                .restored_backup_missing_provenance,
            "an undeclared rollback is not detectable from the old database alone"
        );
        restored.declare_restored_backup().unwrap();
        restored.declare_restored_backup().unwrap();
        let group = restored
            .startup_groups(&["restore".into()])
            .unwrap()
            .remove(0);
        assert!(!group.serving_ready && !group.inventory_ready);
        let mut paths = restored.startup_claim_paths(&pool, &group.claims).unwrap();
        paths[0].destination_ready = true;
        assert_eq!(
            paths[0].disposition(3, true),
            bootstrap::StartupDisposition::Obstructed("restored_backup_missing_provenance")
        );
        assert_eq!(
            restored
                .startup_obstruction("restore", 3, Some(&group.claims))
                .unwrap()
                .unwrap()[0]
                .1,
            "restored_backup_missing_provenance"
        );

        denied(restored.request_broadcast("known", 3, 1, true));
        denied(restored.reserve("new", None, 1, 2));
        denied(restored.allocate_capacity_kind(&pool, AllocationKind::Small, true));
        denied(restored.reauthorize_closed_funding(&job.id, 3));
        denied(restored.refresh_unprepared_quote(&job.id));
        denied(restored.require_preparation_authority("direct-send", None));
        let revision = restored.snapshot().unwrap().0;
        denied(restored.begin_manual_shielding("shield", revision, &job.id, "owned-refund", 1));
        assert_eq!(restored.operation("known").unwrap().attempts, 1);
        assert_eq!(
            restored.prepared_bytes("known").unwrap().as_slice(),
            b"exact saved bytes"
        );
        assert_eq!(
            restored
                .db
                .query_row("SELECT COUNT(*) FROM treasury_preparations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            restored
                .db
                .query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(
            restored
                .db
                .execute("DELETE FROM restored_state", [])
                .is_err()
        );
        assert!(
            restored
                .db
                .execute("UPDATE restored_state SET singleton=1", [])
                .is_err()
        );
        // Observation/accounting still commits, without clearing missing history.
        restored.confirm_spend("known", 2, 1).unwrap();
        restored
            .save_snapshot(revision, b"observed snapshot")
            .unwrap();
        let copy = dir.path().join("marked-backup");
        restored.backup(&copy).unwrap();
        drop(restored);
        let mut reopened = Store::open(&copy, &copy.join("key"), &id).unwrap();
        assert!(
            reopened
                .status()
                .unwrap()
                .restored_backup_missing_provenance
        );
        assert_eq!(reopened.operation("known").unwrap().attempts, 1);
        assert_eq!(reopened.operation("known").unwrap().submission, "CONFIRMED");
        denied(reopened.reserve("after-observation", None, 1, 1));
        denied(reopened.require_funding_attempt(&job.id, 3));
        assert_eq!(reopened.funding_recovery_count(&job.id).unwrap(), 0);
    }
}
