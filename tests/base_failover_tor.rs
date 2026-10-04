#[path = "support/base_failover.rs"]
mod fixture;
#[path = "support/socks.rs"]
mod socks;
use std::collections::BTreeMap;
use x402_treazury::{
    network::{self, IsolationId, Mode, NetworkPolicy},
    rotation::base::BaseRpc,
};

// Isolated test executable because the network policy is process-wide.
#[tokio::test]
async fn fallback_retains_payer_credentials_remote_dns_and_no_direct_egress() {
    let (address, _, server) = fixture::fixture().await;
    let proxy = socks::Socks::start(
        BTreeMap::from([("localhost".into(), address)]),
        socks::Fault::None,
    )
    .await;
    network::install(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(proxy.address),
        ..Default::default()
    })
    .unwrap();
    let rpc = BaseRpc::with_fallbacks(
        &[
            "http://localhost:1/403".into(),
            "http://localhost:2/ok".into(),
        ],
        12,
        120,
    )
    .unwrap();
    // Neither origin port exists: success proves requests traversed the SOCKS fixture.
    rpc.view(fixture::query()).await.unwrap();
    let records = proxy.records.lock().unwrap();
    for wallet in [1, 2] {
        let identity = IsolationId::evm(&format!("0x{wallet:040x}")).unwrap();
        let (user, password) = network::global().credentials(&identity);
        let scoped: Vec<_> = records
            .iter()
            .filter(|r| r.user == user && r.password == password)
            .collect();
        assert!(!scoped.is_empty());
        assert!(
            scoped
                .iter()
                .all(|r| r.address_type == 3 && r.host == "localhost")
        );
        assert!(scoped.iter().any(|r| r.port == 2));
        if wallet == 1 {
            assert!(scoped.iter().any(|r| r.port == 1));
        }
    }
    drop(records);
    server.abort();
}
