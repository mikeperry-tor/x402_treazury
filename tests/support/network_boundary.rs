//! Tests inside network.rs so private resolver/trust seams cannot become config.
use super::*;
use crate::test_socks::{Fault, Socks};
use reqwest::dns::Resolve;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::atomic::{AtomicUsize, Ordering},
};
struct Answers(Mutex<VecDeque<Option<Vec<SocketAddr>>>>);
impl Resolve for Answers {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let next = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected DNS lookup");
        Box::pin(async move {
            let addresses = next.ok_or_else(|| std::io::Error::other("fixture DNS failure"))?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
fn answers(items: Vec<Option<Vec<SocketAddr>>>) -> Arc<Answers> {
    Arc::new(Answers(Mutex::new(items.into())))
}
#[tokio::test]
async fn public_resolver_preserves_public_answers_and_rejects_each_rebinding_result() {
    let good = "1.1.1.1:443".parse().unwrap();
    let good6 = "[2606:4700:4700::1111]:443".parse().unwrap();
    let bad = "127.0.0.1:443".parse().unwrap();
    let bad6 = "[::1]:443".parse().unwrap();
    let lookup = answers(vec![
        Some(vec![good, good6]),
        Some(vec![good, bad]),
        Some(vec![good6, bad6]),
        Some(vec![bad]),
        Some(vec![]),
        None,
    ]);
    let resolver = PublicResolver { lookup };
    let name = || "api.example.com".parse::<reqwest::dns::Name>().unwrap();
    assert_eq!(
        resolver.resolve(name()).await.unwrap().collect::<Vec<_>>(),
        vec![good, good6]
    );
    for _ in 0..5 {
        assert!(resolver.resolve(name()).await.is_err());
    }
    // The real system lookup, using the OS localhost mapping, also passes through
    // the validator. This requires no public DNS or remote connection.
    assert!(
        PublicResolver::default()
            .resolve("localhost".parse().unwrap())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn direct_public_client_uses_guarded_resolver_and_separate_cache_entries() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let (address, server) = crate::test_tls::serve(axum::Router::new().fallback(move || {
        let h = h.clone();
        async move {
            h.fetch_add(1, Ordering::SeqCst);
            "unexpected"
        }
    }))
    .await;
    let mut ctx = NetworkContext::new(NetworkPolicy::default())
        .unwrap()
        .with_test_root(crate::test_tls::CA);
    let dns = answers(vec![
        Some(vec![address]),
        Some(vec!["1.1.1.1:443".parse().unwrap(), address]),
    ]);
    ctx.test_dns = Some(dns.clone());
    let url = "https://api.example.com/";
    let id = IsolationId::discovery(url).unwrap();
    let _trusted = ctx.http(&id, url, Duration::from_secs(2)).unwrap();
    let public = ctx.http_public(&id, url, Duration::from_secs(2)).unwrap();
    assert_eq!(
        ctx.http.lock().unwrap().len(),
        2,
        "public guard is part of cache identity"
    );
    for _ in 0..2 {
        assert!(public.get(url).send().await.is_err());
    }
    assert!(dns.0.lock().unwrap().is_empty());
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    server.abort();
}
#[tokio::test]
async fn http_and_grpc_keep_tls_verification_through_socks() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let (address, server) = crate::test_tls::serve(axum::Router::new().fallback(move || {
        let h = h.clone();
        async move {
            h.fetch_add(1, Ordering::SeqCst);
            (
                [("content-type", "application/grpc"), ("grpc-status", "0")],
                vec![0u8, 0, 0, 0, 2, 16, 1],
            )
        }
    }))
    .await;
    let proxy = Socks::start(
        BTreeMap::from([
            ("grpc.example.com".into(), address),
            ("wrong.example.com".into(), address),
        ]),
        Fault::None,
    )
    .await;
    let policy = NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(proxy.address),
        connect_timeout_seconds: Some(1),
        ..Default::default()
    };
    let identity = IsolationId::treasury("fixture");
    for (trust, url) in [
        (false, "https://grpc.example.com"),
        (true, "https://wrong.example.com"),
    ] {
        let mut ctx = NetworkContext::new(policy.clone()).unwrap();
        if trust {
            ctx = ctx.with_test_root(crate::test_tls::CA);
        }
        assert!(
            ctx.http(&identity, url, Duration::from_secs(2))
                .unwrap()
                .get(url)
                .send()
                .await
                .is_err()
        );
        #[cfg(feature = "zcash")]
        assert!(ctx.grpc(&identity, url).await.is_err());
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "failed TLS reached an application handler"
    );
    let ctx = NetworkContext::new(policy)
        .unwrap()
        .with_test_root(crate::test_tls::CA);
    assert!(
        ctx.http(
            &identity,
            "https://grpc.example.com",
            Duration::from_secs(2)
        )
        .unwrap()
        .get("https://grpc.example.com")
        .send()
        .await
        .unwrap()
        .status()
        .is_success()
    );
    #[cfg(feature = "zcash")]
    {
        use zingo_netutils::{
            Indexer,
            lightwallet_protocol::{BlockId, BlockRange},
        };
        let mut client = ctx
            .grpc(&identity, "https://grpc.example.com")
            .await
            .unwrap();
        let mut stream = client
            .get_block_range(
                BlockRange {
                    pool_types: vec![],
                    start: Some(BlockId {
                        height: 1,
                        hash: vec![],
                    }),
                    end: Some(BlockId {
                        height: 1,
                        hash: vec![],
                    }),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(stream.message().await.unwrap().unwrap().height, 1);
    }
    assert!(
        hits.load(Ordering::SeqCst) > 0,
        "positive TLS control never reached server"
    );
    assert!(
        proxy
            .records
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.address_type == 3
                && (r.user.clone(), r.password.clone()) == ctx.credentials(&identity))
    );
    server.abort();
}

#[tokio::test]
async fn socks_failures_never_reach_a_direct_http_or_grpc_target() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().fallback(move || {
                let h = h.clone();
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    "ok"
                }
            }),
        )
        .await
        .unwrap();
    });
    let url = format!("http://{address}/");
    let id = IsolationId::treasury("no-fallback");
    for fault in [Fault::NoAuth, Fault::BadAuth, Fault::Refuse, Fault::Stall] {
        let proxy = Socks::start(BTreeMap::new(), fault).await;
        let ctx = NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            connect_timeout_seconds: Some(1),
            ..Default::default()
        })
        .unwrap();
        assert!(
            ctx.http(&id, &url, Duration::from_secs(2))
                .unwrap()
                .get(&url)
                .send()
                .await
                .is_err()
        );
        #[cfg(feature = "zcash")]
        assert!(ctx.grpc(&id, &url).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }
    let direct = NetworkContext::new(NetworkPolicy::default()).unwrap();
    assert_eq!(
        direct
            .http(&id, &url, Duration::from_secs(2))
            .unwrap()
            .get(&url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "direct control must be reachable"
    );
    server.abort();
}

#[tokio::test]
async fn interrupted_http_body_is_not_retried() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let c = connections.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            c.fetch_add(1, Ordering::SeqCst);
            let mut request = [0u8; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial",
                )
                .await
                .unwrap();
        }
    });
    let proxy = Socks::start(
        BTreeMap::from([("body.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    let ctx = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(proxy.address),
        ..Default::default()
    })
    .unwrap();
    let url = "http://body.example.com/";
    let response = ctx
        .discovery(url, Duration::from_secs(2))
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();
    assert!(response.text().await.is_err());
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    assert_eq!(proxy.records.lock().unwrap().len(), 1);
    server.abort();
}
