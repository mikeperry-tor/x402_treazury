//! Durable treasury operation contracts. The Zcash adapter lives in treasury/.
//! Preparation must calculate, validate actual fees/input, and atomically journal
//! the resulting wallet snapshot and exact transaction bytes before returning.
use super::store::Store;
use anyhow::{Result, ensure};
use std::future::Future;
use zeroize::Zeroizing;

/// Local canonical proof failures. Display text is diagnostic only; recovery
/// consumers downcast this type rather than interpreting upstream prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryEvidenceError {
    ExpiryNotBuried,
    TipLag,
    StaleSync,
    ChainChanged,
    InputMissing,
    InputMismatch,
    InputSpent,
    IndexerInclusion,
    TransactionMismatch,
    ConfirmationMismatch,
    ConfirmationPending,
}

impl std::fmt::Display for RecoveryEvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ExpiryNotBuried => "transaction expiry not buried",
            Self::TipLag => "expiry recovery requires zero observed tip lag; synchronize again before releasing reservations",
            Self::StaleSync => "expiry recovery sync stale",
            Self::ChainChanged => "expiry recovery chain changed",
            Self::InputMissing => "expiry recovery input missing",
            Self::InputMismatch => "expiry recovery input mismatch",
            Self::InputSpent => "expiry recovery input remains spent or ambiguous",
            Self::IndexerInclusion => "expiry recovery contradicts indexer inclusion",
            Self::TransactionMismatch => "confirmed transaction bytes mismatch",
            Self::ConfirmationMismatch => "confirmation disagrees with wallet sync",
            Self::ConfirmationPending => "treasury_confirmation_pending",
        })
    }
}
impl std::error::Error for RecoveryEvidenceError {}

/// Safe facts from the same wallet read that rejected source confirmation.
/// Diagnostic only: neither these heights nor their persisted prose grant authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationObservation {
    pub lookup_height: u64,
    pub wallet_height: Option<u64>,
    pub scanned_height: Option<u64>,
}
impl std::fmt::Display for ConfirmationObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "confirmation_height_mismatch: lookup_height={} wallet_height={} scanned_height={}",
            self.lookup_height,
            self.wallet_height
                .map_or_else(|| "unconfirmed".into(), |h| h.to_string()),
            self.scanned_height
                .map_or_else(|| "unknown".into(), |h| h.to_string())
        )
    }
}
impl std::error::Error for ConfirmationObservation {}

/// Issued only through a proven read-only path before accepting input ownership
/// or starting calculation. SDK proposal selection can fail on this path.
/// It is not evidence that an interrupted calculation is safe to repeat.
#[derive(Debug)]
pub struct PreparationDeferred;
impl std::fmt::Display for PreparationDeferred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("treasury_preparation_deferred: pre-preparation readiness unavailable; no calculation started")
    }
}
impl std::error::Error for PreparationDeferred {}

/// Current process configuration passed to the serialized treasury owner.
/// This cannot replace the durable qualification permit or current job binding.
#[derive(Clone)]
pub struct ManagedFundingAuthority {
    pub job_id: String,
    pub auto_fund: bool,
    pub max_attempts: u32,
}

pub struct PrepareRequest {
    pub managed_funding: Option<ManagedFundingAuthority>,

    pub operation_id: String,
    pub pool_id: Option<String>,

    pub deadline: u64,
    pub recipient: String,
    pub amount_zatoshis: u64,
    pub max_fee_zatoshis: u64,
    pub max_input_zatoshis: u64,
}

/// Only recoverable from a committed, unresolved outgoing record. No public
/// constructor or byte deserializer can turn an unjournaled calculation into this.
/// Deliberately lacks Debug/Serialize: signed bytes must not enter logs or tools.
pub struct PreparedTransaction {
    operation_id: String,
    identity: Option<crate::network::IsolationId>,
    raw: Zeroizing<Vec<u8>>,
    facts: Option<TransactionFacts>,
}
impl PreparedTransaction {
    pub fn load(store: &Store, operation_id: &str) -> Result<Self> {
        ensure!(
            store.operation_pending(operation_id)?,
            "transaction is not pending"
        );
        let facts = store.operation_facts(operation_id)?;
        let identity = store
            .operation_recipient(operation_id)?
            .as_deref()
            .map(crate::network::IsolationId::evm)
            .transpose()?;
        let identity = match identity {
            Some(identity) => Some(identity),
            None => store.shielding_identity(operation_id, facts.as_ref())?,
        };
        Ok(Self {
            operation_id: operation_id.into(),
            identity,
            raw: store.prepared_bytes(operation_id)?,
            facts,
        })
    }
    pub fn network_identity(&self) -> Result<crate::network::IsolationId> {
        self.identity
            .clone()
            .ok_or_else(|| anyhow::anyhow!("network_identity_missing: durable operation recipient"))
    }
    pub fn facts(&self) -> Option<&TransactionFacts> {
        self.facts.as_ref()
    }
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
}

