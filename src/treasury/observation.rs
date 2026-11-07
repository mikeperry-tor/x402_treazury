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
    pub(super) fn complete(&mut self, success: bool, successful_interval: Duration) {
        self.failures = if success {
            0
        } else {
            self.failures.saturating_add(1)
        };
        let seconds = 90u64
            .saturating_mul(1 << self.failures.saturating_sub(1).min(3))
            .min(300);
        self.due = Some(
            Instant::now()
                + if success {
                    successful_interval
                } else {
                    Duration::from_secs(seconds)
                },
        );
    }
}

/// Use the interval ending at the next block, from the scan already completed.
pub(super) fn scan_interval(network: TreasuryNetwork, scanned_height: u64) -> Duration {
    use zcash_protocol::consensus::{BlockHeight, pow_target_spacing};
    let next = u32::try_from(scanned_height)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    pow_target_spacing(&network.chain(), BlockHeight::from_u32(next))
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
    async fn successful_scan_schedule_follows_next_block_across_nu7() {
        use zcash_protocol::consensus::{NetworkUpgrade, Parameters};
        let network = TreasuryNetwork::Mainnet;
        let activation = u64::from(u32::from(
            network
                .chain()
                .activation_height(NetworkUpgrade::Nu7)
                .unwrap(),
        ));
        let mut schedule = Schedule::default();
        for (height, seconds) in [(activation - 2, 75), (activation - 1, 25), (activation, 25)] {
            let interval = scan_interval(network, height);
            assert_eq!(interval, Duration::from_secs(seconds));
            schedule.complete(true, interval);
            assert_eq!(schedule.due() - Instant::now(), interval);
            assert!(!schedule.ready());
            tokio::time::advance(interval).await;
            assert!(schedule.ready());
        }
    }
    #[tokio::test]
    async fn frequent_source_observers_reuse_scan_without_tip_io_or_renewing_due() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = Treasury::create(
            directory.path().join("state"), directory.path().join("key"), 2_000_000,
            Some(Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into())),
        ).await.unwrap();
        owner
            .checkpoint(SyncObservation {
                phase: SyncPhase::Ready,
                last_error: None,
                snapshot_revision: owner.revision,
                started_snapshot_revision: None,
                checked_at: Some(1),
                checkpoint_at: 1,
                scanned_blocks: 0,
                target_height: Some(2_000_000),
                observed_tip_height: Some(2_000_000),
                height: Some(2_000_000),
                confirmations: 3,
                confirmed_pool_balances_zatoshis: None,
                confirmed_shielded_zatoshis: 0,
                spendable_shielded_zatoshis: 0,
            })
            .await
            .unwrap();
        owner.reusable_sync_revision = Some(owner.revision);
        tokio::time::pause();
        owner.observation.complete(true, Duration::from_secs(25));
        let due = owner.observation.due();
        let revision = owner.revision;
        let stop = CancellationToken::new();
        // No configured endpoint: either a new scan or a tip probe would fail.
        for _ in 0..5 {
            owner.sync_for_observation(&stop).await.unwrap();
            assert_eq!(owner.observation.due(), due);
            assert_eq!(owner.revision, revision);
            tokio::time::advance(Duration::from_secs(5)).await;
        }
        assert!(owner.sync_for_observation(&stop).await.is_err());
        assert_eq!(
            owner.observation.due() - Instant::now(),
            Duration::from_secs(90)
        );
        assert!(owner.status().await.unwrap().treasury_operations.is_empty());
        owner.close().await.unwrap();
    }
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
            owner.observation.complete(true, Duration::from_secs(75));
            assert!(owner.sync_once(&stop).await.is_err());
            assert_eq!(
                std::fs::read_to_string(logs.path())
                    .unwrap()
                    .lines()
                    .count(),
                2
            );
            owner.observation.complete(true, Duration::from_secs(75));
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
    async fn completion_uses_consensus_interval_and_independent_failure_backoff() {
        let mut schedule = Schedule::default();
        assert!(schedule.due() <= Instant::now());
        schedule.complete(false, Duration::ZERO);
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
            schedule.complete(false, Duration::ZERO);
        }
        schedule.complete(true, Duration::from_secs(25));
        assert_eq!(schedule.due() - Instant::now(), Duration::from_secs(25));
    }
}
