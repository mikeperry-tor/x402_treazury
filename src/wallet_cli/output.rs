//! Human-readable summaries of saved observations; no wallet or network access.
use crate::rotation::store::{Status, SyncPhase};
use std::fmt::Write;

fn zec(zatoshis: u64) -> String {
    format!(
        "{}.{:08} ZEC",
        zatoshis / 100_000_000,
        zatoshis % 100_000_000
    )
}

fn height(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".into(), |v| v.to_string())
}

fn timestamp(value: Option<u64>) -> String {
    value
        .and_then(|v| i64::try_from(v).ok())
        .and_then(|v| chrono::DateTime::from_timestamp(v, 0))
        .map_or_else(
            || "unknown".into(),
            |v| v.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        )
}

pub(super) fn status(status: &Status) -> String {
    let mut text = format!(
        "Zcash wallet status\nTreasury ID: {}\nBirthday block: {}\nWallet snapshot revision: {}\n",
        status.treasury_id, status.birthday, status.snapshot_revision
    );
    if let Some(sync) = &status.sync {
        let phase = match sync.phase {
            SyncPhase::Syncing => "syncing",
            SyncPhase::Preparing => "preparing",
            SyncPhase::Ready => "ready",
            SyncPhase::Failed => "failed",
            SyncPhase::Offline => "offline",
        };
        writeln!(text, "\nSync state: {phase}").unwrap();
        writeln!(
            text,
            "Scan complete for current snapshot: {}",
            if status.sync_complete { "yes" } else { "no" }
        )
        .unwrap();
        writeln!(text, "Scanned height: {}", height(sync.height)).unwrap();
        writeln!(
            text,
            "Observed chain tip: {}",
            height(sync.observed_tip_height)
        )
        .unwrap();
        writeln!(text, "Scan target: {}", height(sync.target_height)).unwrap();
        writeln!(text, "Blocks scanned: {}", sync.scanned_blocks).unwrap();
        writeln!(text, "Observation time: {}", timestamp(sync.checked_at)).unwrap();
        writeln!(
            text,
            "Checkpoint time: {}",
            timestamp(Some(sync.checkpoint_at))
        )
        .unwrap();
        writeln!(text, "Required confirmations: {}", sync.confirmations).unwrap();
        if sync.checked_at.is_some() && sync.height.is_some() {
            writeln!(
                text,
                "\nSaved confirmed shielded balance: {}",
                zec(sync.confirmed_shielded_zatoshis)
            )
            .unwrap();
            writeln!(
                text,
                "Saved spendable shielded balance: {}",
                zec(sync.spendable_shielded_zatoshis)
            )
            .unwrap();
            writeln!(
                text,
                "Balances reflect the last saved sync; available funds are checked again before spending."
            )
            .unwrap();
        } else {
            writeln!(
                text,
                "\nShielded balances: unknown (no completed sync observation)"
            )
            .unwrap();
        }
        if let Some(error) = &sync.last_error {
            writeln!(text, "Last sync error: {error}").unwrap();
        }
    } else {
        writeln!(
            text,
            "\nSync state: not yet synced\nShielded balances: unknown"
        )
        .unwrap();
    }
    writeln!(
        text,
        "\nOutgoing transaction pending: {}",
        if status.outgoing_pending { "yes" } else { "no" }
    )
    .unwrap();
    writeln!(text, "Managed wallet pools: {}", status.pools.len()).unwrap();
    for pool in &status.pools {
        writeln!(
            text,
            "  {}: enabled={}, serving ready={}, inventory ready={}, funding degraded={}",
            pool.name,
            pool.enabled,
            pool.serving_ready,
            pool.inventory_ready,
            pool.funding_degraded
        )
        .unwrap();
        writeln!(
            text,
            "    Wallets: {} retained, {} retired; reserve claims: {}",
            pool.retained_wallets,
            pool.retired_wallets,
            pool.reserve_claims.len()
        )
        .unwrap();
    }
    writeln!(
        text,
        "Recorded treasury operations: {}\nRecorded funding jobs: {}",
        status.treasury_operations.len(),
        status.funding_jobs.len()
    )
    .unwrap();
    text
}

#[cfg(test)]
#[test]
fn formats_zec_without_rounding_and_preserves_unknown_values() {
    assert_eq!(zec(1), "0.00000001 ZEC");
    assert_eq!(zec(123_456_789), "1.23456789 ZEC");
    assert_eq!(zec(u64::MAX), "184467440737.09551615 ZEC");
    assert_eq!(height(None), "unknown");
    assert_eq!(timestamp(None), "unknown");
    assert_eq!(timestamp(Some(0)), "1970-01-01 00:00:00 UTC");
}

#[cfg(feature = "zcash")]
pub(super) fn bootstrap(summary: &crate::deployment::BootstrapSummary) -> String {
    let mut text =
        String::from("Wallet bootstrap completed.\n\nAdvisory wallet balance estimates (USDC):\n");
    for balance in &summary.balance_estimates {
        writeln!(
            text,
            "  {} ({}): {} USDC",
            balance.wallet_name, balance.wallet_type, balance.balance_estimate_usdc
        )
        .unwrap();
    }
    if summary.balance_estimates.is_empty() {
        text.push_str("  No enabled small/large wallets were captured.\n");
    }
    text
}
