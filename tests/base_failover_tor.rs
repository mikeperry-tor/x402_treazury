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
    let (address, requests, server) = fixture::fixture().await;
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
    )
    .unwrap();
    // Neither origin port exists: success proves requests traversed the SOCKS fixture.
    rpc.view(fixture::query()).await.unwrap();
    {
        let records = proxy.records.lock().unwrap();
        assert!(
            records
                .iter()
                .all(|r| r.address_type == 3 && r.host == "localhost")
        );
        // A failed wallet cancels its sibling: one wallet must exhaust its
        // three scopes, while the other may stop anywhere in that sequence.
        assert!((4..=7).contains(&records.iter().filter(|r| r.port == 1).count()));
        assert_eq!(
            records.iter().filter(|r| r.port == 2).count(),
            3,
            "fallback discovery plus two independent wallets"
        );
        assert_eq!(
            records
                .iter()
                .map(|r| &r.password)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            records.len()
        );
        for wallet in [1, 2] {
            let identity = IsolationId::evm(&format!("0x{wallet:040x}")).unwrap();
            let transport = network::global()
                .http_participating(
                    &identity,
                    "http://localhost:2/ok",
                    None,
                    false,
                    network::HttpPolicy {
                        allow_http1: true,
                        allow_tls12: true,
                    },
                )
                .unwrap();
            let (user, password) = network::global().credentials(&transport.scope);
            let scoped: Vec<_> = records
                .iter()
                .filter(|r| r.user == user && r.password == password)
                .collect();
            assert_eq!(scoped.len(), 1);
            assert_eq!(scoped[0].port, 2);
            assert_ne!(password, network::global().credentials(&identity).1);
        }
        let calls = requests.lock().unwrap();
        let rejected: Vec<_> = calls
            .iter()
            .filter(|(mode, q)| {
                mode == "403"
                    && q["method"] == "eth_call"
                    && q["params"][1]["blockHash"] == format!("0x{:064x}", 100)
            })
            .collect();
        let mut exhausted = false;
        let mut accounted = 0;
        for wallet in [1, 2] {
            let data = format!("0x70a08231{wallet:064x}");
            let wallet_calls: Vec<_> = rejected
                .iter()
                .filter(|(_, request)| request["params"][0]["data"] == data)
                .collect();
            assert!(wallet_calls.len() <= 3);
            exhausted |= wallet_calls.len() == 3;
            accounted += wallet_calls.len();
            assert!(wallet_calls.windows(2).all(|v| v[0].1 == v[1].1));
        }
        assert!(exhausted, "one wallet must exhaust its endpoint budget");
        assert_eq!(accounted, rejected.len());
    }
    let rpc = BaseRpc::with_fallbacks(
        &[
            "http://localhost:3/limited".into(),
            "http://localhost:4/ok".into(),
        ],
        12,
    )
    .unwrap();
    rpc.view(fixture::query()).await.unwrap();
    let records = proxy.records.lock().unwrap();
    assert_eq!(
        records.iter().filter(|record| record.port == 3).count(),
        1,
        "RPC rejection can fail over without an isolated repeat"
    );
    assert_eq!(records.iter().filter(|record| record.port == 4).count(), 3);
    let calls = requests.lock().unwrap();
    assert_eq!(
        calls.iter().filter(|(mode, _)| mode == "limited").count(),
        1
    );
    server.abort();
}
