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
        self.due = Some(
            Instant::now()
                + if success {
                    Duration::from_secs(60)
                } else {
                    Duration::from_secs(seconds)
                },
        );
    }
}

impl Treasury {
    pub(crate) fn configure_funding_discovery(&mut self, groups: Vec<String>) {
        self.funding_groups = groups;
    }
    /// Waiting source operations share the last actual scan. Cooldown failure
    /// returns unavailable; events cannot turn failed scans into a fast poller.
    pub(crate) async fn sync_for_observation(
        &mut self,
        stop: &CancellationToken,
        context: TreasuryObservationDemand,
    ) -> Result<()> {
        ensure!(!stop.is_cancelled(), "treasury sync cancelled");
        if self.observation.ready() {
            return self.sync_once_with_context(stop, context).await;
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
    async fn successful_scan_waits_one_minute_after_completion() {
        let mut schedule = Schedule::default();
        assert!(schedule.ready());
        schedule.complete(true);
        tokio::time::advance(Duration::from_secs(59)).await;
        assert!(!schedule.ready());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(schedule.ready());
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
        owner.observation.complete(true);
        let due = owner.observation.due();
        let revision = owner.revision;
        let stop = CancellationToken::new();
        // No configured endpoint: either a new scan or a tip probe would fail.
        let context = TreasuryObservationDemand::from_reason(TreasurySyncReason::RequestedSync);
        for _ in 0..12 {
            owner
                .sync_for_observation(&stop, context.clone())
                .await
                .unwrap();
            assert_eq!(owner.observation.due(), due);
            assert_eq!(owner.revision, revision);
            tokio::time::advance(Duration::from_secs(5)).await;
        }
        assert!(owner.sync_for_observation(&stop, context).await.is_err());
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
    async fn completion_uses_one_minute_interval_and_independent_failure_backoff() {
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
        assert_eq!(schedule.due() - Instant::now(), Duration::from_secs(60));
    }
    #[test]
    fn sync_progress_logs_emit_reason_and_optional_wallets() {
        let logs = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(logs.reopen().unwrap())
            .finish();
        let mut with_wallets = TreasuryObservationDemand::from_reason(
            TreasurySyncReason::WalletFundingInsufficientZec,
        );
        with_wallets.insert_reason(TreasurySyncReason::DirectReceive);
        with_wallets.insert_wallet("financial");
        let without_wallets =
            TreasuryObservationDemand::from_reason(TreasurySyncReason::StartupDiscovery);
        tracing::subscriber::with_default(subscriber, || {
            super::log_sync_progress(Some(3_614_778), &with_wallets, false);
            super::log_sync_progress(Some(3_614_781), &with_wallets, true);
            super::log_sync_progress(Some(3_614_782), &without_wallets, true);
        });
        let text = std::fs::read_to_string(logs.path()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        for (line, height) in [(lines[0], "height=3614778"), (lines[1], "height=3614781")] {
            assert!(line.contains("INFO"), "{line}");
            assert!(line.contains(height), "{line}");
            assert!(
                line.contains("reason=\"wallet_funding_insufficient_zec,direct_receive\""),
                "{line}"
            );
            assert!(line.contains("wallets=\"financial\""), "{line}");
        }
        assert!(lines[2].contains("height=3614782"), "{}", lines[2]);
        assert!(
            lines[2].contains("reason=\"startup_discovery\""),
            "{}",
            lines[2]
        );
        assert!(!lines[2].contains("wallets="), "{}", lines[2]);
    }
}
