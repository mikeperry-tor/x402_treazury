//! Connector-stage evidence uses local sockets and faithful retained cause chains.
use super::*;
use crate::test_socks::{Fault, Socks};
use std::{collections::BTreeMap, error::Error, io::ErrorKind};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
struct Cause(&'static str, Box<dyn Error + Send + Sync>);
impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Cause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.1.as_ref())
    }
}

#[test]
fn only_proven_local_refusal_is_permanent() {
    for kind in [
        ErrorKind::ConnectionRefused,
        ErrorKind::TimedOut,
        ErrorKind::ConnectionReset,
    ] {
        let retained = Cause(
            "SOCKS local proxy connection failed",
            Box::new(std::io::Error::other(std::io::Error::from(kind))),
        );
        assert_eq!(
            eligible_retry_cause(&retained),
            kind != ErrorKind::ConnectionRefused
        );
        assert!(
            eligible_retry_cause(&std::io::Error::from(kind)),
            "unlocated {kind:?}"
        );
    }
    // The old opaque dependency wrapper does not establish a local stage/cause.
    assert!(eligible_retry_cause(&Cause(
        "SOCKS error: failed to create underlying connection",
        Box::new(std::io::Error::from(ErrorKind::ConnectionRefused))
    )));
}

fn context(address: SocketAddr) -> NetworkContext {
    NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(address),
        connect_timeout_seconds: Some(1),
        cover_traffic_enabled: Some(false),
        ..Default::default()
    })
    .unwrap()
}
async fn failure(context: &NetworkContext, url: &str) -> reqwest::Error {
    context
        .discovery(url, None)
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap_err()
}

#[tokio::test]
async fn actual_socks_setup_failures_preserve_stage_and_do_not_escape() {
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}/", origin.local_addr().unwrap().port());
    for (fault, eligible) in [
        (Fault::BadAuth, false),
        (Fault::NoAuth, false),
        (Fault::Refuse, true),
        (Fault::Stall, true),
    ] {
        let proxy = Socks::start(BTreeMap::new(), fault).await;
        let error = failure(&context(proxy.address), &url).await;
        assert_eq!(eligible_retry_transport(&error), eligible, "{error:?}");
        assert_eq!(proxy.accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let error = failure(&context(address), &url).await;
    assert!(!eligible_retry_transport(&error));
    let mut source: &(dyn Error + 'static) = &error;
    let mut local = false;
    let mut refused = false;
    loop {
        local |= source.to_string() == "SOCKS local proxy connection failed";
        refused |= source
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == ErrorKind::ConnectionRefused);
        match source.source() {
            Some(next) => source = next,
            None => break,
        }
    }
    assert!(
        local && refused,
        "actual connector must retain stage and typed refusal"
    );

    // A listener speaking a different protocol is a setup failure, not an exit failure.
    let wrong = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = wrong.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = wrong.accept().await.unwrap();
        let mut greeting = [0; 3];
        socket.read_exact(&mut greeting).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
    });
    let error = failure(&context(address), &url).await;
    assert!(!eligible_retry_transport(&error));
    task.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), origin.accept())
            .await
            .is_err(),
        "no setup failure may escape directly to the target"
    );
}

#[tokio::test]
async fn tls_handshake_connect_timeout_and_reset_keep_unsigned_transport_eligibility() {
    for stall in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (hello, observed) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = [0; 5];
            socket.read_exact(&mut header).await.unwrap();
            assert_eq!(header[0], 22, "failure occurs after TLS ClientHello starts");
            let mut bytes = vec![0; u16::from_be_bytes([header[3], header[4]]) as usize];
            socket.read_exact(&mut bytes).await.unwrap();
            hello.send(()).unwrap();
            if stall {
                std::future::pending::<()>().await;
            } else {
                socket.set_zero_linger().unwrap();
            }
        });
        // Direct TLS preserves the reset; a SOCKS forwarding fixture may turn it
        // into EOF. Both modes use this same connector/transport classifier.
        let mut network = NetworkContext::new(NetworkPolicy {
            connect_timeout_seconds: Some(1),
            ..Default::default()
        })
        .unwrap();
        network.test_dns = Some(Arc::new(TlsAddress(address)));
        let url = format!("https://api.example.com:{}", address.port());
        let client = network.discovery(&url, Duration::from_millis(100)).unwrap();
        let error = client.get(&url).send().await.unwrap_err();
        observed.await.unwrap();
        assert!(error.is_connect());
        assert!(eligible_retry_transport(&error));
        assert_eq!(
            error.is_timeout(),
            stall,
            "TLS stall uses connection allowance, not header inactivity"
        );
        server.abort();
        let _ = server.await;
    }
}

struct TlsAddress(SocketAddr);
impl reqwest::dns::Resolve for TlsAddress {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let address = self.0;
        Box::pin(async move { Ok(Box::new([address].into_iter()) as reqwest::dns::Addrs) })
    }
}
