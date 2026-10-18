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
#[derive(Clone)]
struct Fixture {
    signed: Arc<Mutex<Vec<(String, String)>>>,
    requests: Arc<Mutex<Vec<(SocketAddr, Method, String, Vec<u8>)>>>,
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
    f.requests
        .lock()
        .unwrap()
        .push((peer, method, uri.to_string(), body.to_vec()));
    let Some(encoded) = headers.get("payment-signature") else {
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
) -> (
    bool,
    Vec<(String, String)>,
    Vec<(SocketAddr, Method, String, Vec<u8>)>,
    String,
) {
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
    let route = RoutedRequest {
        method: method.into(),
        url: format!("{origin}/pay"),
        query: BTreeMap::from([("original".into(), json!("same & bytes"))]),
        body: (method == "POST").then(|| json!({"original":[1,true,"same"]})),
    };
    let result = if matches!(wire, Wire::HeldSuccess) {
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
