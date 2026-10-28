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
        let mut values = names.iter().flat_map(|name| headers.get_all(*name).iter());
        let Some(header) = values.next() else {
            return Self::default();
        };
        if values.next().is_some() {
            return Self {
                state: "duplicate",
                ..Self::default()
            };
        }
        let value = STANDARD
            .decode(header.as_bytes())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let Some(Value::Object(value)) = value else {
            return Self {
                state: "malformed",
                ..Self::default()
            };
        };
        Self {
            state: "decoded_object",
            error: value.get(error_key).map(payment_error),
            seller_success: value.get("success").and_then(Value::as_bool),
        }
    }
}

fn payment_error(value: &Value) -> &'static str {
    // Exact codes from the pinned x402-types ErrorReason vocabulary. Unknown
    // values can contain credentials, even when shaped like ordinary codes.
    match value.as_str() {
        Some("invalid_format") => "invalid_format",
        Some("invalid_payment_amount") => "invalid_payment_amount",
        Some("invalid_payment_early") => "invalid_payment_early",
        Some("invalid_payment_expired") => "invalid_payment_expired",
        Some("chain_id_mismatch") => "chain_id_mismatch",
        Some("recipient_mismatch") => "recipient_mismatch",
        Some("asset_mismatch") => "asset_mismatch",
        Some("accepted_requirements_mismatch") => "accepted_requirements_mismatch",
        Some("invalid_signature") => "invalid_signature",
        Some("transaction_simulation") => "transaction_simulation",
        Some("insufficient_funds") => "insufficient_funds",
        Some("permit2_allowance_required") => "permit2_allowance_required",
        Some("unsupported_chain") => "unsupported_chain",
        Some("unsupported_scheme") => "unsupported_scheme",
        Some("unexpected_error") => "unexpected_error",
        _ => "unrecognized",
    }
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
