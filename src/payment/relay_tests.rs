//! API fallback integration: actual target bytes, unsigned recovery and paid ownership.
use super::*;
use crate::{
    discovery_relay::SearchRelay,
    network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Mutex, time::Duration};
use tokio_util::sync::CancellationToken;
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP8sAAAAASUVORK5CYII=";

#[derive(Clone, Debug)]
struct Event {
    case: String,
    provider: Option<usize>,
    method: Method,
    uri: String,
    body: Vec<u8>,
    headers: HeaderMap,
    signer: Option<String>,
}
#[derive(Clone, Default)]
struct Fixture {
    events: Arc<Mutex<Vec<Event>>>,
    signed_held: CancellationToken,
    signed_release: CancellationToken,
    origin_held: CancellationToken,
    origin_release: CancellationToken,
}
impl Fixture {
    fn events(&self, case: &str) -> Vec<Event> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.case == case)
            .cloned()
            .collect()
    }
}
fn signer(headers: &HeaderMap) -> Option<String> {
    assert!(!headers.contains_key("x-payment"));
    headers.get("payment-signature").map(|encoded| {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(encoded.as_bytes()).unwrap()).unwrap();
        crate::test_signatures::recover_exact(&payload)
            .to_string()
            .to_ascii_lowercase()
    })
}
fn challenge(url: &str) -> Response {
    let challenge = json!({"x402Version":2,"resource":{"url":url},"accepts":[{
        "scheme":"exact","network":"eip155:8453","asset":USDC,
        "amount":"1","payTo":"0x0000000000000000000000000000000000000003",
        "maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}
    }]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
        "",
    )
        .into_response()
}
fn stalled(status: StatusCode) -> Response {
    (
        status,
        axum::body::Body::from_stream(
            futures_util::stream::once(async {
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"{"))
            })
            .chain(futures_util::stream::pending()),
        ),
    )
        .into_response()
}
fn interrupted(status: StatusCode) -> Response {
    (
        status,
        axum::body::Body::from_stream(
            futures_util::stream::once(async {
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"known status body"))
            })
            .chain(futures_util::stream::once(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
            })),
        ),
    )
        .into_response()
}
fn scenario(case: &str) -> &str {
    case.split_once('-').unwrap().1
}
fn request_url(uri: &Uri, origin: &str) -> String {
    // HTTP/2 may supply an absolute URI. Keep its authority assertion separate
    // from the exact path/query comparison rather than prepending another host.
    if let Some(authority) = uri.authority() {
        assert_eq!(uri.scheme_str(), Some("https"));
        assert_eq!(authority.as_str(), origin.trim_start_matches("https://"));
    }
    format!("{origin}{}", uri.path_and_query().unwrap().as_str())
}
async fn origin(
    State(fixture): State<Fixture>,
    Path(case): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let target = request_url(&uri, "https://api.example.com");
    let signed = signer(&headers);
    fixture.events.lock().unwrap().push(Event {
        case: case.clone(),
        provider: None,
        method,
        uri: uri.path_and_query().unwrap().as_str().to_owned(),
        body: body.to_vec(),
        headers,
        signer: signed.clone(),
    });
    let kind = scenario(&case);
    let kind = kind.strip_prefix("manual-").unwrap_or(kind);
    if kind.starts_with("origin-paid") || kind.starts_with("origin-signed") {
        if signed.is_none() {
            return challenge(&target);
        }
        return match kind {
            "origin-signed400" => (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({"tag":"INVALID_REQUEST_BODY","error":"Invalid body"})),
            )
                .into_response(),
            "origin-signed402" => (StatusCode::PAYMENT_REQUIRED, "{}").into_response(),
            "origin-signed503" => (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(json!({"error":{"code":"temporarily_unavailable"}})),
            )
                .into_response(),
            "origin-signed-stall" => stalled(StatusCode::OK),
            _ => ([("content-type", "text/plain")], "target paid").into_response(),
        };
    }
    assert!(
        signed.is_none(),
        "unsigned fallback must never sign the original blocked target"
    );
    if let Some(status) = kind.strip_prefix("status") {
        return StatusCode::from_u16(status.parse().unwrap())
            .unwrap()
            .into_response();
    }
    match kind {
        "healthy" => ([("content-type", "text/plain")], "origin healthy").into_response(),
        "empty" => StatusCode::NO_CONTENT.into_response(),
        "range" => (
            StatusCode::PARTIAL_CONTENT,
            [
                ("content-type", "text/plain"),
                ("content-range", "bytes 0-4/9"),
            ],
            "range",
        )
            .into_response(),
        "origin-json-invalid" => {
            ([("content-type", "application/json")], "not JSON").into_response()
        }
        "origin-image-invalid" => ([("content-type", "image/png")], "not a PNG").into_response(),
        "large" => ([("content-type", "text/plain")], "x".repeat(100)).into_response(),
        "body-stall" => stalled(StatusCode::OK),
        "body-reset" => interrupted(StatusCode::OK),
        "terminal404-stall" => stalled(StatusCode::NOT_FOUND),
        "terminal401-reset" => interrupted(StatusCode::UNAUTHORIZED),
        "recover" if fixture.events(&case).len() == 1 => StatusCode::FORBIDDEN.into_response(),
        "recover" => ([("content-type", "text/plain")], "origin recovered").into_response(),
        "header-stall" => std::future::pending().await,
        "hold-origin" => {
            fixture.origin_held.cancel();
            fixture.origin_release.cancelled().await;
            StatusCode::FORBIDDEN.into_response()
        }
        _ => StatusCode::FORBIDDEN.into_response(),
    }
}
async fn relay(
    State(fixture): State<Fixture>,
    Path(provider): Path<usize>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    assert_eq!(method, Method::POST);
    assert!(
        !headers.contains_key("authorization"),
        "manual auth must never reach a relay"
    );
    let endpoint = request_url(&uri, "https://spec.example.com");
    let request: Value = serde_json::from_slice(&body).unwrap();
    let target = request["url"].as_str().unwrap();
    assert!(
        target.starts_with("https://api.example.com/target/"),
        "relay recursion or changed target URL"
    );
    let url = reqwest::Url::parse(target).unwrap();
    let case = url.path().rsplit('/').next().unwrap().to_owned();
    let kind = scenario(&case);
    let signed = signer(&headers);
    fixture.events.lock().unwrap().push(Event {
        case: case.clone(),
        provider: Some(provider),
        method,
        uri: uri.path_and_query().unwrap().as_str().to_owned(),
        body: body.to_vec(),
        headers,
        signer: signed.clone(),
    });
    if signed.is_none() {
        let unsigned = fixture
            .events(&case)
            .iter()
            .filter(|event| event.provider == Some(provider) && event.signer.is_none())
            .count();
        if kind == "relay-unsigned503" && provider == 0 && unsigned == 1 {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [("retry-after", "0")],
                "unused",
            )
                .into_response();
        }
        return challenge(&endpoint);
    }
    if kind == "held" {
        fixture.signed_held.cancel();
        fixture.signed_release.cancelled().await;
    }
    if kind == "uncertain" {
        return (StatusCode::BAD_GATEWAY, "PRIVATE_RELAY_ERROR").into_response();
    }
    if kind == "malformed-envelope" {
        return axum::Json(json!({"not":"an attributable envelope"})).into_response();
    }
    let head = request["method"] == "HEAD";
    let status = match kind {
        "relayed402" => 402,
        "relayed404" => 404,
        "relayed503" if provider == 0 => 503,
        _ => 200,
    };
    let image = kind.starts_with("image-");
    let binary = kind == "binary" || kind.starts_with("binary-") || image;
    let mime = if image {
        "image/png"
    } else if binary {
        "application/octet-stream"
    } else if kind == "mapped-invalid" {
        "application/json"
    } else {
        "text/plain"
    };
    let mut response_headers = json!({"content-type":mime});
    if status == 402 {
        response_headers["payment-required"] = json!("must-not-enter-target-signing");
    }
    let text = if head {
        ""
    } else if kind == "mapped-invalid" {
        "{\"text\":\"no image\"}"
    } else {
        "relay result"
    };
    let final_url = if kind == "private-final" {
        "https://127.0.0.1/private"
    } else if kind == "other-origin" {
        "https://other.example.com/result"
    } else {
        target
    };
    let envelope = match provider {
        0 => json!({"url":target,"method":request["method"],"statusCode":status,
            "headers":response_headers,"body":text,"redirectChain":if final_url == target {vec![]} else {vec![final_url]},
            "truncated":kind.ends_with("-truncated"),"error":if kind.ends_with("-failed") {Some("delivery_failed")} else {None}}),
        1 => {
            let mut row = json!({"status":status,"final_url":final_url,"headers":response_headers,"truncated":false});
            if binary && !head {
                row["body_base64"] = json!(if image { PNG } else { "AP8=" });
            } else {
                row["body"] = json!(text);
            }
            json!([row])
        }
        _ => json!({"ok":true,"status":status,"finalUrl":final_url,"headers":response_headers,
            "body":if binary && !head {"AP8="} else {text},"bodyEncoding":if binary && !head {"base64"} else {"text"},"truncated":false}),
    };
    axum::Json(envelope).into_response()
}

