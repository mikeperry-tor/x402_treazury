use super::*;
use crate::{
    network::{Mode, NetworkContext, NetworkPolicy},
    payment::{PaidClient, Payer, SpendPolicy},
    test_socks::{Fault, Socks},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[path = "../../tests/support/h2_capture.rs"]
mod wire;
#[test]
fn stream_limits_and_faults_in_isolated_process() {
    if std::env::var_os("TREAZURY_COVER_PROTOCOL_CHILD").is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(run());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cover::protocol_tests::stream_limits_and_faults_in_isolated_process",
            "--nocapture",
        ])
        .env("TREAZURY_COVER_PROTOCOL_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
async fn run() {
    let capture: wire::Capture = Arc::default();
    let seen = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let acceptor = wire::tls();
    let (cap, requests, payments) = (capture.clone(), seen.clone(), signed.clone());
    let server = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let (tls, cap, requests, payments) = (
                acceptor.clone(),
                cap.clone(),
                requests.clone(),
                payments.clone(),
            );
            children.spawn(async move{
                let io=tls.accept(tcp).await.unwrap();let index={let mut cap=cap.lock().unwrap();let index=cap.len();cap.push(vec![]);index};
                let mut connection=h2::server::Builder::new().max_concurrent_streams(1).handshake::<_,axum::body::Bytes>(wire::Tap{io,capture:cap,index}).await.unwrap();
                let mut held=Vec::new();
                while let Some(Ok((request,mut response)))=connection.accept().await {
                    let path=request.uri().path().to_owned();
                    if path=="/openapi.json" {
                        assert!(!request.headers().contains_key("payment-signature"));
                        let raw=request.headers()["range"].to_str().unwrap().strip_prefix("bytes=").unwrap();let(a,b)=raw.split_once('-').unwrap();let(a,b)=(a.parse::<usize>().unwrap(),b.parse::<usize>().unwrap());
                        let n=requests.fetch_add(1,Ordering::SeqCst);
                        let headers=axum::http::Response::builder().status(206).header("content-range",format!("bytes {a}-{b}/16384")).header("etag","\"v1\"").body(()).unwrap();
                        let Ok(mut body)=response.send_response(headers,false) else {continue;};
                        if n==0 {let _=body.send_data(vec![b'a';b-a+1].into(),true);}else{held.push(body);}
                        continue;
                    }
                    assert!(request.headers().contains_key("x-example-padding"));
                    if path=="/free" {let mut body=response.send_response(axum::http::Response::builder().header("content-type","text/plain").body(()).unwrap(),false).unwrap();body.send_data(axum::body::Bytes::from_static(b"free"),true).unwrap();continue;}
                    if let Some(value)=request.headers().get("payment-signature") {
                        let payload=serde_json::from_slice(&STANDARD.decode(value).unwrap()).unwrap();crate::test_signatures::recover_exact(&payload);
                        *payments.lock().unwrap().entry(path.clone()).or_default()+=1;
                        match path.as_str(){"/reset"=>{response.send_reset(h2::Reason::REFUSED_STREAM);continue;},"/goaway"=>{connection.abrupt_shutdown(h2::Reason::INTERNAL_ERROR);continue;},"/lost"=>break,_=>()}
                        let mut body=response.send_response(axum::http::Response::builder().header("content-type","text/plain").body(()).unwrap(),false).unwrap();body.send_data(axum::body::Bytes::from_static(b"paid"),true).unwrap();
                    } else {
                        let challenge=json!({"x402Version":2,"resource":{"url":format!("https://api.example.com{path}"),"description":"fixture","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"5000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                        response.send_response(axum::http::Response::builder().status(402).header("payment-required",STANDARD.encode(serde_json::to_vec(&challenge).unwrap())).body(()).unwrap(),true).unwrap();
                    }
                }
            });
        }
    });
    let socks = Socks::start(
        BTreeMap::from([("api.example.com".into(), addr)]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            cover_traffic_enabled: true,
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let mut cfg = tests::example_config();
    cfg.start_delay = toml::from_str("distribution='uniform'\nmin_ms=0\nmax_ms=0").unwrap();
    cfg.request_gap = cfg.start_delay.clone();
    cfg.tail = toml::from_str("distribution='uniform'\nmin_ms=1000\nmax_ms=1000").unwrap();
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
    .with_cover(
        Some(cfg.clone()),
        status::Scope {
            listener: "fixture".into(),
            source: "api".into(),
        },
    );
    let route = |path: &str| crate::catalog::RoutedRequest {
        method: "GET".into(),
        url: format!("https://api.example.com{path}"),
        query: BTreeMap::new(),
        body: None,
    };
    assert_eq!(
        client.execute_response(route("/ok")).await.unwrap().bytes,
        b"paid"
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while seen.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // One server stream slot is held indefinitely by cover: a new API must preempt it.
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(2),
            client.execute_response(route("/ok"))
        )
        .await
        .expect("cover blocked a real request at stream limit=1")
        .unwrap()
        .bytes,
        b"paid"
    );
    for path in ["/reset", "/goaway", "/lost"] {
        assert!(
            tokio::time::timeout(Duration::from_secs(3), client.execute_response(route(path)))
                .await
                .unwrap()
                .is_err()
        );
        assert_eq!(
            signed.lock().unwrap().get(path),
            Some(&1),
            "paid request replayed after {path}"
        );
    }
    let second = PaidClient::new(
        Payer::new(&format!("{:064x}", 2), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
    .with_cover(
        Some(cfg.clone()),
        status::Scope {
            listener: "second".into(),
            source: "api".into(),
        },
    );
    assert_eq!(
        second.execute_response(route("/ok")).await.unwrap().bytes,
        b"paid"
    );
    let discovery = PaidClient::unsigned().with_cover(
        Some(cfg),
        status::Scope {
            listener: "unsigned".into(),
            source: "api".into(),
        },
    );
    assert_eq!(
        discovery
            .execute_response(route("/free"))
            .await
            .unwrap()
            .bytes,
        b"free"
    );
    let records = socks.records.lock().unwrap().clone();
    assert!(records.iter().all(|r| r.address_type == 3));
    let tokens = records
        .iter()
        .map(|r| &r.password)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        tokens.len(),
        3,
        "wallet and discovery identities must never share a SOCKS identity"
    );
    crate::network::global()
        .cover
        .as_ref()
        .unwrap()
        .shutdown()
        .await;
    // Parse actual plaintext HEADERS on every TLS connection, including fresh pools.
    let captures = capture.lock().unwrap();
    let mut sensitive = Vec::new();
    for bytes in captures.iter() {
        for block in wire::blocks(bytes) {
            sensitive.extend(wire::never_indexed(&block));
        }
    }
    assert!(sensitive.len() >= 10);
    for (name_huffman, name, _, len) in &sensitive {
        assert!(*name_huffman);
        assert_eq!(
            name,
            &[
                0xf2, 0xb1, 0x7c, 0x8e, 0x9a, 0xe8, 0x2a, 0xd5, 0x8e, 0x49, 0x0d, 0x54, 0xdf
            ],
            "RFC7541 Huffman encoding of x-example-padding"
        );
        assert!(*len > 0);
    }
    assert!(
        sensitive.windows(2).any(|p| p[0].3 != p[1].3),
        "padding must vary encoded lengths"
    );
    drop(captures);
    server.abort();
}

#[tokio::test]
async fn http1_compatibility_keeps_real_calls_and_reports_cover_unavailable() {
    let app = axum::Router::new().fallback(|| async { "ordinary API" });
    let (address, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"http/1.1"]).await;
    let socks = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    let ctx = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address),
        cover_traffic_enabled: true,
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    let identity = crate::network::IsolationId::discovery("https://api.example.com").unwrap();
    let transport = crate::network::HttpPolicy {
        allow_http1: true,
        allow_tls12: false,
    };
    let http = ctx
        .http_policy(
            &identity,
            "https://api.example.com",
            Duration::from_secs(2),
            false,
            transport,
        )
        .unwrap();
    let engine = ctx.cover.as_ref().unwrap();
    let scope = status::Scope {
        listener: "h1".into(),
        source: "api".into(),
    };
    engine.register(scope.clone());
    let owner = registry::Owner {
        runtime: tokio::runtime::Handle::current().id(),
        identity,
        origin: "https://api.example.com".into(),
        transport,
        public_only: false,
        timeout_ms: 240000,
    };
    let mut call = engine
        .begin(
            owner,
            Arc::new(tests::example_config()),
            scope.clone(),
            http.clone(),
        )
        .unwrap();
    call.prioritize().await;
    let mut request = reqwest::Request::new(
        reqwest::Method::GET,
        "https://api.example.com/api".parse().unwrap(),
    );
    assert!(call.pad(&mut request, true).is_none());
    assert!(!request.headers().contains_key("x-example-padding"));
    let response = http.execute(request).await.unwrap();
    assert_eq!(response.version(), reqwest::Version::HTTP_11);
    call.protocol(response.version());
    call.response_headers();
    assert_eq!(response.text().await.unwrap(), "ordinary API");
    assert!(call.advisory().unwrap().contains("cover_http2_unavailable"));
    call.complete();
    drop(call);
    engine.shutdown().await;
    assert!(
        engine
            .status(&[scope])
            .to_string()
            .contains("cover_http2_unavailable")
    );
    server.abort();
}
