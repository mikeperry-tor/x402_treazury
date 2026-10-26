//! Direct targets retain their arguments and price hints through overlapping paid calls.
use super::*;
use crate::{
    catalog::{Config, build_tools},
    network::{Mode, NetworkContext, NetworkPolicy},
    payment::SpendPolicy,
    pricing::runtime::process_hints,
    rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{Store, StoreHandle},
    },
    test_socks::{Fault, Socks},
};
use alloy_primitives::U256;
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::sync::Mutex;

#[test]
fn concurrent_paid_operation_urls_keep_targets_arguments_and_price_hints_separate() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .args([
            "--ignored",
            "--exact",
            "catalog_state::operation_url_tests::paid_operation_urls_child",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[derive(Clone)]
struct Seller {
    overlapping: Arc<tokio::sync::Barrier>,
    requests: Arc<Mutex<Vec<(String, bool)>>>,
    authorizations: Arc<Mutex<Vec<(String, String)>>>,
}

async fn purchase(
    State(seller): State<Seller>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let host = uri
        .host()
        .or_else(|| headers.get("host").and_then(|value| value.to_str().ok()))
        .unwrap();
    let amount = match host {
        "api.example.com" => 100,
        "spec.example.com" => 200,
        _ => panic!("unexpected target {host}"),
    };
    let path = uri.path_and_query().unwrap().as_str();
    assert_eq!(path, "/purchase?fixed=yes&q=original");
    assert_eq!(headers["x-region"], "north");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"item":"map"})
    );
    let resource = format!("https://{host}{path}");
    let signature = headers.get("payment-signature");
    seller
        .requests
        .lock()
        .unwrap()
        .push((host.into(), signature.is_some()));
    if let Some(signature) = signature {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
        assert_eq!(payload["accepted"]["amount"], amount.to_string());
        assert_eq!(payload["resource"]["url"], resource);
        let authorization = &payload["payload"]["authorization"];
        assert_eq!(authorization["value"], amount.to_string());
        let signer = crate::test_signatures::recover_exact(&payload)
            .to_string()
            .to_lowercase();
        assert_eq!(
            authorization["from"].as_str().unwrap().to_lowercase(),
            signer
        );
        seller
            .authorizations
            .lock()
            .unwrap()
            .push((signer, authorization["nonce"].as_str().unwrap().into()));
        return Json(json!({"target":host})).into_response();
    }
    // Neither challenge can return until both independent calls have arrived.
    seller.overlapping.wait().await;
    let challenge = json!({"x402Version":2,"resource":{"url":resource},"accepts":[{
        "scheme":"exact","network":"eip155:8453",
        "asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":amount.to_string(),
        "payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,
        "extra":{"name":"USD Coin","version":"2"}
    }]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
    )
        .into_response()
}

#[tokio::test]
#[ignore = "private network policy; invoked by parent"]
async fn paid_operation_urls_child() {
    tokio::time::timeout(std::time::Duration::from_secs(30), paid_operation_urls())
        .await
        .unwrap();
}

async fn paid_operation_urls() {
    let seller = Seller {
        overlapping: Arc::new(tokio::sync::Barrier::new(2)),
        requests: Default::default(),
        authorizations: Default::default(),
    };
    let app = axum::Router::new()
        .route("/purchase", post(purchase))
        .with_state(seller.clone());
    let (address, server) = crate::test_tls::serve(app).await;
    let proxy = Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), address),
        ]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );

    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let group = state
        .ensure_capacity_group(
            "direct",
            &crate::rotation::test_capacity::small_reserve("0.01"),
            Some(crate::rotation::config::positive_usdc("0.01").unwrap()),
            0,
        )
        .unwrap();
    state
        .allocate_capacity_kind(&group, crate::rotation::store::AllocationKind::Small, true)
        .unwrap();
    let jobs = state.funding_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    state
        .record_credit(&jobs[0].wallet_id, "10000", "fixture", 100)
        .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let pool = ManagedPool::new(
        store.clone(),
        group,
        // Any accidental balance read fails: the fixture serves purchase only.
        BaseRpc::new("https://api.example.com/rpc", 12).unwrap(),
        SpendPolicy::dollars("0.01").unwrap(),
    )
    .unwrap();
    let payer = pool.documentation_address().await.unwrap().to_lowercase();
    let client = PaidClient::managed(Arc::new(pool));
    let tools: Vec<_> = ["api.example.com", "spec.example.com"]
        .into_iter()
        .map(|host| {
            let document = json!({"paths":{"/gateway/purchase":{"post":{
                "x-direct-url":format!("https://{host}/purchase?fixed=yes"),
                "parameters":[{"name":"q","in":"query","schema":{"type":"string"}},
                    {"name":"X-Region","in":"header","schema":{"type":"string"}}],
                "requestBody":{"content":{"application/json":{"schema":{"type":"object",
                    "properties":{"item":{"type":"string"}}}}}}
            }}}});
            BoundTool {
                tool: build_tools(
                    &Config {
                        operation_url_extension: Some("x-direct-url".into()),
                        ..Default::default()
                    },
                    &document,
                    "direct",
                )
                .unwrap()
                .remove(0),
                client: client.clone(),
                base: "https://gateway.example.com/api".into(),
                source: Some(("same-source".into(), 1)),
                help: Arc::new(HelpBinding::new(client.clone())),
            }
        })
        .collect();
    let effective = client
        .clone()
        .with_source_identity("same-source")
        .public_destinations();
    let keys: Vec<_> = tools
        .iter()
        .map(|tool| {
            effective
                .operation_key(
                    Some(("same-source", 1)),
                    &tool.tool.method,
                    &tool.tool.path,
                    tool.tool.request_url.as_deref().unwrap(),
                )
                .unwrap()
        })
        .collect();
    assert_ne!(keys[0], keys[1]);
    assert!(keys.iter().all(|key| process_hints().get(*key).is_none()));
    let args = json!({"q":"original","X-Region":"north","item":"map"});
    let (first, second) = tokio::join!(
        tools[0].invoke_output(args.as_object().unwrap()),
        tools[1].invoke_output(args.as_object().unwrap())
    );
    for (output, host) in [first, second]
        .into_iter()
        .zip(["api.example.com", "spec.example.com"])
    {
        let output = output.unwrap();
        assert!(output.text.contains(host));
        assert!(output.payment_may_have_occurred);
    }
    for (key, amount) in keys.into_iter().zip([100u64, 200]) {
        assert_eq!(process_hints().get(key), Some(U256::from(amount)));
    }
    let gateway_key = effective
        .operation_key(
            Some(("same-source", 1)),
            "POST",
            "/gateway/purchase",
            &tools[0].base,
        )
        .unwrap();
    assert!(process_hints().get(gateway_key).is_none());
    let mut requests = seller.requests.lock().unwrap().clone();
    requests.sort();
    assert_eq!(
        requests,
        [
            ("api.example.com".into(), false),
            ("api.example.com".into(), true),
            ("spec.example.com".into(), false),
            ("spec.example.com".into(), true)
        ]
    );
    {
        let authorizations = seller.authorizations.lock().unwrap();
        assert_eq!(authorizations.len(), 2);
        assert!(authorizations.iter().all(|(signer, _)| signer == &payer));
        assert_ne!(authorizations[0].1, authorizations[1].1);
    }
    client.drain().await;
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM payment_attempts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(tools);
    drop(effective);
    drop(client);
    drop(store);
    worker.await.unwrap();
    server.abort();
}
