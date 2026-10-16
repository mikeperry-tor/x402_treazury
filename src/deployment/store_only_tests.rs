//! Authenticated SDK failure leaves Base admission under its original store owner.
use super::initialization_tests::{env, managed_config, write_spec};
use super::*;
use crate::rotation::store::{AllocationKind, Store};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Fixture {
    signed: Arc<AtomicUsize>,
    signer: String,
}
async fn rpc(Json(v): Json<Value>) -> Json<Value> {
    let result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = v["params"][0].as_str().unwrap();
            let height = if tag == "latest" {
                100
            } else {
                u64::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap()
            };
            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",crate::rotation::base::now().unwrap())})
        }
        "eth_call" => {
            assert_eq!(v["params"][1]["requireCanonical"], true);
            let balance = v["params"][0]["data"]
                .as_str()
                .unwrap()
                .starts_with("0x70a08231");
            json!(format!("0x{:064x}", if balance { 5_000_000u64 } else { 0 }))
        }
        other => panic!("unexpected RPC {other}"),
    };
    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
}
async fn paid(State(f): State<Fixture>, headers: HeaderMap) -> axum::response::Response {
    if let Some(signature) = headers.get("payment-signature") {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
        assert_eq!(
            crate::test_signatures::recover_exact(&payload)
                .to_string()
                .to_lowercase(),
            f.signer.to_lowercase()
        );
        f.signed.fetch_add(1, Ordering::SeqCst);
        return "paid".into_response();
    }
    let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/read"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"3000000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
        "unpaid",
    )
        .into_response()
}

#[tokio::test]
async fn authenticated_sdk_failure_preserves_base_payments_liabilities_and_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut store = Store::create(&state, &key, 1, b"invalid SDK snapshot fixture").unwrap();
    let id = store.id().to_owned();
    let reserves = serde_json::from_value(json!({"small":{"funding_amount_usdc":"5"}})).unwrap();
    let group = store
        .ensure_capacity_group(
            "selected",
            &reserves,
            Some(alloy_primitives::U256::from(5_000_000)),
            0,
        )
        .unwrap();
    store
        .allocate_capacity_kind(&group, AllocationKind::Small, true)
        .unwrap()
        .unwrap();
    let jobs = store.funding_jobs().unwrap();
    let f = Fixture {
        signed: Arc::default(),
        signer: jobs[0].recipient.clone(),
    };
    let before = store.snapshot().unwrap();
    drop(store);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/rpc", post(rpc))
        .route("/read", get(paid))
        .with_state(f.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, format!("{}\n[wallets.static]\nmode='static'\nprivate_key_env='STATIC'\n[servers.static]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='static'\nsources=['api']\n", managed_config().replace("auto_fund=false", "auto_fund=true").replace("http://127.0.0.1:1", &origin))).unwrap();
    let mut env = env();
    env.insert("BASE".into(), format!("{origin}/rpc"));
    env.insert("STATIC".into(), format!("{:064x}", 1));
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    let running = deployment.bind(&env).await.unwrap();
    assert!(running.treasury.is_none());
    assert!(running.funding_runtime.is_none());
    assert!(running.supervisor.is_none());
    let store = running.store_only.as_ref().unwrap().store.clone();
    assert!(Store::open(&state, &key, &id).is_err());
    let pool = running.managed_pools[0].clone();
    pool.reconcile().await.unwrap();
    let payer = PaidClient::managed(pool.clone());
    let route = || crate::catalog::RoutedRequest {
        method: "GET".into(),
        url: format!("{origin}/read"),
        query: Default::default(),
        body: None,
    };
    assert_eq!(payer.execute(route()).await.unwrap(), "paid");
    assert!(
        payer.execute(route()).await.is_err(),
        "unresolved exposure must prevent second payment"
    );
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    let status = store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.funding_jobs.len(), jobs.len());
    assert_eq!(
        status
            .pools
            .iter()
            .map(|p| p.addresses.len())
            .sum::<usize>(),
        1
    );
    let snapshot = store.call(|s| s.snapshot()).await.unwrap();
    assert_eq!(snapshot, before);
    // Reconciliation retains the unresolved authorization; absence of a seller
    // receipt is never release authority.
    pool.reconcile().await.unwrap();
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    drop(payer);
    drop(pool);
    drop(store);
    let address = running
        .addresses()
        .into_iter()
        .find(|(n, _)| n == "static")
        .unwrap()
        .1;
    let stop = CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    let response = reqwest::Client::new()
        .post(format!("http://{address}/mcp"))
        .bearer_auth("fixture-token")
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let store = Store::open(&state, &key, &id).unwrap();
    assert_eq!(store.snapshot().unwrap(), before);
    assert_eq!(store.funding_jobs().unwrap().len(), jobs.len());
    server.abort();
}

