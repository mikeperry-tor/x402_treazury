use super::*;

#[tokio::test]
async fn listener_resolution_preserves_local_addresses_and_ports() {
    for (host, port) in [("localhost", 0), ("127.0.0.1", 8123), ("::1", 8123)] {
        let addresses = listener_addresses(host, port).await.unwrap();
        assert!(!addresses.is_empty());
        assert!(addresses.iter().all(|address| address.port() == port));
    }
}

#[tokio::test]
async fn listener_resolution_accepts_wildcard_and_public_literals() {
    for host in ["0.0.0.0", "::", "192.0.2.1", "2001:db8::1"] {
        let expected = std::net::SocketAddr::new(host.parse().unwrap(), 8123);
        assert_eq!(listener_addresses(host, 8123).await.unwrap(), [expected]);
    }
}
