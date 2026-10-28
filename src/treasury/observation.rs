//! Serialized treasury scan scheduling; no balance or financial authority.
use super::*;
use tokio::time::Instant;

#[derive(Default)]
pub(super) struct Schedule {
    due: Option<Instant>,
    failures: u32,
}
impl Schedule {
    pub(super) fn due(&self) -> Instant {
        self.due.unwrap_or_else(Instant::now)
    }
    pub(super) fn ready(&self) -> bool {
        self.due.is_none_or(|due| Instant::now() >= due)
    }
    pub(super) fn failed(&self) -> bool {
        self.failures > 0
    }
    pub(super) fn complete(&mut self, success: bool) {
        self.failures = if success {
            0
        } else {
            self.failures.saturating_add(1)
        };
        let seconds = 90u64
            .saturating_mul(1 << self.failures.saturating_sub(1).min(3))
            .min(300);
        self.due = Some(Instant::now() + Duration::from_secs(seconds));
    }
}
impl Treasury {
    pub(crate) fn configure_funding_discovery(&mut self, groups: Vec<String>) {
        self.funding_groups = groups;
    }
    /// Waiting source operations share the last actual scan. Cooldown failure
    /// returns unavailable; events cannot turn failed scans into a fast poller.
    pub(crate) async fn sync_for_observation(&mut self, stop: &CancellationToken) -> Result<()> {
        ensure!(!stop.is_cancelled(), "treasury sync cancelled");
        if self.observation.ready() {
            return self.sync_once(stop).await;
        }
        ensure!(
            self.reusable_observation().await?.is_some(),
            "treasury observation unavailable"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn completion_sets_ninety_second_floor_and_independent_failure_backoff() {
        let mut schedule = Schedule::default();
        assert!(schedule.due() <= Instant::now());
        schedule.complete(false);
        for seconds in [90, 180, 300, 300, 300] {
            assert_eq!(
                schedule.due() - Instant::now(),
                Duration::from_secs(seconds)
            );
            let due = schedule.due();
            tokio::time::advance(Duration::from_secs(5)).await;
            // Reading due, wakeups and unrelated completed work never reset it.
            assert_eq!(schedule.due(), due);
            tokio::time::advance(Duration::from_secs(seconds - 5)).await;
            assert_eq!(schedule.due(), Instant::now());
            schedule.complete(false);
        }
        schedule.complete(true);
        assert_eq!(schedule.due() - Instant::now(), Duration::from_secs(90));
    }
}
