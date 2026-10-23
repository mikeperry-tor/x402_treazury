//! Ordered acquisitions use actual unsigned challenges and public unfunded signatures.
use super::*;
use adapters::{Format, TargetRequest};
use axum::{
    extract::State as AxumState,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response as HttpResponse},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use reqwest::Method;
use sequence::Contract;
use serde_json::{Value, json};
use std::sync::Mutex;

const TARGET: &str = "https://example.com/read?exact=%2F%20";

#[derive(Clone, Debug)]
struct Event {
    provider: usize,
    signed: bool,
    body: Value,
}
#[derive(Clone)]
struct Fixture {
    events: Arc<Mutex<Vec<Event>>>,
    responses: Arc<Vec<Value>>,
    stall_first_unsigned_501: bool,
}
async fn handler(
    AxumState(fixture): AxumState<Fixture>,
    uri: Uri,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> HttpResponse {
    let provider: usize = uri.path().trim_start_matches('/').parse().unwrap();
    let signed = headers.contains_key("payment-signature");
    assert!(!headers.contains_key("x-payment"));
    if signed {
        let payload: Value = serde_json::from_slice(
            &STANDARD
                .decode(headers["payment-signature"].as_bytes())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["payload"]["authorization"]["value"], "10000");
    }
    fixture.events.lock().unwrap().push(Event {
        provider,
        signed,
        body,
    });
    if !signed && provider == 0 && fixture.stall_first_unsigned_501 {
        let body = axum::body::Body::from_stream(
            futures_util::stream::once(async {
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"unused error body"))
            })
            .chain(futures_util::stream::pending()),
        );
        return (StatusCode::NOT_IMPLEMENTED, body).into_response();
    }
    if signed {
        return axum::Json(fixture.responses[provider].clone()).into_response();
    }
    let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/relay","description":"fixture","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"10000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
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

