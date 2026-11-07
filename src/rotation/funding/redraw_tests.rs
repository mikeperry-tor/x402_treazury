use super::*;
use crate::rotation::{
    config::{ReserveConfig, ReservesConfig, SizingPolicy},
    store::{AllocationKind, Store, StoreHandle},
};
use serde_json::{Value, json};

struct Fixture {
    _dir: tempfile::TempDir,
    store: StoreHandle,
    job: FundingJob,
    observations: Observations,
    _owner: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        // The store owner is a spawn_blocking task. Dropping the StoreHandle is
        // the supported shutdown path; aborting its JoinHandle does not cancel it.
    }
}
impl Fixture {
    async fn quote(&self, request: QuoteRequest) -> Result<Quote> {
        self.observations.quote(request).await
    }
    async fn saved(&self) -> Option<(String, String)> {
        let job = self.job.id.clone();
        self.store
            .call(move |s| s.quote_terms_snapshot(&job))
            .await
            .unwrap()
    }
}

async fn start(min: &str, max: &str, app: axum::Router) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let reserves = ReservesConfig {
        small: ReserveConfig {
            funding_amount_usdc: None,
            min_funding_amount_usdc: Some(min.into()),
            max_funding_amount_usdc: Some(max.into()),
        },
        large: None,
    };
    let pool = store
        .ensure_capacity_group(
            "pool",
            &reserves,
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
    store
        .allocate_capacity_kind(&pool, AllocationKind::Small, true)
        .unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    let (store, _owner) = StoreHandle::spawn(store);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let observations = Observations {
        store: store.clone(),
        near: crate::rotation::near::NearClient::test_at(&origin).unwrap(),
        base: crate::rotation::base::BaseRpc::new("http://127.0.0.1:1").unwrap(),
    };
    Fixture {
        _dir: dir,
        store,
        job,
        observations,
        _owner,
        server,
    }
}

fn tokens(app: axum::Router) -> axum::Router {
    app.route(
        "/v0/tokens",
        axum::routing::get(|| async {
            axum::Json(
                serde_json::from_str::<Value>(include_str!(
                    "../../../tests/fixtures/near/tokens.json"
                ))
                .unwrap(),
            )
        }),
    )
}

fn request(job: &FundingJob, target: u64, max_output: u64, sizing: SizingPolicy) -> QuoteRequest {
    QuoteRequest {
        job: job.clone(),
        refund: crate::rotation::near::PUBLIC_REFUND_UA.into(),
        target: target.to_string(),
        limits: crate::rotation::near::Limits { max_output },
        sizing,

        confidentiality: "public".into(),
        slippage_bps: 500,
        deadline_seconds: 3600,
    }
}

#[tokio::test]
async fn bridge_floor_redraws_uniformly_and_persists_before_the_next_preview() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let requests = seen.clone();
    let app = tokens(axum::Router::new().route(
        "/v0/quote",
        axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            let mut seen = requests.lock().unwrap();
            let dry_index = seen
                .iter()
                .filter(|prior| prior["swapType"] == "EXACT_OUTPUT")
                .count();
            seen.push(request.clone());
            async move {
                if request["swapType"] == "EXACT_OUTPUT" && dry_index == 0 {
                    return (
                        axum::http::StatusCode::BAD_REQUEST,
                        axum::Json(json!({
                            "message":"Amount is too low for bridge, try at least 4000000"
                        })),
                    );
                }
                let mut quote = json!({
                    "amountIn":"321000",
                    "amountOut":request["amount"],
                    "amountInUsd":"4.00"
                });
                if request["swapType"] == "EXACT_INPUT" {
                    quote["amountOut"] = json!("4500000");
                    quote["minAmountOut"] = json!("4400000");
                    quote["depositAddress"] = json!("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F");
                    quote["deadline"] = request["deadline"].clone();
                }
                (
                    axum::http::StatusCode::OK,
                    axum::Json(json!({"quoteRequest":request,"quote":quote})),
                )
            }
        }),
    ));
    let fixture = start("2", "5", app).await;
    let quote = fixture
        .quote(request(
            &fixture.job,
            2_000_000,
            5_000_000,
            SizingPolicy::Range {
                minimum: 2_000_000,
                maximum: 5_000_000,
            },
        ))
        .await
        .unwrap_or_else(|error| panic!("{error:#}"));
    assert_eq!(quote.output_atomic().unwrap(), 4_500_000);
    let selected = {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0]["swapType"], "EXACT_OUTPUT");
        assert_eq!(seen[0]["amount"], "2000000");
        assert_eq!(seen[1]["swapType"], "EXACT_OUTPUT");
        let selected = seen[1]["amount"].as_str().unwrap().parse::<u64>().unwrap();
        assert!((4_000_000..=5_000_000).contains(&selected));
        assert_eq!(seen[2]["swapType"], "EXACT_INPUT");
        selected
    };
    let saved = fixture.saved().await.unwrap();
    assert_eq!(saved.0, selected.to_string());
    assert_eq!(saved.1, "5000000");
}

