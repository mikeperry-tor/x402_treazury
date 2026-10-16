//! Unverified provider correlations, never canonical transfer or closure proof.
//! Shapes: https://docs.near-intents.org/api-reference/oneclick/check-swap-execution-status
//! `correlationId` is request tracing, not a settlement identifier. Explorer URLs,
//! refund prose and unknown annotations are deliberately not retained.
use super::*;
use alloy_primitives::{B256, U256};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettlementIssue {
    MissingDetails,
    MalformedDetails,
    UnsupportedRoute,
    QuoteTermsMismatch,
    MissingOriginTransactions,
    InvalidOriginTransactions,
    MissingDestinationTransactions,
    InvalidDestinationTransactions,
    InvalidAmountIn,
    InvalidAmountOut,
    InvalidDepositedAmount,
    InvalidRefundedAmount,
}

/// Values reported by the provider. Missing amounts are unknown, never zero.
/// Even a populated value needs independent original-operation and chain proof.
#[derive(Clone)]
pub struct UnverifiedSettlement {
    pub origin_transactions: Vec<B256>,
    pub destination_transactions: Vec<B256>,
    pub amount_in: Option<U256>,
    pub amount_out: Option<U256>,
    pub deposited_amount: Option<U256>,
    pub refunded_amount: Option<U256>,
}

/// Not deserializable: only the deposit/quote-binding validator constructs this.
/// No Debug implementation exposes private addresses, amounts or transaction IDs.
pub struct StatusObservation {
    status: SwapStatus,
    quote_digest: [u8; 32],
    settlement: Option<UnverifiedSettlement>,
    issues: Vec<SettlementIssue>,
}
impl StatusObservation {
    pub fn status(&self) -> &SwapStatus {
        &self.status
    }
    pub fn into_status(self) -> SwapStatus {
        self.status
    }
    /// Digest of the exact original saved quote, including request and response.
    pub fn quote_digest(&self) -> &[u8; 32] {
        &self.quote_digest
    }
    /// No partial candidate set escapes if any known evidence field is malformed
    /// or either chain correlation is absent. These remain unverified candidates.
    pub fn settlement(&self) -> Option<&UnverifiedSettlement> {
        self.settlement.as_ref()
    }
    pub fn issues(&self) -> &[SettlementIssue] {
        &self.issues
    }
}

