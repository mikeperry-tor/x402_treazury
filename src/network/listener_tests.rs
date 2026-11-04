use super::*;

#[tokio::test]
async fn local_listener_resolution_preserves_loopback_addresses_and_ports() {
    for (host, port) in [("localhost", 0), ("127.0.0.1", 8123), ("::1", 8123)] {
        let addresses = loopback_listener_addresses(host, port).await.unwrap();
        assert!(!addresses.is_empty());
        assert!(addresses.iter().all(|address| address.ip().is_loopback()));
        assert!(addresses.iter().all(|address| address.port() == port));
    }
}

#[tokio::test]
async fn local_listener_resolution_rejects_public_and_wildcard_literals() {
    for host in ["0.0.0.0", "::", "192.0.2.1", "2001:db8::1"] {
        assert!(
            loopback_listener_addresses(host, 0)
                .await
                .unwrap_err()
                .to_string()
                .contains("requires a loopback listen address"),
            "{host}"
        );
    }
}
