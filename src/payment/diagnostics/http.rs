//! Fixed projections of untrusted HTTP metadata, never payment authority.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
pub(super) struct HeaderFacts {
    pub(super) state: &'static str,
    pub(super) error: Option<&'static str>,
    pub(super) seller_success: Option<bool>,
}
impl Default for HeaderFacts {
    fn default() -> Self {
        Self {
            state: "absent",
            error: None,
            seller_success: None,
        }
    }
}
impl HeaderFacts {
    pub(super) fn read(
        headers: &reqwest::header::HeaderMap,
        names: &[&str],
        error_key: &str,
    ) -> Self {
        match header_object(headers, names) {
            Ok(value) => Self {
                state: "decoded_object",
                error: value.get(error_key).map(payment_error),
                seller_success: value.get("success").and_then(Value::as_bool),
            },
            Err(state) => Self {
                state,
                ..Self::default()
            },
        }
    }
}

fn header_object(
    headers: &reqwest::header::HeaderMap,
    names: &[&str],
) -> Result<serde_json::Map<String, Value>, &'static str> {
    let mut values = names.iter().flat_map(|name| headers.get_all(*name).iter());
    let header = values.next().ok_or("absent")?;
    if values.next().is_some() {
        return Err("duplicate");
    }
    let decoded = STANDARD
        .decode(header.as_bytes())
        .map_err(|_| "malformed")?;
    match serde_json::from_slice::<Value>(&decoded) {
        Ok(Value::Object(value)) => Ok(value),
        _ => Err("malformed"),
    }
}

/// Untrusted receipt prose belongs only in the private tool error, like the
/// upstream error body. Never pass this projection to tracing or persist it.
pub(in crate::payment) fn payment_response_error(
    headers: &reqwest::header::HeaderMap,
) -> Option<Value> {
    let receipt = header_object(headers, &["payment-response", "x-payment-response"]).ok()?;
    let error: serde_json::Map<String, Value> = ["errorReason", "errorMessage"]
        .into_iter()
        .filter_map(|key| {
            receipt
                .get(key)?
                .as_str()
                .map(|value| (key.to_owned(), Value::String(value.to_owned())))
        })
        .collect();
    (!error.is_empty()).then_some(Value::Object(error))
}

// Pinned x402-types ErrorReason plus the public x402 v2 specification, section 9:
// https://github.com/x402-foundation/x402/blob/main/specs/x402-specification-v2.md#error-handling
// Exact matches only: unknown values can contain credentials or arbitrary prose.
const KNOWN_PAYMENT_ERRORS: &[&str] = &[
    "invalid_format",
    "invalid_payment_amount",
    "invalid_payment_early",
    "invalid_payment_expired",
    "chain_id_mismatch",
    "recipient_mismatch",
    "asset_mismatch",
    "accepted_requirements_mismatch",
    "invalid_signature",
    "transaction_simulation",
    "insufficient_funds",
    "permit2_allowance_required",
    "unsupported_chain",
    "unsupported_scheme",
    "unexpected_error",
    "invalid_exact_evm_payload_authorization_valid_after",
    "invalid_exact_evm_payload_authorization_valid_before",
    "invalid_exact_evm_payload_authorization_value_mismatch",
    "invalid_exact_evm_payload_signature",
    "invalid_exact_evm_payload_recipient_mismatch",
    "invalid_network",
    "invalid_payload",
    "invalid_payment_requirements",
    "invalid_scheme",
    "invalid_x402_version",
    "invalid_transaction_state",
    "unexpected_verify_error",
    "unexpected_settle_error",
    "settlement_pending",
];

fn payment_error(value: &Value) -> &'static str {
    KNOWN_PAYMENT_ERRORS
        .iter()
        .copied()
        .find(|known| value.as_str() == Some(*known))
        .unwrap_or("unrecognized")
}

pub(super) fn body_kind(bytes: &[u8], value: Option<&Value>) -> &'static str {
    match value {
        Some(Value::Object(object)) if object.is_empty() => "empty_object",
        Some(Value::Object(_)) => "object",
        Some(Value::Array(_)) => "array",
        Some(Value::String(_)) => "string",
        Some(Value::Number(_)) => "number",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Null) => "null",
        None if bytes.is_empty() => "empty",
        None => "non_json",
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct OmittedExtensions {
    bazaar: bool,
    builder_code: bool,
    payment_identifier: bool,
    sign_in_with_x: bool,
    other: bool,
}
impl OmittedExtensions {
    pub(super) fn observe(&mut self, extensions: &serde_json::Map<String, Value>) {
        for name in extensions.keys() {
            let flag = match name.as_str() {
                "bazaar" => &mut self.bazaar,
                "builder-code" => &mut self.builder_code,
                "payment-identifier" => &mut self.payment_identifier,
                "sign-in-with-x" => &mut self.sign_in_with_x,
                _ => &mut self.other,
            };
            *flag = true;
        }
    }
}
