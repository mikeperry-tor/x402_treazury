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
                "bound payer identity changed before this authorization could be journaled",
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

/// Current-operation observations used for startup assessment. These are not
/// durable spending authority and are never reconstructed from diagnostic prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundingCondition {
    Disabled,
    QualificationDenied,
    CostLimit,
    AmountLimit,
    InputLimit,
    AuthenticationRequired,
    InsufficientFunds,
    EvidenceInvalid,
}
impl std::fmt::Display for FundingCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Disabled => "funding_disabled",
            Self::QualificationDenied => "qualification_funding_denied",
            Self::CostLimit => "funding_cost_limit_exceeded",
            Self::AmountLimit => "funding_amount_limit_exceeded",
            Self::InputLimit => "quote input limit",
            Self::AuthenticationRequired => "near_authentication_required",
            Self::InsufficientFunds => "treasury_insufficient_spendable_funds",
            Self::EvidenceInvalid => "funding_integrity_conflict",
        })
    }
}
impl std::error::Error for FundingCondition {}

/// A replacement would exceed the durable transaction recovery allowance.
#[derive(Debug)]
pub(crate) struct RecoveryAttemptLimit;
impl std::fmt::Display for RecoveryAttemptLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("recovery_attempt_limit_reached")
    }
}
impl std::error::Error for RecoveryAttemptLimit {}

/// Read-only receive information for an operator-facing capital shortage message.
/// Display stays fixed so generic error chains do not disclose the address.
#[derive(Debug)]
pub(crate) struct TreasuryDepositAddress(pub String);
impl std::fmt::Display for TreasuryDepositAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("treasury deposit address available")
    }
}
impl std::error::Error for TreasuryDepositAddress {}
