//! Process-local measurement of an existing evidence freshness window.
//!
//! Elapsed stamps include system sleep and are never persisted or interpreted as
//! chain time. Expiration retires a lease shared by its certificate clones.
//! Platform contracts: macOS SDK clock_gettime(3), CLOCK_MONOTONIC_RAW;
//! Linux clock_gettime(3), CLOCK_BOOTTIME (https://man7.org/linux/man-pages/man3/clock_gettime.3.html).
use anyhow::{Context, Result};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ObservationClock {
    wall: Duration,
    elapsed: Duration,
}

impl ObservationClock {
    /// Capture before requesting the observation so read latency consumes its
    /// original freshness window even if wall time changes during the request.
    pub(crate) fn capture() -> Result<Self> {
        let elapsed = elapsed_now()?;
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("wall clock before Unix epoch")?;
        Ok(Self { wall, elapsed })
    }

    pub(crate) fn wall_seconds(self) -> u64 {
        self.wall.as_secs()
    }

    pub(crate) fn elapsed(self) -> Result<Duration> {
        elapsed_now()?
            .checked_sub(self.elapsed)
            .context("elapsed clock regressed")
    }

    fn deadline(self, valid_until: u64) -> Duration {
        // Existing wall checks accept the entire valid_until second.
        let end = Duration::from_secs(valid_until).saturating_add(Duration::from_secs(1));
        self.elapsed.saturating_add(end.saturating_sub(self.wall))
    }
}

#[derive(Debug)]
struct LeaseState {
    deadline: Duration,
    expired: bool,
    superseded: bool,
    #[cfg(test)]
    elapsed_override: Option<Duration>,
}

#[derive(Clone, Debug)]
pub(crate) struct FreshnessLease {
    origin: ObservationClock,
    state: Arc<Mutex<LeaseState>>,
}

impl FreshnessLease {
    pub(crate) fn from_observation(origin: ObservationClock, valid_until: u64) -> Self {
        Self {
            origin,
            state: Arc::new(Mutex::new(LeaseState {
                deadline: origin.deadline(valid_until),
                expired: false,
                superseded: false,
                #[cfg(test)]
                elapsed_override: None,
            })),
        }
    }

    pub(crate) fn from_observation_window(origin: ObservationClock, window: Duration) -> Self {
        let lease = Self::from_observation(origin, u64::MAX);
        // Sync's existing elapsed contract accepts equality at the exact window.
        lease.state.lock().expect("new lease").deadline = origin
            .elapsed
            .saturating_add(window)
            .saturating_add(Duration::from_nanos(1));
        lease
    }

    pub(crate) fn is_fresh(&self, wall_now: u64, valid_until: u64) -> Result<bool> {
        let elapsed = elapsed_now()?;
        #[cfg(test)]
        let elapsed = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("freshness lease unavailable"))?
            .elapsed_override
            .unwrap_or(elapsed);
        self.is_fresh_at(wall_now, valid_until, elapsed)
    }

    #[cfg(test)]
    pub(crate) fn advance_for_test(&self, elapsed: Duration) {
        self.state.lock().unwrap().elapsed_override =
            Some(self.origin.elapsed.saturating_add(elapsed));
    }

    fn is_fresh_at(&self, wall_now: u64, valid_until: u64, elapsed: Duration) -> Result<bool> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("freshness lease unavailable"))?;
        if state.superseded {
            return Ok(false);
        }
        state.expired |=
            wall_now > valid_until || elapsed >= state.deadline || elapsed < self.origin.elapsed;
        Ok(!state.expired)
    }

    pub(crate) fn is_superseded(&self) -> Result<bool> {
        Ok(self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("freshness lease unavailable"))?
            .superseded)
    }

    pub(crate) fn restrict_validity(&self, valid_until: u64) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("freshness lease unavailable"))?;
        state.deadline = state.deadline.min(self.origin.deadline(valid_until));
        Ok(())
    }

    /// Equal observations adopt the existing cache lease, preserving old clones
    /// and avoiding a growing chain of leases as observations are rechecked.
    /// Retire detached incoming clones: after cache eviction they must not regain
    /// the longer lifetime discarded by this merge. A later merge may reuse the
    /// saved lease, but supersession itself never expires that saved lease.
    pub(crate) fn retain_earlier(&mut self, saved: &Self) -> Result<()> {
        if Arc::ptr_eq(&self.state, &saved.state) {
            return Ok(());
        }
        let (deadline, expired) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("freshness lease unavailable"))?;
            let expired = state.expired;
            state.superseded = true;
            (state.deadline, expired)
        };
        {
            let mut state = saved
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("freshness lease unavailable"))?;
            state.deadline = state.deadline.min(deadline);
            state.expired |= expired;
        }
        *self = saved.clone();
        Ok(())
    }
}

