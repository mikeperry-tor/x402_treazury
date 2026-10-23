//! Resume cancelled relay waiters' accepted work through its terminal outcome.
use super::*;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug)]
enum Outcome {
    Success,
    TargetFailure,
    RelayFailure,
    UnsignedFailure,
}

#[tokio::test]
async fn cancelled_waiters_keep_serialization_and_publish_terminal_outcomes() {
    for outcome in [
        Outcome::Success,
        Outcome::TargetFailure,
        Outcome::RelayFailure,
        Outcome::UnsignedFailure,
    ] {
        for cancel_during_body in [false, true] {
            if cancel_during_body && matches!(outcome, Outcome::UnsignedFailure) {
                continue;
            }
            let mut body = envelope(TARGET);
            if matches!(outcome, Outcome::TargetFailure) {
                body["statusCode"] = json!(503);
            }
            let (mut relay, f, unused) =
                fixture(body, matches!(outcome, Outcome::RelayFailure), false).await;
            unused.abort();
            let unsigned_arrived = CancellationToken::new();
            let unsigned_release = CancellationToken::new();
            let body_release = CancellationToken::new();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            set_endpoint(
                &mut relay,
                format!("http://{}/curl", listener.local_addr().unwrap()),
            );
            let app = Router::new().route(
                "/curl",
                post({
                    let (arrived, release, body_release, f) = (
                        unsigned_arrived.clone(),
                        unsigned_release.clone(),
                        body_release.clone(),
                        f.clone(),
                    );
                    move |headers: HeaderMap| {
                        let (arrived, release, body_release, f) = (
                            arrived.clone(),
                            release.clone(),
                            body_release.clone(),
                            f.clone(),
                        );
                        async move {
                            let signed = headers.contains_key("payment-signature");
                            if !signed {
                                arrived.cancel();
                                release.cancelled().await;
                                if matches!(outcome, Outcome::UnsignedFailure) {
                                    f.calls.fetch_add(1, Ordering::SeqCst);
                                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                                }
                            }
                            let response = handler(AxumState(f), headers).await;
                            if !signed {
                                return response;
                            }
                            let (parts, body) = response.into_parts();
                            let bytes = axum::body::to_bytes(body, 4096).await.unwrap();
                            let body = futures_util::stream::once(async move {
                                body_release.cancelled().await;
                                Ok::<_, std::io::Error>(bytes)
                            });
                            axum::response::Response::from_parts(
                                parts,
                                axum::body::Body::from_stream(body),
                            )
                        }
                    }
                }),
            );
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let capacity = vec![
                Arc::new(tokio::sync::Semaphore::new(1)),
                Arc::new(tokio::sync::Semaphore::new(1)),
            ];
            let waiter = tokio::spawn({
                let relay = relay.clone();
                let capacity = capacity.clone();
                async move {
                    relay
                        .fetch_with_capacity(TARGET, 200, 4096, "max_spec_bytes", capacity, None)
                        .await
                }
            });
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                unsigned_arrived.cancelled(),
            )
            .await
            .unwrap();
            if cancel_during_body {
                unsigned_release.cancel();
                tokio::time::timeout(std::time::Duration::from_secs(5), f.arrived.notified())
                    .await
                    .unwrap();
            }
            waiter.abort();
            assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
            assert!(
                !is_idle(&relay),
                "accepted owner lost provider serialization: {outcome:?}"
            );
            for gate in &capacity {
                assert_eq!(
                    gate.available_permits(),
                    0,
                    "cancelled waiter released accepted relay capacity: {outcome:?}"
                );
            }

            // A second wallet shares the breaker, but cannot borrow the first
            // wallet's paid response. No same-provider I/O can start before publication.
            let other_url = "https://other.example.com/spec";
            let (mut other, other_fixture, other_server) =
                fixture(envelope(other_url), false, false).await;
            let second = Arc::get_mut(&mut other).unwrap();
            second.state = relay.state.clone();
            second.key = "second-wallet".into();
            second.client = second_wallet(&relay).client;
            let mut queued = Box::pin(other.fetch(other_url, 200, 4096, "max_spec_bytes"));
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(50), queued.as_mut())
                    .await
                    .is_err(),
                "queued fetch bypassed the active owner: {outcome:?}"
            );
            assert_eq!(other_fixture.calls.load(Ordering::SeqCst), 0);
            unsigned_release.cancel();
            body_release.cancel();
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), queued)
                .await
                .unwrap();
            assert_eq!(result.is_err(), matches!(outcome, Outcome::RelayFailure));
            assert_eq!(
                other_fixture.signed.load(Ordering::SeqCst),
                usize::from(!matches!(outcome, Outcome::RelayFailure))
            );
            assert_eq!(
                f.signed.load(Ordering::SeqCst),
                usize::from(!matches!(outcome, Outcome::UnsignedFailure))
            );

            // Lost delivery must publish success, a definitive provider/target
            // failure, or the provider's uncertainty breaker before repetition.
            let calls = f.calls.load(Ordering::SeqCst);
            let repeated = relay.fetch(TARGET, 200, 4096, "max_spec_bytes").await;
            assert_eq!(repeated.is_ok(), matches!(outcome, Outcome::Success));
            if matches!(outcome, Outcome::TargetFailure) {
                assert!(repeated.err().unwrap().is::<TargetFailure>());
            }
            assert_eq!(
                f.calls.load(Ordering::SeqCst),
                calls,
                "replayed cancelled work"
            );
            relay.client.drain().await;
            other.client.drain().await;
            for gate in &capacity {
                assert_eq!(
                    gate.available_permits(),
                    1,
                    "relay capacity leaked: {outcome:?}"
                );
            }
            server.abort();
            other_server.abort();
        }
    }
}

