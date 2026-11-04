//! Public fixture key/CA only. Certificate and hostname verification stays enabled.
#![allow(dead_code)]
use std::{net::SocketAddr, sync::Arc};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    },
};
pub const CA: &[u8] = include_bytes!("../fixtures/tls/ca.pem");
/// Independent caller-supplied client for low-level payment APIs.
pub fn client(url: &str) -> reqwest::Client {
    crate::network::NetworkContext::new(Default::default())
        .unwrap()
        .with_trusted_roots([CA])
        .unwrap()
        .http(
            &crate::network::IsolationId::discovery(url).unwrap(),
            url,
            None,
        )
        .unwrap()
}
/// Explicit immutable private-PKI trust; certificate and hostname checks stay on.
pub fn install_trust() {
    crate::network::install_context(
        crate::network::NetworkContext::new(Default::default())
            .unwrap()
            .with_trusted_roots([CA])
            .unwrap(),
    )
    .unwrap();
}
/// A raw TLS acceptor for fixtures that assert exact HTTP bytes.
pub fn acceptor() -> TlsAcceptor {
    let mut config = config(
        &[&rustls::version::TLS13, &rustls::version::TLS12],
        &[b"h2", b"http/1.1"],
    );
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    TlsAcceptor::from(Arc::new(config))
}
pub async fn listener() -> Listener {
    from_tcp(TcpListener::bind("127.0.0.1:0").await.unwrap())
}
/// Wrap an existing fixture socket after any raw connection-boundary assertions.
pub fn from_tcp(tcp: TcpListener) -> Listener {
    Listener {
        tcp,
        tls: TlsAcceptor::from(Arc::new(config(
            &[&rustls::version::TLS13, &rustls::version::TLS12],
            &[b"h2", b"http/1.1"],
        ))),
        delay: std::time::Duration::ZERO,
    }
}
pub struct Listener {
    tcp: TcpListener,
    tls: TlsAcceptor,
    delay: std::time::Duration,
}
#[derive(Clone, Copy, Debug)]
pub struct PeerAddr(pub SocketAddr);
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, Listener>>
    for PeerAddr
{
    fn connect_info(stream: axum::serve::IncomingStream<'_, Listener>) -> Self {
        Self(*stream.remote_addr())
    }
}
impl axum::serve::Listener for Listener {
    type Io = tokio_rustls::server::TlsStream<TcpStream>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (tcp, address) = self.tcp.accept().await.unwrap();
            // An undelayed handshake must also progress with a paused test clock.
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            if let Ok(Ok(tls)) =
                tokio::time::timeout(std::time::Duration::from_secs(3), self.tls.accept(tcp)).await
            {
                return (tls, address);
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.tcp.local_addr()
    }
}
pub async fn serve(app: axum::Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    serve_with(
        app,
        &[&rustls::version::TLS13, &rustls::version::TLS12],
        &[b"h2", b"http/1.1"],
    )
    .await
}
pub async fn serve_with(
    app: axum::Router,
    versions: &[&'static rustls::SupportedProtocolVersion],
    alpn: &[&[u8]],
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    serve_delayed(app, versions, alpn, std::time::Duration::ZERO).await
}

pub async fn serve_delayed(
    app: axum::Router,
    versions: &[&'static rustls::SupportedProtocolVersion],
    alpn: &[&[u8]],
    delay: std::time::Duration,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let config = config(versions, alpn);
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(
            Listener {
                tcp,
                delay,
                tls: TlsAcceptor::from(Arc::new(config)),
            },
            app,
        )
        .await
        .unwrap();
    });
    (address, task)
}

fn config(versions: &[&'static rustls::SupportedProtocolVersion], alpn: &[&[u8]]) -> ServerConfig {
    let provider = tokio_rustls::rustls::crypto::ring::default_provider();
    let mut config = ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(versions)
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(
                include_bytes!("../fixtures/tls/server.der").to_vec(),
            )],
            PrivatePkcs8KeyDer::from(include_bytes!("../fixtures/tls/server-key.der").to_vec())
                .into(),
        )
        .unwrap();
    config.alpn_protocols = alpn.iter().map(|v| v.to_vec()).collect();
    config
}
