//! Agent presentation uses typed facts; diagnostic chains remain internal.
use super::{HttpFailure, PossiblePayment, diagnostics::ProviderFailure};
use reqwest::StatusCode;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ApiExecution {
    pub non_read: bool,
}
impl ApiExecution {
    /// Keep safe local corrections and possible-payment warnings visible to Rust callers.
    pub(crate) fn attach(self, error: anyhow::Error) -> anyhow::Error {
        let possible = error.is::<PossiblePayment>();
        let correction = error
            .downcast_ref::<crate::catalog::diagnostics::Problem>()
            .map(|problem| problem.0);
        let error = error.context(self);
        let error = match correction {
            Some(message) => error.context(crate::catalog::diagnostics::Problem(message)),
            None => error,
        };
        if possible {
            error.context(PossiblePayment)
        } else {
            error
        }
    }
}
impl std::fmt::Display for ApiExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("API execution failed")
    }
}
impl std::error::Error for ApiExecution {}

/// Facts from one exchange, including explicitly attributed relay destinations.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ResponseFacts {
    /// Positively attributed to an unsigned target exchange, independent of relay fees.
    pub unsigned_target: bool,
    pub provider: Option<StatusCode>,
    pub destination: Option<StatusCode>,
    pub insufficient_funds: bool,
    pub failure: Option<ProviderFailure>,
}
impl ResponseFacts {
    pub(crate) fn attach(self, error: anyhow::Error) -> anyhow::Error {
        let correction = error
            .downcast_ref::<crate::catalog::diagnostics::Problem>()
            .map(|problem| problem.0);
        let error = error.context(self);
        match correction {
            Some(message) => error.context(crate::catalog::diagnostics::Problem(message)),
            None => error,
        }
    }
    pub fn provider(status: StatusCode) -> Self {
        Self {
            unsigned_target: false,
            provider: Some(status),
            destination: None,
            insufficient_funds: false,
            failure: None,
        }
    }
    pub fn from_error(error: &anyhow::Error) -> Option<Self> {
        let mut facts = error.downcast_ref::<Self>().copied().or_else(|| {
            error
                .downcast_ref::<HttpFailure>()
                .map(|http| Self::provider(http.status))
        })?;
        if let Some(http) = error.downcast_ref::<HttpFailure>() {
            facts.failure = http.provider_failure;
        }
        Some(facts)
    }
}
impl std::fmt::Display for ResponseFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("API response facts retained")
    }
}
impl std::error::Error for ResponseFacts {}

/// Only API failures use this path; local argument/management errors keep their corrections.
pub(crate) fn present(error: &anyhow::Error) -> Option<String> {
    let mut text = summary(error)?;
    if error.is::<PossiblePayment>()
        || error
            .downcast_ref::<ApiExecution>()
            .is_some_and(|execution| execution.non_read)
    {
        text.push_str(" Do not retry automatically.");
    }
    Some(text)
}

/// Aggregate envelopes own their single retry instruction across all children.
pub(crate) fn summary(error: &anyhow::Error) -> Option<String> {
    let execution = error.downcast_ref::<ApiExecution>();
    let facts = ResponseFacts::from_error(error);
    if execution.is_none() && facts.is_none() {
        return None;
    }
    let mut text = match facts {
        Some(ResponseFacts {
            provider,
            destination: Some(destination),
            ..
        }) => {
            let suffix = if destination.is_success() {
                ": response unavailable."
            } else {
                "."
            };
            match provider {
                Some(provider) => {
                    format!("Provider API HTTP {provider}; destination HTTP {destination}{suffix}")
                }
                None => format!("Destination HTTP {destination}{suffix}"),
            }
        }
        _ => {
            let kind = kind(error, facts);
            match facts.and_then(|facts| facts.provider) {
                Some(status) => format!("Provider API HTTP {status}: {kind}."),
                None => {
                    let mut text = kind.to_owned();
                    if let Some(first) = text.get_mut(..1) {
                        first.make_ascii_uppercase();
                    }
                    format!("{text}.")
                }
            }
        }
    };
    if let Some(limit) = error.downcast_ref::<crate::limits::DownloadLimit>() {
        text.push_str(&format!(" {limit}"));
    }
    if let Some(limit) = error.downcast_ref::<crate::output::OutputLimit>() {
        text.push_str(&format!(" {limit}"));
    }
    Some(text)
}