fn route(method: &str, kind: &str) -> RoutedRequest {
    RoutedRequest {
        headers: Default::default(),
        method: method.into(),
        url: format!("https://api.example.com/target/{method}-{kind}?literal=%2F%20"),
        query: BTreeMap::from([("q".into(), json!(["one two", "x+y"]))]),
        body: (method == "POST").then(|| json!({"exact":"line\n", "n":1})),
    }
}
fn payer(key: u64) -> Payer {
    Payer::new(
        &format!("{key:064x}"),
        SpendPolicy::dollars("0.01").unwrap(),
    )
    .unwrap()
}
fn attached(relay: Arc<SearchRelay>) -> PaidClient {
    PaidClient::new(payer(181)).with_document_relay(Some(relay))
}
fn relay_fixture() -> Arc<SearchRelay> {
    let entry = |format: &str, provider| {
        (
            format.into(),
            format!("https://spec.example.com/relay/{provider}"),
            crate::catalog::Config::default(),
        )
    };
    Arc::new(
        SearchRelay::fixture_methods(BTreeMap::from([
            ("GET".into(), vec![entry("curl", 0), entry("agproxy", 1)]),
            ("HEAD".into(), vec![entry("horizonpulse", 2)]),
            ("POST".into(), vec![entry("curl", 0), entry("agproxy", 1)]),
        ]))
        .unwrap()
        .with_policy(crate::discovery_relay::Policy {
            api_requests: true,
            ..Default::default()
        }),
    )
}