struct Harness {
    sequence: SearchRelay,
    client: PaidClient,
    events: Arc<Mutex<Vec<Event>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Harness {
    async fn new(entries: Vec<(Format, Value)>) -> Self {
        Self::start(entries, false).await
    }

    async fn with_stalled_unsigned_501(entries: Vec<(Format, Value)>) -> Self {
        Self::start(entries, true).await
    }

    async fn start(entries: Vec<(Format, Value)>, stall_first_unsigned_501: bool) -> Self {
        let events = Arc::default();
        let fixture = Fixture {
            events: Arc::clone(&events),
            responses: Arc::new(
                entries
                    .iter()
                    .map(|(_, response)| response.clone())
                    .collect(),
            ),
            stall_first_unsigned_501,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().fallback(handler).with_state(fixture),
            )
            .await
            .unwrap();
        });
        let entries = entries
            .into_iter()
            .enumerate()
            .map(|(position, (format, _))| {
                let mut entry =
                    SearchRelay::fixture(format!("{base}/{position}"), Config::default())
                        .entries
                        .remove("GET")
                        .unwrap()
                        .pop()
                        .unwrap();
                entry.key = format!("position-{position}");
                entry.provider_key = format!("provider-{position}");
                entry.format = format;
                entry
            })
            .collect();
        Self {
            sequence: SearchRelay {
                entries: BTreeMap::from([("GET".into(), entries)]),
            },
            client: payer("0.01"),
            events,
            task,
        }
    }

    async fn acquire(
        &self,
        target: &TargetRequest,
        contract: Contract,
        limit: usize,
        cache: Option<Arc<tokio::sync::Mutex<State>>>,
    ) -> Result<Arc<Response>> {
        self.acquire_after(target, contract, limit, cache, None)
            .await
    }

    async fn acquire_after(
        &self,
        target: &TargetRequest,
        contract: Contract,
        limit: usize,
        cache: Option<Arc<tokio::sync::Mutex<State>>>,
        initial_delay: Option<crate::network::RetryAfter>,
    ) -> Result<Arc<Response>> {
        self.sequence
            .acquire(
                self.client.clone(),
                target,
                contract,
                limit,
                "max_response_bytes",
                None,
                4096,
                Some(std::time::Duration::from_secs(2)),
                &[],
                cache.map(|state| (state, "wallet")),
                initial_delay,
            )
            .await
    }

    fn assert_order(&self, expected: &[(usize, bool)]) {
        let events = self.events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| (event.provider, event.signed))
                .collect::<Vec<_>>(),
            expected
        );
        for event in events.iter() {
            assert_eq!(
                event.body["url"], TARGET,
                "exact query encoding preserved on each authorization"
            );
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn payer(cap: &str) -> PaidClient {
    PaidClient::new(
        crate::payment::Payer::new(
            &format!("{:064x}", 91),
            crate::payment::SpendPolicy::dollars(cap).unwrap(),
        )
        .unwrap(),
    )
}
fn target(method: Method) -> TargetRequest {
    TargetRequest {
        method,
        url: TARGET.parse().unwrap(),
        headers: Default::default(),
        body: None,
        allowed_origins: vec![],
    }
}
fn curl(status: u16, body: &str) -> Value {
    json!({"url":TARGET,"method":"GET","statusCode":status,"headers":{},"body":body,"redirectChain":[],"error":null})
}
fn agproxy(status: u16, body: &str, truncated: bool) -> Value {
    json!([{"status":status,"final_url":TARGET,"headers":{},"body":body,"truncated":truncated}])
}

#[tokio::test]
async fn definitive_curl_target_failure_advances_once_to_authored_agproxy_entry() {
    let harness = Harness::new(vec![
        (Format::Curl, curl(403, "blocked")),
        (Format::Agproxy, agproxy(200, "{\"complete\":true}", false)),
    ])
    .await;
    let response = harness
        .acquire(&target(Method::GET), Contract::Catalog, 4096, None)
        .await
        .unwrap();
    assert_eq!(response.body, br#"{"complete":true}"#);
    assert_eq!(response.via, "agproxy");
    assert!(
        response.payment_may_have_occurred,
        "both distinct authorizations remain potentially chargeable"
    );
    harness.assert_order(&[(0, false), (0, true), (1, false), (1, true)]);
}

#[tokio::test]
async fn unsupported_post_body_skips_curl_before_dispatch_and_preserves_agproxy_payload() {
    let mut harness = Harness::new(vec![
        (Format::Curl, curl(200, "unused")),
        (Format::Agproxy, agproxy(200, "{\"posted\":true}", false)),
    ])
    .await;
    let entries = harness.sequence.entries.remove("GET").unwrap();
    harness.sequence.entries.insert("POST".into(), entries);
    let mut request = target(Method::POST);
    request.body = Some(br#"{ "exact": [1,2] }"#.to_vec());
    request
        .headers
        .insert("accept", "application/json".parse().unwrap());
    let response = harness
        .acquire(&request, Contract::Catalog, 4096, None)
        .await
        .unwrap();
    assert_eq!(response.body, br#"{"posted":true}"#);
    harness.assert_order(&[(1, false), (1, true)]);
    for event in harness.events.lock().unwrap().iter() {
        assert_eq!(event.body["method"], "POST");
        assert_eq!(event.body["body"], r#"{ "exact": [1,2] }"#);
        assert_eq!(event.body["headers"]["accept"], "application/json");
    }
}

#[tokio::test]
async fn absent_or_empty_method_list_never_dispatches_an_implicit_provider() {
    let mut harness = Harness::new(vec![(Format::Curl, curl(200, "unused"))]).await;
    assert!(
        harness
            .acquire(&target(Method::HEAD), Contract::Catalog, 4096, None)
            .await
            .is_err()
    );
    harness.sequence.entries.insert("POST".into(), vec![]);
    assert!(
        harness
            .acquire(&target(Method::POST), Contract::Catalog, 4096, None)
            .await
            .is_err()
    );
    harness.assert_order(&[]);
}

#[tokio::test]
async fn malformed_paid_envelope_stops_but_explicit_delivery_truncation_advances() {
    let malformed = Harness::new(vec![
        (Format::Curl, json!({"body":"unattributable"})),
        (Format::Agproxy, agproxy(200, "must not buy", false)),
    ])
    .await;
    let error = malformed
        .acquire(&target(Method::GET), Contract::Catalog, 4096, None)
        .await
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("may still charge"));
    malformed.assert_order(&[(0, false), (0, true)]);
    let truncated = Harness::new(vec![
        (Format::Agproxy, agproxy(200, "partial", true)),
        (Format::Agproxy, agproxy(200, "{\"complete\":true}", false)),
    ])
    .await;
    let response = truncated
        .acquire(&target(Method::GET), Contract::Catalog, 4096, None)
        .await
        .unwrap();
    assert_eq!(response.body, br#"{"complete":true}"#);
    truncated.assert_order(&[(0, false), (0, true), (1, false), (1, true)]);
}

#[tokio::test]
async fn unsolicited_catalog_not_modified_stays_terminal_despite_delivery_metadata() {
    for incomplete in [false, true] {
        let mut failed = curl(304, "");
        if incomplete {
            failed["error"] = json!("fixture delivery failure");
        }
        for (format, response) in [
            (Format::Curl, failed),
            (Format::Agproxy, agproxy(304, "", incomplete)),
        ] {
            let harness = Harness::new(vec![
                (format, response),
                (Format::Agproxy, agproxy(200, "{}", false)),
            ])
            .await;
            let error = harness
                .acquire(&target(Method::GET), Contract::Catalog, 4096, None)
                .await
                .err()
                .expect("unsolicited 304 must not purchase another response");
            let failure = error.downcast_ref::<TargetFailure>().unwrap();
            assert_eq!(failure.reason, "unsolicited_not_modified");
            assert_eq!(failure.status, Some(304));
            assert!(error.is::<document::PossiblePayment>());
            harness.assert_order(&[(0, false), (0, true)]);
        }
    }
}

#[tokio::test]
async fn target_partial_documents_stay_terminal_when_relay_also_reports_truncation() {
    for contract in [Contract::Catalog, Contract::Search(|_| Ok(()))] {
        for status in [206, 200] {
            let mut partial = agproxy(status, "{}", true);
            if status == 200 {
                partial[0]["headers"] = json!({"content-range":"bytes 0-1/10"});
            }
            let harness = Harness::new(vec![
                (Format::Agproxy, partial),
                (Format::Agproxy, agproxy(200, "{\"complete\":true}", false)),
            ])
            .await;
            let error = harness
                .acquire(&target(Method::GET), contract, 4096, None)
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.downcast_ref::<TargetFailure>().unwrap().reason,
                "partial_response"
            );
            assert!(format!("{error:#}").contains("may still charge"));
            harness.assert_order(&[(0, false), (0, true)]);
        }
    }
}

#[tokio::test]
async fn curl_delivery_errors_preserve_known_terminal_status_and_header_contracts() {
    for contract in [Contract::Catalog, Contract::Search(|_| Ok(()))] {
        for status in [402, 206, 200] {
            let mut failed = curl(status, "{}");
            failed["error"] = json!("fixture delivery failure");
            if status == 200 {
                failed["headers"] = json!({"content-range":"bytes 0-1/10"});
            }
            let harness = Harness::new(vec![
                (Format::Curl, failed),
                (Format::Agproxy, agproxy(200, "{}", false)),
            ])
            .await;
            let error = harness
                .acquire(&target(Method::GET), contract, 4096, None)
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.downcast_ref::<TargetFailure>().unwrap().reason,
                if status == 402 {
                    "target_payment_required"
                } else {
                    "partial_response"
                }
            );
            assert!(format!("{error:#}").contains("may still charge"));
            harness.assert_order(&[(0, false), (0, true)]);
        }
    }
    // Pricing observes known headers and never buys another probe after 2xx/402.
    for status in [200, 402] {
        let mut failed = curl(status, "not a target document");
        failed["error"] = json!("fixture delivery failure");
        let harness = Harness::new(vec![
            (Format::Curl, failed),
            (Format::Agproxy, agproxy(200, "{}", false)),
        ])
        .await;
        let response = harness
            .acquire(&target(Method::GET), Contract::Pricing, 4096, None)
            .await
            .unwrap();
        assert_eq!(response.status, status);
        assert!(response.payment_may_have_occurred);
        harness.assert_order(&[(0, false), (0, true)]);
    }
}

#[tokio::test]
async fn attributable_curl_delivery_failures_advance_with_known_retry_after_preserved() {
    for status in [0, 200, 503] {
        let mut failed = curl(status, "{}");
        failed["error"] = json!("fixture delivery failure");
        if status == 503 {
            failed["headers"] = json!({"retry-after":"1"});
        }
        let harness = Harness::new(vec![
            (Format::Curl, failed),
            (Format::Agproxy, agproxy(200, "{}", false)),
        ])
        .await;
        let started = std::time::Instant::now();
        let response = harness
            .acquire(&target(Method::GET), Contract::Catalog, 4096, None)
            .await
            .unwrap();
        if status == 503 {
            assert!(started.elapsed() >= std::time::Duration::from_secs(1));
        }
        assert_eq!(response.body, b"{}");
        assert!(response.payment_may_have_occurred);
        harness.assert_order(&[(0, false), (0, true), (1, false), (1, true)]);
    }
}

#[tokio::test]
async fn runtime_documents_advance_no_http_delivery_failure_but_keep_known_404_terminal() {
    for status in [0, 404] {
        let mut first = curl(status, "{}");
        first["error"] = json!("fixture delivery failure");
        let harness = Harness::new(vec![
            (Format::Curl, first),
            (Format::Agproxy, agproxy(200, "{}", false)),
        ])
        .await;
        let result = harness
            .acquire(&target(Method::GET), Contract::Document, 4096, None)
            .await;
        if status == 0 {
            let response = result.unwrap();
            assert_eq!(response.via, "agproxy");
            assert!(response.payment_may_have_occurred);
            harness.assert_order(&[(0, false), (0, true), (1, false), (1, true)]);
        } else {
            let error = result.err().unwrap();
            assert_eq!(
                error.downcast_ref::<TargetFailure>().unwrap().status,
                Some(404)
            );
            assert!(error.is::<document::PossiblePayment>());
            harness.assert_order(&[(0, false), (0, true)]);
        }
    }
}

#[tokio::test]
async fn prepared_document_needs_origin_eligibility_and_existing_authority_before_io() {
    let harness = Harness::new(vec![(Format::Curl, curl(200, "{}"))]).await;
    let options = document::Options::new(
        TARGET,
        crate::network::IsolationId::discovery(TARGET).unwrap(),
        4096,
        "max_spec_bytes",
    );
    let client =
        PaidClient::unsigned().with_document_relay(Some(Arc::new(harness.sequence.clone())));
    let failure = crate::network::acquisition::Failure::status(
        anyhow::anyhow!("fixture origin blocked"),
        StatusCode::FORBIDDEN,
        &HeaderMap::new(),
        crate::network::acquisition::FallbackPolicy::BlockingFailures,
    );
    let error = document::fallback(&client, &options, failure)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "fixture origin blocked");
    let prepared = document::prepare_fallback(&harness.sequence, &options)
        .unwrap()
        .unwrap();
    let failure = crate::network::acquisition::Failure::validation(anyhow::anyhow!(
        "fixture terminal origin"
    ));
    let error = prepared
        .acquire(&harness.client, failure)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "fixture terminal origin");
    harness.assert_order(&[]);
}

#[tokio::test]
async fn relayed_pricing_challenge_is_terminal_even_without_usable_metadata() {
    let harness = Harness::new(vec![
        (Format::Curl, curl(402, "unusable challenge")),
        (Format::Agproxy, agproxy(200, "must not buy", false)),
    ])
    .await;
    let response = harness
        .acquire(&target(Method::GET), Contract::Pricing, 4096, None)
        .await
        .unwrap();
    assert_eq!(response.status, 402);
    harness.assert_order(&[(0, false), (0, true)]);
}

#[tokio::test]
async fn local_response_bounds_origin_policy_and_payment_authority_do_not_advance() {
    for policy in [false, true] {
        let mut first = curl(200, "body exceeds the consumer bound");
        let mut request = target(Method::GET);
        if policy {
            first["redirectChain"] = json!(["https://other.example/read"]);
            request.allowed_origins = vec!["https://example.com".into()];
        }
        let harness = Harness::new(vec![
            (Format::Curl, first),
            (Format::Agproxy, agproxy(200, "must not buy", false)),
        ])
        .await;
        assert!(
            harness
                .acquire(
                    &request,
                    Contract::Catalog,
                    if policy { 4096 } else { 4 },
                    None
                )
                .await
                .is_err()
        );
        harness.assert_order(&[(0, false), (0, true)]);
    }
    let mut harness = Harness::new(vec![
        (Format::Curl, curl(200, "unused")),
        (Format::Agproxy, agproxy(200, "unused", false)),
    ])
    .await;
    harness.client = payer("0.001");
    assert!(
        harness
            .acquire(&target(Method::GET), Contract::Catalog, 4096, None)
            .await
            .is_err()
    );
    harness.assert_order(&[(0, false)]);
    harness.events.lock().unwrap().clear();
    let mut credentials = target(Method::GET);
    credentials.headers.insert(
        "payment-signature",
        "public-forbidden-marker".parse().unwrap(),
    );
    assert!(
        harness
            .acquire(&credentials, Contract::Catalog, 4096, None)
            .await
            .is_err()
    );
    harness.assert_order(&[]);
}

#[tokio::test]
async fn static_provider_failure_and_result_caches_remain_independent() {
    let mut harness = Harness::new(vec![
        (Format::Curl, curl(403, "blocked")),
        (Format::Agproxy, agproxy(200, "{\"complete\":true}", false)),
    ])
    .await;
    let state = Arc::new(tokio::sync::Mutex::new(State::default()));
    let response = harness
        .acquire(
            &target(Method::GET),
            Contract::Catalog,
            4096,
            Some(state.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        state
            .lock()
            .await
            .outcomes
            .values()
            .filter(|outcome| outcome.is_err())
            .count(),
        1
    );
    assert_eq!(
        state
            .lock()
            .await
            .outcomes
            .values()
            .filter(|outcome| outcome.is_ok())
            .count(),
        1
    );
    assert!(
        state.lock().await.stopped.is_empty(),
        "a definitive failure cannot disable its sibling"
    );
    let entries = harness.sequence.entries["GET"].clone();
    harness
        .sequence
        .entries
        .insert("GET".into(), vec![entries[1].clone()]);
    let cached = harness
        .acquire(
            &target(Method::GET),
            Contract::Catalog,
            4096,
            Some(state.clone()),
        )
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&response, &cached));
    harness
        .sequence
        .entries
        .insert("GET".into(), vec![entries[0].clone()]);
    assert!(
        harness
            .acquire(
                &target(Method::GET),
                Contract::Catalog,
                4096,
                Some(state.clone())
            )
            .await
            .is_err(),
        "one provider's cached success cannot masquerade as the failed provider"
    );
    harness.assert_order(&[(0, false), (0, true), (1, false), (1, true)]);
}

