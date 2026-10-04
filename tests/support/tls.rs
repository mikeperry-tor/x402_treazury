//! Public fixture key/CA only. No production trust or verification changes.
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
struct Listener {
    tcp: TcpListener,
    tls: TlsAcceptor,
}
impl axum::serve::Listener for Listener {
    type Io = tokio_rustls::server::TlsStream<TcpStream>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (tcp, address) = self.tcp.accept().await.unwrap();
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
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(
            Listener {
                tcp,
                tls: TlsAcceptor::from(Arc::new(config)),
            },
            app,
        )
        .await
        .unwrap();
    });
    (address, task)
}
