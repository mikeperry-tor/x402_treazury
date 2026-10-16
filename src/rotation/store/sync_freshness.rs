//! Runtime freshness belongs to the observed sync, never its serialized report.
use super::*;
use crate::freshness::{FreshnessLease, ObservationClock};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Capture immediately before requesting the starting chain tip. The caller
/// still owns verification of the scan and snapshot; this token measures only
/// their original freshness window and cannot be reconstructed from JSON.
#[derive(Clone, Debug)]
pub struct SyncObservationStart {
    origin: ObservationClock,
    checked_at: u64,
    max_age_seconds: u64,
    valid_until: u64,
    lease: FreshnessLease,
}

impl SyncObservationStart {
    pub fn capture(max_age_seconds: u64) -> Result<Self> {
        let origin = ObservationClock::capture()?;
        let checked_at = origin.wall_seconds();
        let valid_until = checked_at
            .checked_add(max_age_seconds)
            .context("sync freshness window overflow")?;
        Ok(Self {
            origin,
            checked_at,
            max_age_seconds,
            valid_until,
            lease: FreshnessLease::from_observation_window(
                origin,
                std::time::Duration::from_secs(max_age_seconds),
            ),
        })
    }

    pub fn checked_at(&self) -> u64 {
        self.checked_at
    }

    pub(crate) fn elapsed(&self) -> Result<std::time::Duration> {
        self.origin.elapsed()
    }

    pub(crate) fn fresh(&self, now: u64) -> Result<bool> {
        self.lease.is_fresh(now, self.valid_until)
    }

    #[cfg(test)]
    pub(crate) fn expire_for_test(&self) {
        self.lease
            .advance_for_test(std::time::Duration::from_secs(self.max_age_seconds + 1));
    }