#[test]
fn unsigned_api_relay_policy_and_financial_ownership_in_isolated_processes() {
    const CHILD: &str = "TREAZURY_API_RELAY_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture(&mode));
        return;
    }
    for mode in [
        "direct",
        "tor0",
        "tor1",
        "tor-default",
        "tor-auth",
        "tor-local-refused",
    ] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .env_clear().envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
            .args(["--exact", "payment::relay_tests::unsigned_api_relay_policy_and_financial_ownership_in_isolated_processes", "--nocapture"])
            .env(CHILD, mode).output().unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
struct ResolveLocal(std::net::SocketAddr);
impl reqwest::dns::Resolve for ResolveLocal {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let address = self.0;
        Box::pin(async move { Ok(Box::new([address].into_iter()) as reqwest::dns::Addrs) })
    }
}
async fn fixture(mode: &str) {
    let fixture = Fixture::default();
    let app = axum::Router::new()
        .route("/target/{case}", any(origin))
        .route("/relay/{provider}", any(relay))
        .with_state(fixture.clone());
    let (address, server) = crate::test_tls::serve(app).await;
    let proxy = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), address),
        ]),
        if mode == "tor-auth" {
            crate::test_socks::Fault::BadAuth
        } else {
            crate::test_socks::Fault::None
        },
    )
    .await;
    let refused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let refused_address = refused.local_addr().unwrap();
    drop(refused);
    let tor = mode != "direct";
    let retries = match mode {
        "tor0" => Some(0),
        "tor1" => Some(1),
        _ => None,
    };
    let attempts = if tor { retries.unwrap_or(2) + 1 } else { 1 };
    let mut network = NetworkContext::new(NetworkPolicy {
        mode: if tor { Mode::Tor } else { Mode::Direct },
        socks_endpoint: tor.then_some(if mode == "tor-local-refused" {
            refused_address
        } else {
            proxy.address
        }),
        tor_isolation_retries: retries,
        cover_traffic_enabled: Some(false),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    if !tor {
        network = network.with_test_dns(Arc::new(ResolveLocal(address)));
    }
    crate::network::install_test_context(network);
    let configured = relay_fixture();
    let client = attached(configured.clone());
    if matches!(mode, "tor-auth" | "tor-local-refused") {
        for method in ["GET", "POST"] {
            let progress = SubmissionProgress::default();
            assert!(
                client
                    .execute_response_observed(route(method, "status403"), &progress)
                    .await
                    .is_err()
            );
            assert!(!progress.possible());
        }
        assert!(
            fixture.events.lock().unwrap().is_empty(),
            "permanent local proxy errors cannot reach origin or buy fallback"
        );
        if mode == "tor-auth" {
            assert_eq!(
                proxy.accepted.load(std::sync::atomic::Ordering::SeqCst),
                2,
                "one permanent setup failure per logical request"
            );
        }
        client.drain().await;
        server.abort();
        return;
    }

    // Empty manual input is the same origin/paid-relay policy as omitted input.
    let empty_case = "manual-empty-headers";
    let output = client
        .execute_manual_raw(route("GET", empty_case), Default::default())
        .await
        .unwrap();
    assert!(output.text.contains("relay result"));
    let events = fixture.events("GET-manual-empty-headers");
    assert_eq!(
        events.iter().filter(|e| e.provider.is_none()).count(),
        attempts
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.provider.is_some() && e.signer.is_some())
            .count(),
        1
    );

    // Any nonempty manual set is private, even an innocuous name or only a key.
    for header in ["X-Label", "X-Api-Key", "Idempotency-Key", "Authorization"] {
        for method in ["GET", "POST"] {
            let kind = "manual-status403";
            let case = format!("{method}-{kind}");
            let before = fixture.events(&case).len();
            let headers = crate::discovery::raw::ManualRawHeaders::parse(Some(
                &json!({header:"manual-secret"}),
            ))
            .unwrap();
            let (result, logs) = crate::limits::tests::capture(
                client.execute_manual_raw(route(method, kind), headers),
            )
            .await;
            assert!(result.is_err());
            assert!(!logs.contains("manual-secret"));
            let all_events = fixture.events(&case);
            let events = &all_events[before..];
            assert_eq!(events.len(), if method == "GET" { attempts } else { 1 });
            assert!(
                events
                    .iter()
                    .all(|e| e.provider.is_none() && e.signer.is_none())
            );
            assert!(events.iter().all(|e| e.headers[header] == "manual-secret"));
        }
    }
    for kind in ["manual-healthy", "manual-body-reset"] {
        let headers = crate::discovery::raw::ManualRawHeaders::parse(Some(
            &json!({"Authorization":"Bearer manual-secret"}),
        ))
        .unwrap();
        let result = client
            .execute_manual_raw(route("POST", kind), headers)
            .await;
        assert_eq!(result.is_ok(), kind == "manual-healthy");
        assert_eq!(
            fixture.events(&format!("POST-{kind}")).len(),
            1,
            "manual state-changing unsigned calls gain no status/body replay authority"
        );
    }
    let headers = crate::discovery::raw::ManualRawHeaders::parse(Some(
        &json!({"X-Label":"manual-secret", "Idempotency-Key":"caller-key"}),
    ))
    .unwrap();
    let result = client
        .execute_manual_raw(route("GET", "manual-recover"), headers)
        .await;
    assert_eq!(result.is_ok(), tor && attempts > 1);
    let events = fixture.events("GET-manual-recover");
    assert_eq!(events.len(), if tor && attempts > 1 { 2 } else { 1 });
    assert!(events.iter().all(|e| e.provider.is_none()
        && e.headers["x-label"] == "manual-secret"
        && e.headers["idempotency-key"] == "caller-key"));

    for method in ["GET", "HEAD", "POST"] {
        for status in [400, 403, 408, 429, 500, 502, 503, 504, 501, 505, 520] {
            let kind = format!("status{status}");
            let case = format!("{method}-{kind}");
            let initial = crate::network::global()
                .http_participating(
                    &IsolationId::evm(&payer(181).address).unwrap(),
                    "https://api.example.com",
                    None,
                    false,
                    Default::default(),
                )
                .unwrap();
            let first_credentials = crate::network::global().credentials(&initial.scope);
            let before_records = proxy.records.lock().unwrap().len();
            let output = client.execute_response(route(method, &kind)).await.unwrap();
            assert!(!output.paid_submission, "relay fee is not a target payment");
            assert!(output.relay_payment_may_have_occurred);
            assert!(
                !output.advisories.is_empty(),
                "possible relay fee remains visible"
            );
            assert_eq!(
                output.bytes,
                if method == "HEAD" {
                    b"".as_slice()
                } else {
                    b"relay result".as_slice()
                }
            );
            if method == "HEAD" {
                assert_eq!(output.mime_type.as_deref(), Some("text/plain"));
            }
            let events = fixture.events(&case);
            let origin_count = if [400, 403, 408, 429, 500, 502, 503, 504].contains(&status) {
                attempts
            } else {
                1
            };
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event.provider.is_none())
                    .count(),
                origin_count,
                "{mode} {method} {status}"
            );
            let still_selected = crate::network::global()
                .http_participating(
                    &IsolationId::evm(&payer(181).address).unwrap(),
                    "https://api.example.com",
                    None,
                    false,
                    Default::default(),
                )
                .unwrap();
            assert!(
                Arc::ptr_eq(&initial, &still_selected),
                "relay delivery never promotes origin scope"
            );
            if tor {
                let records = proxy.records.lock().unwrap();
                let mut scopes = std::collections::BTreeSet::from([first_credentials]);
                for record in &records[before_records..] {
                    if record.host == "api.example.com" {
                        scopes.insert((record.user.clone(), record.password.clone()));
                    }
                }
                assert_eq!(
                    scopes.len(),
                    origin_count,
                    "every fresh origin attempt used a distinct Tor scope"
                );
            }
            assert_eq!(
                events.len(),
                origin_count + 2,
                "one origin sequence then unsigned relay challenge and one signed exchange"
            );
            assert!(
                events[..origin_count]
                    .iter()
                    .all(|event| event.provider.is_none() && event.signer.is_none())
            );
            let expected_provider = match method {
                "GET" => 0,
                "HEAD" => 2,
                _ => 1,
            };
            assert_eq!(events[origin_count].provider, Some(expected_provider));
            assert!(events[origin_count].signer.is_none());
            assert!(events[origin_count + 1].signer.is_some());
            let expected_uri = format!("/target/{case}?literal=%2F%20&q=one+two&q=x%2By");
            for event in &events[..origin_count] {
                assert_eq!(event.uri, expected_uri);
                assert_eq!(event.method.as_str(), method);
                if method == "POST" {
                    assert_eq!(event.body, br#"{"exact":"line\n","n":1}"#);
                    assert_eq!(event.headers["content-type"], "application/json");
                }
            }
            let encoded: Value = serde_json::from_slice(&events[origin_count].body).unwrap();
            assert_eq!(
                encoded["url"],
                format!("https://api.example.com{expected_uri}")
            );
            assert_eq!(encoded["method"], method);
            if method == "POST" {
                assert_eq!(encoded["body"].as_str().unwrap().as_bytes(), events[0].body);
                assert_eq!(encoded["headers"]["content-type"], "application/json");
            }
            assert!(encoded["headers"].get("payment-signature").is_none());
            assert!(encoded["headers"].get("authorization").is_none());
        }
        for status in [401, 404, 405, 410, 422, 302, 307] {
            let kind = format!("status{status}");
            assert!(client.execute_response(route(method, &kind)).await.is_err());
            let events = fixture.events(&format!("{method}-{kind}"));
            assert_eq!(
                events.len(),
                1,
                "nonblocking status cannot earn origin retry or fallback"
            );
            assert!(events[0].provider.is_none() && events[0].signer.is_none());
        }
        for kind in ["healthy", "empty", "range"] {
            let output = client.execute_response(route(method, kind)).await.unwrap();
            assert!(output.advisories.is_empty());
            assert!(!output.paid_submission && !output.relay_payment_may_have_occurred);
            assert_eq!(fixture.events(&format!("{method}-{kind}")).len(), 1);
        }
        let output = client
            .execute_response(route(method, "origin-paid"))
            .await
            .unwrap();
        assert!(output.paid_submission);
        assert!(!output.relay_payment_may_have_occurred);
        assert!(output.advisories.is_empty());
        let events = fixture.events(&format!("{method}-origin-paid"));
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.provider.is_none()));
        assert!(events[0].signer.is_none() && events[1].signer.is_some());
        for kind in [
            "relayed402",
            "relayed404",
            "uncertain",
            "malformed-envelope",
            "private-final",
        ] {
            let progress = SubmissionProgress::default();
            let error = client
                .execute_response_observed(route(method, kind), &progress)
                .await
                .unwrap_err();
            assert!(
                progress.possible(),
                "paid fallback failure retains aggregate financial evidence"
            );
            assert!(format!("{error:#}").contains("do not automatically repeat"));
            assert!(!format!("{error:#}").contains("PRIVATE_RELAY_ERROR"));
            let events = fixture.events(&format!("{method}-{kind}"));
            assert_eq!(
                events.len(),
                attempts + 2,
                "terminal or uncertain target/relay response stops the list"
            );
            assert!(
                events
                    .iter()
                    .filter(|event| event.provider.is_none())
                    .all(|event| event.signer.is_none()),
                "relayed target 402 cannot sign/retry the target"
            );
        }
    }

    for method in ["GET", "POST"] {
        let progress = SubmissionProgress::default();
        let output = client
            .execute_response_observed(route(method, "other-origin"), &progress)
            .await
            .unwrap();
        assert!(!output.paid_submission && output.relay_payment_may_have_occurred);
        assert!(progress.possible() && !output.advisories.is_empty());
        assert_eq!(output.bytes, b"relay result");
        let events = fixture.events(&format!("{method}-other-origin"));
        assert_eq!(events.len(), attempts + 2);
        assert!(
            events[..attempts]
                .iter()
                .all(|event| event.provider.is_none() && event.signer.is_none())
        );
        assert!(events[attempts].provider.is_some() && events[attempts].signer.is_none());
        assert!(events[attempts + 1].provider.is_some() && events[attempts + 1].signer.is_some());
    }

    for method in ["GET", "POST"] {
        let timed = client.clone().with_timeout(Duration::from_millis(150));
        for kind in ["body-stall", "body-reset", "header-stall"] {
            let output = timed.execute_response(route(method, kind)).await.unwrap();
            assert!(
                !output.paid_submission
                    && output.relay_payment_may_have_occurred
                    && !output.advisories.is_empty()
            );
            let events = fixture.events(&format!("{method}-{kind}"));
            assert_eq!(
                events.len(),
                attempts + 2,
                "unsigned transport failures use the shared count before fallback"
            );
        }
        for kind in ["terminal404-stall", "terminal401-reset"] {
            let progress = SubmissionProgress::default();
            assert!(
                timed
                    .execute_response_observed(route(method, kind), &progress)
                    .await
                    .is_err()
            );
            assert!(!progress.possible());
            let events = fixture.events(&format!("{method}-{kind}"));
            assert_eq!(
                events.len(),
                1,
                "known terminal HTTP status retains terminal authority when its body subsequently stalls or resets"
            );
            assert!(events[0].provider.is_none() && events[0].signer.is_none());
        }
        for kind in [
            "origin-signed400",
            "origin-signed402",
            "origin-signed503",
            "origin-signed-stall",
        ] {
            let progress = SubmissionProgress::default();
            assert!(
                timed
                    .execute_response_observed(route(method, kind), &progress)
                    .await
                    .is_err()
            );
            assert!(progress.possible());
            let events = fixture.events(&format!("{method}-{kind}"));
            assert_eq!(
                events.len(),
                2,
                "signed failures cannot buy fallback or replay"
            );
            assert!(events.iter().all(|event| event.provider.is_none()));
        }
    }
    let recovered = client
        .execute_response(route("GET", "recover"))
        .await
        .unwrap();
    let events = fixture.events("GET-recover");
    if attempts > 1 {
        assert_eq!(recovered.bytes, b"origin recovered");
        assert!(!recovered.relay_payment_may_have_occurred);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.provider.is_none()));
    } else {
        assert_eq!(recovered.bytes, b"relay result");
        assert!(recovered.relay_payment_may_have_occurred);
        assert_eq!(events.len(), 3);
    }
    let outer_recovery = client
        .execute_response(route("GET", "relay-unsigned503"))
        .await
        .unwrap();
    assert!(outer_recovery.relay_payment_may_have_occurred && !outer_recovery.paid_submission);
    let events = fixture.events("GET-relay-unsigned503");
    assert_eq!(events.len(), attempts + 3);
    let relay_events = &events[attempts..];
    if attempts > 1 {
        assert!(
            relay_events.iter().all(|event| event.provider == Some(0)),
            "relay's own unsigned stage uses its one configured retry sequence"
        );
    } else {
        assert_eq!(
            relay_events
                .iter()
                .map(|event| event.provider.unwrap())
                .collect::<Vec<_>>(),
            [0, 1, 1],
            "one direct/zero-retry prepayment failure advances once without restarting origin"
        );
    }
    let advanced = client
        .execute_response(route("GET", "relayed503"))
        .await
        .unwrap();
    assert!(advanced.relay_payment_may_have_occurred && !advanced.paid_submission);
    let events = fixture.events("GET-relayed503");
    assert_eq!(events.len(), attempts + 4);
    assert_eq!(
        events[attempts..]
            .iter()
            .map(|event| event.provider.unwrap())
            .collect::<Vec<_>>(),
        [0, 0, 1, 1]
    );
    let small = client
        .clone()
        .with_download_limits(8, crate::limits::HELP_BYTES);
    let error = small
        .execute_response(route("GET", "large"))
        .await
        .unwrap_err();
    assert!(error.is::<crate::limits::DownloadLimit>());
    assert_eq!(
        fixture.events("GET-large").len(),
        1,
        "local bounds cannot purchase a relay"
    );

    let mapping: crate::output::ResponseMapping =
        serde_json::from_value(json!({"images":[{"pointer":"/missing","mime_type":"image/png"}]}))
            .unwrap();
    for kind in [
        "origin-json-invalid",
        "origin-image-invalid",
        "mapped-invalid",
    ] {
        let error = client
            .execute_rendered_scoped(
                route("GET", kind),
                None,
                Some(&mapping),
                &Default::default(),
            )
            .await
            .unwrap_err();
        let events = fixture.events(&format!("GET-{kind}"));
        if kind == "mapped-invalid" {
            assert_eq!(events.len(), attempts + 2);
            assert!(
                format!("{error:#}").contains("do not automatically repeat"),
                "post-relay conversion error retains the relay fee"
            );
        } else {
            assert_eq!(
                events.len(),
                1,
                "successful origin output conversion failure never buys another read"
            );
        }
    }

    // AG and Horizon deliver binary bytes through their documented encodings;
    // Curl's text envelope cannot turn a binary target MIME into text success.
    for (format, provider) in [("agproxy", 1), ("horizonpulse", 2)] {
        let relay = Arc::new(
            SearchRelay::fixture_methods(BTreeMap::from([(
                "GET".into(),
                vec![(
                    format.into(),
                    format!("https://spec.example.com/relay/{provider}"),
                    crate::catalog::Config::default(),
                )],
            )]))
            .unwrap()
            .with_policy(crate::discovery_relay::Policy {
                api_requests: true,
                ..Default::default()
            }),
        );
        let binary = attached(relay)
            .execute_response(route("GET", "binary"))
            .await
            .unwrap();
        assert_eq!(binary.bytes, [0, 255]);
        assert_eq!(
            binary.mime_type.as_deref(),
            Some("application/octet-stream")
        );
        assert!(
            !binary.paid_submission
                && binary.relay_payment_may_have_occurred
                && !binary.advisories.is_empty()
        );
    }
    for kind in [
        "binary-truncated",
        "binary-failed",
        "image-truncated",
        "image-failed",
    ] {
        let output = client.execute_response(route("GET", kind)).await.unwrap();
        assert!(!output.paid_submission && output.relay_payment_may_have_occurred);
        assert!(!output.advisories.is_empty());
        if kind.starts_with("image-") {
            assert_eq!(output.bytes, STANDARD.decode(PNG).unwrap());
            assert_eq!(output.mime_type.as_deref(), Some("image/png"));
        } else {
            assert_eq!(output.bytes, [0, 255]);
            assert_eq!(
                output.mime_type.as_deref(),
                Some("application/octet-stream")
            );
        }
        let events = fixture.events(&format!("GET-{kind}"));
        assert_eq!(
            events.len(),
            attempts + 4,
            "definitive incomplete Curl delivery advances once without restarting origin"
        );
        assert_eq!(
            events[attempts..]
                .iter()
                .map(|event| event.provider.unwrap())
                .collect::<Vec<_>>(),
            [0, 0, 1, 1]
        );
        assert!(
            events[attempts..]
                .iter()
                .filter(|event| event.signer.is_some())
                .count()
                == 2,
            "both distinct relay acquisitions retain their possible fees"
        );
    }
    let before = fixture.events("GET-binary").len();
    assert!(
        client
            .execute_response(route("GET", "binary"))
            .await
            .is_err()
    );
    assert_eq!(
        fixture.events("GET-binary").len() - before,
        attempts + 2,
        "Curl binary-MIME failure is terminal and cannot purchase AG"
    );
    let head_relay = Arc::new(
        SearchRelay::fixture_methods(BTreeMap::from([(
            "HEAD".into(),
            vec![(
                "curl".into(),
                "https://spec.example.com/relay/0".into(),
                crate::catalog::Config::default(),
            )],
        )]))
        .unwrap()
        .with_policy(crate::discovery_relay::Policy {
            api_requests: true,
            ..Default::default()
        }),
    );
    let head = attached(head_relay)
        .execute_response(route("HEAD", "binary"))
        .await
        .unwrap();
    assert!(head.bytes.is_empty() && head.relay_payment_may_have_occurred);
    assert_eq!(
        head.mime_type.as_deref(),
        Some("text/plain"),
        "an empty HEAD result remains empty text even when target MIME is binary"
    );

    let origin_only = attached(Arc::new(
        SearchRelay::fixture_methods(BTreeMap::new())
            .unwrap()
            .with_policy(crate::discovery_relay::Policy {
                api_requests: true,
                ..Default::default()
            }),
    ));
    let before = fixture.events("GET-status403").len();
    assert!(
        origin_only
            .execute_response(route("GET", "status403"))
            .await
            .is_err()
    );
    assert_eq!(
        fixture.events("GET-status403").len() - before,
        attempts,
        "an absent method list grants no implicit provider"
    );

    // A captured ordinary static payer also owns its relay fee after replacement.
    let pinned = attached(configured.clone());
    let in_flight = tokio::spawn({
        let client = pinned.clone();
        async move { client.execute_response(route("GET", "hold-origin")).await }
    });
    tokio::time::timeout(Duration::from_secs(5), fixture.origin_held.cancelled())
        .await
        .unwrap();
    pinned.replace_payer(payer(182));
    fixture.origin_release.cancel();
    in_flight.await.unwrap().unwrap();
    let expected: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 181).parse().unwrap();
    let expected_address = expected.address().to_string().to_ascii_lowercase();
    assert_eq!(
        fixture
            .events("GET-hold-origin")
            .last()
            .unwrap()
            .signer
            .as_deref(),
        Some(expected_address.as_str())
    );

    let held = attached(configured.clone());
    let waiter = tokio::spawn({
        let client = held.clone();
        async move { client.execute_response(route("GET", "held")).await }
    });
    tokio::time::timeout(Duration::from_secs(5), fixture.signed_held.cancelled())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(
        held.execute_response(route("POST", "healthy"))
            .await
            .is_ok()
    );
    let mut drain = tokio::spawn({
        let client = held.clone();
        async move { client.drain().await }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut drain)
            .await
            .is_err(),
        "cancelled paid API waiter cannot orphan accepted relay work"
    );
    fixture.signed_release.cancel();
    tokio::time::timeout(Duration::from_secs(5), drain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.events("GET-held").len(),
        attempts + 2,
        "exactly one accepted relay exchange survives cancellation"
    );

    if tor {
        let records = proxy.records.lock().unwrap();
        assert!(
            records.iter().all(|record| record.address_type == 3
                && ["api.example.com", "spec.example.com"].contains(&record.host.as_str())),
            "all destinations remain remotely resolved through Tor"
        );
        let id = IsolationId::evm(&expected.address().to_string()).unwrap();
        assert!(
            records
                .iter()
                .any(|record| record.user == crate::network::global().credentials(&id).0)
        );
    }
    // Opting out affects fallback only: origin successes and target x402 payment still work.
    let defaults = attached(Arc::new(
        (*configured).clone().with_policy(Default::default()),
    ));
    for method in ["GET", "HEAD", "POST"] {
        let before = fixture.events.lock().unwrap().len();
        assert!(
            defaults
                .execute_response(route(method, "status403"))
                .await
                .is_err()
        );
        {
            let captured = fixture.events.lock().unwrap();
            assert_eq!(captured.len() - before, attempts);
            assert!(
                captured[before..]
                    .iter()
                    .all(|event| event.provider.is_none() && event.signer.is_none()),
                "default API-off never dispatches attached known-operative relay"
            );
        }
        let before = fixture.events.lock().unwrap().len();
        let healthy = defaults
            .execute_response(route(method, "healthy"))
            .await
            .unwrap();
        assert!(!healthy.relay_payment_may_have_occurred);
        let paid = defaults
            .execute_response(route(method, "origin-paid"))
            .await
            .unwrap();
        assert!(paid.paid_submission && !paid.relay_payment_may_have_occurred);
        let captured = fixture.events.lock().unwrap();
        assert_eq!(captured.len() - before, 3);
        assert!(
            captured[before..]
                .iter()
                .all(|event| event.provider.is_none())
        );
    }
    defaults.drain().await;
    client.drain().await;
    pinned.drain().await;
    server.abort();
}
