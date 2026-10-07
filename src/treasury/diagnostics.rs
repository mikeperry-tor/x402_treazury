//! Fixed diagnostics only: never emit upstream error prose or endpoint credentials.
pub(crate) fn sync_failure(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        let message = cause.to_string();
        let category = match message.as_str() {
            "indexer connection failed" => "indexer_connection_failed",
            "indexer connection timed out" => "indexer_connection_timeout",
            "indexer tip check timed out" => "indexer_tip_timeout",
            "indexer tip check failed" => "indexer_tip_failed",
            "indexer is not on mainnet" => "indexer_network_mismatch",
            "indexer capability connection timed out" => "indexer_capability_connection_timeout",
            "indexer capability connection failed" => "indexer_capability_connection_failed",
            "indexer Ironwood capability check timed out" => "indexer_capability_timeout",
            "indexer Ironwood capability check failed" => "indexer_capability_failed",
            "indexer returned the wrong Ironwood tree height" => "indexer_tree_height_mismatch",
            "indexer missing Ironwood support" => "indexer_missing_ironwood",
            "treasury sync launch failed" => "sync_launch_failed",
            "treasury sync failed" => "sync_scan_failed",
            "treasury sync worker failed" => "sync_worker_failed",
            "treasury sync interrupted; reopen required" => "sync_reopen_required",
            "treasury balance unavailable" | "treasury spendable balance unavailable" => {
                "balance_unavailable"
            }
            _ if message.starts_with("treasury sync did not reach observed starting tip (") => {
                "sync_target_not_reached"
            }
            _ if message.starts_with("treasury indexer tip regressed below scanned height (") => {
                "indexer_tip_regressed"
            }
            _ if message.starts_with("treasury sync tip lag exceeds ") => "sync_tip_lag_exceeded",
            _ if message
                .starts_with("treasury sync observation exceeded max_sync_age_seconds=") =>
            {
                "sync_observation_expired"
            }
            _ if message.starts_with("treasury_confirmed_spend_reorg:") => "confirmed_spend_reorg",
            _ if message.starts_with("treasury_expiry_reorg:") => "expiry_reorg",
            _ => continue,
        };
        return category;
    }
    "sync_internal_or_storage_failure"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_fixed_and_find_nested_causes() {
        let error = anyhow::anyhow!("indexer tip check failed").context("private endpoint detail");
        assert_eq!(sync_failure(&error), "indexer_tip_failed");
        let error = anyhow::anyhow!("https://user:secret@example.invalid/token");
        assert_eq!(sync_failure(&error), "sync_internal_or_storage_failure");
        let error =
            crate::treasury::freshness::validate(100, 100, 104, std::time::Duration::ZERO, 300)
                .unwrap_err();
        assert_eq!(sync_failure(&error), "sync_tip_lag_exceeded");
        let error = crate::treasury::freshness::validate(
            100,
            100,
            100,
            std::time::Duration::from_secs(301),
            300,
        )
        .unwrap_err();
        assert_eq!(sync_failure(&error), "sync_observation_expired");
    }
}
