//! Direct library calls retain dispatch and body ownership after waiter cancellation.
use super::*;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

#[derive(Clone, Default)]
struct Seller {
    challenged: Arc<Notify>,
    release_challenge: Arc<Notify>,
    submitted: Arc<Notify>,
    release_body: Arc<Notify>,
    unsigned: Arc<AtomicUsize>,
    signed: Arc<AtomicUsize>,
}
async fn serve(State(s): State<Seller>, headers: HeaderMap) -> axum::response::Response {
    if headers.contains_key("payment-signature") {
        s.signed.fetch_add(1, Ordering::SeqCst);
        s.submitted.notify_one();
        let body = futures_util::stream::once(async move {
            s.release_body.notified().await;
            Ok::<_, std::io::Error>("paid body")
        });
        return axum::response::Response::new(axum::body::Body::from_stream(body));
    }
    s.unsigned.fetch_add(1, Ordering::SeqCst);
    s.challenged.notify_one();
    s.release_challenge.notified().await;
    let challenge = serde_json::json!({"x402Version":2,"resource":{"url":"http://localhost/pay"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
    )
        .into_response()
}
#[tokio::test]
async fn canceled_direct_waiter_retains_exact_signed_dispatch_and_body_drain() {
    let seller = Seller::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/pay", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/pay", get(serve))
        .with_state(seller.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // Public deterministic, unfunded fixture signer.
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    );
    let progress = SubmissionProgress::default();
    let waiter = tokio::spawn({
        let client = client.clone();
        let progress = progress.clone();
        async move {
            client
                .execute_response_observed(
                    RoutedRequest {
                        method: "GET".into(),
                        url,
                        query: Default::default(),
                        body: None,
                    },
                    &progress,
                )
                .await
        }
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        seller.challenged.notified(),
    )
    .await
    .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    seller.release_challenge.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        seller.submitted.notified(),
    )
    .await
    .unwrap();
    assert!(progress.possible());
    let drain = tokio::spawn({
        let client = client.clone();
        async move { client.drain().await }
    });
    tokio::task::yield_now().await;
    assert!(
        !drain.is_finished(),
        "retained owner includes response body conversion"
    );
    seller.release_body.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(10), drain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(seller.unsigned.load(Ordering::SeqCst), 1);
    assert_eq!(seller.signed.load(Ordering::SeqCst), 1);
    assert!(
        client
            .execute_response(RoutedRequest {
                method: "GET".into(),
                url: "http://127.0.0.1:1".into(),
                query: Default::default(),
                body: None
            })
            .await
            .unwrap_err()
            .to_string()
            .contains("draining")
    );
    server.abort();
}

#[tokio::test]
async fn retained_outer_owner_reuses_task_and_inner_owner_tracks_drain() {
    let outer = crate::server::work::Work::default();
    let inner = crate::server::work::Work::default();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let waiter = tokio::spawn({
        let inner = inner.clone();
        let entered = entered.clone();
        let release = release.clone();
        async move {
            outer
                .run(async move {
                    let owner_task = tokio::task::id();
                    ownership::run(&inner, async move {
                        assert_eq!(
                            tokio::task::id(),
                            owner_task,
                            "MCP owns the accepted task already"
                        );
                        entered.notify_one();
                        release.notified().await;
                    })
                    .await
                    .unwrap();
                })
                .await
                .unwrap();
        }
    });
    entered.notified().await;
    waiter.abort();
    let drain = tokio::spawn(async move { inner.drain().await });
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(10), drain)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn direct_owner_reuses_existing_foreground_permit_for_managed_signing() {
    let owner = crate::server::work::Work::default();
    ownership::run(&owner, async {
        assert!(ownership::foreground().unwrap().is_none());
        let inner = crate::server::work::Work::default();
        ownership::run(&inner, async {
            assert!(ownership::foreground().unwrap().is_none());
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();
    owner.drain().await;
    assert!(ownership::foreground().unwrap().is_some());
}

#[tokio::test]
async fn accepted_work_reenters_same_owner_during_drain_but_not_a_closed_other_owner() {
    let owner = crate::server::work::Work::default();
    let other = crate::server::work::Work::default();
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let nested_entered = Arc::new(Notify::new());
    let finish = Arc::new(Notify::new());
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn({
        let (owner, other) = (owner.clone(), other.clone());
        let (entered, resume, nested_entered, finish) = (
            entered.clone(),
            resume.clone(),
            nested_entered.clone(),
            finish.clone(),
        );
        async move {
            let nested = owner.clone();
            ownership::run(&owner, async move {
                entered.notify_one();
                resume.notified().await;
                assert!(
                    ownership::run(&other, async { panic!("closed other owner ran") })
                        .await
                        .is_err()
                );
                let result = ownership::run(&nested, async move {
                    nested_entered.notify_one();
                    finish.notified().await;
                    7
                })
                .await;
                result_tx.send(result).unwrap();
            })
            .await
            .unwrap();
        }
    });
    entered.notified().await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    owner.close();
    other.close();
    let drain = tokio::spawn({
        let owner = owner.clone();
        async move { owner.drain().await }
    });
    resume.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), nested_entered.notified())
        .await
        .unwrap();
    assert!(!drain.is_finished());
    finish.notify_one();
    assert_eq!(result_rx.await.unwrap().unwrap(), 7);
    tokio::time::timeout(std::time::Duration::from_secs(5), drain)
        .await
        .unwrap()
        .unwrap();
    assert!(
        ownership::run(&owner, async { panic!("new work ran") })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn retained_owner_without_a_permit_acquires_one_and_nested_work_reuses_it() {
    let listener = crate::server::work::Work::default();
    let payment = crate::server::work::Work::default();
    listener
        .run(async move {
            // A Work token is a lifetime guarantee, never a capacity grant.
            assert!(ownership::foreground().unwrap().is_some());
            ownership::run(&payment, async {
                assert!(ownership::foreground().unwrap().is_none());
                let nested = crate::server::work::Work::default();
                ownership::run(&nested, async {
                    assert!(ownership::foreground().unwrap().is_none());
                })
                .await
                .unwrap();
            })
            .await
            .unwrap();
        })
        .await
        .unwrap();
    listener.drain().await;
    assert!(ownership::foreground().unwrap().is_some());
}