#[tokio::test]
async fn smaller_cached_response_bound_keeps_typed_failure_and_payment_warning_without_io() {
    let harness = Harness::new(vec![
        (Format::Curl, curl(200, "{\"complete\":true}")),
        (Format::Agproxy, agproxy(200, "must not buy", false)),
    ])
    .await;
    let state = Arc::new(tokio::sync::Mutex::new(State::default()));
    let response = harness
        .acquire(
            &target(Method::GET),
            Contract::Catalog,
            4096,
            Some(state.clone()),
        )
        .await
        .unwrap();
    assert!(response.payment_may_have_occurred);
    let error = harness
        .acquire(
            &target(Method::GET),
            Contract::Catalog,
            4,
            Some(state.clone()),
        )
        .await
        .err()
        .unwrap();
    assert!(
        error.is::<crate::limits::DownloadLimit>(),
        "cached rejection retains the typed consumer bound"
    );
    assert!(
        format!("{error:#}").contains("may still charge"),
        "cached paid bytes retain their possible liability"
    );
    let restored = harness
        .acquire(
            &target(Method::GET),
            Contract::Catalog,
            4096,
            Some(state.clone()),
        )
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&response, &restored));
    harness.assert_order(&[(0, false), (0, true)]);
}

#[tokio::test]
async fn resumed_static_traversal_honors_absolute_retry_after_from_cached_failure() {
    let harness = Harness::new(vec![
        (Format::Curl, curl(503, "prior attributable failure")),
        (Format::Agproxy, agproxy(200, "{\"complete\":true}", false)),
    ])
    .await;
    let state = Arc::new(tokio::sync::Mutex::new(State::default()));
    // This is the retained failure state when another provider was not ready.
    // Its absolute delay must survive resumption rather than become new authority.
    let mut failure = TargetFailure::new("http_status", Some(503), true);
    failure.possible = true;
    failure.retry_after = Some(crate::network::RetryAfter::BeyondClock);
    let key = harness.sequence.cache_keys("wallet")[0].clone();
    state
        .lock()
        .await
        .outcomes
        .insert((key.clone(), TARGET.into()), Err(failure));
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(30),
            harness.acquire(
                &target(Method::GET),
                Contract::Catalog,
                4096,
                Some(state.clone())
            )
        )
        .await
        .is_err(),
        "a resumed dependency wait cannot skip the retained server delay"
    );
    harness.assert_order(&[]);
    let retained = state
        .lock()
        .await
        .outcomes
        .get(&(key, TARGET.into()))
        .unwrap()
        .as_ref()
        .err()
        .copied()
        .unwrap();
    assert!(retained.possible);
    assert!(matches!(
        retained.retry_after,
        Some(crate::network::RetryAfter::BeyondClock)
    ));
}
#[tokio::test]
async fn authored_head_list_uses_horizon_without_get_or_outer_post_inheritance() {
    let mut harness = Harness::new(vec![
        (Format::Curl, curl(200, "unused GET provider")),
        (
            Format::Horizonpulse,
            json!({
                "ok": true,
                "status": 200,
                "headers": {"content-type": "text/plain", "x-fixture": "head"},
                "body": "",
                "bodyEncoding": "text",
                "finalUrl": TARGET,
                "truncated": false
            }),
        ),
        (Format::Agproxy, agproxy(200, "unused POST provider", false)),
    ])
    .await;
    let entries = harness.sequence.entries.remove("GET").unwrap();
    harness.sequence.entries = BTreeMap::from([
        ("GET".into(), vec![entries[0].clone()]),
        ("HEAD".into(), vec![entries[1].clone()]),
        ("POST".into(), vec![entries[2].clone()]),
    ]);
    let response = harness
        .acquire(
            &target(Method::HEAD),
            Contract::Search(|bytes| {
                ensure!(bytes.is_empty(), "HEAD must retain an empty target body");
                Ok(())
            }),
            4096,
            None,
        )
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert!(response.body.is_empty());
    assert_eq!(response.headers["content-type"], "text/plain");
    assert_eq!(response.headers["x-fixture"], "head");
    assert_eq!(response.final_url.as_str(), TARGET);
    assert_eq!(response.via, "horizonpulse");
    assert!(response.payment_may_have_occurred);
    harness.assert_order(&[(1, false), (1, true)]);
    for event in harness.events.lock().unwrap().iter() {
        assert_eq!(event.body["method"], "HEAD");
        assert!(event.body.get("body").is_none());
    }
}