/// Single-use submission capability, minted only after a committed send intent.
/// No Clone/Deserialize implementation: callers must journal each retry.
pub struct BroadcastTransaction {
    transaction: PreparedTransaction,
    attempt: i64,
}
impl BroadcastTransaction {
    pub fn request(
        store: &mut Store,
        id: &str,
        now: u64,
        height: u64,
        retry: bool,
    ) -> Result<Self> {
        let transaction = PreparedTransaction::load(store, id)?;
        let attempt = store.request_broadcast(id, now, height, retry)?;
        Ok(Self {
            transaction,
            attempt,
        })
    }
    pub fn transaction(&self) -> &PreparedTransaction {
        &self.transaction
    }
    pub fn attempt(&self) -> i64 {
        self.attempt
    }
}

pub trait TransactionPreparer {
    /// Must check actual permission and capture selected input ownership before calculation.
    /// Calculation never broadcasts. Return only after durable snapshot+bytes commit.
    fn prepare(
        &mut self,
        request: PrepareRequest,
    ) -> impl Future<Output = Result<PreparedTransaction>> + Send;
}
#[derive(Debug, PartialEq, Eq)]
pub enum SubmissionOutcome {
    Accepted,
    Rejected,
    Unknown,
}
#[derive(Debug, PartialEq, Eq)]
pub enum TransactionPresence {
    Mempool,
    Confirmed { height: u64 },
    Absent,
    Unknown,
}

pub trait TransactionSubmission {
    /// Read-only validation before durable broadcast intent. Failure grants no
    /// new preparation authority and never alters historical attempt records.
    fn preflight(
        &mut self,
        _transaction: &PreparedTransaction,
        _rebroadcast: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    /// Submit these exact durable bytes via the independently configured endpoint.
    /// Timeout/cancellation is Unknown, never evidence that inputs can be reused.
    fn submit(
        &mut self,
        transaction: BroadcastTransaction,
    ) -> impl Future<Output = Result<SubmissionOutcome>> + Send;
    /// Derive/verify the txid from the durable bytes. Absent is not proof of expiry;
    /// only verified reconciliation may resolve an outgoing record or release input.
    fn lookup(
        &mut self,
        transaction: &PreparedTransaction,
    ) -> impl Future<Output = Result<TransactionPresence>> + Send;
}

/// Public operation facts; signed bytes remain encrypted separately.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransactionFacts {
    pub txid: String,
    pub expiry_height: u32,
    pub amount_zatoshis: u64,
    pub fee_zatoshis: u64,
    pub deadline: Option<u64>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct OperationStatus {
    pub operation_id: String,
    pub facts: TransactionFacts,
    pub submission: String,
    pub attempts: i64,
}

impl TransactionFacts {
    pub(crate) fn require_protocol_validity(
        &self,
        clock: impl FnOnce() -> Result<u64>,
        tip: u64,
    ) -> Result<()> {
        if let Some(deadline) = self.deadline {
            ensure!(clock()? < deadline, "funding_deadline_expired");
        }
        // N is inclusive for mined transactions. At tip N the next block is N+1.
        // Zero is the consensus no-expiry value and never grants expiry release.
        ensure!(
            self.expiry_height == 0 || tip < u64::from(self.expiry_height),
            "transaction_expired"
        );
        Ok(())
    }
}

#[cfg(test)]
mod validity_tests {
    use super::*;
    #[test]
    fn returned_deadline_and_inclusive_consensus_expiry_are_independent() {
        let mut facts = TransactionFacts {
            txid: "original".into(),
            expiry_height: 10,
            amount_zatoshis: 1,
            fee_zatoshis: 1,
            deadline: Some(20),
        };
        assert!(facts.require_protocol_validity(|| Ok(19), 9).is_ok());
        assert!(facts.require_protocol_validity(|| Ok(20), 9).is_err());
        assert!(facts.require_protocol_validity(|| Ok(19), 10).is_err());
        assert!(
            facts
                .require_protocol_validity(|| anyhow::bail!("clock unavailable"), 9)
                .is_err()
        );
        facts.deadline = None;
        assert!(
            facts
                .require_protocol_validity(|| panic!("shielding does not need wall time"), 9)
                .is_ok()
        );
        assert!(facts.require_protocol_validity(|| Ok(u64::MAX), 9).is_ok());
        facts.expiry_height = 0;
        assert!(
            facts
                .require_protocol_validity(|| Ok(u64::MAX), u64::MAX)
                .is_ok()
        );
    }
}
