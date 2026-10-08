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
            rpc(
                &client(),
                &url,
                "eth_call",
                params.clone(),
                Duration::from_secs(2)
            )
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
    let error = rpc(
        &client(),
        &url,
        "eth_chainId",
        json!(["secret-params"]),
        Duration::from_secs(2),
    )
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
        let error = rpc(
            &client(),
            &url,
            "eth_chainId",
            json!([]),
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        task.await.unwrap();
        assert_eq!(captured.lock().unwrap().len(), 1);
        assert!(!error.to_string().contains("secret"));
    }
    let (url, captured, task) = fixture(vec![(Duration::ZERO, String::new())]).await;
    assert!(
        rpc(
            &client(),
            &url,
            "eth_sendRawTransaction",
            json!(["secret"]),
            Duration::from_secs(2)
        )
        .await
        .is_err()
    );
    assert!(captured.lock().unwrap().is_empty());
    task.abort();
}
#[tokio::test]
async fn attempts_share_one_deadline_and_cancellation_does_not_retry() {
    let (url, captured, task) = fixture(vec![
        (Duration::ZERO, String::new()),
        (Duration::from_secs(5), String::new()),
    ])
    .await;
    let started = Instant::now();
    let error = rpc(
        &client(),
        &url,
        "eth_chainId",
        json!([]),
        Duration::from_millis(400),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("total budget awaiting headers"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(captured.lock().unwrap().len(), 2);
    task.abort();

    let (url, captured, task) = fixture(vec![(Duration::from_secs(5), String::new())]).await;
    let call = tokio::spawn(async move {
        rpc(
            &client(),
            &url,
            "eth_chainId",
            json!([]),
            Duration::from_secs(2),
        )
        .await
    });
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
    let error = rpc(
        &client(),
        &url,
        "eth_chainId",
        json!([]),
        Duration::from_secs(2),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("TLS"), "{error}");
    assert!(!error.to_string().contains("secret"));
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn stalled_body_deadline_reports_body_not_headers() {
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
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });
    let error = rpc(
        &client(),
        &url,
        "eth_chainId",
        json!([]),
        Duration::from_millis(200),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("total budget reading body"));
    assert!(!error.to_string().contains("http://"));
    task.abort();
}