#[tokio::test]
async fn queued_and_cached_relay_calls_do_not_reserve_request_capacity() {
    let (mut relay, fixture, unused) = fixture(envelope(TARGET), false, false).await;
    unused.abort();
    let arrived = CancellationToken::new();
    let release = CancellationToken::new();
    let healthy_calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    set_endpoint(&mut relay, format!("http://{address}/curl"));
    let app = Router::new()
        .route(
            "/curl",
            post({
                let fixture = fixture.clone();
                let arrived = arrived.clone();
                let release = release.clone();
                move |headers: HeaderMap| {
                    let fixture = fixture.clone();
                    let arrived = arrived.clone();
                    let release = release.clone();
                    async move {
                        if headers.contains_key("payment-signature") {
                            arrived.cancel();
                            release.cancelled().await;
                        }
                        handler(AxumState(fixture), headers).await
                    }
                }
            }),
        )
        .route(
            "/healthy",
            axum::routing::get({
                let calls = healthy_calls.clone();
                move || {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        StatusCode::OK
                    }
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let capacity = Arc::new(tokio::sync::Semaphore::new(16));
    let active = tokio::spawn({
        let relay = relay.clone();
        let capacity = capacity.clone();
        async move {
            relay
                .fetch_with_capacity(TARGET, 200, 4096, "max_spec_bytes", vec![capacity], None)
                .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), arrived.cancelled())
        .await
        .unwrap();
    assert_eq!(capacity.available_permits(), 15);
    let mut queued = Vec::new();
    for _ in 0..16 {
        queued.push(tokio::spawn({
            let relay = relay.clone();
            let capacity = capacity.clone();
            async move {
                relay
                    .fetch_with_capacity(TARGET, 200, 4096, "max_spec_bytes", vec![capacity], None)
                    .await
            }
        }));
    }
    // Give every queued owner a chance to reach serialization. The active owner
    // cannot finish until explicitly released, regardless of this fixture wait.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(queued.iter().all(|task| !task.is_finished()));
    assert_eq!(
        capacity.available_permits(),
        15,
        "relay queue reserved idle capacity"
    );
    let healthy_url = format!("http://{address}/healthy");
    let healthy = crate::network::global()
        .discovery(&healthy_url, None)
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_millis(500), async {
        let _permit = capacity.acquire().await.unwrap();
        assert_eq!(
            healthy.get(&healthy_url).send().await.unwrap().status(),
            200
        );
    })
    .await
    .expect("queued relay calls blocked unrelated request capacity");
    assert_eq!(healthy_calls.load(Ordering::SeqCst), 1);
    release.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        active.await.unwrap().unwrap();
        for task in queued {
            task.await.unwrap().unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.signed.load(Ordering::SeqCst), 1);
    assert_eq!(capacity.available_permits(), 16);

    // A cached result must remain available even when no request slot exists.
    let unavailable = Arc::new(tokio::sync::Semaphore::new(0));
    tokio::time::timeout(
        std::time::Duration::from_millis(500),
        relay.fetch_with_capacity(TARGET, 200, 4096, "max_spec_bytes", vec![unavailable], None),
    )
    .await
    .expect("cached relay result waited for request capacity")
    .unwrap();
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 2);
    relay.client.drain().await;
    server.abort();
}