#[tokio::test]
async fn bundled_method_lists_bootstrap_all_actual_formats_from_local_assets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let policy = Policy {
        fallbacks: BTreeMap::from([
            (
                "GET".into(),
                vec![
                    root.join("providers/curl/provider.toml"),
                    root.join("providers/agproxy.toml"),
                ],
            ),
            ("POST".into(), vec![root.join("providers/agproxy.toml")]),
            (
                "HEAD".into(),
                vec![root.join("providers/horizonpulse.toml")],
            ),
        ]),
        ..Default::default()
    };
    let sequence = SearchRelay::load(&policy, &root.join("deployment.toml"))
        .await
        .unwrap();
    assert_eq!(sequence.entries["GET"].len(), 2);
    assert_eq!(sequence.entries["GET"][0].format, Format::Curl);
    assert_eq!(sequence.entries["GET"][1].format, Format::Agproxy);
    assert_eq!(sequence.entries["POST"][0].format, Format::Agproxy);
    assert_eq!(sequence.entries["HEAD"][0].format, Format::Horizonpulse);
}

#[tokio::test]
async fn initial_origin_delay_is_not_consumed_without_a_representable_exchange() {
    let mut harness = Harness::new(vec![(Format::Curl, curl(200, "unused"))]).await;
    let absent = tokio::time::timeout(
        std::time::Duration::from_millis(30),
        harness.acquire_after(
            &target(Method::HEAD),
            Contract::Catalog,
            4096,
            None,
            Some(crate::network::RetryAfter::BeyondClock),
        ),
    )
    .await
    .expect("an absent method list must finish without sleeping");
    assert!(absent.is_err());
    let entries = harness.sequence.entries.remove("GET").unwrap();
    harness.sequence.entries.insert("POST".into(), entries);
    let mut request = target(Method::POST);
    request.body = Some(b"body Curl cannot represent".to_vec());
    let unsupported = tokio::time::timeout(
        std::time::Duration::from_millis(30),
        harness.acquire_after(
            &request,
            Contract::Catalog,
            4096,
            None,
            Some(crate::network::RetryAfter::BeyondClock),
        ),
    )
    .await
    .expect("an unsupported list must finish without sleeping");
    assert!(unsupported.is_err());
    harness.assert_order(&[]);
}

