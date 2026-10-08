//! Padding failure must not replay a signed request or disable independent ranges.
use super::*;
use crate::{
    catalog::RoutedRequest,
    network::{Mode, NetworkContext, NetworkPolicy},
    payment::{PaidClient, Payer, SpendPolicy},
    test_socks::{Fault, Socks},
};
use axum::{
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[test]
fn rejected_padding_and_ranges_are_independent_without_payment_replay() {
    if std::env::var_os("TREAZURY_COVER_REJECTION_CHILD").is_some() {
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .init();
        tokio::runtime::Runtime::new().unwrap().block_on(run());
        return;
    }
    let out=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","cover::rejection_tests::rejected_padding_and_ranges_are_independent_without_payment_replay","--nocapture"]).env("TREAZURY_COVER_REJECTION_CHILD","1").output().unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let logs = String::from_utf8_lossy(&out.stderr);
    for code in [
        "cover_padding_rejected",
        "cover_forbidden",
        "cover_fallback_selected",
    ] {
        assert!(logs.contains(code), "missing {code}: {logs}");
    }
}
async fn run() {
    let signed = Arc::new(AtomicUsize::new(0));
    let refused = Arc::new(AtomicUsize::new(0));
    let padded = Arc::new(AtomicUsize::new(0));
    let fallback_reads = Arc::new(AtomicUsize::new(0));
    let f = fallback_reads.clone();
    let (s, r, p) = (signed.clone(), refused.clone(), padded.clone());
    let waited = refused.clone();
    let app=axum::Router::new()
        .route("/llms.txt",get(move |h:HeaderMap| {let f=f.clone();async move {
            assert!(!h.contains_key("payment-signature") && !h.contains_key("x-payment"));
            f.fetch_add(1,Ordering::SeqCst);
            let range=h["range"].to_str().unwrap().strip_prefix("bytes=").unwrap();
            let (start,end)=range.split_once('-').unwrap();
            let start:usize=start.parse().unwrap();let end:usize=end.parse().unwrap();
            (StatusCode::PARTIAL_CONTENT,[("content-range",format!("bytes {start}-{end}/16384")),("etag","\"fixture\"".into())],vec![b'x';end-start+1])
        }}))
        .route("/refused",get(move || {let r=r.clone();async move {r.fetch_add(1,Ordering::SeqCst);StatusCode::FORBIDDEN}}))
        .route("/paid",get(move |h:HeaderMap|{let s=s.clone();async move{
            if let Some(value)=h.get("payment-signature"){
                let payload:serde_json::Value=serde_json::from_slice(&STANDARD.decode(value.as_bytes()).unwrap()).unwrap();crate::test_signatures::recover_exact(&payload);s.fetch_add(1,Ordering::SeqCst);
                return if h.contains_key("x-example-padding"){StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE.into_response()}else{(StatusCode::OK,"accepted").into_response()};
            }
            let challenge=json!({"x402Version":2,"resource":{"url":"https://api.example.com/paid","description":"fixture","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"5000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
        }}))
        .route("/free",get(move |h:HeaderMap|{let p=p.clone();let waited=waited.clone();async move{
            assert!(!h.contains_key("payment-signature"));
            if h.contains_key("x-example-padding"){p.fetch_add(1,Ordering::SeqCst);}
            let body=axum::body::Body::from_stream(futures_util::stream::once(async move{
                while waited.load(Ordering::SeqCst)==0 {tokio::task::yield_now().await;}
                Ok::<_,std::io::Error>(axum::body::Bytes::from_static(b"free"))
            }));
            ([("content-type","text/plain")],body)
        }}));
    let (addr, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = Socks::start(
        BTreeMap::from([("api.example.com".into(), addr)]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            cover_traffic_enabled: Some(true),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let scope = Scope {
        listener: "main".into(),
        source: "api".into(),
    };
    let client = |n: u64, cfg| {
        PaidClient::new(
            Payer::new(&format!("{n:064x}"), SpendPolicy::dollars("1").unwrap()).unwrap(),
        )
        .with_cover(Some(cfg), scope.clone())
    };
    let route = |path: &str| RoutedRequest {
        method: "GET".into(),
        url: format!("https://api.example.com{path}"),
        query: Default::default(),
        body: None,
    };
    let mut cfg = tests::example_config();
    cfg.ranges_enabled = false;
    let paid = client(10, cfg);
    assert!(paid.execute_response(route("/paid")).await.is_err());
    assert_eq!(
        signed.load(Ordering::SeqCst),
        1,
        "431 must not replay signed payment"
    );
    assert_eq!(
        paid.execute_response(route("/paid")).await.unwrap().bytes,
        b"accepted"
    );
    assert_eq!(
        signed.load(Ordering::SeqCst),
        2,
        "new independent call may pay without disabled padding"
    );
    let mut cfg = tests::example_config();
    cfg.url = "https://api.example.com/refused".into();
    cfg.start_delay = toml::from_str("distribution='uniform'\nmin_ms=0\nmax_ms=0").unwrap();
    let free = client(11, cfg);
    for _ in 0..2 {
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            free.execute_response(route("/free")),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.bytes, b"free");
        assert!(result.advisories.is_empty());
        crate::network::global()
            .cover
            .as_ref()
            .unwrap()
            .wait_experiment_idle()
            .await;
    }
    assert_eq!(
        refused.load(Ordering::SeqCst),
        1,
        "negative range capability is cached"
    );
    assert_eq!(
        padded.load(Ordering::SeqCst),
        2,
        "range refusal must leave API padding available"
    );
    let mut cfg = tests::example_config();
    cfg.url = "https://api.example.com/refused".into();
    cfg.fallback_url = Some("https://api.example.com/llms.txt".into());
    cfg.start_delay = toml::from_str("distribution='uniform'\nmin_ms=0\nmax_ms=0").unwrap();
    cfg.tail = toml::from_str("distribution='uniform'\nmin_ms=1000\nmax_ms=1000").unwrap();
    cfg.volume = toml::from_str("distribution='uniform'\nmin_bytes=1024\nmax_bytes=1024").unwrap();
    cfg.qualification_range_bytes = 1024;
    cfg.ranges = toml::from_str("distribution='uniform'\nmin_bytes=1024\nmax_bytes=1024").unwrap();
    cfg.padding = None;
    let fallback_client = client(12, cfg.clone());
    for _ in 0..2 {
        assert_eq!(
            fallback_client
                .execute_response(route("/free"))
                .await
                .unwrap()
                .bytes,
            b"free"
        );
        crate::network::global()
            .cover
            .as_ref()
            .unwrap()
            .wait_experiment_idle()
            .await;
        tokio::time::sleep(Duration::from_millis(1100)).await; // Start a new episode after its fixed tail.
    }
    assert_eq!(
        refused.load(Ordering::SeqCst),
        2,
        "primary tried once for fallback owner"
    );
    assert_eq!(
        fallback_reads.load(Ordering::SeqCst),
        2,
        "successful fallback retained across episodes"
    );
    cfg.fallback_url = Some("https://api.example.com/refused?fallback=true".into());
    let failed_client = client(13, cfg.clone());
    for _ in 0..2 {
        assert_eq!(
            failed_client
                .execute_response(route("/free"))
                .await
                .unwrap()
                .bytes,
            b"free"
        );
        crate::network::global()
            .cover
            .as_ref()
            .unwrap()
            .wait_experiment_idle()
            .await;
        tokio::time::sleep(Duration::from_millis(1100)).await; // Start a new episode after its fixed tail.
    }
    assert_eq!(
        refused.load(Ordering::SeqCst),
        4,
        "failed primary and fallback are both cached; no retry loop"
    );
    cfg.fallback_url = Some("https://api.example.com/llms.txt".into());
    cfg.max_requests_per_episode = 1;
    let bounded = client(14, cfg);
    assert_eq!(
        bounded
            .execute_response(route("/free"))
            .await
            .unwrap()
            .bytes,
        b"free"
    );
    crate::network::global()
        .cover
        .as_ref()
        .unwrap()
        .wait_experiment_idle()
        .await;
    assert_eq!(refused.load(Ordering::SeqCst), 5);
    assert_eq!(
        fallback_reads.load(Ordering::SeqCst),
        2,
        "fallback cannot exceed episode request budget"
    );
    let engine = crate::network::global().cover.as_ref().unwrap();
    engine.shutdown().await;
    assert_eq!(engine.metrics().in_flight_ranges, 0);
    server.abort();
}
