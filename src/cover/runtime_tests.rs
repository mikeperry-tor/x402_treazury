use super::*;
use crate::{
    network::{Mode, NetworkContext, NetworkPolicy},
    payment::{PaidClient, Payer, SpendPolicy},
    test_socks::{Fault, Socks},
};
use axum::{
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
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

#[test]
fn paid_cover_lifecycle_in_isolated_process() {
    if std::env::var_os("TREAZURY_COVER_FIXTURE_CHILD").is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(paid_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cover::runtime_tests::paid_cover_lifecycle_in_isolated_process",
            "--nocapture",
        ])
        .env("TREAZURY_COVER_FIXTURE_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn fixed(unit: &str, value: u64) -> sampling::Distribution {
    toml::from_str(&format!(
        "distribution='uniform'\nmin_{unit}={value}\nmax_{unit}={value}"
    ))
    .unwrap()
}
async fn paid_fixture() {
    let ranges = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let padding = Arc::new(Mutex::new(Vec::new()));
    let (r, s, p) = (ranges.clone(), signed.clone(), padding.clone());
    let wait_ranges = ranges.clone();
    let held = Arc::new(tokio::sync::Semaphore::new(0));
    let held_range = held.clone();
    let app=axum::Router::new()
        .route("/openapi.json",get(move |h:HeaderMap|{let r=r.clone();let held=held_range.clone();async move{
            assert!(!h.contains_key("payment-signature")&&!h.contains_key("authorization"));
            let raw=h["range"].to_str().unwrap().strip_prefix("bytes=").unwrap();let (a,b)=raw.split_once('-').unwrap();let (a,b)=(a.parse::<usize>().unwrap(),b.parse::<usize>().unwrap());
            let n=r.fetch_add(1,Ordering::SeqCst);
            if n>0 {held.acquire().await.unwrap().forget();}
            (StatusCode::PARTIAL_CONTENT,[("content-range",format!("bytes {a}-{b}/16384")),("etag","\"v1\"".into())],vec![b'a';b-a+1])
        }}))
        .route("/api",get(move |h:HeaderMap|{let(s,p)=(s.clone(),p.clone());let wait_ranges=wait_ranges.clone();async move{
            p.lock().unwrap().push(h.get("x-example-padding").map(|v|v.as_bytes().to_vec()));
            if let Some(header)=h.get("payment-signature") {
                let payload:serde_json::Value=serde_json::from_slice(&STANDARD.decode(header.as_bytes()).unwrap()).unwrap();
                assert_eq!(payload["accepted"]["amount"],"5000");crate::test_signatures::recover_exact(&payload);s.fetch_add(1,Ordering::SeqCst);
                return (StatusCode::OK,[("payment-response",STANDARD.encode(br#"{"success":true}"#))],"paid").into_response();
            }
            while wait_ranges.load(Ordering::SeqCst)<2 {tokio::task::yield_now().await;}
            let challenge=json!({"x402Version":2,"resource":{"url":"https://api.example.com/api","description":"fixture","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"5000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
        }}));
    let (address, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    let context = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address),
        cover_traffic_enabled: true,
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    crate::network::install_test_context(context);
    let mut config = tests::example_config();
    config.start_delay = fixed("ms", 0);
    config.request_gap = fixed("ms", 0);
    config.tail = fixed("ms", 500);
    config.volume = fixed("bytes", 4096);
    config.ranges = fixed("bytes", 1024);
    config.padding.as_mut().unwrap().size = fixed("bytes", 128);
    config.validate().unwrap();
    let scope = status::Scope {
        listener: "a".into(),
        source: "fixture".into(),
    };
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
    .with_cover(Some(config), scope.clone());
    let route = || crate::catalog::RoutedRequest {
        method: "GET".into(),
        url: "https://api.example.com/api".into(),
        query: BTreeMap::new(),
        body: None,
    };
    let (a, b) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            client.execute_response(route()),
            client.execute_response(route())
        )
    })
    .await
    .expect("real calls must finish while range responses are held");
    assert_eq!(held.available_permits(), 0);
    for response in [a.unwrap(), b.unwrap()] {
        assert_eq!(response.bytes, b"paid");
        assert!(response.paid_submission);
    }
    assert_eq!(signed.load(Ordering::SeqCst), 2);
    assert!(ranges.load(Ordering::SeqCst) >= 2);
    {
        let values = padding.lock().unwrap();
        assert_eq!(values.len(), 4);
        for value in values.iter() {
            assert_eq!(value.as_ref().unwrap().len(), 128);
        }
        assert_ne!(values[0], values[1]);
    }
    assert_eq!(
        socks.records.lock().unwrap().len(),
        1,
        "real and range requests should multiplex in fixture"
    );
    let tool:crate::catalog::ToolSpec=serde_json::from_value(json!({"name":"fixture_read","description":"fixture","method":"GET","path":"/api","input_schema":{"type":"object","properties":{}},"param_routes":{},"has_body":false})).unwrap();
    let mcp = crate::server::Server::new(
        vec![tool.clone()],
        client.clone(),
        "https://api.example.com".into(),
        None,
        None,
    );
    use rmcp::ServerHandler;
    assert!(mcp.get_tool(status::TOOL_NAME).is_some());
    let before = ranges.load(Ordering::SeqCst);
    let result = mcp
        .invoke(status::TOOL_NAME, &Default::default())
        .await
        .unwrap();
    assert!(result.contains("fixture") && !result.contains("other"));
    assert_eq!(ranges.load(Ordering::SeqCst), before);
    let unscoped = crate::server::Server::new(
        vec![tool.clone()],
        PaidClient::unsigned(),
        "https://api.example.com".into(),
        None,
        None,
    );
    assert!(unscoped.get_tool(status::TOOL_NAME).is_none());
    assert!(
        unscoped
            .invoke(status::TOOL_NAME, &Default::default())
            .await
            .is_err()
    );
    let mut collision = tool.clone();
    collision.name = status::TOOL_NAME.into();
    assert!(
        crate::server::Server::new(
            vec![collision],
            client.clone(),
            "https://api.example.com".into(),
            None,
            None
        )
        .validate_cover()
        .is_err()
    );
    let engine = crate::network::global().cover.as_ref().unwrap();
    engine.shutdown().await;
    let report = engine.status(&[scope]);
    assert!(report.to_string().contains("cover_shutdown"));
    let before = ranges.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(ranges.load(Ordering::SeqCst), before);
    server.abort();
}
