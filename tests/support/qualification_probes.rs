//! Native probe classification and real loopback controls; no confinement claim.
use super::{qualification_probe, qualification_probe_failure};
use serde_json::json;
use std::{io, time::Duration};

#[test]
fn only_permission_denied_establishes_a_denial_observation() {
    for (kind, denied) in [
        (io::ErrorKind::PermissionDenied, true),
        (io::ErrorKind::ConnectionRefused, false),
        (io::ErrorKind::TimedOut, false),
        (io::ErrorKind::Other, false),
    ] {
        assert_eq!(
            qualification_probe_failure(io::Error::new(kind, "private submitted prose")),
            json!({"reached":false,"denied":denied,"error_kind":format!("{kind:?}")})
        );
    }
}

#[test]
fn native_permission_and_refusal_codes_use_the_same_production_classifier() {
    #[cfg(unix)]
    let codes = [
        (libc::EPERM, "PermissionDenied", true),
        (libc::EACCES, "PermissionDenied", true),
        (libc::ECONNREFUSED, "ConnectionRefused", false),
    ];
    #[cfg(windows)]
    // ERROR_ACCESS_DENIED, WSAEACCES and WSAECONNREFUSED. These classify native
    // codes without asserting that this unfunded test applies OS confinement.
    let codes = [
        (5, "PermissionDenied", true),
        (10013, "PermissionDenied", true),
        (10061, "ConnectionRefused", false),
    ];
    #[cfg(any(unix, windows))]
    for (code, kind, denied) in codes {
        assert_eq!(
            qualification_probe_failure(io::Error::from_raw_os_error(code)),
            json!({"reached":false,"denied":denied,"error_kind":kind})
        );
    }
}

#[tokio::test]
async fn tcp_and_udp_positive_controls_reach_each_literal_loopback_family() {
    for local in ["127.0.0.1:0", "[::1]:0"] {
        let tcp = tokio::net::TcpListener::bind(local).await.unwrap();
        let probe = async {
            let observation = qualification_probe(tcp.local_addr().unwrap(), false)
                .await
                .unwrap();
            // A negative outcome must unwind and drop the pending accept future.
            assert_eq!(observation, json!({"reached":true,"denied":false}));
        };
        let ((), accepted) = tokio::join!(probe, tcp.accept());
        accepted.unwrap();
        let udp = tokio::net::UdpSocket::bind(local).await.unwrap();
        let exchange = async {
            let mut bytes = [0; 64];
            let (size, peer) = udp.recv_from(&mut bytes).await.unwrap();
            assert_eq!(&bytes[..size], b"qualification");
            udp.send_to(&bytes[..size], peer).await.unwrap();
        };
        let probe = async {
            let observation = qualification_probe(udp.local_addr().unwrap(), true)
                .await
                .unwrap();
            // Failure drops the joined exchange; it cannot wait for missing I/O.
            assert_eq!(observation, json!({"reached":true,"denied":false}));
        };
        let ((), ()) = tokio::join!(probe, exchange);
    }
}

#[tokio::test]
async fn bound_unlistening_tcp_endpoint_is_unreached_without_denial() {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    // Keep the port bound but not listening, excluding another owner/port reuse.
    let observation = qualification_probe(socket.local_addr().unwrap(), false)
        .await
        .unwrap();
    // A held, unlistening socket can refuse or silently withhold TCP acceptance
    // depending on the OS. Neither outcome establishes a permission denial.
    assert!(
        observation == json!({"reached":false,"denied":false,"error_kind":"ConnectionRefused"})
            || observation == json!({"reached":false,"denied":false,"error_kind":"timeout"}),
        "{observation}"
    );
}

#[tokio::test]
async fn udp_reply_timeout_after_received_datagram_is_not_a_denial() {
    let receiver = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = receiver.local_addr().unwrap();
    let probe = qualification_probe(address, true);
    tokio::pin!(probe);
    let mut bytes = [0; 64];
    let (size, _) = tokio::select! {
        outcome = &mut probe => panic!("probe completed before its request arrived: {outcome:?}"),
        received = receiver.recv_from(&mut bytes) => received.unwrap(),
    };
    assert_eq!(&bytes[..size], b"qualification");
    // The request reached its held receiver. Withhold the reply and advance only
    // the existing probe timer; closing the receiver would test refusal instead.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        probe.await.unwrap(),
        json!({"reached":false,"denied":false,"error_kind":"timeout"})
    );
    drop(receiver);
}

#[tokio::test]
async fn invalid_probe_destination_is_rejected_before_io() {
    for address in ["192.0.2.1:1", "127.0.0.1:0", "[::1]:0"] {
        assert!(
            qualification_probe(address.parse().unwrap(), false)
                .await
                .unwrap_err()
                .to_string()
                .contains("requires literal loopback")
        );
    }
}
