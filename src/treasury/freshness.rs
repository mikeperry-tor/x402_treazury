//! Bounded sync observations for a treasury exclusively spent by this application.
use crate::rotation::store::SyncObservation;
use anyhow::{Result, ensure};
use std::time::Duration;

#[derive(Debug)]
pub(super) struct FreshObservationRequired;
impl std::fmt::Display for FreshObservationRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "treasury scan retained; acquiring a fresh observation through incremental catch-up",
        )
    }
}
impl std::error::Error for FreshObservationRequired {}

pub(super) const MAX_LAG_BLOCKS: u64 = 3;

pub(super) fn validate(
    target: u64,
    scanned: u64,
    tip: u64,
    elapsed: Duration,
    max_age_seconds: u64,
) -> Result<u64> {
    ensure!(
        scanned >= target,
        "treasury sync did not reach observed starting tip (scanned={scanned}, target={target})"
    );
    let lag = tip.checked_sub(scanned).ok_or_else(|| anyhow::anyhow!(
        "treasury indexer tip regressed below scanned height (scanned={scanned}, tip={tip}); another sync required"
    ))?;
    if lag > MAX_LAG_BLOCKS {
        tracing::warn!(
            target_height = target,
            scanned_height = scanned,
            observed_tip_height = tip,
            lag_blocks = lag,
            max_lag_blocks = MAX_LAG_BLOCKS,
            elapsed_ms = elapsed.as_millis() as u64,
            max_age_seconds,
            "Treasury scan completed beyond allowed tip lag; freshness evidence rejected"
        );
    }
    if lag > MAX_LAG_BLOCKS {
        return Err(FreshObservationRequired.into());
    }
    if elapsed > Duration::from_secs(max_age_seconds) {
        return Err(FreshObservationRequired.into());
    }
    Ok(lag)
}

/// Ordinary spending shares sync's bounded lag; a new observation may not regress.
pub(super) fn spending_ready(
    status: &crate::rotation::store::Status,
    tip: u64,
    now: u64,
) -> Result<()> {
    let sync = status
        .sync
        .as_ref()
        .filter(|s| s.fresh(now, status.snapshot_revision))
        .ok_or_else(|| anyhow::anyhow!("treasury requires a fresh sync before calculation"))?;
    let height = sync
        .height
        .ok_or_else(|| anyhow::anyhow!("missing scanned height"))?;
    ensure!(
        sync.observed_tip_height
            .is_some_and(|observed| tip >= observed),
        "preparation tip regressed"
    );
    ensure!(
        tip.checked_sub(height)
            .is_some_and(|lag| lag <= MAX_LAG_BLOCKS),
        "treasury requires catch-up before calculation"
    );
    Ok(())
}

// Non-inclusion evidence must not inherit ordinary funding's lag allowance.
pub(super) fn require_current_tip(sync: &SyncObservation) -> Result<()> {
    ensure!(
        sync.at_observed_tip(),
        "expiry recovery requires zero observed tip lag; synchronize again before releasing reservations"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_tip_bounds_and_age_do_not_require_a_quiet_chain() {
        for lag in 0..=MAX_LAG_BLOCKS {
            assert_eq!(
                validate(100, 101, 101 + lag, Duration::from_secs(61), 300).unwrap(),
                lag
            );
        }
        assert!(
            validate(100, 101, 105, Duration::ZERO, 300)
                .unwrap_err()
                .is::<FreshObservationRequired>()
        );
        assert!(validate(100, 99, 101, Duration::ZERO, 300).is_err());
        assert!(validate(100, 101, 100, Duration::ZERO, 300).is_err());
        assert!(validate(100, 100, 100, Duration::from_secs(300), 300).is_ok());
        assert!(validate(100, 100, 100, Duration::from_millis(300_001), 300).is_err());
        assert!(validate(u64::MAX, u64::MAX, u64::MAX, Duration::ZERO, 300).is_ok());
    }
}
