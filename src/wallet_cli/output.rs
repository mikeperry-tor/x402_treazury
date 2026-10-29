//! Human-readable summaries of saved observations; no wallet or network access.
#[cfg(test)]
mod tests;
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

fn saved_zec(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown (snapshot balance unavailable)".into(), zec)
}

fn usdc(value: &str) -> String {
    Some(value)
        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|value| alloy_primitives::U256::from_str_radix(value, 10).ok())
        .and_then(|value| alloy_primitives::utils::format_units(value, 6).ok())
        .map_or_else(|| "unknown".into(), |value| format!("{value} USDC"))
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

pub(super) fn status(
    status: &Status,
    balances: Option<&crate::treasury::SnapshotBalances>,
) -> String {
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
                "\nSaved ZEC observations (snapshot revision {})\nLast completed observation: {}\nSaved scanned height: {}",
                status.snapshot_revision,
                timestamp(sync.checked_at),
                height(sync.height)
            )
            .unwrap();
            for (label, amount) in [
                (
                    "Saved confirmed shielded balance (excluding reserved inputs)",
                    balances.map_or(Some(sync.confirmed_shielded_zatoshis), |b| {
                        b.confirmed_shielded
                    }),
                ),
                (
                    "Saved pending shielded incoming/change",
                    balances.and_then(|b| b.pending_shielded),
                ),
                (
                    "Saved spendable shielded balance",
                    balances.map_or(Some(sync.spendable_shielded_zatoshis), |b| {
                        b.spendable_shielded
                    }),
                ),
                (
                    "Saved confirmed transparent balance",
                    balances.and_then(|b| b.confirmed_transparent),
                ),
                (
                    "Saved pending transparent balance",
                    balances.and_then(|b| b.pending_transparent),
                ),
            ] {
                writeln!(text, "{label}: {}", saved_zec(amount)).unwrap();
            }
            writeln!(
                text,
                "These SDK balances exclude reserved inputs; shielded balances also exclude dust. Selected inputs can leave the confirmed balance before broadcast; incoming funds and change may remain pending.\nSpendable funds require sufficient confirmations and usable inputs. Transparent funds may need maturity and shielding. Available funds are checked again before spending."
            )
            .unwrap();
            if balances.is_none() {
                writeln!(text, "Wallet snapshot balances unavailable; confirmed and spendable shielded amounts above are from the last saved sync.").unwrap();
            }
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
    for operation in &status.treasury_operations {
        let source = match operation.submission.as_str() {
            "PREPARED" => "prepared",
            "BROADCAST_REQUESTED" => "broadcast requested; outcome not yet recorded",
            "BROADCAST" => "broadcast; awaiting confirmation",
            "UNKNOWN" => "broadcast outcome unknown",
            "CONFIRMED" => "confirmed",
            "EXPIRED" => "expired",
            _ => "unknown; inspect JSON status",
        };
        writeln!(
            text,
            "\nZEC source transaction {}: {} (fee: {})\n  Saved source status: {source}",
            operation.operation_id,
            zec(operation.facts.amount_zatoshis),
            zec(operation.facts.fee_zatoshis)
        )
        .unwrap();
    }
    for job in &status.funding_jobs {
        use crate::rotation::store::funding::FundingPhase;
        let progress = match job.phase {
            FundingPhase::Allocated => "awaiting quote",
            FundingPhase::Quoted => "quoted; awaiting deposit preparation",
            FundingPhase::Preparing => "preparing deposit",
            FundingPhase::Prepared => "deposit prepared",
            FundingPhase::DepositPending => "awaiting deposit confirmation",
            FundingPhase::Swapping => "awaiting settlement",
            FundingPhase::VerifyingCredit => "awaiting destination credit",
            FundingPhase::Complete => "complete",
            FundingPhase::RefundPending => "refund pending",
            FundingPhase::RecoveryRequired => "reconciling original work",
        };
        writeln!(
            text,
            "Saved swap progress for {} (operation {}): {progress}; target: {}",
            job.pool_name,
            job.operation_id,
            usdc(&job.target)
        )
        .unwrap();
        writeln!(
            text,
            "  Last validated NEAR response observed: {}",
            timestamp(job.provider_observed_at)
        )
        .unwrap();
        if job.timed_out {
            writeln!(text, "  Local wait threshold exceeded; reconciliation continues. This does not establish NEAR's current status or a refund deadline.").unwrap();
        }
        if let Some(error) = &job.last_error {
            writeln!(text, "  Saved funding diagnostic: {error}").unwrap();
        }
    }
    if !status.treasury_operations.is_empty() {
        writeln!(text, "Transaction amounts and pending change describe overlapping funds; do not add them to the balances above.").unwrap();
    }
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
        for claim in &pool.reserve_claims {
            if let Some(job) = &claim.blocked_funding_job_id {
                writeln!(text, "    {} reserve blocked by completed, uncredited funding job {job}; when automatic funding is enabled, replacement follows resolution of original source ownership", claim.slot).unwrap();
            }
        }
        for wallet in pool
            .addresses
            .iter()
            .filter(|wallet| wallet.role != "RETIRED")
        {
            let balance = usdc(&wallet.confirmed_balance);
            writeln!(text, "    {}: estimated balance: {balance}", wallet.address).unwrap();
        }
        let retired = pool
            .addresses
            .iter()
            .filter(|wallet| wallet.role == "RETIRED")
            .count();
        writeln!(text, "    Retired wallets: {retired}").unwrap();
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
