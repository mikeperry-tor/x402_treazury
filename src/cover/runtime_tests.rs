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
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .init();
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
    let logs = String::from_utf8_lossy(&output.stderr);
    for code in ["cover_shutdown", "TREAZURY_COVER_REPORT"] {
        assert!(logs.contains(code), "missing {code}: {logs}");
    }
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
    let balances = Arc::new(Mutex::new(BTreeMap::<String, u64>::new()));
    let rpc_balances = balances.clone();
    let app=axum::Router::new()
        .route("/rpc",axum::routing::post(move |h:HeaderMap,axum::Json(v):axum::Json<serde_json::Value>| {let balances=rpc_balances.clone();async move{
            assert!(!h.contains_key("x-example-padding")&&!h.contains_key("payment-signature"));
            let result=match v["method"].as_str().unwrap(){
                "eth_chainId"=>json!("0x2105"),
                "eth_getBlockByNumber"=>json!({"number":if v["params"][0]=="latest" {"0x64"} else {v["params"][0].as_str().unwrap()},"hash":format!("0x{:064x}",1),"timestamp":format!("0x{:x}",crate::rotation::base::now().unwrap())}),
                "eth_call"=>{let data=v["params"][0]["data"].as_str().unwrap();let value=if data.starts_with("0x70a08231"){let key=format!("0x{}",&data[data.len()-40..]).to_lowercase();*balances.lock().unwrap().get(&key).unwrap_or(&0)}else{0};json!(format!("0x{value:064x}"))},
                _=>panic!("unexpected fixture RPC")
            };axum::Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
        }}))
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
                let body=axum::body::Body::from_stream(futures_util::stream::once(async move {
                    while wait_ranges.load(Ordering::SeqCst)<2 {tokio::task::yield_now().await;}
                    Ok::<_,std::convert::Infallible>(axum::body::Bytes::from_static(b"paid"))
                }));
                return (StatusCode::OK,[("payment-response",STANDARD.encode(br#"{"success":true}"#)),("content-type","text/plain".into())],body).into_response();
            }
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
    let scope = Scope {
        listener: "a".into(),
        source: "fixture".into(),
    };
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
    .with_cover(Some(config.clone()), scope.clone());
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
        assert!(response.advisories.is_empty());
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
    assert!(mcp.get_tool("x402_treazury_cover_status").is_none());
    let before = ranges.load(Ordering::SeqCst);
    assert!(
        mcp.invoke("x402_treazury_cover_status", &Default::default())
            .await
            .is_err()
    );
    assert_eq!(ranges.load(Ordering::SeqCst), before);
    // Synthetic managed pool: the first unsigned payer is empty, standby is funded.
    // Admission must promote before signing, preserving the new identity's cover owner.
    let dir = tempfile::tempdir().unwrap();
    let mut store = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = store.ensure_pool("cover", "0.01").unwrap();
    let state = store.status().unwrap();
    let addresses = &state.pools[0].addresses;
    let active = addresses[0].address.clone();
    for address in addresses {
        store
            .record_credit(&address.id, "10000", &format!("0x{:064x}", 1), 88)
            .unwrap();
    }
    for address in addresses {
        balances.lock().unwrap().insert(
            address.address.to_lowercase(),
            if address.address == active { 0 } else { 10000 },
        );
    }
    let (store, worker) = crate::rotation::store::StoreHandle::spawn(store);
    let manager = Arc::new(
        crate::rotation::manager::ManagedPool::new(
            store.clone(),
            pool,
            crate::rotation::base::BaseRpc::new("https://api.example.com/rpc", 12, 120).unwrap(),
            "0.01",
            SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap(),
    );
    let managed = PaidClient::managed(manager).with_cover(
        Some(config),
        Scope {
            listener: "managed".into(),
            source: "fixture".into(),
        },
    );
    let result = tokio::time::timeout(Duration::from_secs(3), managed.execute_response(route()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.bytes, b"paid");
    assert!(result.paid_submission);
    let after = store.call(|s| s.status()).await.unwrap();
    assert_eq!(after.pools[0].generation, 1);
    drop(managed);
    drop(store);
    worker.await.unwrap();
    let engine = crate::network::global().cover.as_ref().unwrap();
    engine.shutdown().await;
    let before = ranges.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(ranges.load(Ordering::SeqCst), before);
    server.abort();
}
