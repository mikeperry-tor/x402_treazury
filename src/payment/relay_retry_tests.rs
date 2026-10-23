//! Relay legs return typed terminal delays without draining rejected error bodies.
use super::*;
use crate::network::acquisition::{Failure, FailureKind};
use axum::{Router, routing::post};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

#[tokio::test]
async fn direct_relay_failure_retains_delay_and_drops_body_after_one_unsigned_attempt() {
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/relay", listener.local_addr().unwrap());
    let app = Router::new().route(
        "/relay",
        post({
            let calls = calls.clone();
            move |headers: axum::http::HeaderMap| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    assert!(!headers.contains_key("payment-signature"));
                    assert!(!headers.contains_key("x-payment"));
                    axum::http::Response::builder()
                        .status(503)
                        .header("retry-after", "18446744073709551615")
                        .body(axum::body::Body::from_stream(
                            futures_util::stream::pending::<
                                Result<axum::body::Bytes, std::io::Error>,
                            >(),
                        ))
                        .unwrap()
                }
            }
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = PaidClient::unsigned().for_relay();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        client.execute_response(RoutedRequest {
            method: "POST".into(),
            url,
            query: Default::default(),
            body: Some(serde_json::json!({})),
        }),
    )
    .await
    .expect("relay error body or terminal server delay blocked returning the failure");
    let error = result.unwrap_err();
    let failure = error
        .downcast_ref::<Failure>()
        .expect("relay acquisition failure retains its typed metadata");
    assert_eq!(
        failure.kind,
        FailureKind::HttpStatus(reqwest::StatusCode::SERVICE_UNAVAILABLE)
    );
    assert!(matches!(
        failure.retry_after,
        Some(crate::network::RetryAfter::BeyondClock)
    ));
    assert_eq!(
        failure.error.downcast_ref::<HttpFailure>().unwrap().status,
        503
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    client.drain().await;
    task.abort();
}

#[test]
fn relay_attempt_capacity_is_released_during_retry_after_in_isolated_process() {
    const CHILD: &str = "TREAZURY_RELAY_CAPACITY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(capacity_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "payment::relay_retry_tests::relay_attempt_capacity_is_released_during_retry_after_in_isolated_process",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn capacity_fixture() {
    use crate::network::{Mode, NetworkContext, NetworkPolicy};
    use tokio_util::sync::CancellationToken;

    let calls = Arc::new(AtomicUsize::new(0));
    let reached = CancellationToken::new();
    let release = CancellationToken::new();
    let app = Router::new().route(
        "/relay",
        post({
            let calls = calls.clone();
            let reached = reached.clone();
            let release = release.clone();
            move |headers: axum::http::HeaderMap| {
                let calls = calls.clone();
                let reached = reached.clone();
                let release = release.clone();
                async move {
                    assert!(!headers.contains_key("payment-signature"));
                    assert!(!headers.contains_key("x-payment"));
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        reached.cancel();
                        release.cancelled().await;
                        axum::http::Response::builder()
                            .status(503)
                            .header("retry-after", "1")
                            .body(axum::body::Body::empty())
                            .unwrap()
                    } else {
                        axum::http::Response::builder()
                            .status(200)
                            .body(axum::body::Body::from("complete"))
                            .unwrap()
                    }
                }
            }
        }),
    );
    let (address, server) = crate::test_tls::serve(app).await;
    let proxy = crate::test_socks::Socks::start(
        std::collections::BTreeMap::from([("spec.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            tor_isolation_retries: Some(1),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let gate = Arc::new(tokio::sync::Semaphore::new(1));
    let client = PaidClient::unsigned()
        .for_relay()
        .with_attempt_capacity(std::slice::from_ref(&gate));
    let work = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .execute_response(RoutedRequest {
                    method: "POST".into(),
                    url: "https://spec.example.com/relay".into(),
                    query: Default::default(),
                    body: Some(serde_json::json!({})),
                })
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), reached.cancelled())
        .await
        .unwrap();
    assert!(
        gate.try_acquire().is_err(),
        "active HTTP attempt owns existing capacity"
    );
    release.cancel();
    let available = tokio::time::timeout(Duration::from_millis(500), gate.acquire())
        .await
        .expect("retry delay must release capacity before the next HTTP attempt")
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "capacity becomes available during Retry-After"
    );
    drop(available);
    let output = tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(output.bytes, b"complete");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one configured origin sequence"
    );
    assert_eq!(gate.available_permits(), 1);
    client.drain().await;
    server.abort();
}