#[tokio::test]
async fn serving_sdk_fallback_does_not_mask_shared_faults_or_allow_strict_bootstrap() {
    for fault in [
        "schema",
        "key",
        "authentication",
        "identity",
        "owner",
        "strict",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let store = Store::create(&state, &key, 1, b"invalid SDK fixture").unwrap();
        let owner = if fault == "owner" {
            Some(store)
        } else {
            drop(store);
            None
        };
        let mut config = managed_config();
        match fault {
            "schema" | "authentication" => {
                let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
                db.execute(
                    if fault == "schema" {
                        "PRAGMA user_version=15"
                    } else {
                        "UPDATE snapshots SET bytes=zeroblob(length(bytes))"
                    },
                    [],
                )
                .unwrap();
            }
            "key" => std::fs::write(&key, [0u8; 32]).unwrap(),
            "identity" => {
                config = config.replace(
                    "[treasury]",
                    &format!("[treasury]\nid='{}'", uuid::Uuid::new_v4()),
                )
            }
            _ => {}
        }
        write_spec(dir.path());
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, config).unwrap();
        let deployment = Deployment::load_for_serving_with_relay(&path, &env(), Default::default())
            .await
            .unwrap();
        let error = if fault == "strict" {
            deployment
                .initialize_wallets(&env(), Default::default(), None, &BTreeSet::new())
                .await
                .err()
                .expect("strict SDK requirement bypassed")
        } else {
            deployment
                .bind(&env())
                .await
                .err()
                .expect("shared integrity failure bypassed")
        };
        assert_eq!(
            error.is::<crate::treasury::SnapshotUnavailable>(),
            fault == "strict",
            "{fault}: {error:#}"
        );
        drop(owner);
    }
}

#[tokio::test]
async fn store_only_binding_rollback_and_cancelled_serving_release_owner() {
    for mode in [
        "bind_failure",
        "setup_failure",
        "drop_bound",
        "cancel_serve",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let store = Store::create(&state, &key, 1, b"invalid SDK fixture").unwrap();
        let id = store.id().to_owned();
        drop(store);
        write_spec(dir.path());
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = managed_config();
        if mode == "bind_failure" {
            config.push_str(&format!("\n[servers.two]\nlisten='{}'\nbearer_token_env='TOKEN'\nwallet='selected'\nsources=['api']\n", occupied.local_addr().unwrap()));
        }
        if mode == "setup_failure" {
            // Existing managed identity cannot be reclassified as static during
            // setup; failure occurs after the retained store worker starts.
            let mut store = Store::open(&state, &key, &id).unwrap();
            let reserves =
                serde_json::from_value(json!({"small":{"funding_amount_usdc":"5"}})).unwrap();
            store
                .ensure_capacity_group("static", &reserves, None, 0)
                .unwrap();
            drop(store);
            config.push_str("\n[wallets.static]\nmode='static'\nprivate_key_env='STATIC'\n[servers.static]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='static'\nsources=['api']\n");
        }
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, config).unwrap();
        let mut env = env();
        env.insert("STATIC".into(), format!("{:064x}", 1));
        let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
            .await
            .unwrap();
        let bound = deployment.bind(&env).await;
        if matches!(mode, "bind_failure" | "setup_failure") {
            assert!(bound.is_err());
            let store = Store::open(&state, &key, &id)
                .expect("cleanup must complete before returning error");
            assert!(store.funding_jobs().unwrap().is_empty());
        } else {
            let running = bound.unwrap();
            if mode == "drop_bound" {
                drop(running);
            } else {
                let store = running.store_only.as_ref().unwrap().store.clone();
                let task = tokio::spawn(running.serve(CancellationToken::new()));
                // A real store roundtrip gives the serving task a scheduling
                // opportunity without establishing a runtime deadline.
                store.call(|s| Ok(s.id().to_owned())).await.unwrap();
                drop(store);
                task.abort();
                let _ = task.await;
            }
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Ok(store) = Store::open(&state, &key, &id) {
                        assert!(store.funding_jobs().unwrap().is_empty());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("dropped owner did not drain");
        }
    }
}
