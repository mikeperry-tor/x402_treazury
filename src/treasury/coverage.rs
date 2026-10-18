//! Exact scan coverage for expiry/non-inclusion, independent of observation age.
use crate::rotation::store::SyncObservation;
use anyhow::{Result, ensure};

// Non-inclusion evidence must not inherit ordinary funding's lag allowance.
pub(super) fn require_current_tip(sync: &SyncObservation) -> Result<()> {
    ensure!(
        sync.at_observed_tip(),
        crate::rotation::transaction::RecoveryEvidenceError::TipLag
    );
    Ok(())
}
