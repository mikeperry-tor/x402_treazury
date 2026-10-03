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
    ls.get_mut("writer")
        .unwrap()
        .source_management
        .as_mut()
        .unwrap()
        .allowed_targets = Some(vec!["writer".into(), "reader".into(), "sibling".into()]);
    ls.get_mut("reader")
        .unwrap()
        .source_management
        .as_mut()
        .unwrap()
        .wallet = Some("reader_wallet".into());
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
    let added = m
        .invoke(
            "writer",
            "treazury_source_add",
            json!({"candidate":candidate("scoped","process","process"),"idempotency_key":"scoped"}),
        )
        .await
        .unwrap();
    assert_eq!(
        added["wallet_profiles"],
        json!({"writer":"shared","reader":"reader_wallet","sibling":"shared"})
    );
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
        let (url, t) = listen(crate::server::http_app(
            server(&m, id),
            format!("{id}-token"),
        ))
        .await;
        tasks.push(t);
        let client = direct.discovery(&url, Duration::from_secs(3)).unwrap();
        for fallback in [false, true] {
            let params = if fallback {
                json!({"name":"treazury_tool_call","arguments":{"tool_id":name,"arguments":{},"expected_revision":1}})
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
                .json()
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
    hold.store(true, Ordering::SeqCst);
    let pending_server = server(&m, "writer");
    let pending_name = name.clone();
    let pending = tokio::spawn(async move {
        pending_server
            .invoke(&pending_name, &Default::default())
            .await
            .unwrap()
    });
    arrived.notified().await;
    // Keep an actual guarded HTTPS payment in flight while additions and an
    // update atomically replace the shared catalog on multiple worker threads.
    let barrier = Arc::new(tokio::sync::Barrier::new(4));
    *m.commit_barrier.lock().unwrap() = Some(barrier.clone());
    let mut mutations = tokio::task::JoinSet::new();
    for name in ["parallel_a", "parallel_b"] {
        let c = candidate(name, "process", "process");
        let preview = m
            .invoke("writer", "treazury_source_preview", json!({"candidate":c}))
            .await
            .unwrap();
        let manager = m.clone();
        mutations.spawn(async move { manager.invoke("writer", "treazury_source_add", json!({"candidate":c,"preview_id":preview["preview_id"],"idempotency_key":name})).await });
    }
    let manager = m.clone();
    let source = added["source_id"].clone();
    mutations.spawn(async move { manager.invoke("writer", "treazury_source_update", json!({"source_id":source,"expected_revision":1,"selection":{"tags":["read"]},"idempotency_key":"during-payment"})).await });
    tokio::time::timeout(Duration::from_secs(5), barrier.wait())
        .await
        .unwrap();
    while let Some(result) = mutations.join_next().await {
        result.unwrap().unwrap();
    }
    *m.commit_barrier.lock().unwrap() = None;
    assert_eq!(m.catalog.read().generation, 4);
    assert!(!pending.is_finished());
    assert_eq!(signatures.lock().unwrap().len(), 6);
    m.payers["shared"].replace_payer(
        Payer::new(
            &format!("{:064x}", 3),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    );
    release.notify_one();
    let old: Value = serde_json::from_str(&pending.await.unwrap()).unwrap();
    let old_signer: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
    assert_eq!(old["payer"], old_signer.address().to_string());
    let new: Value = serde_json::from_str(
        &server(&m, "sibling")
            .invoke(&name, &Default::default())
            .await
            .unwrap(),
    )
    .unwrap();
    let new_signer: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 3).parse().unwrap();
    assert_eq!(new["payer"], new_signer.address().to_string());
    assert_eq!(requests.load(Ordering::SeqCst), 16);
    assert_eq!(signatures.lock().unwrap().len(), 8);
    let before = requests.load(Ordering::SeqCst);
    m.invoke("writer","treazury_source_update",json!({"source_id":added["source_id"],"expected_revision":2,"selection":{"tags":["write"]},"idempotency_key":"hide"})).await.unwrap();
    for id in ["writer", "reader"] {
        let server = server(&m, id);
        assert!(server.invoke(&name, &Default::default()).await.is_err());
        assert!(
            server
                .invoke(
                    "treazury_tool_call",
                    json!({"tool_id":name,"expected_revision":1,"arguments":{}})
                        .as_object()
                        .unwrap()
                )
                .await
                .is_err()
        );
    }
    assert_eq!(
        requests.load(Ordering::SeqCst),
        before,
        "denied invocation reached seller"
    );
    let mut expected = vec![
        crate::network::global()
            .credentials(&IsolationId::discovery("https://api.example.com").unwrap()),
    ];
    // Spec, help and pricing use different deadlines and therefore distinct pools,
    // but all three retain the same discovery-origin credential.
    expected.extend([expected[0].clone(), expected[0].clone()]);
    for key in [1, 2, 3] {
        let signer: alloy_signer_local::PrivateKeySigner = format!("{key:064x}").parse().unwrap();
        expected.push(
            crate::network::global()
                .credentials(&IsolationId::evm(&signer.address().to_string()).unwrap()),
        );
    }
    let records = proxy.records.lock().unwrap();
    assert_eq!(
        records.len(),
        6,
        "spec/challenge/retry pools should reuse only within identity"
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
