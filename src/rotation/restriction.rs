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
