//! Fixed diagnostics only: never emit upstream error prose or endpoint credentials.

use pepper_sync::error::{
    ContinuityError, MempoolError, ScanError, ServerError, SyncError, SyncModeError,
};
use zingolib::{lightclient::error::LightClientError, wallet::error::WalletError};

/// Sanitize at the library boundary, before logging OR persisting last_error.
/// Deliberately carries no upstream source, message, metadata, or wallet identifiers.
#[derive(Debug)]
struct SyncDiagnostic {
    category: &'static str,
    reason: &'static str,
    grpc_code: Option<tonic::Code>,
}
impl std::fmt::Display for SyncDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: reason={}", self.category, self.reason)?;
        if let Some(code) = self.grpc_code {
            write!(f, " grpc_code={} grpc_status={code:?}", code as i32)?;
        }
        Ok(())
    }
}
impl std::error::Error for SyncDiagnostic {}

pub(crate) fn client_failure(error: LightClientError, launch: bool) -> anyhow::Error {
    let mut diagnostic = SyncDiagnostic {
        category: if launch {
            "sync_launch_failed"
        } else {
            "sync_scan_failed"
        },
        reason: "lightclient_error",
        grpc_code: None,
    };
    diagnostic.reason = match &error {
        LightClientError::SyncError(error) => match error {
            SyncError::ServerError(e) => server_reason(e, &mut diagnostic),
            SyncError::ScanError(e) => scan_reason(e, &mut diagnostic),
            SyncError::MempoolError(MempoolError::ServerError(e)) => {
                server_reason(e, &mut diagnostic)
            }
            SyncError::MempoolError(MempoolError::ShutdownWithoutStream) => {
                "mempool_shutdown_timeout"
            }
            SyncError::SyncModeError(e) => mode_reason(e),
            SyncError::ChainError(..) => "wallet_ahead_of_chain",
            SyncError::BirthdayBelowSapling(..) => "birthday_below_sapling",
            SyncError::ShardTreeError(e) => shard_reason(e),
            SyncError::TruncationError(..) => "truncation_checkpoint_missing",
            SyncError::PoolHistoryReopened { .. } => "pool_history_reopened",
            SyncError::TransparentAddressDerivationError(_) => "transparent_address_derivation",
            SyncError::WalletError(e) => wallet_reason(e, &mut diagnostic),
        },
        LightClientError::SyncLaunchError => "sync_launch",
        LightClientError::SyncNotRunning => "sync_not_running",
        LightClientError::SyncModeError(e) => mode_reason(e),
        LightClientError::IndexerError(status) => {
            diagnostic.grpc_code = Some(status.code());
            "indexer_request_failed"
        }
        LightClientError::ClientError(e) => match e {
            zingo_netutils::GetClientError::InvalidScheme => "indexer_invalid_scheme",
            zingo_netutils::GetClientError::InvalidAuthority => "indexer_invalid_authority",
            zingo_netutils::GetClientError::Transport(_) => "indexer_transport",
        },
        LightClientError::WalletError(e) => wallet_reason(e, &mut diagnostic),
        LightClientError::FileError(_) => "wallet_file_error",
        LightClientError::MigrationError(_) => "migration_error",
        LightClientError::SendError(_) => "send_error",
        LightClientError::Offline => "indexer_not_configured",
    };
    diagnostic.into()
}

fn server_reason(error: &ServerError, diagnostic: &mut SyncDiagnostic) -> &'static str {
    match error {
        ServerError::RequestFailed(status) => {
            diagnostic.grpc_code = Some(status.code());
            "indexer_request_failed"
        }
        ServerError::InvalidFrontier(_) => "indexer_invalid_frontier",
        ServerError::InvalidTransaction(_) => "indexer_invalid_transaction",
        ServerError::InvalidSubtreeRoot => "indexer_invalid_subtree_root",
        ServerError::ChainVerificationError => "chain_verification_failed",
        ServerError::FetcherDropped => "indexer_fetcher_dropped",
        ServerError::GenesisBlockOnly => "indexer_genesis_only",
    }
}

fn mode_reason(error: &SyncModeError) -> &'static str {
    match error {
        SyncModeError::InvalidSyncMode(_) => "invalid_sync_mode",
        SyncModeError::SyncAlreadyRunning => "sync_already_running",
        SyncModeError::SyncNotRunning => "sync_not_running",
        SyncModeError::SyncNotPaused => "sync_not_paused",
    }
}

