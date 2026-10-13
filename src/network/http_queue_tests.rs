//! Real TLS/SOCKS and HTTP/2 capacity; only public fixture headers and loopback I/O.
use super::*;
use crate::test_socks::{Fault, Socks};
use axum::{body::Bytes, http::Response};
use std::collections::BTreeMap;
use tokio::sync::{Notify, mpsc};

use crate::test_h2_capture as wire;

#[tokio::test]
async fn queued_timeout_and_cancellation_never_transmit_headers_or_body() {
    for method in [reqwest::Method::GET, reqwest::Method::POST] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let release = Arc::new(Notify::new());
        let done = release.clone();
        let (seen, mut received) = mpsc::unbounded_channel();
        let capture: wire::Capture = Arc::default();
        capture.lock().unwrap().push(vec![]);
        let captured = capture.clone();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let socket = wire::tls().accept(socket).await.unwrap();
            let socket = wire::Tap {
                io: socket,
                capture: captured,
                index: 0,
            };
            let mut conn = h2::server::Builder::new()
                .max_concurrent_streams(1)
                .handshake::<_, Bytes>(socket)
                .await
                .unwrap();
            let mut replies = tokio::task::JoinSet::new();
            while let Some(Ok((request, mut respond))) = conn.accept().await {
                seen.send(request.uri().path().to_owned()).unwrap();
                let done = done.clone();
                replies.spawn(async move {
                    assert!(!request.headers().contains_key("payment-signature"));
                    let mut body = respond.send_response(Response::new(()), false).unwrap();
                    if request.uri().path() == "/hold" {
                        loop {
                            body.send_data(Bytes::from_static(b"x"), false).unwrap();
                            tokio::select! {
                                _ = done.notified() => break,
                                _ = tokio::time::sleep(Duration::from_millis(50)) => {},
                            }
                        }
                    }
                    body.send_data(Bytes::from_static(b"done"), true).unwrap();
                });
            }
        });
        let socks = Socks::start(
            BTreeMap::from([("api.example.com".into(), address)]),
            Fault::None,
        )
        .await;
        let context = NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA);
        let client = context
            .http_policy(
                &IsolationId::bootstrap(),
                "https://api.example.com",
                Duration::from_millis(300),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        let response = client
            .get("https://api.example.com/hold")
            .send()
            .await
            .unwrap();
        assert_eq!(received.recv().await.as_deref(), Some("/hold"));
        let reading = tokio::spawn(async move { response.bytes().await.unwrap() });
        let pending = |path: &str| {
            let request = client
                .request(method.clone(), format!("https://api.example.com/{path}"))
                .header("payment-signature", "public-fixture-marker");
            if method == reqwest::Method::POST {
                request.body("public request body")
            } else {
                request
            }
            .send()
        };
        let cancelled = tokio::spawn(pending("cancelled"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        assert!(pending("timeout").await.unwrap_err().is_timeout());
        assert!(received.try_recv().is_err());
        release.notify_one();
        reading.await.unwrap();
        client
            .get("https://api.example.com/after")
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(received.recv().await.as_deref(), Some("/after"));
        assert!(received.try_recv().is_err());
        // Count wire HEADERS, not merely requests accepted before a late RST.
        assert_eq!(wire::blocks(&capture.lock().unwrap()[0]).len(), 2);
        assert_eq!(
            socks.records.lock().unwrap().len(),
            1,
            "healthy pooled connection retained"
        );
        server.abort();
        let _ = server.await;
    }
}
