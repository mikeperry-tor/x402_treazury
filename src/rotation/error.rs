//! Stable admission categories, rendered as ordinary MCP tool errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    PayerChanged,
    UnsupportedPayment(&'static str),
    PriceLimit(&'static str),
    PaymentPending(&'static str),
    WalletNotReady(&'static str),
    FundingUnavailable(&'static str),
    FundingRestricted,
    FundingPermitDenied,
    OutcomeUnknown(&'static str),
    ChainRecoveryRequired(&'static str),
}
impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (code, detail): (&str, &str) = match self {
            Self::PayerChanged => (
                "payer_changed_before_payment",
                "wallet rotated before signing; retry this tool call",
            ),
            Self::UnsupportedPayment(s) => ("unsupported_payment", s),
            Self::PriceLimit(s) => ("price_limit", s),
            Self::PaymentPending(s) => ("payment_pending", s),
            Self::WalletNotReady(s) => ("wallet_not_ready", s),
            Self::FundingUnavailable(s) => ("funding_unavailable", s),
            Self::FundingRestricted => (
                "qualification_funding_denied",
                "new funding limit is zero; no replacement wallet or ZEC spend authorized; adequately funded active-wallet calls remain available",
            ),
            Self::FundingPermitDenied => (
                "qualification_funding_denied",
                "funding permit missing, exhausted, changed, or unavailable; inspect the qualification registry; existing funded active-wallet calls remain available",
            ),
            Self::OutcomeUnknown(s) => ("payment_outcome_unknown", s),
            Self::ChainRecoveryRequired(s) => ("chain_recovery_required", s),
        };
        write!(f, "{code}: {detail}")
    }
}
impl std::error::Error for AdmissionError {}
