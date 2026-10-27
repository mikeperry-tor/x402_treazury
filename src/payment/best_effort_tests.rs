//! Offline production PaidClient flows using the pinned seller failure encodings.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    manager::ManagedPool,
    store::{Store, StoreHandle},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{any, post},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
#[derive(Clone, Copy)]
enum Wire {
    V1(LegacyReceipt),
    Verification,
    Settlement,
    Allowance,
    Prose,
    Generic,
    Malformed,
    Oversized,
    StalledVerification,
    StalledSettlement,
    SuccessfulHeader,
    HeldSuccess,
    Expired,
    QualifiedThree,
    DuplicateVerification,
    DuplicateSettlement,
    SettledWithVerification,
    MalformedSettlementWithVerification,
    SettlementWithGenericChallenge,
}
#[derive(Clone, Copy)]
enum LegacyReceipt {
    Verification,
    Failure,
    Success,
    Malformed,
    Duplicate,
    Mixed,
}
type CapturedRequest = (SocketAddr, Method, String, Vec<u8>);
#[derive(Clone)]
struct Fixture {
    signed: Arc<Mutex<Vec<(String, String)>>>,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    rejects: usize,
    wire: Wire,
    db: std::path::PathBuf,
    rpc: Arc<AtomicUsize>,
    second_started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    authority: Option<std::path::PathBuf>,
}
fn challenge() -> Value {
    json!({"x402Version":2,"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"100","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}],"resource":{"url":"https://example.com/pay"}})
}
async fn seller(
    State(f): State<Fixture>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    assert_eq!(headers["idempotency-key"], "same-logical-call-key");
    assert_eq!(headers["x-quote-id"], "same-quote");
    f.requests
        .lock()
        .unwrap()
        .push((peer, method, uri.to_string(), body.to_vec()));
    let Some(encoded) = headers
        .get("payment-signature")
        .or_else(|| headers.get("x-payment"))
    else {
        if matches!(f.wire, Wire::V1(_)) {
            return (StatusCode::PAYMENT_REQUIRED, Json(legacy_challenge())).into_response();
        }
        return (
            StatusCode::PAYMENT_REQUIRED,
            [(
                "payment-required",
                STANDARD.encode(serde_json::to_vec(&challenge()).unwrap()),
            )],
            "{}",
        )
            .into_response();
    };
    let payload: Value =
        serde_json::from_slice(&STANDARD.decode(encoded.as_bytes()).unwrap()).unwrap();
    let payer = payload["payload"]["authorization"]["from"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        crate::test_signatures::recover_exact(&payload)
            .to_string()
            .to_lowercase(),
        payer.to_lowercase()
    );
    let nonce = payload["payload"]["authorization"]["nonce"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        rusqlite::Connection::open(&f.db)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM payment_attempts WHERE nonce=?1",
                [&nonce],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let ordinal = {
        let mut signed = f.signed.lock().unwrap();
        signed.push((payer, nonce));
        signed.len()
    };
    if matches!(f.wire, Wire::Expired)
        && let Some(path) = &f.authority
    {
        rusqlite::Connection::open(path)
            .unwrap()
            .execute("UPDATE cases SET window_end=0", [])
            .unwrap();
    }
    if ordinal > f.rejects {
        if matches!(f.wire, Wire::HeldSuccess) {
            let released = f.release.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            f.second_started.notify_one();
            released.await;
        }
        return "paid".into_response();
    }
    match f.wire {
        Wire::V1(kind) => {
            let mut challenge = legacy_challenge();
            challenge["error"] = json!("insufficient_funds");
            let mut response = (
                StatusCode::PAYMENT_REQUIRED,
                [("payment-required", STANDARD.encode(challenge.to_string()))],
                "{}",
            )
                .into_response();
            if !matches!(kind, LegacyReceipt::Verification) {
                let receipt = json!({"success":matches!(kind, LegacyReceipt::Success),"errorReason":"insufficient_funds","network":"base","transaction":""});
                let encoded = if matches!(kind, LegacyReceipt::Malformed) {
                    "not-base64".into()
                } else {
                    STANDARD.encode(receipt.to_string())
                };
                response
                    .headers_mut()
                    .insert("x-payment-response", encoded.parse().unwrap());
                if matches!(kind, LegacyReceipt::Duplicate | LegacyReceipt::Mixed) {
                    let name = if matches!(kind, LegacyReceipt::Mixed) {
                        "payment-response"
                    } else {
                        "x-payment-response"
                    };
                    response
                        .headers_mut()
                        .append(name, "not-base64".parse().unwrap());
                }
            }
            response
        }
        Wire::SettledWithVerification
        | Wire::MalformedSettlementWithVerification
        | Wire::SettlementWithGenericChallenge => {
            let fixture: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/x402/insufficient-funds.json"
            ))
            .unwrap();
            let mut settlement = fixture["settlement"].clone();
            if matches!(f.wire, Wire::SettledWithVerification) {
                settlement["success"] = json!(true);
            }
            (
                StatusCode::PAYMENT_REQUIRED,
                [
                    (
                        "payment-response",
                        if matches!(f.wire, Wire::MalformedSettlementWithVerification) {
                            "not-base64".into()
                        } else {
                            STANDARD.encode(settlement.to_string())
                        },
                    ),
                    (
                        "payment-required",
                        STANDARD.encode(
                            if matches!(f.wire, Wire::SettlementWithGenericChallenge) {
                                challenge()
                            } else {
                                fixture["verification"].clone()
                            }
                            .to_string(),
                        ),
                    ),
                ],
                "{}",
            )
                .into_response()
        }
        Wire::DuplicateVerification | Wire::DuplicateSettlement => {
            let fixture: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/x402/insufficient-funds.json"
            ))
            .unwrap();
            let (header, value) = if matches!(f.wire, Wire::DuplicateVerification) {
                ("payment-required", fixture["verification"].clone())
            } else {
                ("payment-response", fixture["settlement"].clone())
            };
            let mut response = (
                StatusCode::PAYMENT_REQUIRED,
                [(header, STANDARD.encode(value.to_string()))],
                "{}",
            )
                .into_response();
            response.headers_mut().append(
                header,
                STANDARD
                    .encode(json!({"success":true,"error":"conflicting outcome"}).to_string())
                    .parse()
                    .unwrap(),
            );
            response
        }
        Wire::Verification | Wire::StalledVerification => {
            let fixture: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/x402/insufficient-funds.json"
            ))
            .unwrap();
            let v = fixture["verification"].clone();
            let mut response = (
                StatusCode::PAYMENT_REQUIRED,
                [("payment-required", STANDARD.encode(v.to_string()))],
                "{}",
            )
                .into_response();
            if matches!(f.wire, Wire::StalledVerification) {
                *response.body_mut() = stalled_body();
            }
            response
        }
        Wire::Settlement
        | Wire::Allowance
        | Wire::Oversized
        | Wire::StalledSettlement
        | Wire::SuccessfulHeader
        | Wire::HeldSuccess
        | Wire::Expired
        | Wire::QualifiedThree => {
            let reason = if matches!(f.wire, Wire::Allowance) {
                "insufficient_allowance"
            } else {
                "insufficient_funds"
            };
            let fixture: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/x402/insufficient-funds.json"
            ))
            .unwrap();
            let mut v = fixture["settlement"].clone();
            v["errorReason"] = json!(reason);
            let mut response = (
                if matches!(f.wire, Wire::SuccessfulHeader) {
                    StatusCode::OK
                } else {
                    StatusCode::PAYMENT_REQUIRED
                },
                [("payment-response", STANDARD.encode(v.to_string()))],
                if matches!(f.wire, Wire::Oversized) {
                    "x".repeat(4096)
                } else {
                    "{}".into()
                },
            )
                .into_response();
            if matches!(f.wire, Wire::StalledSettlement) {
                *response.body_mut() = stalled_body();
            }
            response
        }
        Wire::Prose => (StatusCode::PAYMENT_REQUIRED, "insufficient_funds").into_response(),
        Wire::Generic => (StatusCode::PAYMENT_REQUIRED, Json(challenge())).into_response(),
        Wire::Malformed => (
            StatusCode::PAYMENT_REQUIRED,
            [("payment-response", "not-base64")],
            "{}",
        )
            .into_response(),
    }
}
fn stalled_body() -> axum::body::Body {
    axum::body::Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>())
}
async fn run(
    method: &str,
    wire: Wire,
    rejects: usize,
) -> (bool, Vec<(String, String)>, Vec<CapturedRequest>, String) {
    run_private(method, wire, rejects, false).await
}
async fn run_private(
    method: &str,
    wire: Wire,
    rejects: usize,
    private: bool,
) -> (bool, Vec<(String, String)>, Vec<CapturedRequest>, String) {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "pool",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(1_000_000)),
        0
    )
    .unwrap();
    for job in state.funding_jobs().unwrap() {
        state
            .record_credit(&job.wallet_id, "5000000", "fixture", 100)
            .unwrap();
    }
    if matches!(wire, Wire::QualifiedThree) {
        // Seed prior reserve consumption; the retry itself may allocate nothing.
        let first = state
            .status()
            .unwrap()
            .pools
            .remove(0)
            .addresses
            .remove(0)
            .id;
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        db.execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1", [&first])
            .unwrap();
        db.execute(
            "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1 WHERE wallet_id=?1",
            [&first],
        )
        .unwrap();
        drop(db);
        state
            .allocate_capacity_kind(&pool, crate::rotation::store::AllocationKind::Small, true)
            .unwrap();
        for job in state.funding_jobs().unwrap() {
            state
                .record_credit(&job.wallet_id, "5000000", "fixture", 100)
                .unwrap();
        }
    }
    let authority = if matches!(wire, Wire::Expired | Wire::QualifiedThree) {
        Some(crate::qualification::managed_claim_fixture().await)
    } else {
        None
    };
    let f = Fixture {
        signed: Arc::default(),
        requests: Arc::default(),
        rejects,
        wire,
        db: dir.path().join("state/state.sqlite"),
        rpc: Arc::default(),
        second_started: Arc::default(),
        release: Arc::default(),
        authority: authority.as_ref().map(|(_, _, path)| path.clone()),
    };
    let rpc = f.rpc.clone();
    let app = Router::new()
        .route("/pay", any(seller))
        .route(
            "/rpc",
            post(move |Json(v): Json<Value>| {
                let count = rpc.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(v["method"], "eth_call");
                    assert_eq!(v["params"][1], "latest");
                    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":format!("0x{:064x}",17)}))
                }
            }),
        )
        .with_state(f.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let treasury_id = state.id().to_owned();
    let (store, worker) = StoreHandle::spawn(state);
    let manager = Arc::new(
        ManagedPool::new(
            store.clone(),
            pool.clone(),
            BaseRpc::new(&format!("{origin}/rpc"), 12).unwrap(),
            SpendPolicy {
                max_atomic: Some(U256::from(1_000_000)),
            },
        )
        .unwrap(),
    );
    let client = PaidClient::managed(manager.clone()).with_download_limits(
        if matches!(wire, Wire::Oversized) {
            1024
        } else {
            16384
        },
        16384,
    );
    let mut route = RoutedRequest {
        headers: BTreeMap::from([
            ("idempotency-key".into(), "same-logical-call-key".into()),
            ("x-quote-id".into(), "same-quote".into()),
        ]),
        method: method.into(),
        url: format!("{origin}/pay"),
        query: BTreeMap::from([("original".into(), json!("same & bytes"))]),
        body: (method == "POST").then(|| json!({"original":[1,true,"same"]})),
    };
    let result = if private {
        route
            .headers
            .insert("Authorization".into(), "Bearer manual-secret".into());
        let headers = crate::discovery::raw::ManualRawHeaders::parse(Some(
            &serde_json::to_value(&route.headers).unwrap(),
        ))
        .unwrap();
        route.headers.clear();
        let (result, logs) =
            crate::limits::tests::capture(client.execute_manual_raw(route, headers)).await;
        assert!(logs.contains("Private request headers cannot be sent under another wallet"));
        assert!(!logs.contains("same-logical-call-key"));
        Some(result.map(|_| String::new()))
    } else if matches!(wire, Wire::HeldSuccess) {
        let owned = client.clone();
        let waiter = tokio::spawn(async move { owned.execute(route).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            f.second_started.notified(),
        )
        .await
        .unwrap();
        waiter.abort();
        let _ = waiter.await;
        let draining = client.clone();
        let drain = tokio::spawn(async move { draining.drain().await });
        tokio::task::yield_now().await;
        assert!(!drain.is_finished());
        f.release.notify_waiters();
        drain.await.unwrap();
        None
    } else if let Some((_, claim, _)) = &authority {
        Some(claim.invoke(client.execute(route)).await)
    } else {
        Some(
            tokio::time::timeout(std::time::Duration::from_secs(10), client.execute(route))
                .await
                .expect("payment traversal must not wait for an unused failure body"),
        )
    };
    client.drain().await;
    let success = result.as_ref().is_some_and(Result::is_ok);
    let error = result
        .and_then(Result::err)
        .map_or_else(String::new, |e| format!("{e:#}"));
    let signed = f.signed.lock().unwrap().clone();
    let requests = f.requests.lock().unwrap().clone();
    let db = rusqlite::Connection::open(&f.db).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        signed.len() as i64
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM funding_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        if matches!(wire, Wire::QualifiedThree) {
            3
        } else {
            2
        },
        "retry cannot allocate or top up"
    );
    let low: i64 = db
        .query_row(
            "SELECT count(*) FROM wallets WHERE low_balance=1 AND refresh_pending=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if matches!(
        wire,
        Wire::Verification
            | Wire::V1(LegacyReceipt::Verification | LegacyReceipt::Failure)
            | Wire::Settlement
            | Wire::HeldSuccess
            | Wire::QualifiedThree
            | Wire::Oversized
            | Wire::StalledVerification
            | Wire::StalledSettlement
    ) {
        assert_eq!(
            low,
            rejects.min(if matches!(wire, Wire::QualifiedThree) {
                3
            } else {
                2
            }) as i64
        );
    }
    assert_eq!(
        f.rpc.load(Ordering::SeqCst),
        0,
        "payment fallback must never await or perform balance RPC"
    );
    drop(db);
    // The existing maintenance owner performs the scheduled ordinary reads.
    manager.reconcile().await.unwrap();
    assert!(f.rpc.load(Ordering::SeqCst) >= 2);
    let db = rusqlite::Connection::open(&f.db).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM wallets WHERE balance='17' AND low_balance=0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        if matches!(wire, Wire::QualifiedThree) {
            3
        } else {
            2
        }
    );
    drop(db);
    if matches!(wire, Wire::QualifiedThree) {
        let (_, _, path) = authority.as_ref().unwrap();
        let db = rusqlite::Connection::open(path).unwrap();
        for kind in [
            "application_payment",
            "application_receipt",
            "application_challenge",
        ] {
            assert_eq!(
                db.query_row("SELECT count(*) FROM events WHERE kind=?1", [kind], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
                3
            );
        }
        assert_eq!(db.query_row("SELECT sum(CAST(json_extract(detail,'$.amount') AS INTEGER)) FROM events WHERE kind='application_payment'",[],|r|r.get::<_,i64>(0)).unwrap(),300);
    }
    drop(client);
    drop(manager);
    drop(store);
    worker.await.unwrap();
    let restored = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury_id,
    )
    .unwrap();
    assert_eq!(
        restored.qualification_state().unwrap()["payment_attempts"]
            .as_array()
            .unwrap()
            .len(),
        signed.len()
    );
    assert_eq!(
        f.signed.lock().unwrap().len(),
        signed.len(),
        "reopen never replays a logical call"
    );
    drop(restored);
    server.abort();
    (success, signed, requests, error)
}
fn legacy_challenge() -> Value {
    json!({"x402Version":1,"accepts":[{"scheme":"exact","network":"base","asset":USDC,"maxAmountRequired":"100","resource":"http://localhost/pay","description":"legacy fixture","mimeType":"application/json","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]})
}

#[tokio::test]
async fn v1_receipts_govern_traversal_and_preserve_each_authorization() {
    for kind in [
        LegacyReceipt::Verification,
        LegacyReceipt::Failure,
        LegacyReceipt::Success,
        LegacyReceipt::Malformed,
        LegacyReceipt::Duplicate,
        LegacyReceipt::Mixed,
    ] {
        let retry = matches!(kind, LegacyReceipt::Verification | LegacyReceipt::Failure);
        let (success, signed, requests, _) = run("POST", Wire::V1(kind), 1).await;
        assert_eq!(success, retry);
        assert_eq!(signed.len(), if retry { 2 } else { 1 });
        assert_eq!(requests.len(), if retry { 4 } else { 2 });
        if retry {
            assert_ne!(signed[0].0, signed[1].0);
            assert_ne!(signed[0].1, signed[1].1);
            assert_ne!(requests[0].0, requests[2].0);
        }
        for request in &requests {
            assert_eq!(request.1, requests[0].1);
            assert_eq!(request.2, requests[0].2);
            assert_eq!(request.3, requests[0].3);
        }
    }
}
#[tokio::test]
async fn insufficient_funds_traverses_once_with_original_get_and_post_and_isolated_challenges() {
    for method in ["GET", "POST"] {
        for wire in [Wire::Verification, Wire::Settlement] {
            let (success, signed, requests, _) = run(method, wire, 1).await;
            assert!(success);
            assert_eq!(signed.len(), 2);
            assert_ne!(signed[0].0, signed[1].0);
            assert_ne!(signed[0].1, signed[1].1);
            assert_eq!(requests.len(), 4);
            assert_eq!(requests[0].0, requests[1].0);
            assert_eq!(requests[2].0, requests[3].0);
            assert_ne!(requests[0].0, requests[2].0);
            for request in &requests {
                assert_eq!(request.1, requests[0].1);
                assert_eq!(request.2, requests[0].2);
                assert_eq!(request.3, requests[0].3);
            }
        }
    }
}
#[tokio::test]
async fn captured_inventory_exhausts_and_keeps_every_chargeable_authorization() {
    let (success, signed, requests, error) = run("GET", Wire::Settlement, 10).await;
    assert!(!success);
    assert_eq!(signed.len(), 2);
    assert_eq!(requests.len(), 4);
    assert!(error.contains("earlier authorizations may still charge"));
}
#[tokio::test]
async fn prose_allowance_generic_and_malformed_headers_earn_no_retry() {
    for wire in [Wire::Prose, Wire::Allowance, Wire::Generic, Wire::Malformed] {
        let (success, signed, requests, _) = run("GET", wire, 1).await;
        assert!(!success);
        assert_eq!(signed.len(), 1);
        assert_eq!(requests.len(), 2);
    }
}

#[tokio::test]
async fn recognized_failure_headers_allow_fallback_without_reading_oversized_or_stalled_bodies() {
    for method in ["GET", "POST"] {
        for wire in [
            Wire::Oversized,
            Wire::StalledVerification,
            Wire::StalledSettlement,
        ] {
            let (success, signed, requests, error) = run(method, wire, 1).await;
            assert!(success, "recognized headers must suffice: {error}");
            assert_eq!(signed.len(), 2);
            assert_ne!(signed[0].0, signed[1].0);
            assert_ne!(signed[0].1, signed[1].1);
            assert_eq!(requests.len(), 4);
        }
    }
}

#[tokio::test]
async fn duplicate_failure_headers_never_authorize_another_payer() {
    for wire in [Wire::DuplicateVerification, Wire::DuplicateSettlement] {
        let (success, signed, requests, _) = run("POST", wire, 1).await;
        assert!(!success, "ambiguous failure cannot authorize a repeat");
        assert_eq!(signed.len(), 1);
        assert_eq!(requests.len(), 2, "wallet B must not even be contacted");
    }
}

#[tokio::test]
async fn settlement_receipt_governs_retry_when_a_challenge_is_also_present() {
    for wire in [
        Wire::SettledWithVerification,
        Wire::MalformedSettlementWithVerification,
    ] {
        let (success, signed, requests, _) = run("POST", wire, 1).await;
        assert!(!success);
        assert_eq!(signed.len(), 1);
        assert_eq!(requests.len(), 2);
    }
    let (success, signed, requests, _) = run("POST", Wire::SettlementWithGenericChallenge, 1).await;
    assert!(
        success,
        "a generic challenge does not override the settlement failure"
    );
    assert_eq!(signed.len(), 2);
    assert_eq!(requests.len(), 4);
}

#[tokio::test]
async fn successful_paid_response_with_failure_header_never_repeats() {
    let (success, signed, requests, _) = run("GET", Wire::SuccessfulHeader, 1).await;
    assert!(success);
    assert_eq!(signed.len(), 1);
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn cancelled_waiter_and_drain_retain_both_attempts_without_replaying_after_reopen() {
    let (observed_success, signed, requests, _) = run("POST", Wire::HeldSuccess, 1).await;
    assert!(
        !observed_success,
        "cancelled delivery is not reported as success"
    );
    assert_eq!(signed.len(), 2);
    assert_eq!(requests.len(), 4);
}

#[tokio::test]
async fn expired_case_authority_after_a_forbids_even_unsigned_b_request() {
    let (success, signed, requests, _) = run("POST", Wire::Expired, 1).await;
    assert!(!success);
    assert_eq!(signed.len(), 1);
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn qualified_a_b_c_traversal_uses_original_permit_and_retains_all_observations() {
    let (success, signed, requests, _) = run("POST", Wire::QualifiedThree, 2).await;
    assert!(success);
    assert_eq!(signed.len(), 3);
    assert_eq!(requests.len(), 6);
    assert_eq!(
        signed
            .iter()
            .map(|(payer, _)| payer)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}

#[tokio::test]
async fn coherent_transport_retained_across_cache_eviction_and_unsigned_errors_stay_single_shot() {
    let status_override = Arc::new(Mutex::new(None::<StatusCode>));
    let peers = Arc::new(Mutex::new(
        Vec::<(SocketAddr, Method, String, Vec<u8>)>::new(),
    ));
    let signed_count = Arc::new(AtomicUsize::new(0));
    let (status_state, peers_state, signed_state) =
        (status_override.clone(), peers.clone(), signed_count.clone());
    let app = Router::new().route(
        "/pay",
        any(
            move |ConnectInfo(peer): ConnectInfo<SocketAddr>,
                  method: Method,
                  uri: Uri,
                  headers: HeaderMap,
                  body: Bytes| {
                let status_state = status_state.clone();
                let peers_state = peers_state.clone();
                let signed_state = signed_state.clone();
                async move {
                    peers_state.lock().unwrap().push((
                        peer,
                        method,
                        uri.to_string(),
                        body.to_vec(),
                    ));
                    if let Some(status) = *status_state.lock().unwrap() {
                        return (status, "upstream unsigned error").into_response();
                    }
                    if let Some(encoded) = headers.get("payment-signature") {
                        let payload: Value =
                            serde_json::from_slice(&STANDARD.decode(encoded.as_bytes()).unwrap())
                                .unwrap();
                        crate::test_signatures::recover_exact(&payload);
                        signed_state.fetch_add(1, Ordering::SeqCst);
                        return "paid".into_response();
                    }
                    // Evict the global HTTP cache during challenge delivery: the in-flight
                    // PaidClient call must still dispatch its signed request on the exact
                    // pinned transport and open connection.
                    let factory = crate::network::global();
                    for i in 0..260 {
                        let id = crate::network::IsolationId::treasury(&format!("evict-{i}"));
                        let _ = factory.http_transport(
                            &id,
                            &format!("http://127.0.0.1:{}", 10000 + i),
                            None,
                            false,
                            Default::default(),
                            crate::network::SelectionPolicy::Legacy,
                        );
                    }
                    (
                        StatusCode::PAYMENT_REQUIRED,
                        [(
                            "payment-required",
                            STANDARD.encode(serde_json::to_vec(&challenge()).unwrap()),
                        )],
                        "{}",
                    )
                        .into_response()
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let payer = Payer::new(&format!("{:064x}", 7), SpendPolicy::dollars("1").unwrap()).unwrap();
    let identity = crate::network::IsolationId::evm(&payer.address).unwrap();
    let client = PaidClient::new(payer);
    let pre = crate::network::global()
        .http_transport(
            &identity,
            &format!("{origin}/pay"),
            None,
            false,
            Default::default(),
            crate::network::SelectionPolicy::Legacy,
        )
        .unwrap();
    assert_eq!(pre.selection(), crate::network::SelectionPolicy::Legacy);
    assert_eq!(pre.identity, identity);
    assert_eq!(pre.scope, identity);

    for method in ["GET", "POST"] {
        peers.lock().unwrap().clear();
        let route = RoutedRequest {
            headers: Default::default(),
            method: method.into(),
            url: format!("{origin}/pay"),
            query: BTreeMap::from([("q".into(), json!("exact & value"))]),
            body: (method == "POST").then(|| json!({"payload":[1,2,3]})),
        };
        let out = client.execute_response(route).await.unwrap();
        assert_eq!(out.bytes, b"paid");
        assert!(out.paid_submission);
        let seen = peers.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[0].0, seen[1].0,
            "challenge and signed request must reuse the pinned transport connection across cache eviction"
        );
        assert_eq!(seen[0].1, seen[1].1);
        assert_eq!(seen[0].2, seen[1].2);
        assert_eq!(seen[0].3, seen[1].3);
    }
    assert_eq!(signed_count.load(Ordering::SeqCst), 2);

    for code in [
        StatusCode::BAD_REQUEST,
        StatusCode::FORBIDDEN,
        StatusCode::REQUEST_TIMEOUT,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::GATEWAY_TIMEOUT,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        *status_override.lock().unwrap() = Some(code);
        for method in ["GET", "POST"] {
            peers.lock().unwrap().clear();
            let route = RoutedRequest {
                headers: Default::default(),
                method: method.into(),
                url: format!("{origin}/pay"),
                query: BTreeMap::new(),
                body: (method == "POST").then(|| json!({"k":"v"})),
            };
            assert!(client.execute_response(route).await.is_err());
            assert_eq!(
                peers.lock().unwrap().len(),
                1,
                "direct mode unsigned {code} on {method} remains single-shot"
            );
        }
    }
    assert_eq!(signed_count.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
async fn tor_unsigned_status_retries_and_retained_promotion() {
    let result = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "payment::best_effort_tests::tor_unsigned_retry_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

type TorRequest = (SocketAddr, Method, String, Vec<u8>, bool);

#[derive(Clone, Default)]
struct TorScript {
    steps: Arc<Mutex<std::collections::VecDeque<TorStep>>>,
    observed: Arc<Mutex<Vec<TorRequest>>>,
    signed_payers: Arc<Mutex<Vec<alloy_primitives::Address>>>,
}

#[derive(Clone)]
enum TorStep {
    Status(StatusCode, Option<&'static str>),
    FailedBody(StatusCode, bool),
    Malformed402,
    Valid402(u64),
    Free200(Vec<u8>),
    Interrupted200,
    Stalled200,
    Valid402WithHook(u64, Arc<dyn Fn() + Send + Sync>),
    SignedInsufficient402,
    SignedStatus(StatusCode),
    Signed200(Vec<u8>),
}

async fn tor_script_handler(
    State(script): State<TorScript>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let is_signed = headers.contains_key("payment-signature");
    script
        .observed
        .lock()
        .unwrap()
        .push((peer, method, uri.to_string(), body.to_vec(), is_signed));
    if let Some(encoded) = headers.get("payment-signature") {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(encoded.as_bytes()).unwrap()).unwrap();
        let payer = crate::test_signatures::recover_exact(&payload);
        script.signed_payers.lock().unwrap().push(payer);
    }
    let step = script
        .steps
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or(TorStep::Signed200(b"paid".to_vec()));
    match step {
        TorStep::Status(code, retry_after) => {
            let mut resp = (code, "upstream error").into_response();
            if let Some(value) = retry_after {
                resp.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value.parse().unwrap());
            }
            resp
        }
        TorStep::FailedBody(code, stalled) => {
            let body = if stalled {
                stalled_body()
            } else {
                axum::body::Body::from(vec![b'x'; 17])
            };
            (code, [("retry-after", "60")], body).into_response()
        }
        TorStep::Malformed402 => (
            StatusCode::PAYMENT_REQUIRED,
            [("payment-required", "not-valid-base64!!!")],
            "{}",
        )
            .into_response(),
        TorStep::Valid402(amount) => {
            let mut ch = challenge();
            ch["accepts"][0]["amount"] = json!(amount.to_string());
            (
                StatusCode::PAYMENT_REQUIRED,
                [(
                    "payment-required",
                    STANDARD.encode(serde_json::to_vec(&ch).unwrap()),
                )],
                "{}",
            )
                .into_response()
        }
        TorStep::Valid402WithHook(amount, hook) => {
            hook();
            let mut ch = challenge();
            ch["accepts"][0]["amount"] = json!(amount.to_string());
            (
                StatusCode::PAYMENT_REQUIRED,
                [(
                    "payment-required",
                    STANDARD.encode(serde_json::to_vec(&ch).unwrap()),
                )],
                "{}",
            )
                .into_response()
        }
        TorStep::Free200(bytes) => {
            (StatusCode::OK, [("content-type", "text/plain")], bytes).into_response()
        }
        TorStep::SignedInsufficient402 => (
            StatusCode::PAYMENT_REQUIRED,
            [(
                "payment-response",
                STANDARD.encode(
                    serde_json::to_vec(&json!({
                        "success": false,
                        "errorReason": "insufficient_funds",
                        "network": "eip155:8453",
                        "transaction": ""
                    }))
                    .unwrap(),
                ),
            )],
            "{}",
        )
            .into_response(),
        TorStep::Interrupted200 => {
            let stream = futures_util::stream::unfold(0, |phase| async move {
                match phase {
                    0 => Some((Ok(axum::body::Bytes::from_static(b"partial")), 1)),
                    1 => {
                        // Yield Pending so Hyper flushes headers and the first
                        // chunk before interrupting this response's body.
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        Some((
                            Err(std::io::Error::other("fixture interrupted response")),
                            2,
                        ))
                    }
                    _ => None,
                }
            });
            (
                StatusCode::OK,
                [("content-type", "text/plain")],
                axum::body::Body::from_stream(stream),
            )
                .into_response()
        }
        TorStep::Stalled200 => (
            StatusCode::OK,
            [("content-type", "text/plain")],
            stalled_body(),
        )
            .into_response(),
        TorStep::SignedStatus(code) => (code, "signed upstream failure").into_response(),
        TorStep::Signed200(bytes) => {
            (StatusCode::OK, [("content-type", "text/plain")], bytes).into_response()
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "private Tor policy; invoked by parent"]
async fn tor_unsigned_retry_child() {
    use crate::{
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy, SelectionPolicy},
        test_socks::{Fault, Socks},
    };
    let script_a = TorScript::default();
    let script_b = TorScript::default();
    let listener_a = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr_a = listener_a.local_addr().unwrap();
    let listener_b = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr_b = listener_b.local_addr().unwrap();
    let app_a = Router::new()
        .route("/pay", any(tor_script_handler))
        .with_state(script_a.clone());
    let app_b = Router::new()
        .route("/pay", any(tor_script_handler))
        .with_state(script_b.clone());
    let server_a = tokio::spawn(async move {
        axum::serve(
            listener_a,
            app_a.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let server_b = tokio::spawn(async move {
        axum::serve(
            listener_b,
            app_b.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let proxy = Socks::start(
        BTreeMap::from([
            ("api-a.example.com".into(), addr_a),
            ("api-b.example.com".into(), addr_b),
            ("dead.example.com".into(), "127.0.0.1:1".parse().unwrap()),
        ]),
        Fault::None,
    )
    .await;
    let ctx = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(proxy.address),
        connect_timeout_seconds: Some(1),
        ..Default::default()
    })
    .unwrap();
    crate::network::install_test_context(ctx);

    let find_token = |peer: SocketAddr| -> (String, String) {
        let records = proxy.records.lock().unwrap();
        let rec = records
            .iter()
            .find(|r| r.upstream_peer == Some(peer))
            .expect("SOCKS record for peer");
        (rec.user.clone(), rec.password.clone())
    };

    // 1. Status matrix (400, 403, 429, 503) before 402 -> paid 200 and before free 200,
    //    across ordinals 1, 2, 3, plus exhaustion at 3 attempts, on GET and POST.
    let payer = Payer::new(&format!("{:064x}", 11), SpendPolicy::dollars("1").unwrap()).unwrap();
    let id = IsolationId::evm(&payer.address).unwrap();
    let legacy_creds = crate::network::global().credentials(&id);
    let client = PaidClient::new(payer);

    for code in [
        StatusCode::BAD_REQUEST,
        StatusCode::FORBIDDEN,
        StatusCode::REQUEST_TIMEOUT,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::GATEWAY_TIMEOUT,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        for method in ["GET", "POST"] {
            for fails in 0..=2 {
                script_a.observed.lock().unwrap().clear();
                script_a.signed_payers.lock().unwrap().clear();
                {
                    let mut steps = script_a.steps.lock().unwrap();
                    steps.clear();
                    for _ in 0..fails {
                        steps.push_back(TorStep::Status(code, None));
                    }
                    steps.push_back(TorStep::Valid402(1));
                    steps.push_back(TorStep::Signed200(b"paid-ok".to_vec()));
                }
                let make_matrix_route = || RoutedRequest {
                    headers: Default::default(),
                    method: method.into(),
                    url: "http://api-a.example.com/pay".into(),
                    query: BTreeMap::from([("q".into(), json!("exact & value"))]),
                    body: (method == "POST").then(|| json!({"payload":[1,2,3]})),
                };
                let out = client.execute_response(make_matrix_route()).await.unwrap();
                assert_eq!(out.bytes, b"paid-ok");
                assert!(out.paid_submission);
                let obs = script_a.observed.lock().unwrap().clone();
                assert_eq!(obs.len(), fails + 2, "unsigned retries + 402 + signed 200");
                let mut unsigned_tokens = Vec::new();
                for (i, (peer, m, uri, body, signed)) in obs.iter().enumerate() {
                    assert_eq!(body, &obs[0].3, "exact repeated wire bytes");
                    assert_eq!(m.as_str(), method);
                    assert_eq!(uri, "/pay?q=exact+%26+value");
                    if method == "POST" {
                        assert_eq!(
                            serde_json::from_slice::<Value>(body).unwrap(),
                            json!({"payload":[1,2,3]})
                        );
                    } else {
                        assert!(body.is_empty());
                    }
                    let (user, pass) = find_token(*peer);
                    assert_eq!(user, legacy_creds.0, "canonical owner user stays pinned");
                    assert_ne!(pass, legacy_creds.1, "participating scope is fresh");
                    if i <= fails {
                        assert!(!signed);
                        unsigned_tokens.push(pass);
                    } else {
                        assert!(signed);
                        assert_eq!(
                            &pass,
                            unsigned_tokens.last().unwrap(),
                            "signed request must use the exact winning challenge scope"
                        );
                    }
                }
                let unique: std::collections::BTreeSet<_> = unsigned_tokens.iter().collect();
                assert_eq!(
                    unique.len(),
                    unsigned_tokens.len(),
                    "each retry attempt uses a distinct fresh scope"
                );
                let winner = unsigned_tokens.last().unwrap().clone();
                let current_selected = crate::network::global()
                    .http_participating(
                        &id,
                        "http://api-a.example.com/pay",
                        None,
                        false,
                        Default::default(),
                    )
                    .unwrap();
                assert_eq!(
                    crate::network::global()
                        .credentials(&current_selected.scope)
                        .1,
                    winner,
                    "valid 402 promotes winning candidate scope into the cache slot"
                );

                // Free success at each ordinal follows the same method/status policy.
                script_a.observed.lock().unwrap().clear();
                {
                    let mut steps = script_a.steps.lock().unwrap();
                    steps.clear();
                    for _ in 0..fails {
                        steps.push_back(TorStep::Status(code, None));
                    }
                    steps.push_back(TorStep::Free200(b"free-ok".to_vec()));
                }
                let out = client.execute_response(make_matrix_route()).await.unwrap();
                assert_eq!(out.bytes, b"free-ok");
                assert!(!out.paid_submission);
                let obs = script_a.observed.lock().unwrap().clone();
                assert_eq!(obs.len(), fails + 1);
                for entry in &obs {
                    assert_eq!(
                        (&entry.1, &entry.2, &entry.3),
                        (&obs[0].1, &obs[0].2, &obs[0].3)
                    );
                    assert!(!entry.4);
                }
                assert_eq!(
                    find_token(obs[0].0).1,
                    winner,
                    "attempt 1 uses prior winner"
                );
                let free_winner = find_token(obs.last().unwrap().0).1;
                if fails > 0 {
                    assert_ne!(free_winner, winner);
                } else {
                    assert_eq!(free_winner, winner);
                }
                let after_free = crate::network::global()
                    .http_participating(
                        &id,
                        "http://api-a.example.com/pay",
                        None,
                        false,
                        Default::default(),
                    )
                    .unwrap();
                assert_eq!(
                    crate::network::global().credentials(&after_free.scope).1,
                    free_winner,
                    "completed valid unsigned 200 promotes candidate scope"
                );
            }
        }
    }
    // Exhaustion after 3 attempts (selected + 2 candidates) on eligible status:
    let before_exhaust = crate::network::global()
        .http_participating(
            &id,
            "http://api-a.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    script_a.observed.lock().unwrap().clear();
    let signed_before = script_a.signed_payers.lock().unwrap().len();
    {
        let mut steps = script_a.steps.lock().unwrap();
        steps.clear();
        for _ in 0..4 {
            steps.push_back(TorStep::Status(StatusCode::SERVICE_UNAVAILABLE, None));
        }
    }
    let make_route = || RoutedRequest {
        headers: Default::default(),
        method: "POST".into(),
        url: "http://api-a.example.com/pay".into(),
        query: BTreeMap::new(),
        body: Some(json!({"exhaust":true})),
    };
    use tracing::instrument::WithSubscriber;
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let error = client
        .execute_response(make_route())
        .with_subscriber(subscriber)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<HttpFailure>().unwrap().status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let logs = std::fs::read_to_string(log.path()).unwrap();
    let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
    assert_eq!(warnings.len(), 1, "{logs}");
    for expected in [
        "unsigned_attempt=3",
        "max_unsigned_attempts=3",
        "authorization_may_charge=false",
        "last_http_status=503",
    ] {
        assert!(warnings[0].contains(expected), "{logs}");
    }
    assert!(!logs.contains("do not automatically repeat"), "{logs}");
    assert_eq!(
        script_a.observed.lock().unwrap().len(),
        3,
        "exhausts after exactly 3 attempts"
    );
    assert_eq!(script_a.signed_payers.lock().unwrap().len(), signed_before);
    let after_exhaust = crate::network::global()
        .http_participating(
            &id,
            "http://api-a.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    assert!(
        Arc::ptr_eq(&before_exhaust, &after_exhaust),
        "exhausted retries never promote a failed candidate"
    );

    // Every failed participating-Tor status discards its body, including the
    // final attempt. A known exhaustion error must survive stalled/oversized
    // final bodies and must not wait on their Retry-After without a next dispatch.
    let exhaustion_client = client
        .clone()
        .with_download_limits(16, 16)
        .with_timeout(std::time::Duration::from_millis(100));
    let exhaustion_selected = || {
        crate::network::global()
            .http_participating(
                &id,
                "http://api-a.example.com/pay",
                Some(std::time::Duration::from_millis(100)),
                false,
                Default::default(),
            )
            .unwrap()
    };
    for code in [
        StatusCode::BAD_REQUEST,
        StatusCode::FORBIDDEN,
        StatusCode::REQUEST_TIMEOUT,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::GATEWAY_TIMEOUT,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        for method in ["GET", "POST"] {
            for stalled in [false, true] {
                let before = exhaustion_selected();
                script_a.observed.lock().unwrap().clear();
                script_a.signed_payers.lock().unwrap().clear();
                *script_a.steps.lock().unwrap() = [
                    TorStep::Status(code, None),
                    TorStep::Status(code, None),
                    TorStep::FailedBody(code, stalled),
                ]
                .into();
                let mut route = make_route();
                route.method = method.into();
                route.body = (method == "POST").then(|| json!({"exhaust":true}));
                let error = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    exhaustion_client.execute_response(route),
                )
                .await
                .expect("exhaustion must not collect an unused body or honor a final Retry-After")
                .unwrap_err();
                let failure = error.downcast_ref::<HttpFailure>().expect(
                    "preserve the known HTTP status instead of a download/inactivity failure",
                );
                assert_eq!(failure.status, code);
                assert_eq!(failure.detail, "unsigned Tor isolation attempts exhausted");
                assert_eq!(script_a.observed.lock().unwrap().len(), 3);
                assert!(script_a.signed_payers.lock().unwrap().is_empty());
                assert!(Arc::ptr_eq(&before, &exhaustion_selected()));
            }
        }
    }

    // 2. Ineligible unsigned statuses (401, 404, 501, 302) and unsigned transport failure:
    for code in [
        StatusCode::UNAUTHORIZED,
        StatusCode::NOT_FOUND,
        StatusCode::NOT_IMPLEMENTED,
        StatusCode::FOUND,
    ] {
        script_a.observed.lock().unwrap().clear();
        {
            let mut steps = script_a.steps.lock().unwrap();
            steps.clear();
            steps.push_back(TorStep::Status(code, None));
            steps.push_back(TorStep::Free200(b"unreachable".to_vec()));
        }
        assert!(client.execute_response(make_route()).await.is_err());
        assert_eq!(
            script_a.observed.lock().unwrap().len(),
            1,
            "ineligible unsigned status {code} must not retry"
        );
    }
    for method in ["GET", "POST"] {
        let before_dead = proxy.records.lock().unwrap().len();
        let before_signed = script_a.signed_payers.lock().unwrap().len();
        let dead_route = RoutedRequest {
            headers: Default::default(),
            method: method.into(),
            url: "http://dead.example.com/pay".into(),
            query: BTreeMap::new(),
            body: (method == "POST").then(|| json!({"original":"same"})),
        };
        assert!(client.execute_response(dead_route).await.is_err());
        assert_eq!(
            proxy.records.lock().unwrap().len() - before_dead,
            3,
            "eligible unsigned {method} transport failures use the shared attempt sequence"
        );
        assert_eq!(script_a.signed_payers.lock().unwrap().len(), before_signed);
    }

    // 3. No promotion on malformed 402, oversized/invalid output, or failed rendered output;
    //    other-origin and discovery independence; internal relay stays Legacy single-shot.
    let other_origin_pre = crate::network::global()
        .http_participating(
            &id,
            "http://api-b.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    let disc_id = IsolationId::discovery("http://api-a.example.com/pay").unwrap();
    let disc_pre = crate::network::global()
        .http_participating(
            &disc_id,
            "http://api-a.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    let selected_before_bad = crate::network::global()
        .http_participating(
            &id,
            "http://api-a.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();

    // 3a. Malformed 402 on candidate attempt 2 does not promote:
    {
        let mut steps = script_a.steps.lock().unwrap();
        steps.clear();
        steps.push_back(TorStep::Status(StatusCode::FORBIDDEN, None));
        steps.push_back(TorStep::Malformed402);
    }
    assert!(client.execute_response(make_route()).await.is_err());
    let after_malformed = crate::network::global()
        .http_participating(
            &id,
            "http://api-a.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&selected_before_bad, &after_malformed));

    // 3b. Invalid UTF-8 on unmapped execute and failed mapping on execute_rendered_scoped do not promote:
    {
        let mut steps = script_a.steps.lock().unwrap();
        steps.clear();
        steps.push_back(TorStep::Status(StatusCode::FORBIDDEN, None));
        steps.push_back(TorStep::Free200(vec![0xff, 0xfe]));
    }
    assert!(client.execute(make_route()).await.is_err());
    let after_bad_utf8 = crate::network::global()
        .http_participating(
            &id,
            "http://api-a.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&selected_before_bad, &after_bad_utf8));

    {
        let mut steps = script_a.steps.lock().unwrap();
        steps.clear();
        steps.push_back(TorStep::Status(StatusCode::FORBIDDEN, None));
        steps.push_back(TorStep::Free200(b"not-json".to_vec()));
    }
    let mapping = crate::output::ResponseMapping {
        images: vec![crate::output::ImageField {
            pointer: "/image".into(),
            encoding: crate::output::ImageEncoding::Base64,
            mime_type: Some("image/png".into()),
            mime_pointer: None,
        }],
    };
    assert!(
        client
            .execute_rendered_scoped(
                make_route(),
                None,
                Some(&mapping),
                &crate::output::ImageLimits::default()
            )
            .await
            .is_err()
    );
    let after_bad_render = crate::network::global()
        .http_participating(
            &id,
            "http://api-a.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&selected_before_bad, &after_bad_render));

    // Rejected, interrupted and truly inactive unsigned bodies cannot promote;
    // the same failures after a candidate challenge cannot replay signed work.
    let bounded = client
        .clone()
        .with_download_limits(16, 16)
        .with_timeout(std::time::Duration::from_millis(100));
    let bounded_selected = || {
        crate::network::global()
            .http_participating(
                &id,
                "http://api-a.example.com/pay",
                Some(std::time::Duration::from_millis(100)),
                false,
                Default::default(),
            )
            .unwrap()
    };
    let before_bodies = bounded_selected();
    for body_failure in [
        TorStep::Free200(vec![b'x'; 17]),
        TorStep::Interrupted200,
        TorStep::Stalled200,
    ] {
        script_a.observed.lock().unwrap().clear();
        script_a.signed_payers.lock().unwrap().clear();
        *script_a.steps.lock().unwrap() = [
            TorStep::Status(StatusCode::FORBIDDEN, None),
            body_failure.clone(),
        ]
        .into();
        let transport_failure =
            matches!(body_failure, TorStep::Interrupted200 | TorStep::Stalled200);
        if transport_failure {
            script_a
                .steps
                .lock()
                .unwrap()
                .push_back(body_failure.clone());
        }
        let error = bounded.execute_response(make_route()).await.unwrap_err();
        if matches!(body_failure, TorStep::Interrupted200 | TorStep::Stalled200) {
            assert!(
                error.chain().any(|e| e
                    .downcast_ref::<reqwest::Error>()
                    .is_some_and(reqwest::Error::is_decode)),
                "must fail collecting the response body: {error:#}"
            );
        }
        assert_eq!(
            script_a.observed.lock().unwrap().len(),
            if transport_failure { 3 } else { 2 }
        );
        assert!(script_a.signed_payers.lock().unwrap().is_empty());
        assert!(
            Arc::ptr_eq(&before_bodies, &bounded_selected()),
            "invalid unsigned body cannot promote"
        );
    }
    // Use the ordinary download bound for 402 envelope validation, while keeping
    // the short fixture inactivity allowance for stalled signed bodies.
    let paid_bodies = client
        .clone()
        .with_timeout(std::time::Duration::from_millis(100));
    for body_failure in [
        TorStep::Free200(vec![0xff]),
        TorStep::Interrupted200,
        TorStep::Stalled200,
    ] {
        script_a.observed.lock().unwrap().clear();
        script_a.signed_payers.lock().unwrap().clear();
        *script_a.steps.lock().unwrap() = [
            TorStep::Status(StatusCode::FORBIDDEN, None),
            TorStep::Valid402(1),
            body_failure.clone(),
            TorStep::Free200(b"forbidden replay".to_vec()),
        ]
        .into();
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let error = paid_bodies
            .execute(make_route())
            .with_subscriber(subscriber)
            .await
            .unwrap_err();
        let logs = std::fs::read_to_string(log.path()).unwrap();
        let terminal: Vec<_> = logs
            .lines()
            .filter(|line| line.contains("WARN") && line.contains("Payment call failed"))
            .collect();
        assert_eq!(terminal.len(), 1, "{logs}");
        assert!(
            terminal[0].contains("authorization_may_charge=true"),
            "{logs}"
        );
        assert!(
            terminal[0].contains("do not automatically repeat"),
            "{logs}"
        );
        assert!(!logs.contains("forbidden replay"), "{logs}");
        if matches!(body_failure, TorStep::Interrupted200 | TorStep::Stalled200) {
            assert!(
                error.chain().any(|e| e
                    .downcast_ref::<reqwest::Error>()
                    .is_some_and(reqwest::Error::is_decode)),
                "must fail collecting the paid response body: {error:#}"
            );
        }
        assert_eq!(script_a.observed.lock().unwrap().len(), 3);
        assert_eq!(script_a.signed_payers.lock().unwrap().len(), 1);
        let obs = script_a.observed.lock().unwrap().clone();
        assert_eq!(find_token(obs[1].0), find_token(obs[2].0));
    }

    // 3c. Same-wallet other-origin and discovery identity on same origin remain untouched:
    let other_origin_post = crate::network::global()
        .http_participating(
            &id,
            "http://api-b.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&other_origin_pre, &other_origin_post));
    let disc_post = crate::network::global()
        .http_participating(
            &disc_id,
            "http://api-a.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&disc_pre, &disc_post));

    // 3d. Internal relay client (SelectionPolicy::Legacy) stays single-attempt and uses legacy credentials:
    let relay_client = PaidClient::new(
        Payer::new(&format!("{:064x}", 11), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
    .with_selection(SelectionPolicy::Legacy);
    script_a.observed.lock().unwrap().clear();
    {
        let mut steps = script_a.steps.lock().unwrap();
        steps.clear();
        steps.push_back(TorStep::Status(StatusCode::SERVICE_UNAVAILABLE, None));
        steps.push_back(TorStep::Free200(b"unreachable".to_vec()));
    }
    assert!(relay_client.execute_response(make_route()).await.is_err());
    let relay_obs = script_a.observed.lock().unwrap().clone();
    assert_eq!(
        relay_obs.len(),
        1,
        "Legacy selection relay call makes 1 attempt"
    );
    assert_eq!(
        find_token(relay_obs[0].0),
        legacy_creds,
        "Legacy selection uses deterministic identity token"
    );

    // 4. Interleaved valid 402 with replacement and cache eviction before signing:
    let id_for_hook = id.clone();
    let hook = Arc::new(move || {
        let factory = crate::network::global();
        let current = factory
            .http_participating(
                &id_for_hook,
                "http://api-a.example.com/pay",
                None,
                false,
                Default::default(),
            )
            .unwrap();
        let competing = factory.http_candidate(&current).unwrap();
        assert!(factory.promote_http(&current, &competing));
        for i in 0..260 {
            let evict_id = IsolationId::treasury(&format!("tor-evict-{i}"));
            let _ = factory.http_transport(
                &evict_id,
                &format!("http://127.0.0.1:{}", 20000 + i),
                None,
                false,
                Default::default(),
                SelectionPolicy::Legacy,
            );
        }
    });
    script_a.observed.lock().unwrap().clear();
    {
        let mut steps = script_a.steps.lock().unwrap();
        steps.clear();
        steps.push_back(TorStep::Status(StatusCode::FORBIDDEN, None));
        steps.push_back(TorStep::Valid402WithHook(1, hook));
        steps.push_back(TorStep::Signed200(b"interleaved-ok".to_vec()));
    }
    let out = client.execute_response(make_route()).await.unwrap();
    assert_eq!(out.bytes, b"interleaved-ok");
    let obs = script_a.observed.lock().unwrap().clone();
    assert_eq!(obs.len(), 3);
    assert_eq!(
        find_token(obs[1].0).1,
        find_token(obs[2].0).1,
        "in-flight call retains its winning challenge scope across competing promotion and cache eviction"
    );

    // 5. Signed 400, 403, 429, 503 never replay or create a second signature:
    for code in [
        StatusCode::BAD_REQUEST,
        StatusCode::FORBIDDEN,
        StatusCode::REQUEST_TIMEOUT,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::GATEWAY_TIMEOUT,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        script_a.observed.lock().unwrap().clear();
        script_a.signed_payers.lock().unwrap().clear();
        {
            let mut steps = script_a.steps.lock().unwrap();
            steps.clear();
            steps.push_back(TorStep::Valid402(1));
            steps.push_back(TorStep::SignedStatus(code));
            steps.push_back(TorStep::Signed200(b"never".to_vec()));
        }
        assert!(client.execute_response(make_route()).await.is_err());
        assert_eq!(
            script_a.observed.lock().unwrap().len(),
            2,
            "signed {code} must never trigger an unsigned or signed replay"
        );
        assert_eq!(script_a.signed_payers.lock().unwrap().len(), 1);
    }

    // 6. Combined managed structured insufficiency + payer handoff + unsigned circuit retry
    //    + Retry-After + explicit owner abort + qualification authority check:
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"tor managed fixture",
    )
    .unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        store,
        "pool",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(5_000_000)),
        0
    )
    .unwrap();
    for job in store.funding_jobs().unwrap() {
        store
            .record_credit(&job.wallet_id, "5000000", "fixture", 100)
            .unwrap();
    }
    let (handle, _worker) = StoreHandle::spawn(store);
    let manager = Arc::new(
        ManagedPool::new(
            handle.clone(),
            pool.clone(),
            BaseRpc::new(&format!("http://{addr_b}/rpc"), 12).unwrap(),
            SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap(),
    );
    let managed = PaidClient::managed(manager);
    script_b.observed.lock().unwrap().clear();
    script_b.signed_payers.lock().unwrap().clear();
    {
        let mut steps = script_b.steps.lock().unwrap();
        steps.clear();
        // Wallet A: attempt 1 -> 503, attempt 2 -> Valid402(1), signed -> Insufficient402
        steps.push_back(TorStep::Status(StatusCode::SERVICE_UNAVAILABLE, None));
        steps.push_back(TorStep::Valid402(1));
        steps.push_back(TorStep::SignedInsufficient402);
        // Wallet B: attempt 1 -> 403, attempt 2 -> Valid402(1), signed -> Signed200
        steps.push_back(TorStep::Status(StatusCode::FORBIDDEN, None));
        steps.push_back(TorStep::Valid402(1));
        steps.push_back(TorStep::Signed200(b"managed-paid".to_vec()));
    }
    let managed_route = RoutedRequest {
        headers: Default::default(),
        method: "POST".into(),
        url: "http://api-b.example.com/pay".into(),
        query: BTreeMap::from([("mode".into(), json!("handoff"))]),
        body: Some(json!({"step":"combined"})),
    };
    let (_qualified_dir, claim, qualification_path) =
        crate::qualification::managed_claim_fixture().await;
    let out = claim
        .invoke(managed.execute_response(managed_route))
        .await
        .unwrap();
    assert_eq!(out.bytes, b"managed-paid");
    let obs_b = script_b.observed.lock().unwrap().clone();
    assert_eq!(
        obs_b.len(),
        6,
        "A1 unsigned 503, A2 unsigned 402, A2 signed 402, B1 unsigned 403, B2 unsigned 402, B2 signed 200"
    );
    let payers = script_b.signed_payers.lock().unwrap().clone();
    assert_eq!(payers.len(), 2);
    assert_ne!(
        payers[0], payers[1],
        "handoff switches from wallet A to wallet B"
    );
    let tok_a1 = find_token(obs_b[0].0);
    let tok_a2 = find_token(obs_b[1].0);
    let tok_a_signed = find_token(obs_b[2].0);
    let tok_b1 = find_token(obs_b[3].0);
    let tok_b2 = find_token(obs_b[4].0);
    let tok_b_signed = find_token(obs_b[5].0);
    assert_eq!(tok_a1.0, tok_a2.0);
    assert_ne!(tok_a1.1, tok_a2.1);
    assert_eq!(
        tok_a2, tok_a_signed,
        "wallet A signed uses promoted A2 scope"
    );
    assert_eq!(tok_b1.0, tok_b2.0);
    assert_ne!(tok_b1.1, tok_b2.1);
    assert_eq!(
        tok_b2, tok_b_signed,
        "wallet B signed uses promoted B2 scope"
    );
    let slot_a = crate::network::global()
        .http_participating(
            &IsolationId::evm(&payers[0].to_string()).unwrap(),
            "http://api-b.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    let slot_b = crate::network::global()
        .http_participating(
            &IsolationId::evm(&payers[1].to_string()).unwrap(),
            "http://api-b.example.com/pay",
            None,
            false,
            Default::default(),
        )
        .unwrap();
    assert_eq!(
        crate::network::global().credentials(&slot_a.scope),
        tok_a2,
        "wallet A's participating slot holds promoted A2 scope"
    );
    assert_eq!(
        crate::network::global().credentials(&slot_b.scope),
        tok_b2,
        "wallet B's participating slot holds promoted B2 scope"
    );
    let qualification_db = rusqlite::Connection::open(qualification_path).unwrap();
    for kind in [
        "application_payment",
        "application_receipt",
        "application_challenge",
    ] {
        assert_eq!(
            qualification_db
                .query_row("SELECT count(*) FROM events WHERE kind=?1", [kind], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            2
        );
    }
    assert_eq!(qualification_db.query_row("SELECT sum(CAST(json_extract(detail,'$.amount') AS INTEGER)) FROM events WHERE kind='application_payment'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    let status_after = handle.call(|s| s.status()).await.unwrap();
    assert_eq!(
        status_after.pools[0].addresses.len(),
        2,
        "no wallet allocation during unsigned retry or insufficient-funds traversal"
    );
    assert_eq!(
        status_after.funding_jobs.len(),
        2,
        "only the two initial seeded funding jobs exist; no new funding job allocated"
    );
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2,
        "both signed attempts are journaled while unsigned failures add zero liabilities"
    );

    server_a.abort();
    server_b.abort();
}

#[tokio::test]
async fn prior_wallet_liability_suppresses_relay_after_later_unsigned_exhaustion() {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .args([
            "--ignored",
            "--exact",
            "payment::best_effort_tests::prior_wallet_liability_relay_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "private Tor policy; invoked by parent"]
async fn prior_wallet_liability_relay_child() {
    use crate::network::{IsolationId, Mode, NetworkContext, NetworkPolicy};
    let scopes = Arc::new(Mutex::new(Vec::<crate::network::ScopeObservation>::new()));
    let observed_scopes = scopes.clone();
    crate::network::install_http_scope_observer(move |observation| {
        observed_scopes.lock().unwrap().push(observation.clone());
        Ok(())
    })
    .unwrap();
    let script = TorScript::default();
    let relay_calls = Arc::new(AtomicUsize::new(0));
    let rpc_calls = Arc::new(AtomicUsize::new(0));
    // Existing scripted response semantics are reused; peer matching is not part
    // of this fixture. Canonical payer identities are checked on the SOCKS wire.
    let app = Router::new()
        .route("/pay", any(tor_script_handler))
        .with_state(script.clone())
        .layer(axum::Extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap())))
        .route("/relay", post({
            let calls = relay_calls.clone();
            move |Json(request): Json<Value>| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(request["method"], "GET");
                    assert_eq!(request["url"], "https://api.example.com/pay?original=same+%26+bytes");
                    Json(json!({"url":request["url"],"method":"GET","statusCode":200,"headers":{"content-type":"text/plain"},"body":"relay-positive-control","redirectChain":[],"error":null}))
                }
            }
        }))
        .route("/rpc", post({
            let calls = rpc_calls.clone();
            move |Json(request): Json<Value>| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":format!("0x{:064x}",17)}))
                }
            }
        }));
    let (address, server) = crate::test_tls::serve(app).await;
    let proxy = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let relay = Arc::new(
        crate::discovery_relay::SearchRelay::fixture(
            "https://spec.example.com/relay".into(),
            crate::catalog::Config::default(),
        )
        .with_policy(crate::discovery_relay::Policy {
            api_requests: true,
            ..Default::default()
        }),
    );
    let route = || RoutedRequest {
        headers: Default::default(),
        method: "GET".into(),
        url: "https://api.example.com/pay".into(),
        query: BTreeMap::from([("original".into(), json!("same & bytes"))]),
        body: None,
    };
    let static_client = PaidClient::new(
        Payer::new(&format!("{:064x}", 13), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
    .with_document_relay(Some(relay.clone()));
    *script.steps.lock().unwrap() = [
        TorStep::Status(StatusCode::FORBIDDEN, None),
        TorStep::Status(StatusCode::FORBIDDEN, None),
        TorStep::Status(StatusCode::FORBIDDEN, None),
    ]
    .into();
    assert_eq!(
        static_client.execute_response(route()).await.unwrap().bytes,
        b"relay-positive-control"
    );
    assert_eq!(
        relay_calls.load(Ordering::SeqCst),
        1,
        "the same public URL and configured handle must have an operative fallback path"
    );
    assert_eq!(script.observed.lock().unwrap().len(), 3);
    assert!(script.signed_payers.lock().unwrap().is_empty());
    relay_calls.store(0, Ordering::SeqCst);
    proxy.records.lock().unwrap().clear();
    scopes.lock().unwrap().clear();
    script.observed.lock().unwrap().clear();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"managed relay liability fixture",
    )
    .unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        store,
        "pool",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(5_000_000)),
        0
    )
    .unwrap();
    for job in store.funding_jobs().unwrap() {
        store
            .record_credit(&job.wallet_id, "5000000", "fixture", 100)
            .unwrap();
    }
    let captured = store.status().unwrap().pools.remove(0).addresses;
    assert_eq!(captured.len(), 2);
    let (handle, worker) = StoreHandle::spawn(store);
    let manager = Arc::new(
        ManagedPool::new(
            handle.clone(),
            pool,
            BaseRpc::new("https://api.example.com/rpc", 12).unwrap(),
            SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap(),
    );
    let managed = PaidClient::managed(manager.clone()).with_document_relay(Some(relay));
    *script.steps.lock().unwrap() = [
        TorStep::Valid402(1),
        TorStep::SignedInsufficient402,
        TorStep::Status(StatusCode::SERVICE_UNAVAILABLE, None),
        TorStep::Status(StatusCode::SERVICE_UNAVAILABLE, None),
        TorStep::Status(StatusCode::SERVICE_UNAVAILABLE, None),
    ]
    .into();
    let error = managed.execute_response(route()).await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<HttpFailure>().unwrap().status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let observed = script.observed.lock().unwrap().clone();
    assert_eq!(
        observed.len(),
        5,
        "A's challenge/signature, then B's complete unsigned attempt sequence"
    );
    assert_eq!(
        observed.iter().map(|request| request.4).collect::<Vec<_>>(),
        [false, true, false, false, false]
    );
    let payers = script.signed_payers.lock().unwrap().clone();
    assert_eq!(payers.len(), 1, "B never receives a challenge or signs");
    let a = payers[0].to_string();
    assert!(
        captured
            .iter()
            .any(|wallet| wallet.address.eq_ignore_ascii_case(&a))
    );
    let b = &captured
        .iter()
        .find(|wallet| !wallet.address.eq_ignore_ascii_case(&a))
        .unwrap()
        .address;
    let owner_a = crate::network::global().credentials(&IsolationId::evm(&a).unwrap());
    let owner_b = crate::network::global().credentials(&IsolationId::evm(b).unwrap());
    assert_ne!(owner_a, owner_b);
    let records = proxy.records.lock().unwrap().clone();
    assert!(
        records
            .iter()
            .all(|record| record.host == "api.example.com")
    );
    let observations = scopes.lock().unwrap().clone();
    let mut order: Vec<_> = records
        .into_iter()
        .map(|record| {
            let wire_scope = (record.user, record.password);
            observations
                .iter()
                .find(|observation| observation.scope == wire_scope)
                .expect("every actual SOCKS request belongs to an observed owner-bound scope")
                .canonical
                .clone()
        })
        .collect();
    order.dedup();
    assert_eq!(
        order,
        [owner_a, owner_b],
        "actual SOCKS scope credentials preserve the captured A-to-B payer handoff"
    );
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let (journaled, payer): (i64, String) = db
        .query_row("SELECT count(*),payer FROM payment_attempts", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(journaled, 1);
    assert!(payer.eq_ignore_ascii_case(&a));
    assert_eq!(
        db.query_row("SELECT count(*) FROM wallets", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM funding_jobs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        rpc_calls.load(Ordering::SeqCst),
        0,
        "unsigned failure and handoff cannot start funding or balance RPC"
    );
    assert_eq!(
        relay_calls.load(Ordering::SeqCst),
        0,
        "A's possible liability suppresses relay handoff after B's unsigned failure"
    );
    drop(db);

    // The operative handle must also remain unused after a signed HTTP, body or
    // output failure in an ordinary single-wallet call.
    for failure in [
        TorStep::SignedStatus(StatusCode::SERVICE_UNAVAILABLE),
        TorStep::Free200(vec![0xff]),
        TorStep::Interrupted200,
    ] {
        script.observed.lock().unwrap().clear();
        script.signed_payers.lock().unwrap().clear();
        *script.steps.lock().unwrap() = [
            TorStep::Valid402(1),
            failure,
            TorStep::Signed200(b"forbidden replay".to_vec()),
        ]
        .into();
        assert!(static_client.execute(route()).await.is_err());
        assert_eq!(script.observed.lock().unwrap().len(), 2);
        assert_eq!(script.signed_payers.lock().unwrap().len(), 1);
        assert_eq!(relay_calls.load(Ordering::SeqCst), 0);
    }
    managed.drain().await;
    static_client.drain().await;
    drop(managed);
    drop(manager);
    drop(handle);
    worker.await.unwrap();
    server.abort();
}

#[tokio::test]
async fn manual_headers_stop_insufficiency_with_original_liability_and_no_second_wallet_io() {
    for method in ["GET", "POST"] {
        for wire in [Wire::Verification, Wire::Settlement] {
            let (success, signed, requests, error) = run_private(method, wire, 1, true).await;
            assert!(!success);
            assert_eq!(signed.len(), 1);
            assert_eq!(requests.len(), 2);
            assert!(error.contains("private headers cannot follow another wallet"));
            assert!(error.contains("may still charge"));
        }
    }
}