#[test]
fn relay_outer_partial_responses_never_replay_or_wait_for_body_in_isolated_process() {
    const CHILD: &str = "TREAZURY_RELAY_PARTIAL_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(partial_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "payment::relay_retry_tests::relay_outer_partial_responses_never_replay_or_wait_for_body_in_isolated_process",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn partial_fixture() {
    use crate::network::{Mode, NetworkContext, NetworkPolicy};
    use axum::{extract::State, response::IntoResponse};
    use futures_util::StreamExt;

    #[derive(Clone)]
    struct PartialCase {
        paid: bool,
        status: u16,
        range_header: bool,
        forever: bool,
        calls: Arc<std::sync::Mutex<Vec<bool>>>,
    }
    async fn respond(
        State(case): State<PartialCase>,
        uri: axum::http::Uri,
        headers: axum::http::HeaderMap,
    ) -> axum::response::Response {
        use base64::Engine;
        let signature = headers.get("payment-signature");
        assert!(!headers.contains_key("x-payment"));
        assert!(
            !headers.contains_key("range"),
            "relay envelope POST does not request partial content"
        );
        case.calls.lock().unwrap().push(signature.is_some());
        if case.paid && signature.is_none() {
            let challenge = serde_json::json!({"x402Version":2,"resource":{"url":format!("https://spec.example.com{uri}")},"accepts":[{
                "scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,
                "amount":"1","payTo":"0x0000000000000000000000000000000000000003",
                "maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}
            }]});
            return (
                axum::http::StatusCode::PAYMENT_REQUIRED,
                [(
                    "payment-required",
                    STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
                )],
                "",
            )
                .into_response();
        }
        if let Some(signature) = signature {
            assert!(case.paid);
            let payload: Value =
                serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
            crate::test_signatures::recover_exact(&payload);
        }
        let mut response = axum::http::Response::builder().status(case.status);
        if case.range_header {
            response = response.header("content-range", "bytes 0-1/999");
        }
        let body = if case.forever {
            // A syntactically complete JSON value does not establish complete
            // envelope delivery; the HTTP body continues forever.
            axum::body::Body::from_stream(
                futures_util::stream::once(async {
                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"{}"))
                })
                .chain(futures_util::stream::pending()),
            )
        } else {
            axum::body::Body::from("ordinary range body")
        };
        response.body(body).unwrap()
    }

    let mut routes = Router::new();
    let mut cases = Vec::new();
    for paid in [false, true] {
        for (status, range_header) in [(206, false), (200, true)] {
            let case = PartialCase {
                paid,
                status,
                range_header,
                forever: true,
                calls: Default::default(),
            };
            let path = format!("/partial-{paid}-{status}");
            routes = routes.route(&path, post(respond).with_state(case.clone()));
            cases.push((path, case));
        }
    }
    let ordinary = PartialCase {
        paid: false,
        status: 206,
        range_header: true,
        forever: false,
        calls: Default::default(),
    };
    routes = routes.route("/ordinary", post(respond).with_state(ordinary.clone()));
    let (address, server) = crate::test_tls::serve(routes).await;
    let proxy = crate::test_socks::Socks::start(
        std::collections::BTreeMap::from([("spec.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            tor_isolation_retries: Some(2),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    for (path, case) in cases {
        let payer = Payer::new(
            &format!("{:064x}", 107),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap();
        let client = PaidClient::new(payer).for_relay();
        let progress = SubmissionProgress::default();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            client.execute_response_observed(
                RoutedRequest {
                    method: "POST".into(),
                    url: format!("https://spec.example.com{path}"),
                    query: Default::default(),
                    body: Some(serde_json::json!({})),
                },
                &progress,
            ),
        )
        .await
        .expect("partial relay envelope must be rejected from headers without draining its body");
        let error = result.unwrap_err();
        let failure = error.downcast_ref::<Failure>();
        let cause = failure.map(|failure| &failure.error).unwrap_or(&error);
        assert!(cause.is::<IncompleteRelayResponse>(), "{error:#}");
        if let Some(failure) = failure {
            assert!(!failure.retry && !failure.fallback);
        }
        assert_eq!(
            progress.possible(),
            case.paid,
            "only signed delivery retains possible payment evidence"
        );
        assert_eq!(
            *case.calls.lock().unwrap(),
            if case.paid {
                vec![false, true]
            } else {
                vec![false]
            },
            "partial unsigned or signed envelopes cannot earn another attempt"
        );
        client.drain().await;
    }
    let client = PaidClient::unsigned();
    let output = client
        .execute_response(RoutedRequest {
            method: "POST".into(),
            url: "https://spec.example.com/ordinary".into(),
            query: Default::default(),
            body: None,
        })
        .await
        .unwrap();
    assert!(!output.paid_submission);
    assert_eq!(output.bytes, b"ordinary range body");
    assert_eq!(*ordinary.calls.lock().unwrap(), [false]);
    client.drain().await;
    server.abort();
}
