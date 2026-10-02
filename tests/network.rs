#[path = "support/socks.rs"]
mod socks;
use socks::{Fault, Socks};
use std::{collections::BTreeMap, time::Duration};
use x402_treazure::network::{IsolationId, Mode, NetworkContext, NetworkPolicy, SocksAuth};
fn policy(proxy: &Socks) -> NetworkPolicy {
    NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(proxy.address),
        connect_timeout_seconds: Some(1),
        ..Default::default()
    }
}
async fn origin() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().fallback(|| async { "ok" });
    (
        address,
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }),
    )
}
#[test]
fn deterministic_tokens_and_strict_policy() {
    assert_eq!(
        NetworkPolicy::default().inspection()["connect_timeout_seconds"],
        15
    );
    assert_eq!(
        NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some("127.0.0.1:9150".parse().unwrap()),
            ..Default::default()
        }
        .inspection()["connect_timeout_seconds"],
        30
    );
    let a = IsolationId::evm("0x00000000000000000000000000000000000000aA").unwrap();
    let b = IsolationId::evm("0x00000000000000000000000000000000000000aa").unwrap();
    assert_eq!(a, b);
    assert_eq!(
        a.token("x402_treazury"),
        "9cbb1f514df07edeb7a58d45e3d50e3164cfd214463a7312a80ae8f3bd499ad7"
    );
    assert_ne!(a.token("x"), a.token("y"));
    assert_ne!(
        a,
        IsolationId::treasury("0x00000000000000000000000000000000000000aa")
    );
    assert_ne!(IsolationId::bootstrap(), IsolationId::bootstrap());
    assert_eq!(
        IsolationId::discovery("https://EXAMPLE.com/a?secret=1").unwrap(),
        IsolationId::discovery("https://example.com:443/b").unwrap()
    );
    assert!(
        NetworkPolicy {
            mode: Mode::Tor,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        NetworkPolicy {
            socks_endpoint: Some("127.0.0.1:9150".parse().unwrap()),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some("1.1.1.1:9150".parse().unwrap()),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
#[tokio::test]
async fn http_remote_dns_credentials_reuse_and_separation() {
    let (address, server) = origin().await;
    let proxy = Socks::start(
        BTreeMap::from([("unresolvable.invalid".into(), address)]),
        Fault::None,
    )
    .await;
    let context = NetworkContext::new(policy(&proxy)).unwrap();
    let a = IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
    let b = IsolationId::evm("0x0000000000000000000000000000000000000002").unwrap();
    for id in [&a, &a, &b] {
        let client = context
            .http(id, "http://unresolvable.invalid/", Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            client
                .get("http://unresolvable.invalid/")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
    }
    let records = proxy.records.lock().unwrap();
    assert_eq!(records.len(), 2, "same identity must reuse its connection");
    for (record, id) in records.iter().zip([&a, &b]) {
        assert_eq!(record.address_type, 3);
        assert_eq!(record.host, "unresolvable.invalid");
        assert_eq!(
            (record.user.clone(), record.password.clone()),
            context.credentials(id)
        );
    }
    assert_ne!(records[0].password, records[1].password);
    server.abort();
}
#[tokio::test]
async fn auth_downgrades_refusals_and_timeouts_never_go_direct() {
    let (address, server) = origin().await;
    for fault in [Fault::NoAuth, Fault::BadAuth, Fault::Refuse, Fault::Stall] {
        let proxy = Socks::start(BTreeMap::new(), fault).await;
        let ctx = NetworkContext::new(policy(&proxy)).unwrap();
        let url = format!("http://{address}/");
        assert!(
            ctx.discovery(&url, Duration::from_secs(2))
                .unwrap()
                .get(&url)
                .send()
                .await
                .is_err()
        );
        #[cfg(feature = "zcash")]
        assert!(
            ctx.grpc(&IsolationId::treasury("failure-test"), &url)
                .await
                .is_err()
        );
    }
    let proxy = Socks::start(BTreeMap::new(), Fault::None).await;
    let ctx = NetworkContext::new(policy(&proxy)).unwrap();
    drop(proxy);
    tokio::task::yield_now().await;
    let url = format!("http://{address}/");
    assert!(
        ctx.discovery(&url, Duration::from_secs(2))
            .unwrap()
            .get(url)
            .send()
            .await
            .is_err()
    );
    server.abort();
}
#[tokio::test]
async fn legacy_credentials_and_cache_eviction_keep_identity() {
    let proxy = Socks::start(BTreeMap::new(), Fault::None).await;
    let mut settings = policy(&proxy);
    settings.socks_auth = Some(SocksAuth::Legacy);
    let ctx = NetworkContext::new(settings.clone()).unwrap();
    let id = IsolationId::treasury("uuid");
    let credentials = ctx.credentials(&id);
    assert_eq!(credentials.0, "x402_treazury");
    for _ in 0..260 {
        ctx.http(
            &IsolationId::bootstrap(),
            "https://example.com",
            Duration::from_secs(2),
        )
        .unwrap();
    }
    assert_eq!(ctx.credentials(&id), credentials);
    assert_eq!(
        NetworkContext::new(settings).unwrap().credentials(&id),
        credentials
    );
}

#[cfg(feature = "zcash")]
#[tokio::test]
async fn grpc_remote_dns_authenticated_streaming_and_channel_isolation() {
    use zingo_netutils::{
        Indexer,
        lightwallet_protocol::{BlockId, BlockRange},
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().fallback(|| async {
        let frames = vec![0, 0, 0, 0, 2, 16, 1, 0, 0, 0, 0, 2, 16, 2];
        (
            [("content-type", "application/grpc"), ("grpc-status", "0")],
            frames,
        )
    });
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let proxy = Socks::start(BTreeMap::from([("grpc.invalid".into(), addr)]), Fault::None).await;
    let ctx = NetworkContext::new(policy(&proxy)).unwrap();
    for id in [
        IsolationId::treasury("one"),
        IsolationId::treasury("one"),
        IsolationId::treasury("two"),
    ] {
        let mut client = ctx.grpc(&id, "http://grpc.invalid:1234").await.unwrap();
        let mut stream = client
            .get_block_range(
                BlockRange {
                    pool_types: vec![],
                    start: Some(BlockId {
                        height: 1,
                        hash: vec![],
                    }),
                    end: Some(BlockId {
                        height: 2,
                        hash: vec![],
                    }),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(stream.message().await.unwrap().unwrap().height, 1);
        assert_eq!(stream.message().await.unwrap().unwrap().height, 2);
    }
    assert_eq!(proxy.records.lock().unwrap().len(), 2);
    assert!(
        proxy
            .records
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.address_type == 3)
    );
    server.abort();
}

#[test]
fn policy_inspection_does_not_connect_or_require_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.toml");
    std::fs::write(
        &source,
        "spec = 'http://unreachable.invalid/openapi.json'\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treazure"))
        .args(["--show-config", "--config"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["network"]["mode"], "direct");
    let policy = dir.path().join("tor.toml");
    std::fs::write(
        &policy,
        "[network]\nmode='tor'\nsocks_endpoint='127.0.0.1:1'\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treazure"))
        .args(["--show-config", "--config"])
        .arg(&source)
        .arg("--network-config")
        .arg(&policy)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["network"]["mode"], "tor");
    assert_eq!(value["network"]["isolation_namespace"], "x402_treazury");
    assert_eq!(value["network"]["socks_auth"], "tor_extended");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treazure"))
        .args(["--meta-config", "missing.toml", "--network-config"])
        .arg(policy)
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[cfg(feature = "zcash")]
#[tokio::test]
#[ignore = "requires an explicit live Tor SOCKS endpoint; read-only, no wallet or funds"]
async fn live_tor_unfunded_smoke() {
    use zingo_netutils::Indexer;
    let address = std::env::var("TOR_SMOKE_SOCKS")
        .expect("set TOR_SMOKE_SOCKS")
        .parse()
        .unwrap();
    let ctx = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(address),
        connect_timeout_seconds: Some(60),
        ..Default::default()
    })
    .unwrap();
    let url = "https://zec.rocks/";
    for id in [
        IsolationId::treasury("smoke-one"),
        IsolationId::treasury("smoke-two"),
    ] {
        let result = ctx
            .http(&id, url, Duration::from_secs(90))
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap();
        assert!(result.status().is_success());
        result.bytes().await.unwrap();
    }
    let mut indexer = ctx
        .grpc(
            &IsolationId::treasury("smoke-grpc"),
            "https://zec.rocks:443",
        )
        .await
        .unwrap();
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
    assert!(!tree.ironwood_tree.is_empty());
}

#[tokio::test]
async fn public_only_tor_uses_remote_dns_and_existing_isolation_without_fallback() {
    let proxy = Socks::start(BTreeMap::new(), Fault::Refuse).await;
    let context = NetworkContext::new(policy(&proxy)).unwrap();
    let url = "https://api.example.com/openapi.json";
    for id in [
        IsolationId::discovery(url).unwrap(),
        IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap(),
    ] {
        let client = context
            .http_public(&id, url, Duration::from_secs(2))
            .unwrap();
        assert!(client.get(url).send().await.is_err());
        let records = proxy.records.lock().unwrap();
        let record = records.last().unwrap();
        assert_eq!(record.host, "api.example.com");
        assert_eq!(record.address_type, 3);
        assert_eq!(
            (record.user.clone(), record.password.clone()),
            context.credentials(&id)
        );
    }
    assert_eq!(proxy.records.lock().unwrap().len(), 2);
    assert!(
        context
            .http_public(
                &IsolationId::bootstrap(),
                "https://127.0.0.1/",
                Duration::from_secs(2)
            )
            .is_err()
    );
}