    fn validate(&self, observation: &SyncObservation) -> Result<()> {
        ensure!(
            observation.phase == SyncPhase::Ready
                && observation.checked_at == Some(self.checked_at)
                && observation.max_age_seconds == self.max_age_seconds,
            "sync freshness observation binding mismatch"
        );
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct SyncCertificate {
    serialized: String,
    revision: i64,
    valid_until: u64,
    lease: FreshnessLease,
    revoked: Arc<AtomicBool>,
}

/// A persisted observation is always readable. Its optional runtime certificate
/// is required by spending, submission, reuse and expiry-recovery consumers.
pub(crate) struct RuntimeSyncObservation {
    observation: SyncObservation,
    certificate: Option<SyncCertificate>,
}

impl std::ops::Deref for RuntimeSyncObservation {
    type Target = SyncObservation;
    fn deref(&self) -> &Self::Target {
        &self.observation
    }
}

impl RuntimeSyncObservation {
    pub(crate) fn into_observation(self) -> SyncObservation {
        self.observation
    }

    pub(crate) fn fresh(&self, now: u64, revision: i64) -> Result<bool> {
        let Some(certificate) = &self.certificate else {
            return Ok(false);
        };
        if certificate.revoked.load(Ordering::Acquire)
            || certificate.revision != revision
            || self.observation.snapshot_revision != revision
            || serde_json::to_string(&self.observation)? != certificate.serialized
        {
            return Ok(false);
        }
        Ok(certificate.lease.is_fresh(now, certificate.valid_until)?
            && self.observation.wall_fresh_for_reporting(now, revision))
    }
}

impl Store {
    /// Persist a successful observed scan and retain its original runtime age.
    /// Plain `save_sync_snapshot` remains persistence-only, including on reopen.
    pub fn save_observed_sync_snapshot(
        &mut self,
        expected: i64,
        bytes: &[u8],
        observation: SyncObservation,
        start: SyncObservationStart,
    ) -> Result<i64> {
        self.save_observed_source_snapshot(expected, bytes, observation, start, None)
    }

    pub(crate) fn save_observed_source_snapshot(
        &mut self,
        expected: i64,
        bytes: &[u8],
        observation: SyncObservation,
        start: SyncObservationStart,
        source: Option<&source_support::Pass>,
    ) -> Result<i64> {
        start.validate(&observation)?;
        let revision = self.save_wallet_snapshot(
            expected,
            bytes,
            Some(observation),
            None,
            source.map(|pass| (pass, &start)),
        )?;
        // The writer sets the committed revision and checkpoint timestamp. Bind
        // those final bytes rather than a pre-transaction copy of the report.
        let serialized: String = self.db.query_row(
            "SELECT observation FROM treasury_sync WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let finalized: SyncObservation = serde_json::from_str(&serialized)?;
        start.validate(&finalized)?;
        ensure!(
            finalized.snapshot_revision == revision,
            "sync revision binding mismatch"
        );
        self.sync_certificate = Some(SyncCertificate {
            serialized,
            revision,
            valid_until: start.valid_until,
            lease: start.lease,
            revoked: Arc::new(AtomicBool::new(false)),
        });
        Ok(revision)
    }

    pub(super) fn revoke_sync_certificate(&mut self) {
        if let Some(certificate) = self.sync_certificate.take() {
            certificate.revoked.store(true, Ordering::Release);
        }
    }

    pub(super) fn runtime_sync_observation(
        &self,
        observation: SyncObservation,
    ) -> RuntimeSyncObservation {
        RuntimeSyncObservation {
            observation,
            certificate: self.sync_certificate.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(start: &SyncObservationStart) -> SyncObservation {
        SyncObservation {
            phase: SyncPhase::Ready,
            last_error: None,
            snapshot_revision: 1,
            checked_at: Some(start.checked_at()),
            checkpoint_at: 0,
            scanned_blocks: 1,
            target_height: Some(100),
            height: Some(100),
            observed_tip_height: Some(100),
            confirmations: 1,
            max_age_seconds: start.max_age_seconds,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 100,
            spendable_shielded_zatoshis: 100,
        }
    }

    fn fixture(dir: &Path) -> Store {
        Store::create(&dir.join("state"), &dir.join("key"), 1, b"wallet").unwrap()
    }

    #[test]
    fn persisted_ready_json_cannot_mint_runtime_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = fixture(dir.path());
        let start = SyncObservationStart::capture(300).unwrap();
        store
            .save_sync_snapshot(1, b"scanned", Some(observation(&start)))
            .unwrap();
        let state = store.treasury_sync_state().unwrap();
        assert!(
            !state
                .sync
                .unwrap()
                .fresh(start.checked_at(), state.snapshot_revision)
                .unwrap()
        );
    }

    #[test]
    fn exact_observation_lease_survives_clones_and_is_revoked_by_phase_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = fixture(dir.path());
        let start = SyncObservationStart::capture(300).unwrap();
        let checked = start.checked_at();
        let revision = store
            .save_observed_sync_snapshot(1, b"scanned", observation(&start), start)
            .unwrap();
        let held = store.treasury_sync_state().unwrap().sync.unwrap();
        assert!(held.fresh(checked, revision).unwrap());
        store.set_sync_phase(SyncPhase::Syncing).unwrap();
        assert!(!held.fresh(checked, revision).unwrap());
        store.set_sync_phase(SyncPhase::Ready).unwrap();
        assert!(
            !store
                .treasury_sync_state()
                .unwrap()
                .sync
                .unwrap()
                .fresh(checked, revision)
                .unwrap()
        );
    }

    #[test]
    fn forward_correction_expiration_is_sticky() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = fixture(dir.path());
        let start = SyncObservationStart::capture(300).unwrap();
        let checked = start.checked_at();
        let revision = store
            .save_observed_sync_snapshot(1, b"scanned", observation(&start), start)
            .unwrap();
        let held = store.treasury_sync_state().unwrap().sync.unwrap();
        assert!(!held.fresh(checked + 301, revision).unwrap());
        assert!(!held.fresh(checked, revision).unwrap());
    }

    #[test]
    fn suspend_elapsed_expires_despite_unchanged_wall_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = fixture(dir.path());
        let start = SyncObservationStart::capture(300).unwrap();
        let checked = start.checked_at();
        let lease = start.lease.clone();
        let revision = store
            .save_observed_sync_snapshot(1, b"scanned", observation(&start), start)
            .unwrap();
        let held = store.treasury_sync_state().unwrap().sync.unwrap();
        assert!(held.fresh(checked, revision).unwrap());
        lease.advance_for_test(std::time::Duration::from_secs(301));
        assert!(!held.fresh(checked, revision).unwrap());
    }

    #[test]
    fn snapshot_mutation_and_reopen_cannot_revive_observation_authority() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = fixture(dir.path());
        let start = SyncObservationStart::capture(300).unwrap();
        let checked = start.checked_at();
        let revision = store
            .save_observed_sync_snapshot(1, b"scanned", observation(&start), start)
            .unwrap();
        let held = store.treasury_sync_state().unwrap().sync.unwrap();
        store.save_snapshot(revision, b"mutated").unwrap();
        assert!(!held.fresh(checked, revision).unwrap());
        let start = SyncObservationStart::capture(300).unwrap();
        let revision = store
            .save_observed_sync_snapshot(revision + 1, b"rescanned", observation(&start), start)
            .unwrap();
        assert!(
            store
                .treasury_sync_state()
                .unwrap()
                .sync
                .unwrap()
                .fresh(crate::rotation::base::now().unwrap(), revision)
                .unwrap()
        );
        let id = store.id().to_owned();
        drop(store);
        let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        assert!(
            !store
                .treasury_sync_state()
                .unwrap()
                .sync
                .unwrap()
                .fresh(crate::rotation::base::now().unwrap(), revision)
                .unwrap()
        );
    }