pub(super) fn parse(quote: &Quote, value: &Value) -> Result<StatusObservation> {
    let deposit = quote
        .deposit
        .as_deref()
        .context("dry quote has no status")?;
    ensure!(
        value["quoteResponse"]["quote"]["depositAddress"].as_str() == Some(deposit),
        "status deposit mismatch"
    );
    check_echo(&quote.request, &value["quoteResponse"]["quoteRequest"])?;
    let status = match value["status"].as_str() {
        Some("PENDING_DEPOSIT") => SwapStatus::PendingDeposit,
        Some("KNOWN_DEPOSIT_TX") => SwapStatus::KnownDeposit,
        Some("INCOMPLETE_DEPOSIT") => SwapStatus::IncompleteDeposit,
        Some("PROCESSING") => SwapStatus::Processing,
        Some("SUCCESS") => SwapStatus::Success,
        Some("REFUNDED") => SwapStatus::Refunded,
        Some("FAILED") => SwapStatus::Failed,
        _ => SwapStatus::Unknown,
    };
    let quote_digest = Sha256::digest(serde_json::to_vec(quote)?).into();
    let mut observation = StatusObservation {
        status,
        quote_digest,
        settlement: None,
        issues: vec![],
    };
    // Hash encoding is interpreted only for our accepted Zcash -> Base route.
    if quote.request["originAsset"] != ZEC
        || quote.request["destinationAsset"] != USDC
        || quote.request["swapType"] != "EXACT_OUTPUT"
        || quote.request["depositType"] != "ORIGIN_CHAIN"
        || quote.request["recipientType"] != "DESTINATION_CHAIN"
    {
        observation.issues.push(SettlementIssue::UnsupportedRoute);
        return Ok(observation);
    }
    let saved = &quote.response["quote"];
    let echoed = &value["quoteResponse"]["quote"];
    if ["amountIn", "amountOut"].iter().any(|field| {
        !matches!((atomic_amount(saved.get(*field)), atomic_amount(echoed.get(*field))),
            (Ok(Some(a)), Ok(Some(b))) if a == b)
    }) || !matches!((timestamp(&saved["deadline"]), timestamp(&echoed["deadline"])),
        (Ok(a), Ok(b)) if a == b)
        || saved.get("depositMemo") != echoed.get("depositMemo")
    {
        observation.issues.push(SettlementIssue::QuoteTermsMismatch);
    }
    let details = match value.get("swapDetails") {
        None | Some(Value::Null) => {
            observation.issues.push(SettlementIssue::MissingDetails);
            return Ok(observation);
        }
        Some(Value::Object(details)) => details,
        _ => {
            observation.issues.push(SettlementIssue::MalformedDetails);
            return Ok(observation);
        }
    };
    let origin = transactions(details.get("originChainTxHashes"), false);
    let destination = transactions(details.get("destinationChainTxHashes"), true);
    let origins = evidence_field(
        origin,
        SettlementIssue::MissingOriginTransactions,
        SettlementIssue::InvalidOriginTransactions,
        &mut observation.issues,
    );
    let destinations = evidence_field(
        destination,
        SettlementIssue::MissingDestinationTransactions,
        SettlementIssue::InvalidDestinationTransactions,
        &mut observation.issues,
    );
    let mut amount = |name: &str, issue| match atomic_amount(details.get(name)) {
        Ok(amount) => amount,
        Err(()) => {
            observation.issues.push(issue);
            None
        }
    };
    let amount_in = amount("amountIn", SettlementIssue::InvalidAmountIn);
    let amount_out = amount("amountOut", SettlementIssue::InvalidAmountOut);
    let deposited_amount = amount("depositedAmount", SettlementIssue::InvalidDepositedAmount);
    let refunded_amount = amount("refundedAmount", SettlementIssue::InvalidRefundedAmount);
    if observation.issues.is_empty() {
        observation.settlement = Some(UnverifiedSettlement {
            origin_transactions: origins.unwrap(),
            destination_transactions: destinations.unwrap(),
            amount_in,
            amount_out,
            deposited_amount,
            refunded_amount,
        });
    }
    Ok(observation)
}

fn evidence_field<T>(
    value: Result<Option<T>, ()>,
    missing: SettlementIssue,
    invalid: SettlementIssue,
    issues: &mut Vec<SettlementIssue>,
) -> Option<T> {
    match value {
        Ok(Some(value)) => Some(value),
        Ok(None) => {
            issues.push(missing);
            None
        }
        Err(()) => {
            issues.push(invalid);
            None
        }
    }
}

fn transactions(value: Option<&Value>, base: bool) -> Result<Option<Vec<B256>>, ()> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let entries = value.as_array().ok_or(())?;
    if entries.is_empty() {
        return Ok(None);
    }
    let mut hashes = Vec::with_capacity(entries.len());
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let raw = entry.get("hash").and_then(Value::as_str).ok_or(())?;
        let digits = if base {
            raw.strip_prefix("0x").ok_or(())?
        } else {
            raw
        };
        if digits.len() != 64 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(());
        }
        let hash: B256 = digits.parse().map_err(|_| ())?;
        if !seen.insert(hash) {
            return Err(());
        }
        hashes.push(hash);
    }
    Ok(Some(hashes))
}
fn atomic_amount(value: Option<&Value>) -> Result<Option<U256>, ()> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let digits = value.as_str().ok_or(())?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    U256::from_str_radix(digits, 10).map(Some).map_err(|_| ())
}

#[cfg(test)]
mod tests;