#[tokio::test]
async fn cached_paid_success_bypasses_initial_origin_delay_without_another_fee() {
    let harness = Harness::new(vec![(Format::Curl, curl(200, "{\"complete\":true}"))]).await;
    let state = Arc::new(tokio::sync::Mutex::new(State::default()));
    let response = harness
        .acquire(
            &target(Method::GET),
            Contract::Catalog,
            4096,
            Some(state.clone()),
        )
        .await
        .unwrap();
    let cached = tokio::time::timeout(
        std::time::Duration::from_millis(30),
        harness.acquire_after(
            &target(Method::GET),
            Contract::Catalog,
            4096,
            Some(state.clone()),
            Some(crate::network::RetryAfter::BeyondClock),
        ),
    )
    .await
    .expect("cached success must finish without sleeping")
    .unwrap();
    assert!(Arc::ptr_eq(&response, &cached));
    assert!(cached.payment_may_have_occurred);
    harness.assert_order(&[(0, false), (0, true)]);
}

#[tokio::test]
async fn cached_failure_without_server_delay_cannot_erase_initial_origin_delay() {
    let harness = Harness::new(vec![
        (Format::Curl, curl(403, "prior attributable failure")),
        (Format::Agproxy, agproxy(200, "{\"complete\":true}", false)),
    ])
    .await;
    let state = Arc::new(tokio::sync::Mutex::new(State::default()));
    let mut failure = TargetFailure::new("http_status", Some(403), true);
    failure.possible = true;
    assert!(failure.retry_after.is_none());
    let key = harness.sequence.cache_keys("wallet")[0].clone();
    state
        .lock()
        .await
        .outcomes
        .insert((key, TARGET.into()), Err(failure));
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(30),
            harness.acquire_after(
                &target(Method::GET),
                Contract::Catalog,
                4096,
                Some(state.clone()),
                Some(crate::network::RetryAfter::BeyondClock),
            ),
        )
        .await
        .is_err(),
        "cached failure traversal must preserve the independent origin delay"
    );
    harness.assert_order(&[]);
}

