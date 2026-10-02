//! Contracts for the next treasury milestone. There is no network implementation.
//! Preparation must calculate, validate actual fees/input, and atomically journal
//! the resulting wallet snapshot and exact transaction bytes before returning.
use super::store::Store;
use anyhow::{Result, ensure};
use std::future::Future;
use zeroize::Zeroizing;

pub struct PrepareRequest {
    pub operation_id: String,
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
    raw: Zeroizing<Vec<u8>>,
}
impl PreparedTransaction {
    pub fn load(store: &Store, operation_id: &str) -> Result<Self> {
        ensure!(
            store.operation_pending(operation_id)?,
            "transaction is not pending"
        );
        Ok(Self {
            operation_id: operation_id.into(),
            raw: store.prepared_bytes(operation_id)?,
        })
    }
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub fn bytes(&self) -> &[u8] {
        &self.raw
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
    /// Submit these exact durable bytes via the independently configured endpoint.
    /// Timeout/cancellation is Unknown, never evidence that inputs can be reused.
    fn submit(
        &mut self,
        transaction: &PreparedTransaction,
    ) -> impl Future<Output = Result<SubmissionOutcome>> + Send;
    /// Derive/verify the txid from the durable bytes. Absent is not proof of expiry;
    /// only verified reconciliation may resolve an outgoing record or release input.
    fn lookup(
        &mut self,
        transaction: &PreparedTransaction,
    ) -> impl Future<Output = Result<TransactionPresence>> + Send;
}
