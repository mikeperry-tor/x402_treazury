//! End-to-end dynamic wallet binding with public-only HTTPS transport intact.
use super::*;
#[tokio::test]
async fn dynamic_wallet_signatures_and_tor_identity_match_listener_scope() {
    let result = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "discovery::tests::dynamic_scope::scope_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "private process network policy; exercised by parent"]
async fn scope_child() {
    use crate::{
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
        test_socks::{Fault, Socks},
    };
    use axum::{
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use base64::Engine;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let requests = Arc::new(AtomicUsize::new(0));
    let signatures = Arc::new(Mutex::new(vec![]));
    let hold = Arc::new(AtomicBool::new(false));
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (pause, entered, resume) = (hold.clone(), arrived.clone(), release.clone());
    let (u, s) = (requests.clone(), signatures.clone());
    let seller = move |headers: HeaderMap| {
        let (u, s) = (u.clone(), s.clone());
        let (pause, entered, resume) = (pause.clone(), entered.clone(), resume.clone());
        async move {
            u.fetch_add(1, Ordering::SeqCst);
            if let Some(header) = headers.get("payment-signature") {
                let payload: Value = serde_json::from_slice(
                    &base64::engine::general_purpose::STANDARD
                        .decode(header.as_bytes())
                        .unwrap(),
                )
                .unwrap();
                let address = crate::test_signatures::recover_exact(&payload);
                s.lock().unwrap().push(address.to_string());
                return axum::Json(json!({"payer": address.to_string()})).into_response();
            }
            if pause.swap(false, Ordering::SeqCst) {
                entered.notify_one();
                resume.notified().await;
            }
            let challenge = json!({"x402Version":2,"resource":{"url":"https://api.example.com/read","description":"read","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (
                StatusCode::PAYMENT_REQUIRED,
                [(
                    "payment-required",
                    base64::engine::general_purpose::STANDARD
                        .encode(serde_json::to_vec(&challenge).unwrap()),
                )],
            )
                .into_response()
        }
    };
    let (address, task) = crate::test_tls::serve(
        axum::Router::new()
            .route("/openapi.json", get(|| async { axum::Json(spec()) }))
            .route("/help", get(|| async { "fixture help" }))
            .route("/read", get(seller)),
    )
    .await;
    let proxy = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let p = policy(None);
    let mut ls = listeners(false);
    ls.insert("sibling".into(), ls["reader"].clone());
    ls.get_mut("reader").unwrap().wallet = Some("reader_wallet".into());
    let shared = payer();
    let other = PaidClient::new(
        Payer::new(
            &format!("{:064x}", 2),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    );
    let catalog = Arc::new(CatalogState::new(CatalogSnapshot {
        generation: 0,
        views: ls.keys().map(|id| (id.clone(), vec![])).collect(),
    }));
    let m = Manager::new(
        p,
        ls,
        BTreeMap::from([("shared".into(), shared), ("reader_wallet".into(), other)]),
        catalog,
        vec![],
    )
    .await
    .unwrap();
    // No seeded cache or fixture client replacement: registration fetches HTTPS.
    for owner in ["writer", "reader", "sibling"] {
        let added = m
            .invoke(
                owner,
                "x402_treazury_source_add",
                json!({"spec_url":"https://api.example.com/openapi.json","name":"scoped"}),
            )
            .await
            .unwrap();
        assert_eq!(added["targets"], json!([owner]));
    }
    // Help and pricing are discovery traffic even when invoked from a paid binding.
    let mut help = m.catalog.read().views["writer"]
        .iter()
        .find(|b| b.tool.method == "GET")
        .unwrap()
        .clone();
    help.tool.help_url = Some("https://api.example.com/help".into());
    assert_eq!(
        help.invoke(&Default::default()).await.unwrap(),
        "fixture help"
    );
    assert_eq!(
        help.invoke(&Default::default()).await.unwrap(),
        "fixture help"
    );
    let cfg = crate::catalog::Config {
        probe_pricing: true,
        ..Default::default()
    };
    let tools = m.catalog.read().views["writer"]
        .iter()
        .map(|b| b.tool.clone())
        .collect::<Vec<_>>();
    let prices = crate::pricing::PricingCache::default()
        .discover(&cfg, &spec(), &tools, "https://api.example.com")
        .await
        .unwrap();
    assert_eq!(prices.len(), 1);
    assert_eq!(requests.swap(0, Ordering::SeqCst), 1);
    let name = m.catalog.read().views["writer"]
        .iter()
        .find(|b| b.tool.method == "GET")
        .unwrap()
        .tool
        .name
        .clone();
    let direct = NetworkContext::new(NetworkPolicy::default()).unwrap();
    let mut tasks = vec![];
    for (id, key) in [("writer", 1), ("reader", 2), ("sibling", 1)] {
        let bound = m.catalog.read().views[id]
            .iter()
            .find(|b| b.tool.method == "GET")
            .unwrap()
            .clone();
        let name = bound.tool.name.clone();
        let reference = m.tool_reference(id, &bound);
        let (url, t) = listen(crate::server::http_app(
            server(&m, id),
            format!("{id}-token"),
        ))
        .await;
        tasks.push(t);
        let client = direct.discovery(&url, Duration::from_secs(3)).unwrap();
        for fallback in [false, true] {
            let params = if fallback {
                json!({"name":"x402_treazury_tool_call","arguments":{"tool_ref":reference,"arguments":{}}})
            } else {
                json!({"name":name,"arguments":{}})
            };
            let response: Value = client
                .post(format!("{url}/mcp"))
                .bearer_auth(format!("{id}-token"))
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params}))
                .send()
                .await
                .unwrap()
                .mcp_json()
                .await
                .unwrap();
            assert_ne!(response["result"]["isError"], true, "{response}");
            let body: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            let signer: alloy_signer_local::PrivateKeySigner =
                format!("{key:064x}").parse().unwrap();
            assert_eq!(body["payer"], signer.address().to_string());
        }
    }
    assert_eq!(requests.load(Ordering::SeqCst), 12);
    assert_eq!(signatures.lock().unwrap().len(), 6);
    let before = requests.load(Ordering::SeqCst);
    assert!(
        server(&m, "reader")
            .invoke(&name, &Default::default())
            .await
            .is_err()
    );
    assert!(
        server(&m, "hidden")
            .invoke(&name, &Default::default())
            .await
            .is_err()
    );
    assert_eq!(requests.load(Ordering::SeqCst), before);
    let mut expected = vec![
        crate::network::global()
            .credentials(&IsolationId::discovery("https://api.example.com").unwrap()),
    ];
    // Public-only imports retain a separate pool. Help and pricing share the
    // same idle policy and discovery credential.
    expected.push(expected[0].clone());
    for key in [1, 2] {
        let signer: alloy_signer_local::PrivateKeySigner = format!("{key:064x}").parse().unwrap();
        expected.push(
            crate::network::global()
                .credentials(&IsolationId::evm(&signer.address().to_string()).unwrap()),
        );
    }
    let records = proxy.records.lock().unwrap();
    assert_eq!(
        records.len(),
        4,
        "pools separate identity and public destination policy"
    );
    assert_eq!(
        records
            .iter()
            .map(|r| (r.user.clone(), r.password.clone()))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        records
            .iter()
            .all(|r| r.host == "api.example.com" && r.address_type == 3)
    );
    drop(records);
    for t in tasks {
        t.abort();
    }
    task.abort();
}
