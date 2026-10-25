//! Business headers and generated keys survive the existing payment lifecycle.
use super::*;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde_json::json;
use std::{sync::Mutex, time::Duration};
use tokio::sync::Notify;

#[derive(Clone, Copy)]
enum Outcome {
    Success,
    Rejected,
    BrokenBody,
    UnsignedSuccess,
}

#[derive(Clone)]
struct Seller {
    outcome: Outcome,
    hold_challenge: bool,
    challenged: Arc<Notify>,
    release: Arc<Notify>,
    seen: Arc<Mutex<Vec<(bool, String, String)>>>,
    authorization: Arc<Mutex<Vec<Option<String>>>>,
    signers: Arc<Mutex<Vec<String>>>,
    nonces: Arc<Mutex<Vec<String>>>,
}

async fn respond(State(seller): State<Seller>, headers: HeaderMap) -> axum::response::Response {
    let signed = headers.contains_key("payment-signature");
    seller.seen.lock().unwrap().push((
        signed,
        headers["idempotency-key"].to_str().unwrap().to_owned(),
        headers["x-quote-id"].to_str().unwrap().to_owned(),
    ));
    seller.authorization.lock().unwrap().push(
        headers
            .get("authorization")
            .map(|v| v.to_str().unwrap().to_owned()),
    );
    if matches!(seller.outcome, Outcome::UnsignedSuccess) {
        assert!(!signed);
        return "authenticated unsigned success".into_response();
    }
    if signed {
        let payload: Value = serde_json::from_slice(
            &STANDARD
                .decode(headers["payment-signature"].as_bytes())
                .unwrap(),
        )
        .unwrap();
        seller.nonces.lock().unwrap().push(
            payload["payload"]["authorization"]["nonce"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
        seller
            .signers
            .lock()
            .unwrap()
            .push(crate::test_signatures::recover_exact(&payload).to_string());
        return match seller.outcome {
            Outcome::Success => "order accepted".into_response(),
            Outcome::UnsignedSuccess => unreachable!(),
            Outcome::Rejected => (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response(),
            Outcome::BrokenBody => axum::response::Response::new(axum::body::Body::from_stream(
                futures_util::stream::once(async {
                    Err::<axum::body::Bytes, _>(std::io::Error::other("fixture body failure"))
                }),
            )),
        };
    }
    seller.challenged.notify_one();
    if seller.hold_challenge {
        seller.release.notified().await;
    }
    let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/orders"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
    )
        .into_response()
}

async fn fixture(
    outcome: Outcome,
    hold_challenge: bool,
) -> (Seller, String, tokio::task::JoinHandle<()>) {
    let seller = seller(outcome, hold_challenge);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/orders", post(respond))
        .with_state(seller.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (seller, base, server)
}

fn seller(outcome: Outcome, hold_challenge: bool) -> Seller {
    Seller {
        outcome,
        hold_challenge,
        challenged: Default::default(),
        release: Default::default(),
        seen: Default::default(),
        authorization: Default::default(),
        signers: Default::default(),
        nonces: Default::default(),
    }
}

fn client() -> PaidClient {
    // Public deterministic unfunded signer; only the localhost fixture sees it.
    PaidClient::new(
        Payer::new(&format!("{:064x}", 109), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
}

fn route(base: &str) -> RoutedRequest {
    let document = json!({"openapi":"3.0.3","paths":{"/orders":{"post":{
        "parameters":[
            {"name":"X-Quote-Id","in":"header","required":true,"schema":{"type":"string"}},
            {"name":"Idempotency-Key","in":"header","required":true,"schema":{"type":"string"}}
        ],"responses":{}
    }}}});
    let tool = crate::catalog::build_tools(&Default::default(), &document, "fixture")
        .unwrap()
        .pop()
        .unwrap();
    tool.route(
        base,
        json!({"X-Quote-Id":"quote-from-prior-call"})
            .as_object()
            .unwrap(),
    )
    .unwrap()
}

fn assert_exchange(seen: &[(bool, String, String)]) {
    assert_eq!(
        seen.len(),
        2,
        "one unsigned challenge and one signed dispatch"
    );
    assert!(!seen[0].0);
    assert!(seen[1].0);
    assert_eq!(seen[0].1, seen[1].1);
    uuid::Uuid::parse_str(&seen[0].1).expect("generated key is a UUID");
    assert_eq!(seen[0].2, "quote-from-prior-call");
    assert_eq!(seen[0].2, seen[1].2);
}

#[test]
fn direct_routed_requests_cannot_override_reserved_headers_or_inject_values() {
    for name in [
        "Authorization",
        "Host",
        "Content-Length",
        "Content-Type",
        "Connection",
        "Cookie",
        "Proxy-Authorization",
        "Payment-Signature",
        "X-Payment",
        "Payment-Required",
        "Payment-Response",
        "X-Payment-Response",
    ] {
        let request = RoutedRequest {
            method: "POST".into(),
            url: "https://api.example.com/orders".into(),
            headers: std::collections::BTreeMap::from([(name.into(), "fixture".into())]),
            query: Default::default(),
            body: None,
        };
        assert!(build_request(request).is_err(), "reserved header: {name}");
    }
    let mut request = route("https://api.example.com");
    request
        .headers
        .insert("x-quote-id".into(), "secret\r\ninjected: value".into());
    let error = build_request(request).unwrap_err();
    assert!(!format!("{error:#}").contains("secret"));

    let mut request = route("https://api.example.com");
    request.headers.insert("X-Quote-Id".into(), "other".into());
    assert!(
        build_request(request).is_err(),
        "case variants cannot create duplicate headers"
    );
}

#[tokio::test]
async fn generated_key_and_quote_survive_signing_and_new_calls_get_new_keys() {
    let (seller, base, server) = fixture(Outcome::Success, false).await;
    let client = client();
    for _ in 0..2 {
        assert_eq!(
            client.execute(route(&base)).await.unwrap(),
            "order accepted"
        );
    }
    client.drain().await;
    let seen = seller.seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    assert_exchange(&seen[..2]);
    assert_exchange(&seen[2..]);
    assert_ne!(
        seen[0].1, seen[2].1,
        "same arguments can intentionally buy twice"
    );
    server.abort();
}

#[tokio::test]
async fn idempotency_key_does_not_authorize_replay_after_signed_failure_or_uncertainty() {
    for outcome in [Outcome::Rejected, Outcome::BrokenBody] {
        let (seller, base, server) = fixture(outcome, false).await;
        let client = client();
        let progress = SubmissionProgress::default();
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            client.execute_response_observed(route(&base), &progress),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(
            progress.possible(),
            "signed failure remains potentially chargeable"
        );
        client.drain().await;
        assert_exchange(&seller.seen.lock().unwrap());
        server.abort();
    }
}

#[tokio::test]
async fn canceled_waiter_keeps_generated_key_and_quote_for_accepted_purchase() {
    let (seller, base, server) = fixture(Outcome::Success, true).await;
    let client = client();
    let request = route(&base);
    let progress = SubmissionProgress::default();
    let waiter = tokio::spawn({
        let client = client.clone();
        let progress = progress.clone();
        async move { client.execute_response_observed(request, &progress).await }
    });
    tokio::time::timeout(Duration::from_secs(10), seller.challenged.notified())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    seller.release.notify_one();
    tokio::time::timeout(Duration::from_secs(10), client.drain())
        .await
        .unwrap();
    assert!(progress.possible());
    assert_exchange(&seller.seen.lock().unwrap());
    server.abort();
}

fn manual_request(base: &str) -> (RoutedRequest, crate::discovery::raw::ManualRawHeaders) {
    let mut route = route(base);
    route.headers.clear();
    let headers = crate::discovery::raw::ManualRawHeaders::parse(Some(
        &json!({"X-Quote-Id":"quote-from-prior-call", "Idempotency-Key":"caller-owned-key", "Authorization":"Bearer manual-secret"}),
    ))
    .unwrap();
    (route, headers)
}

#[tokio::test]
async fn manual_key_survives_signing_and_reuse_is_a_separate_call() {
    let (seller, base, server) = fixture(Outcome::Success, false).await;
    let client = client();
    for _ in 0..2 {
        let (route, headers) = manual_request(&base);
        client.execute_manual_raw(route, headers).await.unwrap();
    }
    client.drain().await;
    assert_eq!(
        *seller.authorization.lock().unwrap(),
        vec![Some("Bearer manual-secret".into()); 4]
    );
    let nonces = seller.nonces.lock().unwrap();
    assert_eq!(nonces.len(), 2);
    assert_ne!(
        nonces[0], nonces[1],
        "key reuse is a separately chargeable authorization"
    );
    let seen = seller.seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    for pair in seen.chunks(2) {
        assert!(!pair[0].0 && pair[1].0);
        assert_eq!(pair[0].1, "caller-owned-key");
        assert_eq!(pair[0].1, pair[1].1);
        assert_eq!(pair[0].2, pair[1].2);
    }
    server.abort();
}

#[tokio::test]
async fn manual_headers_do_not_replay_signed_failures_or_uncertainty() {
    for outcome in [Outcome::Rejected, Outcome::BrokenBody] {
        let (seller, base, server) = fixture(outcome, false).await;
        let client = client();
        let (route, headers) = manual_request(&base);
        let progress = SubmissionProgress::default();
        let (result, logs) = crate::limits::tests::capture(client.execute_finished(
            route,
            headers,
            &progress,
            None,
            |output| Ok((output, true)),
        ))
        .await;
        assert!(result.is_err());
        assert!(progress.possible());
        assert!(!logs.contains("manual-secret"));
        assert!(!format!("{:#}", result.as_ref().unwrap_err()).contains("manual-secret"));
        assert!(!logs.contains("caller-owned-key"));
        assert!(!logs.contains("quote-from-prior-call"));
        client.drain().await;
        assert_eq!(seller.seen.lock().unwrap().len(), 2);
        server.abort();
    }
}

#[tokio::test]
async fn manual_authorization_unsigned_success_has_no_signature_or_automatic_injection() {
    let (seller, base, server) = fixture(Outcome::UnsignedSuccess, false).await;
    let client = client();
    let (request, headers) = manual_request(&base);
    let (result, logs) =
        crate::limits::tests::capture(client.execute_manual_raw(request, headers)).await;
    assert!(
        result
            .unwrap()
            .text
            .contains("authenticated unsigned success")
    );
    assert!(!logs.contains("manual-secret"));
    client.execute(route(&base)).await.unwrap();
    client.drain().await;
    assert_eq!(
        *seller.authorization.lock().unwrap(),
        vec![Some("Bearer manual-secret".into()), None]
    );
    assert!(seller.signers.lock().unwrap().is_empty());
    assert_eq!(seller.seen.lock().unwrap().len(), 2);
    server.abort();
}

#[tokio::test]
async fn manual_authorization_survives_cancellation_and_static_payer_replacement() {
    let (seller, base, server) = fixture(Outcome::Success, true).await;
    let client = client();
    let original = Payer::new(&format!("{:064x}", 109), SpendPolicy::dollars("1").unwrap())
        .unwrap()
        .address;
    let replacement =
        Payer::new(&format!("{:064x}", 110), SpendPolicy::dollars("1").unwrap()).unwrap();
    let replacement_address = replacement.address.clone();
    let (request, headers) = manual_request(&base);
    let waiter = tokio::spawn({
        let client = client.clone();
        async move { client.execute_manual_raw(request, headers).await }
    });
    tokio::time::timeout(Duration::from_secs(10), seller.challenged.notified())
        .await
        .unwrap();
    client.replace_payer(replacement);
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    seller.release.notify_one();
    // Completion of the retained owner must include the original signed send.
    tokio::time::timeout(Duration::from_secs(10), client.drain())
        .await
        .unwrap();
    assert_eq!(*seller.signers.lock().unwrap(), vec![original]);
    assert_eq!(
        *seller.authorization.lock().unwrap(),
        vec![Some("Bearer manual-secret".into()); 2]
    );
    assert_ne!(seller.signers.lock().unwrap()[0], replacement_address);
    assert_eq!(seller.seen.lock().unwrap().len(), 2);
    server.abort();
}

#[tokio::test]
async fn concurrent_manual_credentials_and_keys_stay_with_each_signed_request() {
    let seller = seller(Outcome::Success, false);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let app = Router::new().route(
        "/orders",
        post({
            let seller = seller.clone();
            let observed = observed.clone();
            let barrier = barrier.clone();
            move |headers: HeaderMap| {
                let seller = seller.clone();
                let observed = observed.clone();
                let barrier = barrier.clone();
                async move {
                    let signed = headers.contains_key("payment-signature");
                    observed.lock().unwrap().push((
                        headers["authorization"].to_str().unwrap().to_owned(),
                        headers["idempotency-key"].to_str().unwrap().to_owned(),
                        headers["x-quote-id"].to_str().unwrap().to_owned(),
                        signed,
                    ));
                    if !signed {
                        barrier.wait().await;
                    }
                    respond(State(seller), headers).await
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = client();
    let cases = [
        ("Bearer caller-a", "key-a", "quote-a"),
        ("Bearer caller-b", "key-b", "quote-b"),
    ];
    let mut calls = Vec::new();
    for (auth, key, quote) in cases {
        let mut request = route(&base);
        request.headers.clear();
        let headers = crate::discovery::raw::ManualRawHeaders::parse(Some(&json!({
            "Authorization":auth, "Idempotency-Key":key, "X-Quote-Id":quote
        })))
        .unwrap();
        let client = client.clone();
        calls.push(tokio::spawn(async move {
            client.execute_manual_raw(request, headers).await
        }));
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        // Both calls have disclosed their distinct unsigned headers before either
        // receives a challenge, ensuring overlapping retained operations.
        barrier.wait().await;
        for call in calls {
            assert_eq!(call.await.unwrap().unwrap().text, "order accepted");
        }
        client.drain().await;
    })
    .await
    .expect("concurrent manual calls stalled");
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 4);
    for (auth, key, quote) in cases {
        for signed in [false, true] {
            assert_eq!(
                observed
                    .iter()
                    .filter(|entry| **entry == (auth.into(), key.into(), quote.into(), signed))
                    .count(),
                1
            );
        }
    }
    let nonces = seller.nonces.lock().unwrap();
    assert_eq!(nonces.len(), 2);
    assert_ne!(
        nonces[0], nonces[1],
        "concurrent calls own separate payment authorizations"
    );
    server.abort();
}