fn elapsed_now() -> Result<Duration> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        #[cfg(target_os = "macos")]
        let clock = libc::CLOCK_MONOTONIC_RAW;
        #[cfg(target_os = "linux")]
        let clock = libc::CLOCK_BOOTTIME;
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: value is writable, correctly aligned timespec storage; these
        // clock IDs provide suspend-inclusive elapsed time on their platforms.
        let result = unsafe { libc::clock_gettime(clock, &mut value) };
        anyhow::ensure!(result == 0, "suspend-aware clock unavailable");
        anyhow::ensure!(
            value.tv_sec >= 0 && (0..1_000_000_000).contains(&value.tv_nsec),
            "invalid suspend-aware clock value"
        );
        Ok(Duration::new(value.tv_sec as u64, value.tv_nsec as u32))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    anyhow::bail!("suspend-aware evidence clock unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(wall: u64, elapsed: u64, valid_until: u64) -> FreshnessLease {
        FreshnessLease::from_observation(
            ObservationClock {
                wall: Duration::from_secs(wall),
                elapsed: Duration::from_secs(elapsed),
            },
            valid_until,
        )
    }

    #[test]
    fn backward_wall_and_suspend_do_not_renew_age() {
        let lease = lease(100, 500, 110);
        assert!(
            lease
                .is_fresh_at(101, 110, Duration::from_secs(510))
                .unwrap()
        );
        assert!(
            !lease
                .is_fresh_at(100, 110, Duration::from_secs(511))
                .unwrap()
        );
        assert!(
            !lease
                .is_fresh_at(100, 110, Duration::from_secs(512))
                .unwrap()
        );
    }

    #[test]
    fn forward_expiration_is_shared_and_sticky_after_correction() {
        let lease = lease(100, 500, 110);
        let clone = lease.clone();
        assert!(
            !clone
                .is_fresh_at(111, 110, Duration::from_secs(501))
                .unwrap()
        );
        assert!(
            !lease
                .is_fresh_at(101, 110, Duration::from_secs(502))
                .unwrap()
        );
    }

    #[test]
    fn equivalent_observations_share_original_expiration_without_chains() {
        let saved = lease(100, 500, 110);
        let mut incoming = lease(100, 505, 110);
        incoming.retain_earlier(&saved).unwrap();
        assert!(Arc::ptr_eq(&saved.state, &incoming.state));
        assert!(
            !incoming
                .is_fresh_at(105, 110, Duration::from_secs(511))
                .unwrap()
        );
        assert!(
            !saved
                .is_fresh_at(105, 110, Duration::from_secs(506))
                .unwrap()
        );
    }

    #[test]
    fn superseded_incoming_clones_stay_retired_without_expiring_saved_lease() {
        let saved = lease(100, 500, 110);
        let mut incoming = lease(100, 505, 110);
        let mut detached = incoming.clone();
        incoming.retain_earlier(&saved).unwrap();
        assert!(
            !detached
                .is_fresh_at(100, 110, Duration::from_secs(506))
                .unwrap()
        );
        assert!(
            saved
                .is_fresh_at(100, 110, Duration::from_secs(506))
                .unwrap()
        );
        detached.retain_earlier(&saved).unwrap();
        assert!(
            detached
                .is_fresh_at(100, 110, Duration::from_secs(506))
                .unwrap()
        );
        incoming.retain_earlier(&saved).unwrap();
        assert!(
            saved
                .is_fresh_at(100, 110, Duration::from_secs(506))
                .unwrap()
        );
    }

    #[test]
    fn genuinely_expired_superseded_clone_retains_expiration_on_later_merge() {
        let first = lease(100, 500, 110);
        let mut incoming = lease(100, 505, 110);
        let mut detached = incoming.clone();
        assert!(
            !incoming
                .is_fresh_at(111, 110, Duration::from_secs(506))
                .unwrap()
        );
        incoming.retain_earlier(&first).unwrap();
        let replacement = lease(100, 506, 110);
        detached.retain_earlier(&replacement).unwrap();
        assert!(
            !replacement
                .is_fresh_at(100, 110, Duration::from_secs(507))
                .unwrap()
        );
    }

    #[test]
    fn shorter_validity_keeps_origin_and_elapsed_regression_retires_lease() {
        let lease = lease(100, 500, 120);
        lease.restrict_validity(105).unwrap();
        assert!(
            !lease
                .is_fresh_at(101, 120, Duration::from_secs(506))
                .unwrap()
        );
        let other = FreshnessLease::from_observation(lease.origin, 120);
        assert!(
            !other
                .is_fresh_at(100, 120, Duration::from_secs(499))
                .unwrap()
        );
    }
}
