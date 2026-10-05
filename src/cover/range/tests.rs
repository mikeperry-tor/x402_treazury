use super::*;
use crate::{
    cover::{Limits, budget::Budget},
    network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
    test_socks::{Fault, Socks},
};
use axum::{http::HeaderMap, response::IntoResponse};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[tokio::test]
async fn unsigned_ranges_validate_every_response_and_never_follow_or_pay() {
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let app = axum::Router::new().fallback(move |uri: axum::http::Uri, headers: HeaderMap| {
        let calls = calls.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(headers["range"], "bytes=0-1023");
            assert_eq!(headers["accept-encoding"], "identity");
            for name in ["payment-signature", "x-payment", "authorization", "cookie"] {
                assert!(!headers.contains_key(name));
            }
            if uri.path() == "/chunkshort" || uri.path() == "/chunklarge" {
                let len = if uri.path() == "/chunkshort" {
                    1000
                } else {
                    2048
                };
                let chunks = futures_util::stream::iter([Ok::<_, std::io::Error>(
                    axum::body::Bytes::from(vec![b'a'; len]),
                )]);
                return (
                    axum::http::StatusCode::PARTIAL_CONTENT,
                    [("content-range", "bytes 0-1023/4096")],
                    axum::body::Body::from_stream(chunks),
                )
                    .into_response();
            }
            if uri.path() == "/slow" {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            let mut status = 206;
            let mut range = "bytes 0-1023/4096";
            let mut encoding = "identity";
            let mut body = vec![b'a'; 1024];
            let mut etag = "\"v1\"";
            match uri.path() {
                "/ignored" => {
                    status = 200;
                    body = vec![b'a'; 100000];
                }
                "/redirect" => status = 302,
                "/payment" => status = 402,
                "/auth" => status = 401,
                "/forbidden" => status = 403,
                "/limited" => status = 429,
                "/unsatisfiable" => status = 416,
                "/compressed" => encoding = "gzip",
                "/wrong" => range = "bytes 1-1024/4096",
                "/unknown" => range = "bytes 0-1023/*",
                "/short" => {
                    body.pop();
                }
                "/oversize" => body.push(b'a'),
                "/changed" => etag = "\"v2\"",
                _ => (),
            }
            (
                axum::http::StatusCode::from_u16(status).unwrap(),
                [
                    ("content-range", range),
                    ("content-encoding", encoding),
                    ("etag", etag),
                    ("location", "https://evil.example.com/redirect"),
                ],
                body,
            )
                .into_response()
        }
    });
    let (address, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    let ctx = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    let http = ctx
        .http_policy(
            &IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap(),
            "https://api.example.com",
            Duration::from_secs(3),
            false,
            Default::default(),
        )
        .unwrap();
    let cases = [
        ("ok", None),
        ("chunkshort", Some("cover_body_truncated")),
        ("chunklarge", Some("cover_body_overflow")),
        ("slow", Some("cover_deadline")),
        ("ignored", Some("cover_range_ignored")),
        ("redirect", Some("cover_redirect_refused")),
        ("payment", Some("cover_payment_refused")),
        ("auth", Some("cover_auth_required")),
        ("forbidden", Some("cover_forbidden")),
        ("limited", Some("cover_rate_limited")),
        ("unsatisfiable", Some("cover_range_unsatisfiable")),
        ("compressed", Some("cover_compression_refused")),
        ("wrong", Some("cover_content_range_mismatch")),
        ("unknown", Some("cover_content_range_invalid")),
        ("short", Some("cover_content_length_mismatch")),
        ("oversize", Some("cover_content_length_mismatch")),
        ("changed", Some("cover_representation_changed")),
    ];
    for (path, error) in cases {
        let mut config = crate::cover::tests::example_config();
        config.url = format!("https://api.example.com/{path}");
        let b = Budget::new(Limits::default());
        let reservation = b.reserve(Instant::now(), 1024, 0, true).unwrap();
        let mut range = Range::qualification(&config);
        if path == "changed" {
            range.expected = Some(Representation {
                length: 4096,
                validator: Some(("etag".into(), "\"v1\"".into())),
            });
        }
        let result = download(
            &http,
            &config,
            range,
            Instant::now()
                + if path == "slow" {
                    Duration::from_millis(50)
                } else {
                    Duration::from_secs(3)
                },
            reservation,
            None,
        )
        .await;
        assert_eq!(result.as_ref().err().map(|e| e.0), error, "{path}");
    }
    assert_eq!(count.load(Ordering::SeqCst), cases.len());
    assert!(
        socks
            .records
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.address_type == 3)
    );
    server.abort();
}
