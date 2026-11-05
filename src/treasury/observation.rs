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

    #[tokio::test]
    async fn sync_failure_transition_is_visible_once_until_success_without_changing_backoff() {
        if crate::test_log_capture::run_in_child(
            "treasury::observation::tests::sync_failure_transition_is_visible_once_until_success_without_changing_backoff",
        ) {
            return;
        }
        use tracing::instrument::WithSubscriber;
        let directory = tempfile::tempdir().unwrap();
        let mut owner = Treasury::create(
            directory.path().join("state"), directory.path().join("key"), 2_000_000,
            Some(Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into())),
        ).await.unwrap();
        let logs = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(logs.reopen().unwrap())
            .finish();
        let stop = CancellationToken::new();
        tokio::time::pause();
        async {
            // No configured endpoint: a real refused scan, without network I/O.
            for seconds in [90, 180] {
                assert!(owner.sync_once(&stop).await.is_err());
                assert_eq!(
                    owner.observation.due() - Instant::now(),
                    Duration::from_secs(seconds)
                );
            }
            assert_eq!(
                std::fs::read_to_string(logs.path())
                    .unwrap()
                    .lines()
                    .count(),
                1
            );
            // Existing successful-completion state resets the same schedule.
            owner.observation.complete(true);
            assert!(owner.sync_once(&stop).await.is_err());
            assert_eq!(
                std::fs::read_to_string(logs.path())
                    .unwrap()
                    .lines()
                    .count(),
                2
            );
            owner.observation.complete(true);
            stop.cancel();
            assert!(owner.sync_once(&stop).await.is_err());
        }
        .with_subscriber(subscriber)
        .await;
        let text = std::fs::read_to_string(logs.path()).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        for line in text.lines() {
            assert!(
                line.contains("INFO") && line.contains("treasury_sync_unavailable"),
                "{line}"
            );
            assert!(line.contains("accepted work remains owned"), "{line}");
            assert!(
                !line.contains("retry") && !line.contains("WARN") && !line.contains("ERROR"),
                "{line}"
            );
        }
        assert!(owner.status().await.unwrap().treasury_operations.is_empty());
        owner.close().await.unwrap();
    }
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
