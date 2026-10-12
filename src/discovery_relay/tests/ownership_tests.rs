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
            Arc::get_mut(&mut relay).unwrap().endpoint =
                format!("http://{}/curl", listener.local_addr().unwrap());
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
            let waiter = tokio::spawn({
                let relay = relay.clone();
                async move { relay.fetch(TARGET, 200, 4096, "max_spec_bytes").await }
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
                relay.state.try_lock().is_err(),
                "accepted owner lost its lock: {outcome:?}"
            );

            // A second wallet shares the breaker, but cannot borrow the first
            // wallet's paid response. No provider I/O can start before publication.
            let other_url = "https://other.example/spec";
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

            // Lost delivery must still publish the response, target-local failure,
            // or global breaker before a repeated caller inspects it.
            if !matches!(outcome, Outcome::UnsignedFailure) {
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
            }
            relay.client.drain().await;
            other.client.drain().await;
            server.abort();
            other_server.abort();
        }
    }
}