#[tokio::test]
async fn paid_provider_retry_after_leaves_shared_cached_work_available() {
    let mut blocked = curl(503, "definitive target failure");
    blocked["headers"] = json!({"retry-after": u64::MAX.to_string()});
    let waiting = Arc::new(
        Harness::new(vec![
            (Format::Curl, blocked),
            (Format::Agproxy, agproxy(200, "must not buy", false)),
        ])
        .await,
    );
    let mut sibling = Harness::new(vec![(Format::Curl, curl(200, "{\"cached\":true}"))]).await;
    let entry = &mut sibling.sequence.entries.get_mut("GET").unwrap()[0];
    entry.key = "cached-sibling-position".into();
    entry.provider_key = "cached-sibling-provider".into();
    let state = Arc::new(tokio::sync::Mutex::new(State::default()));
    let cached = sibling
        .acquire(
            &target(Method::GET),
            Contract::Catalog,
            4096,
            Some(state.clone()),
        )
        .await
        .unwrap();
    let first_key = (
        waiting.sequence.cache_keys("wallet")[0].clone(),
        TARGET.into(),
    );
    let owned = waiting.clone();
    let shared = state.clone();
    let pending = tokio::spawn(async move {
        owned
            .acquire(&target(Method::GET), Contract::Catalog, 4096, Some(shared))
            .await
    });
    // Wait for the actual signed response's retained outcome, not a fixture
    // handler's earlier receipt of request headers. This cadence is test-only.
    let failure = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outcome = state.lock().await.outcomes.get(&first_key).cloned();
            if let Some(Err(failure)) = outcome {
                break failure;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the paid provider outcome must finish before relay handoff waits");
    assert!(failure.possible && failure.advance);
    assert!(matches!(
        failure.retry_after,
        Some(crate::network::RetryAfter::BeyondClock)
    ));
    let reused = tokio::time::timeout(
        std::time::Duration::from_millis(30),
        sibling.acquire(
            &target(Method::GET),
            Contract::Catalog,
            4096,
            Some(state.clone()),
        ),
    )
    .await
    .expect("another provider's cached work must remain available during handoff delay")
    .unwrap();
    assert!(Arc::ptr_eq(&cached, &reused));
    assert!(
        !pending.is_finished(),
        "the next provider still owes the server delay"
    );
    waiting.assert_order(&[(0, false), (0, true)]);
    sibling.assert_order(&[(0, false), (0, true)]);
    pending.abort();
    assert!(pending.await.err().unwrap().is_cancelled());
}

#[tokio::test]
async fn disabled_paid_provider_rejects_new_target_without_waiting_for_origin_delay() {
    for reject in [false, true] {
        let body = if reject {
            super::tests::envelope(super::tests::TARGET)
        } else {
            json!({"body": "unattributable paid response"})
        };
        let (relay, fixture, task) = super::tests::fixture(body, reject, false).await;
        assert!(
            relay
                .fetch(super::tests::TARGET, 200, 4096, "max_spec_bytes")
                .await
                .is_err()
        );
        assert!(super::tests::is_idle(&relay));
        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            relay.fetch_with_capacity(
                "https://example.com/different-target.json",
                200,
                4096,
                "max_spec_bytes",
                Vec::new(),
                Some(crate::network::RetryAfter::BeyondClock),
            ),
        )
        .await
        .expect("a disabled provider must reject before entering the origin handoff delay");
        assert!(outcome.is_err());
        assert!(super::tests::is_idle(&relay));
        assert_eq!(fixture.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(fixture.signed.load(std::sync::atomic::Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn unsigned_provider_501_drops_stalled_body_before_next_paid_provider() {
    let harness = Harness::with_stalled_unsigned_501(vec![
        (Format::Curl, curl(200, "must not sign first provider")),
        (Format::Agproxy, agproxy(200, "{\"complete\":true}", false)),
    ])
    .await;
    let response = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        harness.acquire(&target(Method::GET), Contract::Catalog, 4096, None),
    )
    .await
    .expect("unsigned provider error headers must advance without reading the stalled body")
    .unwrap();
    assert_eq!(response.body, br#"{"complete":true}"#);
    assert_eq!(response.via, "agproxy");
    harness.assert_order(&[(0, false), (1, false), (1, true)]);
}