#[tokio::test]
async fn redrawn_choice_is_persisted_before_a_failing_next_preview() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let requests = seen.clone();
    let (paused, mut paused_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, tokio::sync::oneshot::Sender<()>)>();
    let app = tokens(axum::Router::new().route(
        "/v0/quote",
        axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            let mut seen = requests.lock().unwrap();
            let dry_index = seen
                .iter()
                .filter(|prior| prior["swapType"] == "EXACT_OUTPUT")
                .count();
            seen.push(request.clone());
            let paused = paused.clone();
            async move {
                if request["swapType"] == "EXACT_OUTPUT" && dry_index == 0 {
                    return (
                        axum::http::StatusCode::BAD_REQUEST,
                        axum::Json(json!({
                            "message":"Amount is too low for bridge, try at least 4000000"
                        })),
                    );
                }
                if request["swapType"] == "EXACT_OUTPUT" {
                    let (finished, wait) = tokio::sync::oneshot::channel();
                    paused
                        .send((request["amount"].as_str().unwrap().to_owned(), finished))
                        .unwrap();
                    let _ = wait.await;
                }
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(json!({"message":"temporary failure"})),
                )
            }
        }),
    ));
    let fixture = start("2", "5", app).await;
    let (result, redrawn) = tokio::join!(
        async {
            fixture
                .quote(request(
                    &fixture.job,
                    2_000_000,
                    5_000_000,
                    SizingPolicy::Range {
                        minimum: 2_000_000,
                        maximum: 5_000_000,
                    },
                ))
                .await
        },
        async {
            let (redrawn, finished) =
                tokio::time::timeout(std::time::Duration::from_secs(30), paused_rx.recv())
                    .await
                    .expect("second preview started")
                    .unwrap();
            assert!((4_000_000..=5_000_000).contains(&redrawn.parse::<u64>().unwrap()));
            assert_eq!(
                fixture.saved().await.unwrap(),
                (redrawn.clone(), "5000000".into())
            );
            finished.send(()).unwrap();
            redrawn
        }
    );
    result.err().expect("post-redraw preview fails");
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(fixture.saved().await.unwrap(), (redrawn, "5000000".into()));
}

#[tokio::test]
async fn executable_zec_minimum_does_not_redraw_the_usdc_choice() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let requests = seen.clone();
    let app = tokens(axum::Router::new().route(
        "/v0/quote",
        axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            requests.lock().unwrap().push(request.clone());
            async move {
                if request["swapType"] == "EXACT_INPUT" {
                    return (
                        axum::http::StatusCode::BAD_REQUEST,
                        axum::Json(json!({
                            "message":"Amount is too low for bridge, try at least 400000"
                        })),
                    );
                }
                (
                    axum::http::StatusCode::OK,
                    axum::Json(json!({"quoteRequest":request,"quote":{
                        "amountIn":"321000",
                        "amountOut":request["amount"],
                        "amountInUsd":"3.00"
                    }})),
                )
            }
        }),
    ));
    let fixture = start("2", "5", app).await;
    let job = fixture.job.clone();
    fixture
        .store
        .call(move |s| s.select_quote_terms(&job.id, &job.operation_id, "2000000", "3000000"))
        .await
        .unwrap();
    let error = fixture
        .quote(request(
            &fixture.job,
            3_000_000,
            5_000_000,
            SizingPolicy::Range {
                minimum: 2_000_000,
                maximum: 5_000_000,
            },
        ))
        .await
        .err()
        .expect("executable offer rejected");
    assert_eq!(
        error
            .downcast_ref::<crate::rotation::near::BridgeMinimum>()
            .unwrap()
            .0,
        400000
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(
        fixture.saved().await.unwrap(),
        ("3000000".into(), "5000000".into())
    );
}

#[tokio::test]
async fn bridge_floor_above_the_ceiling_stops_without_a_second_preview() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = calls.clone();
    let app = tokens(axum::Router::new().route(
        "/v0/quote",
        axum::routing::post(move |axum::Json(_request): axum::Json<Value>| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(json!({
                        "message":"Amount is too low for bridge, try at least 6000000"
                    })),
                )
            }
        }),
    ));
    let fixture = start("2", "5", app).await;
    let error = fixture
        .quote(request(
            &fixture.job,
            2_000_000,
            5_000_000,
            SizingPolicy::Range {
                minimum: 2_000_000,
                maximum: 5_000_000,
            },
        ))
        .await
        .err()
        .expect("oversized bridge floor must stop");
    assert!(error.is::<crate::rotation::error::FundingCondition>());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(fixture.saved().await.is_none());
}