fn kind(error: &anyhow::Error, facts: Option<ResponseFacts>) -> &'static str {
    if let Some(problem) = error.downcast_ref::<crate::catalog::diagnostics::Problem>() {
        return problem.0;
    }
    if facts.is_some_and(|facts| facts.insufficient_funds) {
        return "insufficient funds";
    }
    if let Some(facts) = facts {
        match facts.failure {
            Some(ProviderFailure::InvalidRequestBody) => {
                return "invalid arguments; check the tool arguments";
            }
            Some(ProviderFailure::PaymentSettlementFailed) => return "payment rejected",
            Some(
                ProviderFailure::TemporarilyUnavailable | ProviderFailure::InlineRefreshFailed,
            ) => return "provider unavailable",
            None => {}
        }
    }
    if let Some(admission) = error.downcast_ref::<crate::rotation::error::AdmissionError>() {
        use crate::rotation::error::AdmissionError::*;
        return match admission {
            UnsupportedPayment(_) => "payment requirements unsupported",
            PriceLimit(_) => "payment exceeds the authorized amount",
            WalletNotReady(_) | PayerChanged => "payment wallet unavailable",
            FundingUnavailable(_) => "wallet funding unavailable",
            PaymentPending(_) => "payment pending",
            FundingRestricted | FundingPermitDenied => "funding not authorized",
            OutcomeUnknown(_) => "payment outcome unknown",
            ChainRecoveryRequired(_) => "wallet reconciliation required",
        };
    }
    if let Some(relay) = crate::discovery_relay::RelayFailure::retained(error)
        && relay.kind() != "http_status"
    {
        return relay.explanation();
    }
    if let Some(status) = facts.and_then(|facts| facts.provider) {
        return match status.as_u16() {
            402 => "payment rejected",
            403 => "access denied",
            404 => "not found",
            429 => "rate limited",
            500..=599 => "provider unavailable",
            _ => "response unavailable",
        };
    }
    if error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|http| http.is_connect())
    {
        "provider unavailable"
    } else {
        "response unavailable"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_marker_keeps_rust_payment_warning_visible() {
        let error = ApiExecution { non_read: false }
            .attach(anyhow::anyhow!("PRIVATE").context(PossiblePayment));
        assert!(error.to_string().contains("do not automatically repeat"));
        assert_eq!(
            present(&error).unwrap(),
            "Response unavailable. Do not retry automatically."
        );
    }

    #[tokio::test]
    async fn static_description_repair_is_debug_only() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        if crate::test_log_capture::run_in_child(
            "payment::presentation::tests::static_description_repair_is_debug_only",
        ) {
            return;
        }
        let challenge =
            serde_json::json!({"x402Version":2,"resource":{"description":"x".repeat(501)}});
        let mut response: reqwest::Response = axum::http::Response::builder()
            .status(StatusCode::PAYMENT_REQUIRED)
            .header("payment-required", STANDARD.encode(challenge.to_string()))
            .body(Vec::<u8>::new())
            .unwrap()
            .into();
        let (result, logs) = crate::limits::tests::capture(async {
            super::super::sanitize_challenge(&mut response)
        })
        .await;
        result.unwrap();
        let adjusted: serde_json::Value = serde_json::from_slice(
            &STANDARD
                .decode(response.headers()["payment-required"].as_bytes())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            adjusted["resource"]["description"].as_str().unwrap().len(),
            500
        );
        assert!(logs.contains("DEBUG"));
        assert!(!logs.contains("WARN"));
    }

    fn rejected(status: StatusCode) -> anyhow::Error {
        anyhow::Error::new(HttpFailure {
            status,
            detail: "insufficient_funds: PRIVATE contract execution reverted {} null".into(),
            provider_failure: None,
        })
        .context(crate::rotation::manager::OMITTED_EXTENSIONS)
        .context(PossiblePayment)
        .context(ApiExecution { non_read: false })
    }

    #[test]
    fn only_structured_insufficiency_changes_the_payment_rejection_kind() {
        let error = rejected(StatusCode::PAYMENT_REQUIRED);
        assert_eq!(
            present(&error).unwrap(),
            "Provider API HTTP 402 Payment Required: payment rejected. Do not retry automatically."
        );
        let facts = ResponseFacts {
            insufficient_funds: true,
            ..ResponseFacts::provider(StatusCode::PAYMENT_REQUIRED)
        };
        let error = error.context(facts);
        assert_eq!(
            present(&error).unwrap(),
            "Provider API HTTP 402 Payment Required: insufficient funds. Do not retry automatically."
        );
        assert!(error.is::<HttpFailure>());
        assert!(error.is::<PossiblePayment>());
        assert!(format!("{error:#}").contains("PRIVATE contract"));
    }

    #[test]
    fn statuses_and_recognized_argument_categories_remain_distinct() {
        for (status, kind) in [
            (403, "access denied"),
            (404, "not found"),
            (429, "rate limited"),
            (503, "provider unavailable"),
        ] {
            let status = StatusCode::from_u16(status).unwrap();
            assert_eq!(
                present(&rejected(status)).unwrap(),
                format!("Provider API HTTP {status}: {kind}. Do not retry automatically.")
            );
        }
        let error = anyhow::Error::new(HttpFailure {
            status: StatusCode::BAD_REQUEST,
            detail: "PRIVATE".into(),
            provider_failure: Some(ProviderFailure::InvalidRequestBody),
        });
        assert_eq!(
            present(&error).unwrap(),
            "Provider API HTTP 400 Bad Request: invalid arguments; check the tool arguments."
        );
        assert!(
            present(&anyhow::anyhow!(
                "missing required path parameter company_id"
            ))
            .is_none()
        );
    }

    #[test]
    fn body_conversion_and_sanitization_keep_actual_exchange_facts() {
        let error = anyhow::anyhow!("PRIVATE decode context")
            .context(ResponseFacts::provider(StatusCode::CREATED))
            .context(PossiblePayment)
            .context(ApiExecution { non_read: false });
        let shared =
            crate::discovery_relay::document::SharedFailure::capture(&error, "help_failed");
        assert_eq!(shared.status(), Some(201));
        assert_eq!(
            present(&shared.error()).unwrap(),
            "Provider API HTTP 201 Created: response unavailable. Do not retry automatically."
        );
        let unknown =
            anyhow::anyhow!("PRIVATE transport trace").context(ApiExecution { non_read: true });
        assert_eq!(
            present(&unknown).unwrap(),
            "Response unavailable. Do not retry automatically."
        );
        let known = unknown.context(ResponseFacts {
            provider: Some(StatusCode::CREATED),
            destination: Some(StatusCode::NOT_FOUND),
            ..ResponseFacts::provider(StatusCode::CREATED)
        });
        assert_eq!(
            present(&known).unwrap(),
            "Provider API HTTP 201 Created; destination HTTP 404 Not Found. Do not retry automatically."
        );
    }

    #[test]
    fn rejection_limits_stay_visible_without_upstream_prose() {
        let error = crate::limits::exceeded("API response", "max_response_bytes", 123)
            .context(ResponseFacts::provider(StatusCode::OK))
            .context(PossiblePayment);
        let text = present(&error).unwrap();
        assert!(text.contains("max_response_bytes=123"));
        assert!(text.contains("no partial content returned"));
        assert_eq!(text.matches("Do not retry automatically.").count(), 1);
    }

    #[test]
    fn admission_retains_challenge_status_and_signed_transport_does_not_inherit_it() {
        use crate::rotation::error::AdmissionError;
        let error = anyhow::Error::new(AdmissionError::PriceLimit("PRIVATE configuration"))
            .context(ResponseFacts::provider(StatusCode::PAYMENT_REQUIRED))
            .context(ApiExecution { non_read: false });
        assert_eq!(
            present(&error).unwrap(),
            "Provider API HTTP 402 Payment Required: payment exceeds the authorized amount."
        );
        let transport = anyhow::Error::new(AdmissionError::OutcomeUnknown(
            "signed request transport failed",
        ))
        .context(PossiblePayment)
        .context(ApiExecution { non_read: false });
        assert_eq!(
            present(&transport).unwrap(),
            "Payment outcome unknown. Do not retry automatically."
        );
        assert!(transport.is::<AdmissionError>());
        let conversion = anyhow::anyhow!("PRIVATE invalid image")
            .context(ResponseFacts {
                provider: Some(StatusCode::CREATED),
                destination: Some(StatusCode::OK),
                ..Default::default()
            })
            .context(PossiblePayment);
        assert_eq!(
            present(&conversion).unwrap(),
            "Provider API HTTP 201 Created; destination HTTP 200 OK: response unavailable. Do not retry automatically."
        );
    }
}