fn scan_reason(error: &ScanError, diagnostic: &mut SyncDiagnostic) -> &'static str {
    match error {
        ScanError::ServerError(e) => server_reason(e, diagnostic),
        ScanError::ContinuityError(ContinuityError::HeightDiscontinuity { .. }) => {
            "block_height_discontinuity"
        }
        ScanError::ContinuityError(ContinuityError::HashDiscontinuity { .. }) => {
            "block_hash_discontinuity"
        }
        ScanError::EncodingError(_) => "compact_encoding_invalid",
        ScanError::InvalidSaplingNullifier(_) => "sapling_nullifier_invalid",
        ScanError::InvalidOrchardNullifierLength(_) => "orchard_nullifier_length",
        ScanError::InvalidOrchardNullifier => "orchard_nullifier_invalid",
        ScanError::InvalidSaplingOutput => "sapling_output_invalid",
        ScanError::InvalidOrchardAction => "orchard_action_invalid",
        ScanError::IncorrectTreeSize { .. } => "tree_size_mismatch",
        ScanError::IncorrectTxid { .. } => "transaction_id_mismatch",
        ScanError::DecryptedNoteDataNotFound(_) => "decrypted_note_data_missing",
        ScanError::InvalidMemoBytes(_) => "memo_bytes_invalid",
        ScanError::AddressParseError(_) => "address_parse_failed",
    }
}

/// Match variants only: tree positions and addresses can reveal wallet activity.
fn shard_reason(
    error: &shardtree::error::ShardTreeError<std::convert::Infallible>,
) -> &'static str {
    use shardtree::error::{InsertionError, QueryError, ShardTreeError};
    match error {
        ShardTreeError::Query(QueryError::NotContained(_)) => "shard_query_not_contained",
        ShardTreeError::Query(QueryError::CheckpointPruned) => "shard_checkpoint_pruned",
        ShardTreeError::Query(QueryError::TreeIncomplete(_)) => "shard_tree_incomplete",
        ShardTreeError::Insert(InsertionError::NotContained(_)) => "shard_insert_not_contained",
        ShardTreeError::Insert(InsertionError::OutOfRange(..)) => "shard_insert_out_of_range",
        ShardTreeError::Insert(InsertionError::Conflict(_)) => "shard_root_conflict",
        ShardTreeError::Insert(InsertionError::CheckpointOutOfOrder) => {
            "shard_checkpoint_out_of_order"
        }
        ShardTreeError::Insert(InsertionError::TreeFull) => "shard_tree_full",
        ShardTreeError::Insert(InsertionError::InputMalformed(_)) => "shard_input_malformed",
        ShardTreeError::Insert(InsertionError::MarkedRetentionInvalid) => {
            "shard_marked_retention_invalid"
        }
        ShardTreeError::Storage(never) => match *never {},
    }
}

fn wallet_reason(error: &WalletError, diagnostic: &mut SyncDiagnostic) -> &'static str {
    match error {
        WalletError::CalculatedTxScanError(e) => scan_reason(e, diagnostic),
        WalletError::NoSyncData => "wallet_no_sync_data",
        WalletError::SyncIncomplete => "wallet_sync_incomplete",
        WalletError::CheckpointNotFound { .. } => "wallet_checkpoint_missing",
        WalletError::ShardTreeError(e) => shard_reason(e),
        WalletError::TransactionRead(_) => "wallet_transaction_read",
        WalletError::TransactionWrite(_) => "wallet_transaction_write",
        WalletError::MigrationStateCorrupt(_) => "wallet_migration_state_corrupt",
        WalletError::MigrationBoundNoteMissing(_) => "wallet_migration_note_missing",
        WalletError::MigrationInvalidTransition { .. } => "wallet_migration_invalid_transition",
        _ => "wallet_error",
    }
}

/// Both background owner loops share the same bounded, credential-free fields.
pub(crate) fn warn_sync_failure(error: &anyhow::Error) {
    let diagnostic = error.downcast_ref::<SyncDiagnostic>();
    tracing::warn!(
        category = sync_failure(error),
        reason = diagnostic.map(|d| d.reason),
        grpc_code = diagnostic.and_then(|d| d.grpc_code).map(|c| c as i32),
        grpc_status = diagnostic
            .and_then(|d| d.grpc_code)
            .map(|c| format!("{c:?}")),
        "treasury sync unavailable"
    );
}

