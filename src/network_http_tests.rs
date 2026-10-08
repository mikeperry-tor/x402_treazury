//! Real TLS through authenticated SOCKS; no external destinations or funded keys.
use super::*;
use crate::test_socks::{Fault, Socks};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Semaphore, mpsc};

fn context(socks: &Socks) -> NetworkContext {
    NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address),
        connect_timeout_seconds: Some(2),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA)
}
fn wallet(n: u8) -> IsolationId {
    IsolationId::evm(&format!("0x{n:040x}")).unwrap()
}
fn client(ctx: &NetworkContext, id: &IsolationId, policy: HttpPolicy) -> reqwest::Client {
    ctx.http_policy(
        id,
        "https://api.example.com",
        Duration::from_secs(3),
        false,
        policy,
    )
    .unwrap()
}

#[tokio::test]
async fn http2_multiplexes_in_one_identity_and_separates_wallets_and_discovery() {
    let (entered, mut arrivals) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let gate = release.clone();
    let app = axum::Router::new()
        .route("/warm", axum::routing::get(|| async { "warm" }))
        .route(
            "/call",
            axum::routing::get(move || {
                let (entered, gate) = (entered.clone(), gate.clone());
                async move {
                    entered.send(()).unwrap();
                    gate.acquire().await.unwrap().forget();
                    "ok"
                }
            }),
        );
    let (address, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    let ctx = context(&socks);
    let a = client(&ctx, &wallet(1), HttpPolicy::default());
    let response = a.get("https://api.example.com/warm").send().await.unwrap();
    assert_eq!(response.version(), reqwest::Version::HTTP_2);
    response.bytes().await.unwrap();
    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..4 {
        // Independently requesting a client models callers across listeners.
        let http = client(&ctx, &wallet(1), HttpPolicy::default());
        calls.spawn(async move {
            let response = http
                .get("https://api.example.com/call")
                .send()
                .await
                .unwrap();
            assert_eq!(response.version(), reqwest::Version::HTTP_2);
            assert_eq!(response.text().await.unwrap(), "ok");
        });
    }
    for _ in 0..4 {
        tokio::time::timeout(Duration::from_secs(2), arrivals.recv())
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(
        socks.records.lock().unwrap().len(),
        1,
        "all four live streams share TLS/SOCKS connection"
    );
    // A new wallet and discovery must not borrow the active wallet connection.
    for id in [
        wallet(2),
        IsolationId::discovery("https://api.example.com").unwrap(),
    ] {
        let response = client(&ctx, &id, HttpPolicy::default())
            .get("https://api.example.com/warm")
            .send()
            .await
            .unwrap();
        assert_eq!(response.version(), reqwest::Version::HTTP_2);
        response.bytes().await.unwrap();
    }
    let records = socks.records.lock().unwrap().clone();
    assert_eq!(records.len(), 3);
    for i in 0..3 {
        assert_eq!(records[i].address_type, 3);
        for j in i + 1..3 {
            assert_ne!(records[i].password, records[j].password);
        }
    }
    release.add_permits(4);
    while let Some(result) = calls.join_next().await {
        result.unwrap();
    }
    server.abort();
}

#[tokio::test]
async fn compatibility_flags_are_independent_and_cannot_reuse_permissive_connections() {
    for (versions, alpn, permissive, expected) in [
        (
            vec![&rustls::version::TLS13],
            vec![b"http/1.1".as_slice()],
            HttpPolicy {
                allow_http1: true,
                allow_tls12: false,
            },
            reqwest::Version::HTTP_11,
        ),
        (
            vec![&rustls::version::TLS12],
            vec![b"h2".as_slice()],
            HttpPolicy {
                allow_http1: false,
                allow_tls12: true,
            },
            reqwest::Version::HTTP_2,
        ),
    ] {
        let (address, server) = crate::test_tls::serve_with(
            axum::Router::new().fallback(|| async { "ok" }),
            &versions,
            &alpn,
        )
        .await;
        let socks = Socks::start(
            BTreeMap::from([("api.example.com".into(), address)]),
            Fault::None,
        )
        .await;
        let ctx = context(&socks);
        let legacy = client(&ctx, &wallet(1), permissive);
        let response = legacy.get("https://api.example.com").send().await.unwrap();
        assert_eq!(response.version(), expected);
        response.bytes().await.unwrap();
        // Same identity/origin/timeout, differing policy: do not reuse the successful connection.
        assert!(
            client(&ctx, &wallet(1), HttpPolicy::default())
                .get("https://api.example.com")
                .send()
                .await
                .is_err()
        );
        assert_eq!(
            socks.records.lock().unwrap().len(),
            2,
            "one strict attempt; no protocol downgrade retry"
        );
        // The compatible connection is still usable.
        legacy
            .get("https://api.example.com")
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(socks.records.lock().unwrap().len(), 2);
        server.abort();
    }
}

#[tokio::test]
async fn http_responses_renew_idle_budget_and_share_one_timeout_policy() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                    assert!(request.len() <= 8192);
                }
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                for _ in 0..8 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if stream.write_all(b"x").await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let ctx = NetworkContext::new(NetworkPolicy::default()).unwrap();
    let id = IsolationId::discovery(&url).unwrap();
    let budget = Duration::from_millis(500);
    // Scale the idle interval down for this real-socket regression test.
    let download = ctx
        .http_policy(&id, &url, budget, false, HttpPolicy::COMPATIBLE)
        .unwrap();
    let api = ctx
        .http_policy(&id, &url, budget, false, HttpPolicy::COMPATIBLE)
        .unwrap();
    let (download, api) = tokio::join!(
        async { download.get(&url).send().await.unwrap().bytes().await },
        async { api.get(&url).send().await.unwrap().bytes().await },
    );
    assert_eq!(download.unwrap().as_ref(), b"xxxxxxxx");
    assert_eq!(api.unwrap().as_ref(), b"xxxxxxxx");
    assert_eq!(ctx.http.lock().unwrap().len(), 1);
    server.await.unwrap();
}

#[tokio::test]
async fn http_responses_time_out_on_stalled_headers_or_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for headers in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
                assert!(request.len() <= 8192);
            }
            if headers {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nx")
                    .await
                    .unwrap();
            }
            std::future::pending::<()>().await;
        });
        let ctx = NetworkContext::new(NetworkPolicy::default()).unwrap();
        let id = IsolationId::discovery(&url).unwrap();
        let http = ctx
            .http_policy(
                &id,
                &url,
                Duration::from_millis(200),
                false,
                HttpPolicy::COMPATIBLE,
            )
            .unwrap();
        let result = async { http.get(&url).send().await?.bytes().await }.await;
        assert!(result.unwrap_err().is_timeout());
        server.abort();
        let _ = server.await;
    }
}
