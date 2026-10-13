//! Connection establishment and each response own independent progress clocks.
use super::*;
use crate::test_socks::{Fault, Socks};
use std::collections::BTreeMap;

struct LoopbackDns(SocketAddr, Duration);
impl reqwest::dns::Resolve for LoopbackDns {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let (address, delay) = (self.0, self.1);
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            Ok(Box::new([address].into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[tokio::test]
async fn dns_socks_and_tls_do_not_consume_header_inactivity() {
    let read = Duration::from_millis(300);
    let delay = Duration::from_millis(600);
    for mode in [Mode::Direct, Mode::Tor] {
        for tls in [false, true] {
            let app = axum::Router::new().fallback(|| async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                "ok"
            });
            let (address, server) = crate::test_tls::serve_delayed(
                app,
                &[&rustls::version::TLS13],
                &[b"h2"],
                if tls { delay } else { Duration::ZERO },
            )
            .await;
            let socks = Socks::start(
                BTreeMap::from([("api.example.com".into(), address)]),
                if tls {
                    Fault::None
                } else {
                    Fault::Delay(delay)
                },
            )
            .await;
            let mut context = NetworkContext::new(NetworkPolicy {
                mode: mode.clone(),
                socks_endpoint: (mode == Mode::Tor).then_some(socks.address),
                connect_timeout_seconds: Some(2),
                cover_traffic_enabled: Some(false),
                ..Default::default()
            })
            .unwrap()
            .with_test_root(crate::test_tls::CA);
            context.test_dns = Some(Arc::new(LoopbackDns(
                address,
                if tls { Duration::ZERO } else { delay },
            )));
            let url = format!("https://api.example.com:{}", address.port());
            let client = context
                .http_policy(
                    &IsolationId::bootstrap(),
                    &url,
                    read,
                    false,
                    HttpPolicy::default(),
                )
                .unwrap();
            let started = tokio::time::Instant::now();
            assert_eq!(
                client.get(&url).send().await.unwrap().text().await.unwrap(),
                "ok"
            );
            assert!(started.elapsed() >= delay);
            // The same pooled connection receives a fresh response allowance.
            assert_eq!(
                client.get(&url).send().await.unwrap().text().await.unwrap(),
                "ok"
            );
            if mode == Mode::Tor {
                assert_eq!(socks.records.lock().unwrap().len(), 1);
            }
            server.abort();
            let _ = server.await;
        }
    }
}

#[tokio::test]
async fn stalled_socks_uses_connect_limit_not_read_limit() {
    let socks = Socks::start(BTreeMap::new(), Fault::Stall).await;
    let context = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address),
        connect_timeout_seconds: Some(1),
        cover_traffic_enabled: Some(false),
        ..Default::default()
    })
    .unwrap();
    let client = context
        .http(
            &IsolationId::bootstrap(),
            "https://api.example.com",
            Duration::from_millis(100),
        )
        .unwrap();
    let started = tokio::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        client.get("https://api.example.com").send(),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.is_timeout());
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert!(socks.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn multiplexed_response_progress_cannot_renew_sibling_headers_or_body() {
    let app = axum::Router::new()
        .route(
            "/headers",
            axum::routing::get(|| async { std::future::pending::<String>().await }),
        )
        .route(
            "/body",
            axum::routing::get(|| async {
                axum::body::Body::from_stream(futures_util::stream::pending::<
                    Result<Bytes, std::io::Error>,
                >())
            }),
        )
        .route(
            "/progress",
            axum::routing::get(|| async {
                axum::body::Body::from_stream(futures_util::stream::unfold(0, |n| async move {
                    if n == 12 {
                        return None;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Some((Ok::<_, std::io::Error>(Bytes::from_static(b"x")), n + 1))
                }))
            }),
        );
    use axum::body::Bytes;
    let (address, server) = crate::test_tls::serve(app).await;
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
    let progress = client
        .get("https://api.example.com/progress")
        .send()
        .await
        .unwrap();
    let reading = progress.bytes();
    let headers = client.get("https://api.example.com/headers").send();
    let body = async {
        client
            .get("https://api.example.com/body")
            .send()
            .await?
            .bytes()
            .await
    };
    let (progress, headers, body) = tokio::join!(reading, headers, body);
    assert_eq!(progress.unwrap().len(), 12);
    assert!(headers.unwrap_err().is_timeout());
    assert!(body.unwrap_err().is_timeout());
    assert_eq!(socks.records.lock().unwrap().len(), 1);
    server.abort();
}

#[tokio::test]
async fn consumer_backpressure_does_not_reject_already_available_body_data() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\na")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        socket.write_all(b"b").await.unwrap();
    });
    let context = NetworkContext::new(NetworkPolicy::default()).unwrap();
    let client = context.discovery(&url, Duration::from_millis(300)).unwrap();
    let mut response = client.get(&url).send().await.unwrap();
    assert_eq!(response.chunk().await.unwrap().unwrap().as_ref(), b"a");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"b");
    server.await.unwrap();
}
