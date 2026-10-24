//! Untrusted TLS stays untrusted on each fresh scope; only a later valid peer wins.
use super::*;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    },
};

// Public fixture certificate, self-signed with the repository's public TLS fixture
// key. It is intentionally absent from the immutable test trust roots.
const UNTRUSTED: &str = "MIIDEzCCAfugAwIBAgIUSLbZ/H0OOLZiVs06zv8QOqmJSJEwDQYJKoZIhvcNAQELBQAwGjEYMBYGA1UEAwwPYXBpLmV4YW1wbGUuY29tMB4XDTI2MTAyMDA2MjIyNFoXDTQ2MTAxNTA2MjIyNFowGjEYMBYGA1UEAwwPYXBpLmV4YW1wbGUuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAoLKu6cRlH+Ds87H9YqAZAjXcelrR+Ui1BdlKldnoiIFyB0CCMhCeaC3gKGK6a8BTjjQ7rO/hn/jduGTsWdHyQH1BIy77vx77css5PaClcKRufVeruLGu8kDkwV7mjmKK+sn13oIE2W9oRMDa4g3krtG9gZFHIjPHQ1emLe1NQSHvcrYA8Us/vbL5f70WLHIiplu9P9TeZClPlYkQewHKGZdFysXbL4h9yzKOOTe3ogzHUfpwSNgjlSXtm81u7nt09W/T0mZeneinpavLyPbgkMMGN/i5zf+l/Vk+KUbft66RM+6dYF3KHsNpSiMVwxo1dOLIYilcGsEItTIG0ZNh3QIDAQABo1EwTzAaBgNVHREEEzARgg9hcGkuZXhhbXBsZS5jb20wDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCB4AwEwYDVR0lBAwwCgYIKwYBBQUHAwEwDQYJKoZIhvcNAQELBQADggEBAIy2kfaYsTXwW2EDfCixkW+msbCkYdWJUBTsGtuJvF5gofYsfvZQDY/IVPc6osSFnlqZXNRYK5I4su4RvXUcJUmUYE2Xiucb2rgwnlxP+F4KP6bpt6+5QbD1sa0UD/K3J231nMaoP6X/sr02QgGgkaqqwvb8ml+6LKk1ZZdTPCA5p1+J3Pz5X0/Ck9tkYGH0IYesBfMOi52lYZq/Hlf3UN+UWrVr6IQEDnRmyvlN4S2fIVJPwPLSb/5CrvnIJsW027dV2+gSAuRDeq4cCbBjQjRbKH+9YcztafrxF7qNXtERdCnzCSzna0Cdj7ZNQmMgct7TK6G+GRrVuofyHyIp6Z0=";
struct Listener {
    tcp: TcpListener,
    good: TlsAcceptor,
    bad: TlsAcceptor,
    remaining: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    body: Arc<BodyFault>,
    http1: bool,
    handshake_eofs: Arc<AtomicUsize>,
}
struct BodyFault {
    connections: AtomicUsize,
    skip: AtomicUsize,
    responses: AtomicUsize,
    status_first: AtomicUsize,
    headers: Semaphore,
    flipped: AtomicUsize,
}
impl Default for BodyFault {
    fn default() -> Self {
        Self {
            connections: AtomicUsize::new(0),
            skip: AtomicUsize::new(0),
            responses: AtomicUsize::new(0),
            status_first: AtomicUsize::new(0),
            headers: Semaphore::new(0),
            flipped: AtomicUsize::new(0),
        }
    }
}
#[allow(
    deprecated,
    reason = "try_update requires Rust 1.95; the declared MSRV is 1.93"
)]
fn take(n: &AtomicUsize) -> bool {
    n.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
}
struct Socket {
    tcp: TcpStream,
    handshake: Arc<AtomicBool>,
    corrupt: bool,
    first_write: bool,
    body: Arc<BodyFault>,
}
impl AsyncRead for Socket {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.tcp).poll_read(cx, buf)
    }
}
impl AsyncWrite for Socket {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if !self.corrupt || !self.handshake.load(Ordering::SeqCst) {
            return Pin::new(&mut self.tcp).poll_write(cx, bytes);
        }
        if !self.first_write {
            let result = Pin::new(&mut self.tcp).poll_write(cx, bytes);
            if matches!(result,Poll::Ready(Ok(n)) if n>0) {
                self.first_write = true;
                self.body.headers.add_permits(1);
            }
            return result;
        }
        let mut corrupt = bytes.to_vec();
        let index = corrupt.len().saturating_sub(1);
        if let Some(byte) = corrupt.get_mut(index) {
            *byte ^= 1;
        }
        let result = Pin::new(&mut self.tcp).poll_write(cx, &corrupt);
        if matches!(result,Poll::Ready(Ok(n)) if n>index) {
            self.corrupt = false;
            self.body.flipped.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.tcp).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.tcp).poll_shutdown(cx)
    }
}
impl axum::serve::Listener for Listener {
    type Io = tokio_rustls::server::TlsStream<Socket>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (mut socket, address) = self.tcp.accept().await.unwrap();
            self.connections.fetch_add(1, Ordering::SeqCst);
            if take(&self.handshake_eofs) {
                use tokio::io::AsyncReadExt;
                let mut header = [0; 5];
                socket.read_exact(&mut header).await.unwrap();
                assert_eq!(header[0], 22, "ClientHello handshake record");
                let mut hello = vec![0; u16::from_be_bytes([header[3], header[4]]) as usize];
                socket.read_exact(&mut hello).await.unwrap();
                drop(socket);
                continue;
            }
            let handshake = Arc::new(AtomicBool::new(false));
            let corrupt = self.http1 && !take(&self.body.skip) && take(&self.body.connections);
            let socket = Socket {
                tcp: socket,
                handshake: handshake.clone(),
                corrupt,
                first_write: false,
                body: self.body.clone(),
            };
            let bad = take(&self.remaining);
            if let Ok(stream) = if bad {
                self.bad.accept(socket).await
            } else {
                self.good.accept(socket).await
            } {
                handshake.store(true, Ordering::SeqCst);
                return (stream, address);
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.tcp.local_addr()
    }
}
fn acceptor(cert: Vec<u8>, http1: bool) -> TlsAcceptor {
    let provider = tokio_rustls::rustls::crypto::ring::default_provider();
    let mut config = ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert)],
            PrivatePkcs8KeyDer::from(
                include_bytes!("../../tests/fixtures/tls/server-key.der").to_vec(),
            )
            .into(),
        )
        .unwrap();
    config.send_tls13_tickets = 0;
    config.alpn_protocols = if http1 {
        vec![b"http/1.1".to_vec()]
    } else {
        vec![b"h2".to_vec()]
    };
    TlsAcceptor::from(Arc::new(config))
}
#[derive(Clone, Default)]
struct StateData {
    calls: Arc<Mutex<Vec<String>>>,
    signed_tls_failure: Arc<AtomicUsize>,
    body: Arc<BodyFault>,
    handshake_eofs: Arc<AtomicUsize>,
}
async fn catalog(State(s): State<StateData>) -> Response {
    s.calls.lock().unwrap().push("catalog".into());
    axum::Json(json!({"openapi":"3.0.0","paths":{}})).into_response()
}
async fn api(State(s): State<StateData>, headers: HeaderMap) -> Response {
    assert!(!headers.contains_key("payment-signature"));
    s.calls.lock().unwrap().push("api".into());
    ([("content-type", "text/plain")], "free").into_response()
}
async fn base(
    State(s): State<StateData>,
    headers: HeaderMap,
    axum::Json(value): axum::Json<Value>,
) -> Response {
    assert!(!headers.contains_key("payment-signature"));
    assert_eq!(value["method"], "eth_call");
    s.calls.lock().unwrap().push("base".into());
    axum::Json(json!({"jsonrpc":"2.0","id":1,"result":format!("0x{:064x}",7)})).into_response()
}
async fn signed(State(s): State<StateData>, headers: HeaderMap) -> Response {
    assert!(
        !headers.contains_key("payment-signature"),
        "signed TLS failure cannot reach the HTTP handler"
    );
    s.calls.lock().unwrap().push("signed-challenge".into());
    s.signed_tls_failure.store(1, Ordering::SeqCst);
    let challenge = json!({"x402Version":2,"resource":{"url":"https://spec.example.com/signed","description":"fixture","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [
            (
                "payment-required",
                STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
            ),
            ("connection", "close".into()),
        ],
        "",
    )
        .into_response()
}
async fn body_response(State(s): State<StateData>, uri: Uri, headers: HeaderMap) -> Response {
    let signed = headers.contains_key("payment-signature");
    s.calls.lock().unwrap().push(format!(
        "{}:{}",
        uri.path(),
        if signed { "signed" } else { "unsigned" }
    ));
    if signed {
        assert_eq!(uri.path(), "/body-challenge");
        let payload: Value = serde_json::from_slice(
            &STANDARD
                .decode(headers["payment-signature"].as_bytes())
                .unwrap(),
        )
        .unwrap();
        crate::test_signatures::recover_exact(&payload);
        return ([("content-type", "text/plain")], "paid").into_response();
    }
    if take(&s.body.status_first) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response_headers = HeaderMap::new();
    response_headers.insert("content-type", "application/json".parse().unwrap());
    let status = if uri.path() == "/body-challenge" {
        let challenge = json!({"x402Version":2,"resource":{"url":"https://spec.example.com/body-challenge","description":"fixture","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
        response_headers.insert(
            "payment-required",
            STANDARD
                .encode(serde_json::to_vec(&challenge).unwrap())
                .parse()
                .unwrap(),
        );
        StatusCode::PAYMENT_REQUIRED
    } else {
        StatusCode::OK
    };
    let body = if uri.path() == "/body-base" {
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"result":format!("0x{:064x}",7)}))
            .unwrap()
    } else {
        br#"{"openapi":"3.0.0","paths":{}}"#.to_vec()
    };
    let body = if take(&s.body.responses) {
        let state = s.body.clone();
        axum::body::Body::from_stream(futures_util::stream::once(async move {
            state.headers.acquire().await.unwrap().forget();
            Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(body))
        }))
    } else {
        axum::body::Body::from(body)
    };
    (status, response_headers, body).into_response()
}
async fn serve(
    s: StateData,
    http1: bool,
    remaining: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    let body = s.body.clone();
    let handshake_eofs = s.handshake_eofs.clone();
    let app = axum::Router::new()
        .route("/catalog", get(catalog))
        .route("/api", get(api))
        .route("/base", post(base))
        .route("/signed", get(signed))
        .route("/body-free", get(body_response))
        .route("/body-challenge", get(body_response))
        .route("/body-catalog", get(body_response))
        .route("/body-base", post(body_response))
        .with_state(s);
    let listener = Listener {
        tcp,
        good: acceptor(
            include_bytes!("../../tests/fixtures/tls/server.der").to_vec(),
            http1,
        ),
        bad: acceptor(STANDARD.decode(UNTRUSTED).unwrap(), http1),
        remaining,
        connections,
        body,
        http1,
        handshake_eofs,
    };
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, task)
}
struct Loopback;
impl reqwest::dns::Resolve for Loopback {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async {
            Ok(Box::new(std::iter::once(
                "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            )) as reqwest::dns::Addrs)
        })
    }
}
#[test]
fn tls_failures_retry_only_unsigned_tor_consumers_in_isolated_process() {
    if std::env::var_os("TREAZURY_TLS_RETRY_CHILD").is_none() {
        for mode in ["tor", "tor0", "tor1", "direct"] {
            let output=std::process::Command::new(std::env::current_exe().unwrap()).env_clear()
                .envs(std::env::var("LLVM_PROFILE_FILE").map(|v|("LLVM_PROFILE_FILE",v)))
                .env("TREAZURY_TLS_RETRY_CHILD",mode)
                .args(["--exact","network::tls_retry_tests::tls_failures_retry_only_unsigned_tor_consumers_in_isolated_process","--nocapture"]).output().unwrap();
            assert!(
                output.status.success(),
                "{mode}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    tokio::runtime::Runtime::new().unwrap().block_on(fixture());
}
async fn fixture() {
    let mode = std::env::var("TREAZURY_TLS_RETRY_CHILD").unwrap();
    let tor = mode.starts_with("tor");
    let retries = match mode.as_str() {
        "tor0" => Some(0),
        "tor1" => Some(1),
        _ => None,
    };
    let state = StateData::default();
    let remaining = Arc::new(AtomicUsize::new(0));
    let connections = Arc::new(AtomicUsize::new(0));
    let (address, server) =
        serve(state.clone(), false, remaining.clone(), connections.clone()).await;
    let signed_connections = Arc::new(AtomicUsize::new(0));
    let (signed_address, signed_server) = serve(
        state.clone(),
        true,
        state.signed_tls_failure.clone(),
        signed_connections.clone(),
    )
    .await;
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), signed_address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    let mut context = NetworkContext::new(NetworkPolicy {
        mode: if tor { Mode::Tor } else { Mode::Direct },
        socks_endpoint: tor.then_some(socks.address),
        read_timeout_seconds: Some(2),
        tor_isolation_retries: retries,
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    if !tor {
        context.test_dns = Some(Arc::new(Loopback));
    }
    install_test_context(context);
    if tor {
        state.handshake_eofs.store(1, Ordering::SeqCst);
        let error = global()
            .discovery("https://api.example.com:9999/api", None)
            .unwrap()
            .get("https://api.example.com:9999/api")
            .send()
            .await
            .unwrap_err();
        assert!(
            tls_failure(&error),
            "pinned tokio-rustls handshake EOF is TLS evidence"
        );
        let mut source = std::error::Error::source(&error);
        let mut observed = false;
        while let Some(error) = source {
            observed |= error.downcast_ref::<std::io::Error>().is_some_and(|io| {
                io.kind() == std::io::ErrorKind::UnexpectedEof
                    && io.to_string() == "tls handshake eof"
            });
            source = error.source();
        }
        assert!(
            observed,
            "actual failed ClientHello must expose the local TLS EOF marker"
        );
    }
    let mut port = 10000;
    for eof in [false, true] {
        if eof && !tor {
            continue;
        }
        for consumer in ["catalog", "api", "base"] {
            for failures in if eof {
                vec![1]
            } else if tor {
                vec![0, 1, 2, 3]
            } else {
                vec![3]
            } {
                remaining.store(if eof { 0 } else { failures }, Ordering::SeqCst);
                state
                    .handshake_eofs
                    .store(if eof { failures } else { 0 }, Ordering::SeqCst);
                state.calls.lock().unwrap().clear();
                let before_connections = connections.load(Ordering::SeqCst);
                let before_records = socks.records.lock().unwrap().len();
                let endpoint = format!(
                    "https://api.example.com:{}/{}",
                    if tor { port } else { address.port() },
                    consumer
                );
                port += 1;
                let policy = if consumer == "base" {
                    HttpPolicy::COMPATIBLE
                } else {
                    HttpPolicy::default()
                };
                let payer = crate::payment::Payer::new(
                    &format!("{:064x}", 71),
                    crate::payment::SpendPolicy::dollars("1").unwrap(),
                )
                .unwrap();
                let identity = if consumer == "api" {
                    IsolationId::evm(&payer.address).unwrap()
                } else if consumer == "base" {
                    IsolationId::evm("0x0000000000000000000000000000000000000042").unwrap()
                } else {
                    IsolationId::discovery(&endpoint).unwrap()
                };
                let initial = global()
                    .http_participating(&identity, &endpoint, None, false, policy)
                    .unwrap();
                let success = match consumer {
                    "catalog" => {
                        let cfg = crate::catalog::Config {
                            spec: endpoint.clone(),
                            http_cache_enabled: false,
                            ..Default::default()
                        };
                        crate::catalog::load_json_discovery(&cfg).await.is_ok()
                    }
                    "api" => crate::payment::PaidClient::new(payer)
                        .execute_response(crate::catalog::RoutedRequest {
                            headers: Default::default(),
                            method: "GET".into(),
                            url: endpoint.clone(),
                            query: BTreeMap::new(),
                            body: None,
                        })
                        .await
                        .is_ok(),
                    _ => crate::rotation::base::BaseRpc::new(&endpoint, 12)
                        .unwrap()
                        .current_balance("0x0000000000000000000000000000000000000042")
                        .await
                        .is_ok(),
                };
                let max_attempts = global().max_http_attempts(false);
                let expected_success = tor && failures < max_attempts;
                assert_eq!(
                    success, expected_success,
                    "{consumer} {failures} untrusted handshakes"
                );
                let attempts = (failures + 1).min(max_attempts);
                assert_eq!(
                    connections.load(Ordering::SeqCst) - before_connections,
                    attempts,
                    "{consumer} exact finite TLS attempts"
                );
                assert_eq!(
                    state.calls.lock().unwrap().len(),
                    usize::from(expected_success),
                    "untrusted handshakes never reach application HTTP"
                );
                let selected = global()
                    .http_participating(&identity, &endpoint, None, false, policy)
                    .unwrap();
                assert_eq!(
                    Arc::ptr_eq(&initial, &selected),
                    !expected_success || failures == 0,
                    "only verified successful TLS/output promotes"
                );
                let records = socks.records.lock().unwrap();
                let records = &records[before_records..];
                assert_eq!(records.len(), if tor { attempts } else { 0 });
                if tor {
                    assert_eq!(
                        records
                            .iter()
                            .map(|r| &r.password)
                            .collect::<std::collections::BTreeSet<_>>()
                            .len(),
                        attempts,
                        "every TLS retry has distinct SOCKS credentials and physical connection"
                    );
                }
            }
        }
    }
    if tor {
        state.calls.lock().unwrap().clear();
        let endpoint = "https://spec.example.com/signed";
        let payer = crate::payment::Payer::new(
            &format!("{:064x}", 72),
            crate::payment::SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap();
        let client = crate::payment::PaidClient::new(payer).with_transport(HttpPolicy::COMPATIBLE);
        let error = client
            .execute_response(crate::catalog::RoutedRequest {
                headers: Default::default(),
                method: "GET".into(),
                url: endpoint.into(),
                query: BTreeMap::new(),
                body: None,
            })
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("charge"),
            "signed TLS uncertainty retains payment warning"
        );
        assert_eq!(
            signed_connections.load(Ordering::SeqCst),
            2,
            "one accepted challenge then exactly one failed signed TLS handshake"
        );
        assert_eq!(
            *state.calls.lock().unwrap(),
            ["signed-challenge"],
            "signed TLS failure never obtains another challenge or signature"
        );
    }
    body_cases(
        &state,
        &signed_connections,
        &socks,
        tor,
        signed_address.port(),
    )
    .await;
    server.abort();
    signed_server.abort();
}

async fn body_cases(
    state: &StateData,
    connections: &Arc<AtomicUsize>,
    socks: &crate::test_socks::Socks,
    tor: bool,
    direct_port: u16,
) {
    // Establish the client-side phase too: headers are accepted before the
    // encrypted body fails. Consumer rows below therefore test body recovery.
    state.body.connections.store(1, Ordering::SeqCst);
    state.body.responses.store(1, Ordering::SeqCst);
    state.body.skip.store(0, Ordering::SeqCst);
    state.body.status_first.store(0, Ordering::SeqCst);
    let probe = format!(
        "https://spec.example.com:{}/body-free",
        if tor { 19999 } else { direct_port }
    );
    let transport = global()
        .http_participating(
            &IsolationId::discovery(&probe).unwrap(),
            &probe,
            None,
            false,
            HttpPolicy::COMPATIBLE,
        )
        .unwrap();
    let response = transport
        .client
        .get(&probe)
        .send()
        .await
        .expect("valid HTTP headers before corrupted TLS body");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let error = response.bytes().await.unwrap_err();
    assert!(
        tls_failure(&error),
        "body decryption failure retains TLS evidence"
    );
    let cases = if tor {
        vec![
            ("free", 1, 0),
            ("challenge", 1, 0),
            ("catalog", 1, 0),
            ("base", 1, 0),
            ("free", 1, 1),
            ("free", 3, 0),
        ]
    } else {
        vec![
            ("free", 1, 0),
            ("challenge", 1, 0),
            ("catalog", 1, 0),
            ("base", 1, 0),
        ]
    };
    for (index, (consumer, failures, status_first)) in cases.into_iter().enumerate() {
        state.calls.lock().unwrap().clear();
        state.body.connections.store(failures, Ordering::SeqCst);
        state.body.responses.store(failures, Ordering::SeqCst);
        state
            .body
            .status_first
            .store(status_first, Ordering::SeqCst);
        state.body.skip.store(status_first, Ordering::SeqCst);
        let flipped = state.body.flipped.load(Ordering::SeqCst);
        let physical = connections.load(Ordering::SeqCst);
        let records_before = socks.records.lock().unwrap().len();
        let endpoint = format!(
            "https://spec.example.com:{}/body-{consumer}",
            if tor {
                20000 + index as u16
            } else {
                direct_port
            }
        );
        let payer = crate::payment::Payer::new(
            &format!("{:064x}", 73),
            crate::payment::SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap();
        let identity = if matches!(consumer, "free" | "challenge") {
            IsolationId::evm(&payer.address).unwrap()
        } else if consumer == "base" {
            IsolationId::evm("0x0000000000000000000000000000000000000043").unwrap()
        } else {
            IsolationId::discovery(&endpoint).unwrap()
        };
        let initial = global()
            .http_participating(&identity, &endpoint, None, false, HttpPolicy::COMPATIBLE)
            .unwrap();
        let success = match consumer {
            "catalog" => crate::catalog::load_json_discovery(&crate::catalog::Config {
                spec: endpoint.clone(),
                allow_http1: true,
                allow_tls12: true,
                http_cache_enabled: false,
                ..Default::default()
            })
            .await
            .is_ok(),
            "base" => crate::rotation::base::BaseRpc::new(&endpoint, 12)
                .unwrap()
                .current_balance("0x0000000000000000000000000000000000000043")
                .await
                .is_ok(),
            _ => crate::payment::PaidClient::new(payer)
                .with_transport(HttpPolicy::COMPATIBLE)
                .execute_response(crate::catalog::RoutedRequest {
                    headers: Default::default(),
                    method: "GET".into(),
                    url: endpoint.clone(),
                    query: BTreeMap::new(),
                    body: None,
                })
                .await
                .is_ok(),
        };
        let max_attempts = global().max_http_attempts(false);
        let expected_success = tor && failures + status_first < max_attempts;
        assert_eq!(
            success, expected_success,
            "{consumer} real TLS body corruption"
        );
        let attempts = if tor {
            (failures + status_first + 1).min(max_attempts)
        } else {
            1
        };
        assert_eq!(
            connections.load(Ordering::SeqCst) - physical,
            attempts,
            "body TLS and status share one finite scope budget"
        );
        assert_eq!(
            state.body.flipped.load(Ordering::SeqCst) - flipped,
            failures.min(attempts.saturating_sub(status_first)),
            "actual encrypted body records were corrupted after valid headers"
        );
        let calls = state.calls.lock().unwrap().clone();
        let signed = usize::from(consumer == "challenge" && expected_success);
        assert_eq!(calls.len(), attempts + signed);
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.ends_with(":signed"))
                .count(),
            signed,
            "only a complete successful challenge body may lead to one signature"
        );
        let selected = global()
            .http_participating(&identity, &endpoint, None, false, HttpPolicy::COMPATIBLE)
            .unwrap();
        assert_eq!(Arc::ptr_eq(&initial, &selected), !expected_success);
        let records = socks.records.lock().unwrap();
        let records = &records[records_before..];
        assert_eq!(records.len(), if tor { attempts } else { 0 });
        if tor {
            assert_eq!(
                records
                    .iter()
                    .map(|record| &record.password)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                attempts
            );
        }
    }
}
