use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Requests = Arc<Mutex<Vec<Value>>>;
async fn fixture(
    responses: Vec<(Duration, String)>,
) -> (reqwest::Url, Requests, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/secret-url?key=secret-key",
        listener.local_addr().unwrap()
    )
    .parse()
    .unwrap();
    let requests: Requests = Arc::new(Mutex::new(vec![]));
    let captured = requests.clone();
    let task = tokio::spawn(async move {
        for (delay, response) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(socket.read_u8().await.unwrap());
                assert!(headers.len() < 8192, "fixture header limit exceeded");
            }
            let headers = String::from_utf8(headers).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap();
            assert!(length < 8192, "fixture request limit exceeded");
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            captured
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&body).unwrap());
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    (url, requests, task)
}
fn response(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn client() -> reqwest::Client {
    crate::network::discovery("https://rpc-test.invalid", Duration::from_secs(2)).unwrap()
}
#[tokio::test]
async fn interrupted_reads_retry_identical_canonical_payload_once() {
    for broken in [
        "",
        "HTTP/1.1 200 OK\r\nContent-Length: 999\r\nConnection: close\r\n\r\n{",
    ] {
        let (url, captured, task) = fixture(vec![
            (Duration::ZERO, broken.into()),
            (
                Duration::ZERO,
                response(200, r#"{"jsonrpc":"2.0","id":1,"result":"0x01"}"#),
            ),
        ])
        .await;
        let params = json!([{"data":"secret-wallet-data"}, {"blockHash":"pinned-hash","requireCanonical":true}]);
        assert_eq!(
            rpc(&client(), &url, "eth_call", params.clone())
                .await
                .unwrap(),
            "0x01"
        );
        task.await.unwrap();
        let requests = captured.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        assert_eq!(requests[0]["params"], params);
    }
}
#[tokio::test]
async fn persistent_transport_failure_is_bounded_and_redacted() {
    use tracing::instrument::WithSubscriber;
    let logs = super::super::tests::Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();

    let (url, captured, task) = fixture(vec![(Duration::ZERO, String::new()); 2]).await;
    let error = rpc(&client(), &url, "eth_chainId", json!(["secret-params"]))
        .with_subscriber(subscriber)
        .await
        .unwrap_err();
    task.await.unwrap();
    assert_eq!(captured.lock().unwrap().len(), 2);
    let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(
        logged.contains("retry=true") && logged.contains("retry=false"),
        "{logged}"
    );
    let text = format!(
        "{} {logged}",
        crate::rotation::base::safe_diagnostic(&error)
    );
    assert!(text.contains("phase send"), "{text}");
    for secret in ["secret-url", "secret-key", "secret-params", "127.0.0.1"] {
        assert!(!text.contains(secret), "{text}");
    }
}
#[tokio::test]
async fn rejected_invalid_and_write_requests_never_retry() {
    for raw in [
        response(429, "secret rate limit body"),
        response(503, "secret server body"),
        response(200, "secret malformed JSON"),
        response(
            200,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"secret RPC body"}}"#,
        ),
        response(200, r#"{"jsonrpc":"2.0","id":2,"result":"0x01"}"#),
        response(200, r#"{"jsonrpc":"2.0","id":1,"result":null}"#),
    ] {
        let (url, captured, task) = fixture(vec![(Duration::ZERO, raw)]).await;
        let error = rpc(&client(), &url, "eth_chainId", json!([]))
            .await
            .unwrap_err();
        task.await.unwrap();
        assert_eq!(captured.lock().unwrap().len(), 1);
        assert!(!error.to_string().contains("secret"));
    }
    let (url, captured, task) = fixture(vec![(Duration::ZERO, String::new())]).await;
    assert!(
        rpc(&client(), &url, "eth_sendRawTransaction", json!(["secret"]))
            .await
            .is_err()
    );
    assert!(captured.lock().unwrap().is_empty());
    task.abort();
}
#[tokio::test]
async fn cancellation_does_not_retry() {
    let (url, captured, task) = fixture(vec![(Duration::from_secs(5), String::new())]).await;
    let call = tokio::spawn(async move { rpc(&client(), &url, "eth_chainId", json!([])).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while captured.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    tokio::time::sleep(BACKOFF * 2).await;
    assert_eq!(captured.lock().unwrap().len(), 1);
    task.abort();
}

#[tokio::test]
async fn untrusted_tls_is_reported_without_retry_or_verification_bypass() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/tls/server.der"
            ))
            .to_vec(),
        )],
        PrivatePkcs8KeyDer::from(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/tls/server-key.der"
            ))
            .to_vec(),
        )
        .into(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("https://{}/secret-url", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    let task = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            assert!(acceptor.accept(socket).await.is_err());
        }
    });
    let error = rpc(&client(), &url, "eth_chainId", json!([]))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("TLS"), "{error}");
    assert!(!error.to_string().contains("secret"));
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn stalled_body_inactivity_reports_body_not_headers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let task = tokio::spawn(async move {
        let mut held = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(socket.read_u8().await.unwrap());
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
                .await
                .unwrap();
            held.push(socket);
        }
        std::future::pending::<()>().await;
        drop(held);
    });
    let error = rpc(
        &crate::network::discovery("https://rpc-test.invalid", Duration::from_millis(100)).unwrap(),
        &url,
        "eth_chainId",
        json!([]),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("timeout"));
    assert!(error.to_string().contains("phase body"));
    assert!(!error.to_string().contains("http://"));
    task.abort();
}