#[tokio::test]
async fn bridge_minimum_must_strictly_increase_within_the_authorized_ceiling() {
    for (mode, expected_calls, expected_error) in [
        ("non_increasing", 2, "near_bridge_minimum_not_increasing"),
        ("above_ceiling", 7, "funding_amount_limit_exceeded"),
    ] {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let app = tokens(axum::Router::new().route(
            "/v0/quote",
            axum::routing::post(move |axum::Json(_request): axum::Json<Value>| {
                let index = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    let minimum = if mode == "non_increasing" {
                        4_000_000
                    } else {
                        (4 + index) * 1_000_000
                    };
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        axum::Json(json!({
                            "message":format!("Amount is too low for bridge, try at least {minimum}")
                        })),
                    )
                }
            }),
        ));
        let fixture = start("2", "9", app).await;
        let error = fixture
            .quote(request(
                &fixture.job,
                2_000_000,
                9_000_000,
                SizingPolicy::Fixed {
                    target: 2_000_000,
                    maximum: 9_000_000,
                },
            ))
            .await
            .err()
            .unwrap_or_else(|| panic!("{mode}: expected unsigned failure"));
        assert!(
            format!("{error:#}").contains(expected_error),
            "{mode}: {error:#}"
        );
        if mode == "non_increasing" {
            assert!(
                safe_error(&error, &FundingPhase::Allocated)
                    .contains("provider floor did not rise"),
                "{mode}: diagnostic lost"
            );
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            expected_calls,
            "{mode}"
        );
    }
}

#[tokio::test]
async fn three_valid_floor_increases_continue_to_a_successful_quote() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let requests = seen.clone();
    let app = tokens(axum::Router::new().route(
        "/v0/quote",
        axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            let mut seen = requests.lock().unwrap();
            let index = seen.len();
            seen.push(request.clone());
            async move {
                if index < 3 {
                    return (
                        axum::http::StatusCode::BAD_REQUEST,
                        axum::Json(json!({
                            "message":format!("Amount is too low for bridge, try at least {}", (3 + index) * 1_000_000)
                        })),
                    );
                }
                let mut quote = json!({
                    "amountIn":"321000",
                    "amountOut":"5000000",
                    "amountInUsd":"5.00"
                });
                if request["swapType"] == "EXACT_INPUT" {
                    quote["minAmountOut"] = json!("4750000");
                    quote["depositAddress"] = json!("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F");
                    quote["deadline"] = request["deadline"].clone();
                }
                (
                    axum::http::StatusCode::OK,
                    axum::Json(json!({"quoteRequest":request,"quote":quote})),
                )
            }
        }),
    ));
    let fixture = start("2", "9", app).await;
    let quote = fixture
        .quote(request(
            &fixture.job,
            2_000_000,
            9_000_000,
            SizingPolicy::Fixed {
                target: 2_000_000,
                maximum: 9_000_000,
            },
        ))
        .await
        .unwrap_or_else(|error| panic!("{error:#}"));
    assert_eq!(quote.output_atomic().unwrap(), 5_000_000);
    assert_eq!(
        fixture.saved().await.unwrap(),
        ("5000000".into(), "9000000".into())
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 5, "four dry previews and one executable quote");
    for (request, expected) in seen
        .iter()
        .take(4)
        .zip(["2000000", "3000000", "4000000", "5000000"])
    {
        assert_eq!(request["swapType"], "EXACT_OUTPUT");
        assert_eq!(request["amount"], expected);
        assert_eq!(
            request["deadline"], seen[0]["deadline"],
            "retries never renew authority"
        );
    }
    assert_eq!(seen[4]["swapType"], "EXACT_INPUT");
}

