//! Shared HTTP representation boundaries, including real runtime additions.
use super::*;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::json;
use std::{collections::BTreeMap, sync::Mutex, time::Duration};

fn full_headers() -> BTreeMap<String, String> {
    let mut represented = HeaderMap::new();
    let mut headers = BTreeMap::new();
    for index in 0.. {
        let name = format!("x-field-{index}");
        if represented
            .try_insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_static("private-header-value"),
            )
            .is_err()
        {
            return headers;
        }
        headers.insert(name, "private-header-value".into());
    }
    unreachable!()
}

fn route(headers: BTreeMap<String, String>) -> RoutedRequest {
    RoutedRequest {
        method: "POST".into(),
        url: "https://api.example.com/orders".into(),
        headers,
        query: Default::default(),
        body: Some(json!({"order":1})),
    }
}

fn request(mut route: RoutedRequest, manual: bool) -> Result<reqwest::Request> {
    let headers = manual.then(|| serde_json::to_value(std::mem::take(&mut route.headers)).unwrap());
    let mut request = build_request(route)?;
    crate::discovery::raw::ManualRawHeaders::parse(headers.as_ref())?.apply(&mut request)?;
    reserve_runtime_headers(&mut request, Some("x-example-padding"))?;
    Ok(request)
}

#[test]
fn routed_header_and_body_overflow_returns_value_free_errors() {
    let mut headers = full_headers();
    // A full business map also needs a JSON Content-Type slot.
    for body in [None, Some(json!({"order":1}))] {
        if body.is_none() {
            headers.insert("x-one-too-many".into(), "private-header-value".into());
        } else {
            headers.remove("x-one-too-many");
        }
        let error = build_request(RoutedRequest {
            body,
            ..route(headers.clone())
        })
        .unwrap_err();
        assert!(error.to_string().contains("representation capacity"));
        assert!(!format!("{error:#?}").contains("private-header-value"));
    }
}

#[test]
fn manual_and_routed_headers_leave_room_for_every_runtime_slot() {
    let headers = full_headers();
    let maximum = headers.len();
    // JSON content type plus accept, host, length, signature and padding.
    let near: BTreeMap<_, _> = headers.into_iter().take(maximum - 6).collect();
    for manual in [false, true] {
        let mut request = request(route(near.clone()), manual).unwrap();
        for name in [
            "accept",
            "host",
            "content-length",
            "payment-signature",
            "x-example-padding",
        ] {
            request
                .headers_mut()
                .try_insert(
                    HeaderName::from_static(name),
                    HeaderValue::from_static("runtime"),
                )
                .unwrap();
        }
        assert_eq!(request.headers().len(), maximum);
        let mut oversized = near.clone();
        oversized.insert("x-one-too-many".into(), "private-header-value".into());
        let error = self::request(route(oversized), manual).unwrap_err();
        assert!(!format!("{error:#?}").contains("private-header-value"));
    }
}

