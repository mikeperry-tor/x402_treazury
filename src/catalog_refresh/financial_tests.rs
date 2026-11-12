//! A sibling's catalog failure cannot revoke accepted signed payment work.
use super::*;
use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::FutureExt;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

#[tokio::test]
async fn source_expiry_preserves_inflight_signed_payment_without_replay() {
    let unsigned = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let gone = Arc::new(AtomicUsize::new(0));
    let paid_status = Arc::new(std::sync::atomic::AtomicU16::new(200));
    let oversized_body = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let submitted = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = Router::new()
        .route(
            "/paid",
            get({
                let oversized_body = oversized_body.clone();
                let paid_status = paid_status.clone();
                let unsigned = unsigned.clone();
                let signed = signed.clone();
                let submitted = submitted.clone();
                let release = release.clone();
                move |headers: HeaderMap| {
                    let oversized_body = oversized_body.clone();
                    let paid_status = paid_status.clone();
                    let unsigned = unsigned.clone();
                    let signed = signed.clone();
                    let submitted = submitted.clone();
                    let release = release.clone();
                    async move {
                        if let Some(signature) = headers.get("payment-signature") {
                            let signature: serde_json::Value = serde_json::from_slice(
                                &STANDARD.decode(signature.as_bytes()).unwrap(),
                            )
                            .unwrap();
                            assert_eq!(signature["accepted"]["amount"], "100");
                            assert!(
                                signature["payload"]["signature"]
                                    .as_str()
                                    .is_some_and(|value| !value.is_empty())
                            );
                            signed.fetch_add(1, Ordering::SeqCst);
                            submitted.notify_one();
                            release.notified().await;
                            return (
                                StatusCode::from_u16(paid_status.load(Ordering::SeqCst)).unwrap(),
                                if oversized_body.load(Ordering::SeqCst) {
                                    "x".repeat(2048)
                                } else {
                                    "original paid success".into()
                                },
                            )
                                .into_response();
                        }
                        unsigned.fetch_add(1, Ordering::SeqCst);
                        let challenge = json!({
                            "x402Version":2,
                            "resource":{"url":"https://localhost/paid"},
                            "accepts":[{
                                "scheme":"exact", "network":"eip155:8453",
                                "asset":crate::payment::USDC, "amount":"100",
                                "payTo":"0x0000000000000000000000000000000000000003",
                                "maxTimeoutSeconds":60,
                                "extra":{"name":"USD Coin","version":"2"}
                            }]
                        });
                        (
                            StatusCode::PAYMENT_REQUIRED,
                            [(
                                "payment-required",
                                STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
                            )],
                        )
                            .into_response()
                    }
                }
            }),
        )
        .route(
            "/gone",
            get({
                let gone = gone.clone();
                move |headers: HeaderMap| {
                    gone.fetch_add(1, Ordering::SeqCst);
                    assert!(!headers.contains_key("payment-signature"));
                    async { (StatusCode::NOT_FOUND, "missing") }
                }
            }),
        );
    let (address, http) = crate::test_tls::serve(app).await;
    // Public, deterministic, unfunded fixture key; no managed treasury exists.
    let client = crate::payment::PaidClient::new(
        crate::payment::Payer::new(
            &format!("{:064x}", 1),
            crate::payment::SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    );
    let tools = crate::catalog::build_tools(
        &crate::catalog::Config::default(),
        &json!({"paths":{"/paid":{"get":{}},"/gone":{"get":{}}}}),
        "fixture",
    )
    .unwrap();
    let mut bound = crate::catalog_state::bind(
        tools
            .into_iter()
            .map(|tool| (tool, client.clone(), format!("https://{address}")))
            .collect(),
    );
    let callbacks = Arc::new(AtomicUsize::new(0));
    let seen = callbacks.clone();
    let binding = Binding::new(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    for tool in &mut bound {
        tool.refresh = Some(binding.clone());
    }
    let paid = bound
        .iter()
        .find(|tool| tool.tool.path == "/paid")
        .unwrap()
        .clone();
    let gone_tool = bound
        .iter()
        .find(|tool| tool.tool.path == "/gone")
        .unwrap()
        .clone();
    let accepted = tokio::spawn({
        let paid = paid.clone();
        async move { paid.invoke_output(&Default::default()).await }
    });
    submitted.notified().await;
    assert!(gone_tool.invoke_output(&Default::default()).await.is_err());
    assert!(binding.refreshing());
    assert_eq!(callbacks.load(Ordering::SeqCst), 1);
    release.notify_one();
    let output = accepted.await.unwrap().unwrap();
    assert_eq!(output.text, "original paid success");
    assert!(output.payment_may_have_occurred);
    assert!(binding.check().is_ok());
    // A signed application-level miss must not start another refresh, even
    // after the previous refresh has failed and released its coalescing claim.
    binding.fail();
    for status in [404, 405, 410] {
        paid_status.store(status, Ordering::SeqCst);
        release.notify_one();
        let error = paid.invoke_output(&Default::default()).await.unwrap_err();
        assert!(
            !crate::payment::ResponseFacts::from_error(&error)
                .unwrap()
                .unsigned_target
        );
        assert!(!binding.refreshing());
    }
    let mut bounded = paid.clone();
    bounded.client = bounded.client.with_download_limits(1024, 4096);
    paid_status.store(404, Ordering::SeqCst);
    oversized_body.store(true, Ordering::SeqCst);
    release.notify_one();
    let error = bounded
        .invoke_output(&Default::default())
        .await
        .unwrap_err();
    assert!(error.is::<crate::limits::DownloadLimit>());
    assert!(
        !crate::payment::ResponseFacts::from_error(&error)
            .unwrap()
            .unsigned_target
    );
    assert!(!binding.refreshing());
    let mut bounded_unsigned = gone_tool.clone();
    bounded_unsigned.client = bounded_unsigned.client.with_download_limits(1, 4096);
    let error = bounded_unsigned
        .invoke_output(&Default::default())
        .await
        .unwrap_err();
    assert!(error.is::<crate::limits::DownloadLimit>());
    assert!(
        crate::payment::ResponseFacts::from_error(&error)
            .unwrap()
            .unsigned_target
    );
    assert!(binding.refreshing());
    // Successful replacement revokes new admission while an already submitted
    // signed request still owns its original response and must finish once.
    paid_status.store(200, Ordering::SeqCst);
    oversized_body.store(false, Ordering::SeqCst);
    // Earlier requests left notification permits; consume them before the held phase.
    while submitted.notified().now_or_never().is_some() {}
    let accepted = tokio::spawn({
        let paid = paid.clone();
        async move { paid.invoke_output(&Default::default()).await }
    });
    submitted.notified().await;
    let catalog = crate::catalog_state::CatalogState::default();
    catalog.publish_retiring(Default::default(), Some(&binding));
    assert!(binding.check().is_err());
    assert!(paid.invoke_output(&Default::default()).await.is_err());
    assert!(gone_tool.invoke_output(&Default::default()).await.is_err());
    release.notify_one();
    let output = accepted.await.unwrap().unwrap();
    assert_eq!(output.text, "original paid success");
    assert!(output.payment_may_have_occurred);
    client.drain().await;
    assert_eq!(unsigned.load(Ordering::SeqCst), 6);
    assert_eq!(signed.load(Ordering::SeqCst), 6);
    assert_eq!(gone.load(Ordering::SeqCst), 2);
    assert_eq!(callbacks.load(Ordering::SeqCst), 2);
    http.abort();
    let _ = http.await;
}
