use super::*;
use axum::{
    Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use serde_json::json;
use std::sync::Mutex;

fn legacy() -> Value {
    json!({
        "x402Version":1,
        "accepts":[{
            "scheme":"exact","network":"base","asset":USDC,"maxAmountRequired":"10000",
            "resource":"https://localhost/patents","description":"Patent lookup",
            "mimeType":"application/json","payTo":"0x0000000000000000000000000000000000000003",
            "maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}
        }]
    })
}

#[tokio::test]
async fn provider_x402_version_selects_one_static_signing_protocol() {
    let signed = Arc::new(Mutex::new(Vec::new()));
    let seen = signed.clone();
    let app = Router::new().route(
        "/{kind}",
        get(move |Path(kind): Path<String>, headers: HeaderMap| {
            let seen = seen.clone();
            async move {
                assert!(
                    !(headers.contains_key("x-payment")
                        && headers.contains_key("payment-signature"))
                );
                for (header, version) in [("x-payment", 1), ("payment-signature", 2)] {
                    if let Some(signature) = headers.get(header) {
                        let payload: Value = serde_json::from_slice(
                            &STANDARD.decode(signature.as_bytes()).unwrap(),
                        ).unwrap();
                        assert_eq!(payload["x402Version"], version);
                        let auth = &payload["payload"]["authorization"];
                        assert_eq!(auth["value"], "10000");
                        assert_eq!(auth["to"], "0x0000000000000000000000000000000000000003");
                        crate::test_signatures::recover_exact(&payload);
                        seen.lock().unwrap().push((kind, version));
                        return "paid".into_response();
                    }
                }
                let body = legacy();
                let mut header = body.clone();
                match kind.as_str() {
                    "duplicate" => {
                        let mut headers = HeaderMap::new();
                        for _ in 0..2 {
                            headers.append("payment-required", STANDARD.encode(header.to_string()).parse().unwrap());
                        }
                        return (StatusCode::PAYMENT_REQUIRED, headers, Json(body)).into_response();
                    }
                    "v2-only" => {
                        header["x402Version"] = json!(2);
                        return (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(header.to_string()))], "seller wrapper").into_response();
                    }
                    "v1-header" => return (
                        StatusCode::PAYMENT_REQUIRED,
                        [("payment-required", STANDARD.encode(header.to_string()))],
                        "seller wrapper",
                    ).into_response(),
                    "conflict" => header["accepts"][0]["maxAmountRequired"] = json!("20000"),
                    "malformed-v2" => {
                        header["x402Version"] = json!(2);
                        header["resource"] = json!("not an object");
                    }
                    "valid-v2" => header = json!({
                        "x402Version":2,"resource":{"url":"https://localhost/valid-v2"},
                        "accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,
                            "amount":"10000","payTo":"0x0000000000000000000000000000000000000003",
                            "maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]
                    }),
                    _ => header["x402Version"] = json!(2),
                }
                (
                    StatusCode::PAYMENT_REQUIRED,
                    [("payment-required", STANDARD.encode(header.to_string()))],
                    Json(body),
                ).into_response()
            }
        }),
    );
    let (address, task) = crate::test_tls::serve(app).await;
    for (version, kind, cap, accepted) in [
        (None, "hybrid", "0.01", false),
        (Some(X402Version::V1), "hybrid", "0.01", true),
        (Some(X402Version::V1), "hybrid", "0.009999", false),
        (Some(X402Version::V1), "v1-header", "0.01", true),
        (Some(X402Version::V1), "conflict", "0.02", false),
        (Some(X402Version::V1), "duplicate", "0.01", false),
        (Some(X402Version::V1), "v2-only", "0.01", false),
        (Some(X402Version::V2), "malformed-v2", "0.01", false),
        (Some(X402Version::V2), "v1-header", "0.01", false),
        (Some(X402Version::V1), "valid-v2", "0.01", true),
        (Some(X402Version::V2), "valid-v2", "0.01", true),
    ] {
        let payer = Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars(cap).unwrap()).unwrap();
        let result = PaidClient::new(payer)
            .with_x402_version(version)
            .execute(RoutedRequest {
                method: "GET".into(),
                url: format!("https://{address}/{kind}"),
                json_media_type: None,
                headers: Default::default(),
                query: Default::default(),
                body: None,
            })
            .await;
        assert_eq!(
            result.is_ok(),
            accepted,
            "{version:?} {kind} {cap}: {result:?}"
        );
        let expected = match kind {
            "conflict" => PinError::ConflictingV1Offers,
            "duplicate" => PinError::DuplicateHeaders,
            "v2-only" => PinError::Missing(X402Version::V1),
            "v1-header" if version == Some(X402Version::V2) => PinError::Missing(X402Version::V2),
            _ => continue,
        };
        let error = result.unwrap_err();
        assert_eq!(error.downcast_ref::<PinError>(), Some(&expected));
        assert!(
            api_error_summary(&error)
                .unwrap()
                .contains(expected.reason())
        );
        let relay = crate::discovery_relay::RelayFailure::from_error(&error);
        assert_eq!(relay.kind(), expected.code());
        assert_eq!(relay.status(), Some(402));
        assert_eq!(relay.explanation(), expected.reason());
    }
    assert_eq!(
        *signed.lock().unwrap(),
        [
            ("hybrid".into(), 1),
            ("v1-header".into(), 1),
            ("valid-v2".into(), 1),
            ("valid-v2".into(), 2)
        ]
    );
    task.abort();
}
