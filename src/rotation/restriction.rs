//! Process-local restrictions for supervised qualification. Never grants authority.
use super::error::AdmissionError;
use anyhow::Result;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FundingRestriction {
    #[default]
    Ordinary,
    /// Existing payments/reconciliation remain possible; no new allocation or ZEC spend.
    DenyNewFunding,
}
impl FundingRestriction {
    pub(crate) fn require_new_funding(self, action: &'static str) -> Result<()> {
        if self == Self::DenyNewFunding {
            tracing::warn!(
                action,
                code = "qualification_funding_denied",
                "qualification funding limit is zero; new funding denied; existing active-wallet calls and reconciliation remain available"
            );
            anyhow::bail!(AdmissionError::FundingRestricted);
        }
        Ok(())
    }
}

/// Allocation/preparation restriction installed before managed pool setup by a
/// supervised owner. Implementations must durably reserve the complete batch
/// before returning, and reuse job IDs for the same treasury/pool/sequence.
/// A permit never bypasses ordinary payment, quote, or treasury checks.
pub trait FundingPermits: Send + Sync {
    fn validate_treasury(&self, treasury: &str) -> Result<()>;
    /// Called by the exclusive store owner before installing the restriction.
    /// The registry implementation also atomically reserves every missing pool's
    /// bootstrap pair before any allocation, retaining orphan permits on restart.
    fn validate_state(&self, state: &serde_json::Value) -> Result<()>;
    fn reserve_allocations(
        &self,
        treasury: &str,
        pool_name: &str,
        sequences: &[i64],
    ) -> Result<Vec<String>>;
    fn check_preparation(&self, treasury: &str, job: &str, input_with_fee: u64) -> Result<()>;
    fn reserve_preparation(
        &self,
        treasury: &str,
        job: &str,
        operation: &str,
        input_with_fee: u64,
    ) -> Result<()>;
    fn retire_unprepared(&self, treasury: &str, job: &str, operation: &str) -> Result<()>;
}

pub(crate) fn permit_result<T>(result: Result<T>, action: &'static str) -> Result<T> {
    result.map_err(|error| {
        tracing::warn!(action, code = "qualification_funding_denied",
            "funding permit unavailable; no new funding authorized; inspect qualification registry; existing funded active-wallet calls remain available");
        error.context(AdmissionError::FundingPermitDenied)
    })
}