#[tokio::test]
async fn an_expired_original_deadline_prevents_another_sizing_request() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };
    const EPOCH: u64 = 2_000_000_000;
    let clock = Arc::new(AtomicU64::new(EPOCH));
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let (requests, response_clock) = (seen.clone(), clock.clone());
    let app = tokens(axum::Router::new().route(
        "/v0/quote",
        axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            let mut seen = requests.lock().unwrap();
            seen.push(request);
            let minimum = if seen.len() == 1 {
                3_000_000
            } else {
                // The second progressing response reaches the original deadline.
                // Advance only this acquisition's clock, without real-time sleeps.
                response_clock.store(EPOCH + 60, Ordering::SeqCst);
                4_000_000
            };
            async move {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(json!({
                        "message": format!("Amount is too low for bridge, try at least {minimum}")
                    })),
                )
            }
        }),
    ));
    let fixture = start("2", "5", app).await;
    let mut request = request(
        &fixture.job,
        2_000_000,
        5_000_000,
        SizingPolicy::Fixed {
            target: 2_000_000,
            maximum: 5_000_000,
        },
    );
    request.deadline_seconds = 60;
    let error = fixture
        .observations
        .quote_with_clock(request, || Ok(clock.load(Ordering::SeqCst)))
        .await
        .err()
        .expect("expired quote");
    assert!(error.is::<crate::rotation::near::QuoteWindowExpired>());
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "no further preview or executable quote");
        assert_eq!(seen[0]["amount"], "2000000");
        assert_eq!(seen[1]["amount"], "3000000");
        for request in seen.iter() {
            assert_eq!(request["swapType"], "EXACT_OUTPUT");
            assert_eq!(request["deadline"], seen[0]["deadline"]);
        }
    }
    assert_eq!(
        fixture.saved().await.unwrap(),
        ("4000000".into(), "5000000".into())
    );
    let job = fixture.job.clone();
    fixture
        .store
        .call(move |s| {
            let current = s.funding_job(&job.id)?.unwrap();
            assert_eq!(current.phase, FundingPhase::Allocated);
            assert_eq!(current.operation_id, job.operation_id);
            let facts = s.funding_facts(&job.id, &job.operation_id)?;
            assert!(!facts.accepted_quote && !facts.preparation_accepted);
            assert_eq!(facts.source_attempts, 0);
            assert!(facts.source_submission.is_none());
            assert!(s.prepared_bytes(&job.operation_id).is_err());
            Ok(())
        })
        .await
        .unwrap();
}

async fn redraw_swap_types(
    sizing: SizingPolicy,
    bridge_floor: u64,
) -> (u64, Vec<(String, String)>) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
    let requests = seen.clone();
    let app = tokens(axum::Router::new().route(
        "/v0/quote",
        axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            let mut seen = requests.lock().unwrap();
            let dry_index = seen
                .iter()
                .filter(|(kind, _)| kind == "EXACT_OUTPUT")
                .count();
            seen.push((
                request["swapType"].as_str().unwrap().into(),
                request["amount"].as_str().unwrap().into(),
            ));
            drop(seen);
            async move {
                if request["swapType"] == "EXACT_OUTPUT" && dry_index == 0 {
                    return (
                        axum::http::StatusCode::BAD_REQUEST,
                        axum::Json(json!({
                            "message":format!("Amount is too low for bridge, try at least {bridge_floor}")
                        })),
                    );
                }
                let mut quote = json!({
                    "amountIn":"321000",
                    "amountOut":request["amount"],
                    "amountInUsd":"4.00"
                });
                if request["swapType"] == "EXACT_INPUT" {
                    quote["amountOut"] = json!("4500000");
                    quote["minAmountOut"] = json!("4400000");
                    quote["depositAddress"] = json!("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F");
                    quote["deadline"] = request["deadline"].clone();
                }
                (
                    axum::http::StatusCode::OK,
                    axum::Json(json!({"quoteRequest":request,"quote":quote})),
                )
            }
        }),
    ));
    let fixture = start("2", "5", app).await;
    fixture
        .quote(request(&fixture.job, 2_000_000, 5_000_000, sizing))
        .await
        .unwrap_or_else(|error| panic!("{error:#}"));
    let seen = seen.lock().unwrap().clone();
    (
        seen.iter()
            .filter(|(kind, _)| kind == "EXACT_OUTPUT")
            .nth(1)
            .map(|(_, amount)| amount.parse().unwrap())
            .unwrap(),
        seen,
    )
}

#[tokio::test]
async fn fixed_mode_bridge_floor_uses_deterministic_adjustment_and_succeeds() {
    let (selected, seen) = redraw_swap_types(
        SizingPolicy::Fixed {
            target: 2_000_000,
            maximum: 5_000_000,
        },
        4_000_000,
    )
    .await;
    assert_eq!(selected, 4_000_000);
    assert_eq!(
        seen.into_iter().map(|(kind, _)| kind).collect::<Vec<_>>(),
        ["EXACT_OUTPUT", "EXACT_OUTPUT", "EXACT_INPUT"]
    );
}

#[tokio::test]
async fn bridge_floor_equal_to_the_ceiling_redraws_to_the_ceiling() {
    let (selected, seen) = redraw_swap_types(
        SizingPolicy::Range {
            minimum: 2_000_000,
            maximum: 5_000_000,
        },
        5_000_000,
    )
    .await;
    assert_eq!(selected, 5_000_000);
    assert_eq!(
        seen.into_iter().map(|(kind, _)| kind).collect::<Vec<_>>(),
        ["EXACT_OUTPUT", "EXACT_OUTPUT", "EXACT_INPUT"]
    );
}