    #[test]
    fn failed_commit_and_mismatched_origin_never_publish_proof() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = fixture(dir.path());
        let start = SyncObservationStart::capture(300).unwrap();
        let mut mismatched = observation(&start);
        mismatched.checked_at = Some(start.checked_at() - 1);
        assert!(
            store
                .save_observed_sync_snapshot(1, b"invalid", mismatched, start.clone())
                .is_err()
        );
        assert_eq!(store.snapshot().unwrap().0, 1);
        store.db.execute_batch("CREATE TRIGGER reject_snapshot BEFORE INSERT ON snapshots BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END").unwrap();
        assert!(
            store
                .save_observed_sync_snapshot(1, b"failed", observation(&start), start)
                .is_err()
        );
        assert!(store.sync_certificate.is_none());
        assert_eq!(store.snapshot().unwrap().0, 1);
    }

    #[test]
    fn late_checkpoint_keeps_scan_without_renewing_its_expired_origin() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = fixture(dir.path());
        let start = SyncObservationStart::capture(300).unwrap();
        let checked = start.checked_at();
        start
            .lease
            .advance_for_test(std::time::Duration::from_secs(301));
        let revision = store
            .save_observed_sync_snapshot(1, b"useful scan", observation(&start), start)
            .unwrap();
        assert_eq!(&*store.snapshot().unwrap().1, b"useful scan");
        assert!(
            !store
                .treasury_sync_state()
                .unwrap()
                .sync
                .unwrap()
                .fresh(checked, revision)
                .unwrap()
        );
        let next = SyncObservationStart::capture(300).unwrap();
        let checked = next.checked_at();
        let revision = store
            .save_observed_sync_snapshot(
                revision,
                b"incremental catch-up",
                observation(&next),
                next,
            )
            .unwrap();
        assert!(
            store
                .treasury_sync_state()
                .unwrap()
                .sync
                .unwrap()
                .fresh(checked, revision)
                .unwrap()
        );
    }
    #[test]
    fn prepared_snapshot_revokes_held_preflight_proof() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = fixture(dir.path());
        let start = SyncObservationStart::capture(300).unwrap();
        let checked = start.checked_at();
        let revision = store
            .save_observed_sync_snapshot(1, b"scanned", observation(&start), start)
            .unwrap();
        let held = store.treasury_sync_state().unwrap().sync.unwrap();
        assert!(held.fresh(checked, revision).unwrap());
        store.reserve("source", None, 1, 10).unwrap();
        store
            .prepare("source", revision, b"calculated", b"signed")
            .unwrap();
        assert!(!held.fresh(checked, revision).unwrap());
    }
}
