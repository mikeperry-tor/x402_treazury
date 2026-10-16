//! Durable treasury operation contracts. The Zcash adapter lives in treasury/.
//! Preparation must calculate, validate actual fees/input, and atomically journal
//! the resulting wallet snapshot and exact transaction bytes before returning.
use super::store::Store;
use anyhow::{Result, ensure};
use std::future::Future;
use zeroize::Zeroizing;

// Conservative delivery allowance retained pending the authoritative Zcash
// 1Click arrival/route contract; never derive settlement from local broadcast.
// All pre-preparation and first-submission gates use this one policy constant.
pub const MIN_QUOTE_VALIDITY_SECONDS: u64 = 300;

/// Local canonical proof failures. Display text is diagnostic only; recovery
/// consumers downcast this type rather than interpreting upstream prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryEvidenceError {
    ExpiryNotBuried,
    ExpiryNotInvalidated,
    TipLag,
    StaleSync,
    ChainChanged,
    InputMissing,
    InputMismatch,
    InputSpent,
    IndexerInclusion,
    TransactionMismatch,
    ConfirmationMismatch,
}

impl std::fmt::Display for RecoveryEvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ExpiryNotBuried => "transaction expiry not buried",
            Self::ExpiryNotInvalidated => "sync has not invalidated expired transaction",
            Self::TipLag => "expiry recovery requires zero observed tip lag; synchronize again before releasing reservations",
            Self::StaleSync => "expiry recovery sync stale",
            Self::ChainChanged => "expiry recovery chain changed",
            Self::InputMissing => "expiry recovery input missing",
            Self::InputMismatch => "expiry recovery input mismatch",
            Self::InputSpent => "expiry recovery input remains spent or ambiguous",
            Self::IndexerInclusion => "expiry recovery contradicts indexer inclusion",
            Self::TransactionMismatch => "confirmed transaction bytes mismatch",
            Self::ConfirmationMismatch => "confirmation disagrees with wallet sync",
        })
    }
}
impl std::error::Error for RecoveryEvidenceError {}

/// Issued only through a proven read-only path before proposal/preparation starts.
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
    recipient_identity: Option<String>,
    raw: Zeroizing<Vec<u8>>,
    facts: Option<TransactionFacts>,
}
impl PreparedTransaction {
    pub fn load(store: &Store, operation_id: &str) -> Result<Self> {
        ensure!(
            store.operation_pending(operation_id)?,
            "transaction is not pending"
        );
        Ok(Self {
            operation_id: operation_id.into(),
            recipient_identity: store.operation_recipient(operation_id)?,
            raw: store.prepared_bytes(operation_id)?,
            facts: store.operation_facts(operation_id)?,
        })
    }
    pub fn network_identity(&self) -> Result<crate::network::IsolationId> {
        crate::network::IsolationId::evm(self.recipient_identity.as_deref().ok_or_else(|| {
            anyhow::anyhow!("network_identity_missing: durable operation recipient")
        })?)
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
    /// Must check fresh spend readiness and reserve input+fee before calculation.
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
    pub deadline: u64,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct OperationStatus {
    pub operation_id: String,
    pub facts: TransactionFacts,
    pub submission: String,
    pub attempts: i64,
}