pub(crate) fn sync_failure(error: &anyhow::Error) -> &'static str {
    if let Some(diagnostic) = error.downcast_ref::<SyncDiagnostic>() {
        return diagnostic.category;
    }
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
    fn shard_failures_distinguish_query_and_insertion_in_both_sync_paths() {
        use shardtree::error::{InsertionError, QueryError, ShardTreeError};
        let cases = [
            (
                ShardTreeError::Query(QueryError::CheckpointPruned),
                "shard_checkpoint_pruned",
            ),
            (
                ShardTreeError::Query(QueryError::TreeIncomplete(vec![])),
                "shard_tree_incomplete",
            ),
            (
                ShardTreeError::Insert(InsertionError::CheckpointOutOfOrder),
                "shard_checkpoint_out_of_order",
            ),
            (
                ShardTreeError::Insert(InsertionError::TreeFull),
                "shard_tree_full",
            ),
            (
                ShardTreeError::Insert(InsertionError::MarkedRetentionInvalid),
                "shard_marked_retention_invalid",
            ),
        ];
        for (cause, reason) in cases {
            for error in [
                LightClientError::SyncError(SyncError::ShardTreeError(cause.clone())),
                LightClientError::WalletError(WalletError::ShardTreeError(cause)),
            ] {
                let error = client_failure(error, false);
                assert_eq!(
                    error.to_string(),
                    format!("sync_scan_failed: reason={reason}")
                );
                assert_eq!(error.chain().count(), 1);
            }
        }
    }

    #[test]
    fn grpc_causes_survive_nested_sync_errors_without_private_data() {
        for wrap in 0..4 {
            let mut status = tonic::Status::unavailable(
                "https://user:secret@example.invalid/token wallet=private-wallet",
            );
            status
                .metadata_mut()
                .insert("authorization", "secret-token".parse().unwrap());
            let error = match wrap {
                0 => LightClientError::IndexerError(status),
                1 => LightClientError::SyncError(SyncError::ServerError(
                    ServerError::RequestFailed(status),
                )),
                2 => LightClientError::SyncError(SyncError::ScanError(ScanError::ServerError(
                    ServerError::RequestFailed(status),
                ))),
                _ => LightClientError::SyncError(SyncError::MempoolError(
                    MempoolError::ServerError(ServerError::RequestFailed(status)),
                )),
            };
            let error = client_failure(error, false);
            assert_eq!(
                error.chain().count(),
                1,
                "raw upstream error must not be retained"
            );
            let saved = error.to_string();
            assert!(saved.contains("reason=indexer_request_failed"), "{saved}");
            assert!(
                saved.contains("grpc_code=14 grpc_status=Unavailable"),
                "{saved}"
            );
            assert_eq!(sync_failure(&error), "sync_scan_failed");
            let log = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(log.reopen().unwrap())
                .finish();
            {
                let _guard = tracing::subscriber::set_default(subscriber);
                warn_sync_failure(&error.context("private outer context"));
            }
            let logs = std::fs::read_to_string(log.path()).unwrap();
            assert!(logs.contains(r#"category="sync_scan_failed""#), "{logs}");
            assert!(
                logs.contains(r#"reason="indexer_request_failed""#),
                "{logs}"
            );
            assert!(logs.contains("grpc_code=14"), "{logs}");
            assert!(logs.contains("Unavailable"), "{logs}");
            for text in [&saved, &logs] {
                for forbidden in ["secret", "example.invalid", "private", "authorization"] {
                    assert!(!text.contains(forbidden), "{text}");
                }
            }
        }
    }

    #[test]
    fn scan_wallet_and_launch_failures_have_distinct_safe_reasons() {
        let cases = [
            (
                LightClientError::SyncError(SyncError::ScanError(ScanError::IncorrectTreeSize {
                    shielded_protocol: zcash_protocol::PoolType::ORCHARD,
                    height: 123u32.into(),
                    block_metadata_size: 456,
                    calculated_size: 789,
                })),
                "tree_size_mismatch",
            ),
            (
                LightClientError::SyncError(SyncError::ServerError(
                    ServerError::InvalidTransaction(std::io::Error::other("private transaction")),
                )),
                "indexer_invalid_transaction",
            ),
            (
                LightClientError::SyncError(SyncError::WalletError(
                    WalletError::MigrationStateCorrupt("private wallet".into()),
                )),
                "wallet_migration_state_corrupt",
            ),
            (
                LightClientError::SyncError(SyncError::MempoolError(
                    MempoolError::ShutdownWithoutStream,
                )),
                "mempool_shutdown_timeout",
            ),
            (LightClientError::SyncNotRunning, "sync_not_running"),
        ];
        for (error, reason) in cases {
            let error = client_failure(error, false);
            assert_eq!(
                error.to_string(),
                format!("sync_scan_failed: reason={reason}")
            );
            assert!(!format!("{error:?}").contains("private"));
        }
        let error = client_failure(LightClientError::SyncLaunchError, true);
        assert_eq!(sync_failure(&error), "sync_launch_failed");
        assert!(error.to_string().contains("reason=sync_launch"));
        let error = client_failure(
            LightClientError::IndexerError(tonic::Status::deadline_exceeded("private timeout")),
            false,
        );
        assert!(
            error
                .to_string()
                .contains("grpc_code=4 grpc_status=DeadlineExceeded")
        );
    }

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
