use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use tracing::instrument::WithSubscriber;

#[test]
fn header_claims_are_projected_without_prose_or_ambiguous_values() {
    let read = |entries: &[(&str, String)]| {
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in entries {
            headers.append(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        HeaderFacts::read(
            &headers,
            &["payment-response", "x-payment-response"],
            "errorReason",
        )
    };
    assert_eq!(read(&[]).state, "absent");
    for value in [
        "PRIVATE_HEADER".to_owned(),
        STANDARD.encode("[]"),
        STANDARD.encode("null"),
    ] {
        let facts = read(&[("payment-response", value)]);
        assert_eq!(facts.state, "malformed");
        assert_eq!(facts.error, None);
    }
    let value = STANDARD.encode(json!({"success":false,"errorReason":"insufficient_funds","transaction":"PRIVATE_TRANSACTION","payer":"PRIVATE_PAYER"}).to_string());
    let facts = read(&[("x-payment-response", value.clone())]);
    assert_eq!(facts.error, Some("insufficient_funds"));
    assert_eq!(facts.seller_success, Some(false));
    assert!(!format!("{facts:?}").contains("PRIVATE"));
    for names in [
        ["payment-response", "payment-response"],
        ["payment-response", "x-payment-response"],
    ] {
        let facts = read(&[(names[0], value.clone()), (names[1], value.clone())]);
        assert_eq!(facts.state, "duplicate");
        assert_eq!(facts.error, None);
        assert_eq!(facts.seller_success, None);
    }
    let value = STANDARD
        .encode(json!({"success":"PRIVATE_SUCCESS","errorReason":"PRIVATE_REASON"}).to_string());
    let facts = read(&[("payment-response", value)]);
    assert_eq!(facts.error, Some("unrecognized"));
    assert_eq!(facts.seller_success, None);
    assert!(!format!("{facts:?}").contains("PRIVATE"));
}

#[tokio::test]
async fn observed_failures_explain_safe_causes_at_warn_without_replay_authority() {
    if crate::test_log_capture::run_in_child(
        "payment::diagnostics::http_tests::observed_failures_explain_safe_causes_at_warn_without_replay_authority",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(log.reopen().unwrap())
        .finish();
    scope(async {
        omitted_extensions(json!({"bazaar":{"PRIVATE_EXTENSION_VALUE":true},"PRIVATE_EXTENSION_NAME":{}}).as_object().unwrap());
        omitted_extensions(json!({"sign-in-with-x":{}}).as_object().unwrap());
        let progress = SubmissionProgress::default();
        progress.mark_possible();
        for (status, body) in [
            (402, "{}"),
            (400, r#"{"tag":"INVALID_REQUEST_BODY","error":"PRIVATE_VALIDATION_PROSE","requestId":"PRIVATE_REQUEST_ID"}"#),
            (503, r#"{"error":{"code":"temporarily_unavailable","message":"PRIVATE_PROVIDER_MESSAGE"}}"#),
        ] {
            let response = axum::http::Response::builder().status(status).body(body).unwrap();
            stage("response_body");
            let result = super::super::PaidClient::unsigned().response_output(response.into(), true, false).await;
            assert!(result.is_err());
            finish(&result, &progress, false);
        }
    }).with_subscriber(subscriber).await;
    let logs = std::fs::read_to_string(log.path()).unwrap();
    let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
    assert_eq!(warnings.len(), 3, "{logs}");
    for expected in [
        "last_http_status=402",
        "response_body_kind=\"empty_object\"",
        "cause and settlement outcome remain unconfirmed",
        "last_payment_required_state=\"absent\"",
        "last_payment_response_state=\"absent\"",
    ] {
        assert!(warnings[0].contains(expected), "{logs}");
    }
    assert!(
        warnings[1].contains("provider_failure=\"INVALID_REQUEST_BODY\""),
        "{logs}"
    );
    assert!(warnings[1].contains("check the tool arguments"), "{logs}");
    assert!(
        warnings[2].contains("provider_failure=\"temporarily_unavailable\""),
        "{logs}"
    );
    for warning in warnings {
        for expected in [
            "authorization_may_charge=true",
            "do not automatically repeat",
            "bazaar: true",
            "sign_in_with_x: true",
            "other: true",
        ] {
            assert!(warning.contains(expected), "{logs}");
        }
    }
    assert!(!logs.contains("PRIVATE"), "{logs}");
}

#[tokio::test]
async fn response_facts_replace_previous_claims_and_classify_body_shapes() {
    scope(async {
        let response = axum::http::Response::builder()
            .status(402)
            .header(
                "payment-required",
                STANDARD
                    .encode(json!({"error":"invalid_signature","secret":"PRIVATE"}).to_string()),
            )
            .header(
                "payment-response",
                STANDARD.encode(
                    json!({"success":false,"errorReason":"insufficient_funds"}).to_string(),
                ),
            )
            .body("")
            .unwrap();
        super::response(&response.into());
        OBSERVATION.with(|cell| {
            assert_eq!(cell.get().payment_required.error, Some("invalid_signature"));
            assert_eq!(
                cell.get().payment_response.error,
                Some("insufficient_funds")
            );
        });
        for (body, kind) in [
            ("", "empty"),
            ("{}", "empty_object"),
            ("[]", "array"),
            ("null", "null"),
            ("true", "boolean"),
            ("42", "number"),
            ("\"PRIVATE\"", "string"),
            ("PRIVATE_HTML", "non_json"),
            ("{\"PRIVATE\":1}", "object"),
        ] {
            response_body(body.as_bytes());
            assert_eq!(
                OBSERVATION.with(|cell| cell.get().response_body_kind),
                Some(kind)
            );
        }
        let response = axum::http::Response::builder()
            .status(503)
            .body("")
            .unwrap();
        super::response(&response.into());
        OBSERVATION.with(|cell| {
            let observation = cell.get();
            assert_eq!(observation.payment_required.state, "absent");
            assert_eq!(observation.payment_response.state, "absent");
            assert_eq!(observation.response_body_kind, None);
        });
    })
    .await;
}

#[test]
fn private_receipt_projection_rejects_ambiguity_and_nonstring_fields() {
    let encode = |value: serde_json::Value| STANDARD.encode(value.to_string());
    for values in [
        vec![],
        vec![("payment-response", "PRIVATE_MALFORMED".to_owned())],
        vec![("payment-response", encode(json!([])))],
        vec![(
            "payment-response",
            encode(json!({"errorReason":{},"errorMessage":42,"payer":"PRIVATE_PAYER"})),
        )],
        vec![
            (
                "payment-response",
                encode(json!({"errorReason":"PRIVATE_A"})),
            ),
            (
                "x-payment-response",
                encode(json!({"errorReason":"PRIVATE_B"})),
            ),
        ],
        vec![
            (
                "payment-response",
                encode(json!({"errorReason":"PRIVATE_A"})),
            ),
            (
                "payment-response",
                encode(json!({"errorReason":"PRIVATE_B"})),
            ),
        ],
    ] {
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in values {
            headers.append(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        assert_eq!(payment_response_error(&headers), None);
    }
    for name in ["payment-response", "x-payment-response"] {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(name, encode(json!({"errorReason":"PRIVATE_REASON","errorMessage":"PRIVATE_MESSAGE","payer":"PRIVATE_PAYER","transaction":"PRIVATE_TRANSACTION","extensions":{"secret":"PRIVATE_EXTENSION"}})).parse().unwrap());
        assert_eq!(
            payment_response_error(&headers),
            Some(json!({"errorReason":"PRIVATE_REASON","errorMessage":"PRIVATE_MESSAGE"}))
        );
    }
}

#[tokio::test]
async fn private_payment_explanations_never_enter_logs_or_grant_replay() {
    if crate::test_log_capture::run_in_child(
        "payment::diagnostics::http_tests::private_payment_explanations_never_enter_logs_or_grant_replay",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(log.reopen().unwrap())
        .finish();
    scope(async {
        let progress = SubmissionProgress::default();
        progress.mark_possible();
        for reason in ["invalid_exact_evm_payload_signature", "PRIVATE_UNKNOWN_CODE"] {
            let response = axum::http::Response::builder().status(402)
                .header("payment-response", STANDARD.encode(json!({"success":false,"errorReason":reason,"errorMessage":"PRIVATE_EXPLANATION","payer":"PRIVATE_PAYER","transaction":"PRIVATE_TRANSACTION"}).to_string()))
                .body("{}").unwrap();
            let result = super::super::PaidClient::unsigned().response_output(response.into(), true, false).await;
            let result = super::super::with_payment_context(progress.possible(), result);
            let error = format!("{:#}", result.as_ref().unwrap_err());
            assert!(error.contains(reason));
            assert!(error.contains("PRIVATE_EXPLANATION"));
            assert!(error.contains("settlement unverified"));
            assert!(error.contains("do not automatically repeat"));
            for private in ["PRIVATE_PAYER", "PRIVATE_TRANSACTION"] { assert!(!error.contains(private), "{error}"); }
            finish(&result, &progress, false);
        }
        let response = axum::http::Response::builder().status(503)
            .body(r#"{"status":"unavailable","reason":"inline-refresh-failed","charged":false,"endpointId":"PRIVATE_ENDPOINT"}"#).unwrap();
        let result = super::super::PaidClient::unsigned().response_output(response.into(), true, false).await;
        finish(&result, &progress, false);
    }).with_subscriber(subscriber).await;
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        logs.contains("last_payment_response_error=\"invalid_exact_evm_payload_signature\""),
        "{logs}"
    );
    assert!(
        logs.contains("last_payment_response_error=\"unrecognized\""),
        "{logs}"
    );
    assert!(
        logs.contains("provider_failure=\"inline-refresh-failed\""),
        "{logs}"
    );
    assert!(
        logs.contains("could not refresh the requested data"),
        "{logs}"
    );
    assert_eq!(
        logs.lines()
            .filter(|line| line.contains("authorization_may_charge=true")
                && line.contains("do not automatically repeat"))
            .count(),
        3,
        "{logs}"
    );
    assert!(!logs.contains("PRIVATE"), "{logs}");
    assert!(!logs.contains("charged=false"), "{logs}");
}