#[tokio::test]
async fn progressing_rpc_body_survives_former_total_deadline() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(socket.read_u8().await.unwrap());
        }
        let body = br#"{"jsonrpc":"2.0","id":1,"result":"0x2105"}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        for chunk in body.chunks(body.len().div_ceil(4)) {
            tokio::time::sleep(Duration::from_secs(4)).await;
            socket.write_all(chunk).await.unwrap();
        }
    });
    let http =
        crate::network::discovery("https://rpc-test.invalid", Duration::from_secs(6)).unwrap();
    let started = Instant::now();
    let value = tokio::time::timeout(
        Duration::from_secs(30),
        rpc(&http, &url, "eth_chainId", json!([])),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(value, "0x2105");
    assert!(started.elapsed() > Duration::from_secs(15));
    task.await.unwrap();
}

#[test]
fn acquisition_only_failures_allow_configured_fallback_but_not_local_errors() {
    for category in ["invalid JSON body", "invalid envelope", "missing result"] {
        let failure = RpcFailure::new("eth_chainId", category, "decode", Some(200));
        assert!(failure.can_failover());
    }
    let mut tls = RpcFailure::new("eth_chainId", "connect", "send", None);
    tls.tls = true;
    assert!(tls.can_failover());
    let local = RpcFailure::new("eth_chainId", "request construction", "send", None);
    assert!(!local.can_failover());
}

#[tokio::test]
async fn oversized_responses_reject_before_decoding_without_retry_and_allow_fallback() {
    use tracing::instrument::WithSubscriber;
    let limit = crate::limits::RESPONSE_BYTES;
    let too_large = "s".repeat(limit + 1);
    // Known length rejects without waiting for any body; the other encodings
    // exercise cumulative streaming bounds without a Content-Length shortcut.
    for raw in [
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            limit + 1
        ),
        format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n1\r\ns\r\n0\r\n\r\n",
            limit,
            &too_large[..limit]
        ),
        format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{too_large}"),
    ] {
        let logs = super::super::tests::Logs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let (url, captured, server) = fixture(vec![(Duration::ZERO, raw)]).await;
        let error = rpc(&client(), &url, "eth_call", json!(["secret-params"]))
            .with_subscriber(subscriber)
            .await
            .unwrap_err();
        server.await.unwrap();
        let failure = error.downcast_ref::<RpcFailure>().unwrap();
        assert_eq!(failure.category, "response too large");
        assert_eq!(failure.phase, "body");
        assert_eq!(failure.http, Some(200));
        assert!(failure.can_failover());
        assert_eq!(captured.lock().unwrap().len(), 1);
        let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert!(logged.contains("retry=false"), "{logged}");
        assert!(!logged.contains("retry=true"), "{logged}");
        let diagnostic = format!("{error} {logged}");
        for secret in ["secret-url", "secret-key", "secret-params", "127.0.0.1"] {
            assert!(!diagnostic.contains(secret), "{diagnostic}");
        }
    }
}

#[tokio::test]
async fn exact_byte_boundary_and_concurrent_healthy_rpc_remain_usable() {
    let limit = crate::limits::RESPONSE_BYTES;
    let json = r#"{"jsonrpc":"2.0","id":1,"result":"0x01"}"#;
    let body = format!("{json}{}", " ".repeat(limit - json.len()));
    for raw in [
        response(200, &body),
        format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{limit:x}\r\n{body}\r\n0\r\n\r\n"
        ),
    ] {
        let (url, _, server) = fixture(vec![(Duration::ZERO, raw)]).await;
        let (healthy, _, healthy_server) =
            fixture(vec![(Duration::ZERO, response(200, json))]).await;
        let http = client();
        let (large, small) = tokio::join!(
            rpc(&http, &url, "eth_chainId", json!([])),
            rpc(&http, &healthy, "eth_chainId", json!([])),
        );
        assert_eq!(large.unwrap(), "0x01");
        assert_eq!(small.unwrap(), "0x01");
        server.await.unwrap();
        healthy_server.await.unwrap();
    }
}