#[test]
fn near_capacity_signing_and_optional_padding_in_isolated_process() {
    const CHILD: &str = "TREAZURY_HEADER_CAPACITY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(wire_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "payment::header_capacity_tests::near_capacity_signing_and_optional_padding_in_isolated_process", "--nocapture"])
        .env(CHILD, "1").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn wire_fixture() {
    let mut headers = full_headers();
    for _ in 0..6 {
        headers.pop_last();
    }
    let headers = Arc::new(headers);
    // Only the synthetic server raises its incoming header-list allowance.
    let header_bytes: usize = headers.iter().map(|(k, v)| k.len() + v.len() + 32).sum();
    let server_header_bytes = u32::try_from(header_bytes + 8192).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn({
        let headers = headers.clone();
        let calls = calls.clone();
        async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                let headers = headers.clone();
                let calls = calls.clone();
                children.spawn(async move {
                    let tls = crate::test_h2_capture::tls().accept(tcp).await.unwrap();
                    let mut connection = h2::server::Builder::new()
                        .max_header_list_size(server_header_bytes)
                        .handshake::<_, axum::body::Bytes>(tls)
                        .await
                        .unwrap();
                    while let Some(request) = connection.accept().await {
                        let (request, response) = request.unwrap();
                        let headers = headers.clone();
                        let calls = calls.clone();
                        tokio::spawn(async move {
                            exchange(request, response, &headers, &calls).await;
                        });
                    }
                });
            }
        }
    });
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        crate::network::NetworkContext::new(crate::network::NetworkPolicy {
            mode: crate::network::Mode::Tor,
            socks_endpoint: Some(socks.address),
            tor_isolation_retries: Some(0),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let table: toml::Table = toml::from_str(
        include_str!("../../docs/cover-traffic.md")
            .split("```toml\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap(),
    )
    .unwrap();
    let mut cover: crate::cover::Config = table["cover_traffic"].clone().try_into().unwrap();
    cover.ranges_enabled = false;
    cover.validate().unwrap();
    let client = client_for_capacity().with_cover(
        Some(cover),
        crate::cover::Scope {
            listener: "capacity".into(),
            source: "fixture".into(),
        },
    );
    for manual in [false, true] {
        for near in [true, false] {
            let mut route = route(if near {
                (*headers).clone()
            } else {
                BTreeMap::from([("x-label".into(), "small".into())])
            });
            route.url = format!(
                "https://api.example.com/{}-{}",
                if manual { "manual" } else { "routed" },
                if near { "near" } else { "small" }
            );
            let url = route.url.clone();
            let result = tokio::time::timeout(Duration::from_secs(20), async {
                if manual {
                    let values = serde_json::to_value(std::mem::take(&mut route.headers)).unwrap();
                    client
                        .execute_manual_raw(
                            route,
                            crate::discovery::raw::ManualRawHeaders::parse(Some(&values)).unwrap(),
                        )
                        .await
                        .map(|o| o.text)
                } else {
                    client.execute(route).await
                }
            })
            .await
            .expect("near-capacity request stalled")
            .unwrap();
            assert_eq!(result, "paid");
            let calls = calls.lock().unwrap();
            let calls: Vec<_> = calls.iter().filter(|(seen, _)| seen == &url).collect();
            assert_eq!(calls.len(), 2);
            assert!(!calls[0].1 && calls[1].1);
        }
    }
    client.drain().await;
    server.abort();
}

fn client_for_capacity() -> PaidClient {
    PaidClient::new(
        Payer::new(&format!("{:064x}", 199), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
}

async fn exchange(
    request: axum::http::Request<h2::RecvStream>,
    mut response: h2::server::SendResponse<axum::body::Bytes>,
    expected: &BTreeMap<String, String>,
    calls: &Mutex<Vec<(String, bool)>>,
) {
    let (parts, mut body) = request.into_parts();
    let mut bytes = Vec::new();
    while let Some(data) = body.data().await {
        let data = data.unwrap();
        body.flow_control().release_capacity(data.len()).unwrap();
        bytes.extend_from_slice(&data);
    }
    assert_eq!(bytes, br#"{"order":1}"#);
    let near = parts.uri.path().ends_with("near");
    let headers = &parts.headers;
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["content-length"], bytes.len().to_string());
    assert!(headers.contains_key("accept"));
    let signed = headers.contains_key("payment-signature");
    assert_eq!(headers.contains_key("x-example-padding"), !near);
    let business = if near {
        for (name, value) in expected {
            assert_eq!(headers[name], value);
        }
        expected.len()
    } else {
        assert_eq!(headers["x-label"], "small");
        1
    };
    assert_eq!(
        headers.len(),
        business
            + 3
            + usize::from(signed)
            + usize::from(!near)
            + usize::from(headers.contains_key("host"))
    );
    if signed {
        let payload: Value = serde_json::from_slice(
            &STANDARD
                .decode(headers["payment-signature"].as_bytes())
                .unwrap(),
        )
        .unwrap();
        crate::test_signatures::recover_exact(&payload);
    }
    calls.lock().unwrap().push((parts.uri.to_string(), signed));
    if signed {
        let mut body = response
            .send_response(
                axum::http::Response::builder()
                    .header("content-type", "text/plain")
                    .body(())
                    .unwrap(),
                false,
            )
            .unwrap();
        body.send_data(axum::body::Bytes::from_static(b"paid"), true)
            .unwrap();
    } else {
        let challenge = json!({"x402Version":2,"resource":{"url":parts.uri.to_string()},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
        response
            .send_response(
                axum::http::Response::builder()
                    .status(402)
                    .header(
                        "payment-required",
                        STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
                    )
                    .body(())
                    .unwrap(),
                true,
            )
            .unwrap();
    }
}
