//! Bounded sync observations for a treasury exclusively spent by this application.
use crate::rotation::store::SyncObservation;
use anyhow::{Result, ensure};
use std::time::Duration;

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
    ensure!(
        lag <= MAX_LAG_BLOCKS,
        "treasury sync tip lag exceeds {MAX_LAG_BLOCKS}-block limit (scanned={scanned}, tip={tip}, lag={lag}); another sync required"
    );
    ensure!(
        elapsed <= Duration::from_secs(max_age_seconds),
        "treasury sync observation exceeded max_sync_age_seconds={max_age_seconds}; another sync required"
    );
    Ok(lag)
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
                .to_string()
                .contains("4")
        );
        assert!(validate(100, 99, 101, Duration::ZERO, 300).is_err());
        assert!(validate(100, 101, 100, Duration::ZERO, 300).is_err());
        assert!(validate(100, 100, 100, Duration::from_secs(300), 300).is_ok());
        assert!(validate(100, 100, 100, Duration::from_millis(300_001), 300).is_err());
        assert!(validate(u64::MAX, u64::MAX, u64::MAX, Duration::ZERO, 300).is_ok());
    }
}
