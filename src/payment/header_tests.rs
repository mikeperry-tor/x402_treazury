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
}

#[derive(Clone)]
struct Seller {
    outcome: Outcome,
    hold_challenge: bool,
    challenged: Arc<Notify>,
    release: Arc<Notify>,
    seen: Arc<Mutex<Vec<(bool, String, String)>>>,
}

async fn respond(State(seller): State<Seller>, headers: HeaderMap) -> axum::response::Response {
    let signed = headers.contains_key("payment-signature");
    seller.seen.lock().unwrap().push((
        signed,
        headers["idempotency-key"].to_str().unwrap().to_owned(),
        headers["x-quote-id"].to_str().unwrap().to_owned(),
    ));
    if signed {
        return match seller.outcome {
            Outcome::Success => "order accepted".into_response(),
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
    let seller = Seller {
        outcome,
        hold_challenge,
        challenged: Default::default(),
        release: Default::default(),
        seen: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/orders", post(respond))
        .with_state(seller.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (seller, base, server)
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
