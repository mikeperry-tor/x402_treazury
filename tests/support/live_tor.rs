//! Opt-in real network qualification. Only public dummy identities; never wallet keys.
use super::*;
use serde_json::json;
use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use zingo_netutils::Indexer;
const ORIGIN: &str = "https://zec.rocks/";
fn context(timeout: u64) -> NetworkContext {
    NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(
            std::env::var("TOR_SMOKE_SOCKS")
                .expect("set TOR_SMOKE_SOCKS")
                .parse()
                .unwrap(),
        ),
        isolation_namespace: Some(
            std::env::var("TOR_SMOKE_NAMESPACE")
                .unwrap_or_else(|_| format!("qualification_{}", uuid::Uuid::new_v4().simple())),
        ),
        connect_timeout_seconds: Some(timeout),
        ..Default::default()
    })
    .unwrap()
}
fn report(ctx: &NetworkContext, label: &str, id: &IsolationId) {
    let (user, password) = ctx.credentials(id);
    // These are synthetic qualification identities, not operator wallet credentials.
    println!(
        "TOR_IDENTITY {}",
        json!({"label":label,"user":user,"password":password})
    );
}
pub async fn smoke() {
    let ctx = context(60);
    let a = IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
    let b = IsolationId::evm("0x0000000000000000000000000000000000000002").unwrap();
    let treasury = IsolationId::treasury("qualification-only-treasury");
    let discovery = IsolationId::discovery(ORIGIN).unwrap();
    for (label, id) in [
        ("evm_a", &a),
        ("evm_b", &b),
        ("treasury", &treasury),
        ("discovery", &discovery),
    ] {
        report(&ctx, label, id);
    }
    for (label, id) in [
        ("evm_a", &a),
        ("evm_b", &b),
        ("evm_a", &a),
        ("discovery", &discovery),
    ] {
        let response = ctx
            .http(id, ORIGIN, Duration::from_secs(90))
            .unwrap()
            .get(ORIGIN)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        x402_treazury::limits::read(
            response,
            4 * 1024 * 1024,
            "Tor qualification page",
            "qualification_max_bytes",
        )
        .await
        .unwrap();
        println!("TOR_REQUEST {}", json!({"label":label,"protocol":"https"}));
    }
    for (label, id) in [("treasury", &treasury), ("evm_a", &a)] {
        let mut indexer = ctx.grpc(id, "https://zec.rocks:443").await.unwrap();
        let info = indexer
            .get_lightd_info(Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(info.chain_name, "main");
        assert!(info.block_height > 3_428_143);
        let tree = indexer
            .get_tree_state(
                zingo_netutils::lightwallet_protocol::BlockId {
                    height: info.block_height,
                    hash: vec![],
                },
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        assert_eq!(tree.height, info.block_height);
        assert!(!tree.ironwood_tree.is_empty());
        println!(
            "TOR_REQUEST {}",
            json!({"label":label,"protocol":"grpc","height":tree.height})
        );
    }
}
pub async fn unavailable() {
    let ctx = context(1);
    let id = IsolationId::treasury("stopped-proxy");
    assert!(
        ctx.http(&id, ORIGIN, Duration::from_secs(3))
            .unwrap()
            .get(ORIGIN)
            .send()
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(
            Duration::from_secs(4),
            ctx.grpc(&id, "https://zec.rocks:443")
        )
        .await
        .unwrap()
        .is_err()
    );
    println!("TOR_UNAVAILABLE http_and_grpc_failed_without_proxy");
}
pub fn egress_controls() {
    let denied = match std::env::var("TOR_EGRESS_DENIED").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => panic!("set TOR_EGRESS_DENIED=0 or 1"),
    };
    for name in ["TCP4", "TCP6", "UDP4", "UDP6"] {
        let destination = std::env::var(format!("TOR_CONTROL_{name}")).expect("control endpoint");
        let result = if name.starts_with("TCP") {
            TcpStream::connect_timeout(&destination.parse().unwrap(), Duration::from_secs(2))
                .map(|_| ())
        } else {
            (|| {
                let socket = UdpSocket::bind(if name == "UDP4" {
                    "127.0.0.1:0"
                } else {
                    "[::1]:0"
                })?;
                socket.set_read_timeout(Some(Duration::from_secs(2)))?;
                socket.send_to(b"qualification", &destination)?;
                let mut reply = [0; 32];
                let n = socket.recv(&mut reply)?;
                assert_eq!(&reply[..n], b"qualification");
                Ok::<_, std::io::Error>(())
            })()
        };
        println!(
            "TOR_EGRESS {}",
            json!({"probe":name,"allowed":result.is_ok(),"error":result.as_ref().err().map(ToString::to_string)})
        );
        assert_eq!(result.is_err(), denied, "{name}: {result:?}");
    }
    // This exercises the OS resolver, including its IPC path, not just raw UDP.
    let result = ("zec.rocks", 443).to_socket_addrs();
    let allowed = result
        .as_ref()
        .is_ok_and(|addresses| addresses.clone().next().is_some());
    println!(
        "TOR_EGRESS {}",
        json!({"probe":"system_dns","allowed":allowed,"error":result.as_ref().err().map(ToString::to_string)})
    );
    assert_eq!(allowed, !denied, "system DNS: {result:?}");
}
