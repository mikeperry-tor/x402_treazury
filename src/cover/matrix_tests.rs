//! Opt-in, deterministic localhost MCP comparison matrix; no live provider traffic.
use super::*;
use crate::{
    network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
    payment::{PaidClient, Payer, SpendPolicy},
    test_socks::{Fault, Socks},
};
use axum::{
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};
#[test]
#[ignore = "opt-in offline cover distribution/profile experiment; writes target/cover/matrix.json"]
fn cover_distribution_matrix() {
    if std::env::var_os("TREAZURY_COVER_MATRIX_CHILD").is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(run());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cover::matrix_tests::cover_distribution_matrix",
            "--ignored",
            "--nocapture",
        ])
        .env("TREAZURY_COVER_MATRIX_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn dist(family: &str, unit: &str, min: u64, max: u64) -> sampling::Distribution {
    let mut v = json!({"distribution":family});
    v[format!("min_{unit}")] = json!(min);
    v[format!("max_{unit}")] = json!(max);
    let mid = (min + max) / 2;
    match family {
        "exponential" => v[format!("mean_{unit}")] = json!(mid as f64),
        "weibull" => {
            v[format!("scale_{unit}")] = json!(mid as f64);
            v["shape"] = json!(0.8);
        }
        "log_normal" => {
            v[format!("median_{unit}")] = json!(mid as f64);
            v["sigma"] = json!(0.65);
        }
        "weighted_discrete" => {
            v[format!("values_{unit}")] = json!([min, mid, max]);
            v["weights"] = json!([1, 2, 1]);
        }
        _ => (),
    };
    serde_json::from_value(v).unwrap()
}
async fn run() {
    let app = axum::Router::new()
        .route(
            "/api/{size}",
            get(
                |axum::extract::Path(size): axum::extract::Path<usize>| async move {
                    let stream = futures_util::stream::once(async move {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![b'a'; size]))
                    });
                    (
                        [("content-type", "text/plain")],
                        axum::body::Body::from_stream(stream),
                    )
                },
            ),
        )
        .route(
            "/openapi.json",
            get(|h: HeaderMap| async move {
                assert!(!h.contains_key("payment-signature"));
                let raw = h["range"].to_str().unwrap().strip_prefix("bytes=").unwrap();
                let (a, b) = raw.split_once('-').unwrap();
                let (a, b) = (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap());
                tokio::time::sleep(Duration::from_millis(2)).await;
                (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        ("content-range", format!("bytes {a}-{b}/16384")),
                        ("etag", "\"v1\"".into()),
                    ],
                    vec![b'b'; b - a + 1],
                )
            }),
        );
    let (address, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            cover_traffic_enabled: true,
            cover_limits: Limits {
                max_requests_per_window: 4096,
                max_cover_body_bytes_per_window: 16_777_216,
                max_padding_value_bytes_per_window: 1_048_576,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let engine = crate::network::global().cover.as_ref().unwrap();
    let local = NetworkContext::new(NetworkPolicy::default()).unwrap();
    let mut samples = vec![];
    let mut index = 0u64;
    for profile in ["none", "ranges", "padding", "combined"] {
        for family in [
            "uniform",
            "exponential",
            "weibull",
            "log_normal",
            "weighted_discrete",
        ] {
            for width in 1..=3 {
                for repeat in 0..2 {
                    index += 1;
                    engine.reset_experiment(index);
                    let size = [0, 32, 8192][(index % 3) as usize];
                    let mut cfg = tests::example_config();
                    cfg.concurrency = width;
                    cfg.ranges_enabled = profile != "padding";
                    cfg.volume = dist(family, "bytes", 2048, 4096);
                    cfg.ranges = dist(family, "bytes", 512, 1024);
                    cfg.start_delay = dist(family, "ms", 0, 2);
                    cfg.request_gap = dist(family, "ms", 0, 2);
                    cfg.tail =
                        toml::from_str("distribution='uniform'\nmin_ms=50\nmax_ms=50").unwrap();
                    cfg.padding.as_mut().unwrap().size = dist(family, "bytes", 32, 512);
                    if profile == "ranges" {
                        cfg.padding = None;
                    }
                    cfg.validate().unwrap();
                    let client = PaidClient::new(
                        Payer::new(&format!("{index:064x}"), SpendPolicy::dollars("1").unwrap())
                            .unwrap(),
                    )
                    .with_cover(
                        (profile != "none").then_some(cfg),
                        status::Scope {
                            listener: "matrix".into(),
                            source: format!("case_{index}"),
                        },
                    );
                    let tool:crate::catalog::ToolSpec=serde_json::from_value(json!({"name":"read","description":"fixture","method":"GET","path":format!("/api/{size}"),"input_schema":{"type":"object","properties":{}},"param_routes":{},"has_body":false})).unwrap();
                    let mcp = crate::server::Server::new(
                        vec![tool],
                        client,
                        "https://api.example.com".into(),
                        None,
                        None,
                    );
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
                    let task = tokio::spawn(async move {
                        axum::serve(listener, crate::server::http_app(mcp, "fixture".into()))
                            .await
                            .unwrap();
                    });
                    let http = local
                        .http(
                            &IsolationId::discovery(&endpoint).unwrap(),
                            &endpoint,
                            Duration::from_secs(3),
                        )
                        .unwrap();
                    let start = tokio::time::Instant::now();
                    let result:serde_json::Value=http.post(&endpoint).bearer_auth("fixture").header("accept","application/json, text/event-stream").json(&json!({"jsonrpc":"2.0","id":index,"method":"tools/call","params":{"name":"read","arguments":{}}})).send().await.unwrap().json().await.unwrap();
                    let elapsed = start.elapsed().as_micros();
                    assert_eq!(result["result"]["isError"], false, "{result}");
                    assert_eq!(
                        result["result"]["content"][0]["text"]
                            .as_str()
                            .unwrap()
                            .len(),
                        size
                    );
                    engine.wait_experiment_idle().await;
                    let metrics = engine.metrics();
                    assert_eq!(metrics.in_flight_ranges, 0);
                    assert!(metrics.peak_in_flight_ranges <= width as u64);
                    if matches!(profile, "ranges" | "combined") {
                        assert!(metrics.qualified_ranges > 0, "{profile}/{family}/{width}");
                    } else {
                        assert_eq!(metrics.range_requests, 0);
                    }
                    if matches!(profile, "padding" | "combined") {
                        assert_eq!(metrics.padding_requests, 1);
                    } else {
                        assert_eq!(metrics.padding_requests, 0);
                    }
                    samples.push(json!({"profile":profile,"distribution":family,"concurrency":width,"repeat":repeat,"test_seed":index,"response_bytes":size,"api_micros":elapsed,"metrics":metrics}));
                    task.abort();
                    let _ = task.await;
                }
            }
        }
    }
    engine.shutdown().await;
    server.abort();
    std::fs::create_dir_all("target/cover").unwrap();
    std::fs::write("target/cover/matrix.json",serde_json::to_vec_pretty(&json!({"version":1,"network":"localhost TLS-over-SOCKS fixture; not real Tor","samples":samples,"privacy_qualification":false})).unwrap()).unwrap();
}
