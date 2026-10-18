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
            assert!(v["params"][1] == "latest" || v["params"][1]["requireCanonical"] == true);
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
    base_service("sdk", false).await;
}
#[tokio::test]
async fn funding_only_failures_preserve_base_payments_and_static_service() {
    for fault in ["credential", "quote", "fee"] {
        base_service(fault, false).await;
    }
}
#[tokio::test]
async fn enabled_relay_preserves_base_and_static_service_when_source_funding_is_unavailable() {
    for fault in ["sdk", "credential", "quote", "fee"] {
        base_service(fault, true).await;
    }
}
async fn base_service(fault: &str, relay_enabled: bool) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut store = if fault == "sdk" {
        Store::create(&state, &key, 1, b"invalid SDK snapshot fixture").unwrap()
    } else {
        let owner = crate::treasury::Treasury::create(state.clone(),key.clone(),2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
        let id = owner.status().await.unwrap().treasury_id;
        owner.close().await.unwrap();
        Store::open(&state, &key, &id).unwrap()
    };
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
    for job in &jobs {
        store
            .record_funding_balance(
                &job.id,
                &job.operation_id,
                &job.wallet_id,
                &job.recipient,
                alloy_primitives::U256::from(5_000_000),
            )
            .unwrap();
    }
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
        .route("/free", get(|| async { "static-free" }))
        .with_state(f.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::fs::write(
        dir.path().join("spec.json"),
        r#"{"operations":[{"method":"GET","path":"/read"},{"method":"GET","path":"/free"}]}"#,
    )
    .unwrap();
    let path = dir.path().join("deployment.toml");
    let mut config = managed_config()
        .replace("auto_fund=false", "auto_fund=true")
        .replace("http://127.0.0.1:1", &origin);
    config = match fault {
        "credential" => config.replace("[funding]", "[funding]\nnear_api_key_env='NEAR'"),
        "quote" => config.replace("[funding]", "[funding]\nquote_deadline_seconds=0"),
        "fee" => config.replace(
            "[treasury]",
            "[treasury]\nmax_funding_transaction_fee_zec='invalid'",
        ),
        _ => config,
    };
    if relay_enabled {
        let provider = Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/provider.toml");
        config.push_str(&format!(
            "\n[discovery_relay]\nprovider='{}'\nwallet='selected'\nserve=true\n",
            provider.display()
        ));
    }
    config.push_str("\n[wallets.static]\nmode='static'\nprivate_key_env='STATIC'\n[servers.static]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='static'\nsources=['api']\n");
    std::fs::write(&path, config).unwrap();
    let mut env = env();
    env.insert("BASE".into(), format!("{origin}/rpc"));
    env.insert("STATIC".into(), format!("{:064x}", 1));
    env.insert("NEAR".into(), "malformed\nheader".into());
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    let running = deployment.bind(&env).await.unwrap();
    assert!(running.treasury.is_none());
    assert!(running.funding_runtime.is_none());
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
    assert_eq!(payer.execute(route()).await.unwrap(), "paid");
    assert_eq!(f.signed.load(Ordering::SeqCst), 2);
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
    assert_eq!(f.signed.load(Ordering::SeqCst), 2);
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
    let response = reqwest::Client::new().post(format!("http://{address}/mcp"))
        .bearer_auth("fixture-token").header("accept","application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"api_free","arguments":{}}}))
        .send().await.unwrap();
    assert!(response.text().await.unwrap().contains("static-free"));
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
    for (fault, relay_enabled) in [
        "schema",
        "key",
        "authentication",
        "identity",
        "owner",
        "strict",
    ]
    .into_iter()
    .flat_map(|fault| {
        [false, true]
            .into_iter()
            .filter(move |relay| fault != "strict" || !relay)
            .map(move |relay| (fault, relay))
    }) {
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
        if relay_enabled {
            let provider =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/provider.toml");
            config.push_str(&format!(
                "\n[discovery_relay]\nprovider='{}'\nwallet='selected'\nserve=true\n",
                provider.display()
            ));
        }
        write_spec(dir.path());
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, config).unwrap();
        let loaded =
            Deployment::load_for_serving_with_relay(&path, &env(), Default::default()).await;
        let error = match loaded {
            Err(error) => error,
            Ok(deployment) if fault == "strict" => deployment
                .initialize_wallets(&env(), Default::default(), None, &BTreeSet::new())
                .await
                .err()
                .expect("strict SDK requirement bypassed"),
            Ok(deployment) => deployment
                .bind(&env())
                .await
                .err()
                .expect("shared integrity failure bypassed"),
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

#[tokio::test]
async fn dependency_declaration_failures_are_scoped_but_shared_failures_remain_fatal() {
    let cases = [
        ("[treasury]", "confirmations=0", true),
        (
            "[treasury]",
            "max_funding_transaction_fee_zec='invalid'",
            true,
        ),
        ("[treasury]", "max_refund_shielding_fee_zec='invalid'", true),
        ("[treasury]", "indexer_url='http://untrusted.example'", true),
        ("[funding]", "near_api_key_env='invalid-name'", true),
        ("[funding]", "near_user_session_env='MISSING'", true),
        ("[funding]", "confidentiality='unknown'", true),
        ("[funding]", "slippage_bps=1001", true),
        ("[funding]", "poll_seconds=0", true),
        ("[funding]", "swap_timeout_seconds=0", true),
        ("[funding]", "quote_deadline_seconds=1", true),
        ("[funding]", "base_confirmations=0", false),
    ];
    for (section, declaration, scoped) in cases {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let store = Store::create(&state, &key, 1, b"invalid SDK fixture").unwrap();
        let id = store.id().to_owned();
        drop(store);
        write_spec(dir.path());
        let path = dir.path().join("deployment.toml");
        let config = managed_config()
            .replace("max_refund_shielding_fee_zec='0.001'\n", "")
            .replace(section, &format!("{section}\n{declaration}"));
        std::fs::write(&path, config).unwrap();
        let loaded =
            Deployment::load_for_serving_with_relay(&path, &env(), Default::default()).await;
        let deployment = loaded.unwrap();
        let strict = deployment
            .initialize_wallets(&env(), Default::default(), None, &BTreeSet::new())
            .await;
        assert!(strict.is_err(), "strict preflight accepted {declaration}");
        let running = deployment.bind(&env()).await.unwrap();
        assert!(running.store_only.is_some(), "{declaration}");
        assert_eq!(running.managed_pools.is_empty(), !scoped, "{declaration}");
        assert!(running.funding_runtime.is_none());
        // No accepted work exists before serve. Use the normal close sequence
        // without detaching the owner while checking every declaration.
        let mut running = running;
        let owner = running.store_only.take().unwrap();
        drop(running);
        owner.close().await.unwrap();
        let store = Store::open(&state, &key, &id).unwrap();
        assert!(store.funding_jobs().unwrap().is_empty());
        drop(store);
        // The same source fault must never hide a shared authentication fault.
        std::fs::write(&key, [0u8; 32]).unwrap();
        let deployment = Deployment::load_for_serving_with_relay(&path, &env(), Default::default())
            .await
            .unwrap();
        assert!(
            deployment.bind(&env()).await.is_err(),
            "source fallback hid wrong key: {declaration}"
        );
    }
}

#[tokio::test]
async fn base_configuration_failure_preserves_static_service_and_refuses_managed_io() {
    for (fault, relay_enabled) in ["declaration", "missing_env", "insecure_endpoint"]
        .into_iter()
        .flat_map(|fault| [false, true].into_iter().map(move |relay| (fault, relay)))
    {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let store = Store::create(&state, &key, 1, b"invalid SDK fixture").unwrap();
        let id = store.id().to_owned();
        let before = store.snapshot().unwrap();
        drop(store);
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let provider = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", provider.local_addr().unwrap());
        let app = Router::new().fallback(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                "static-ok"
            }
        });
        let server = tokio::spawn(async move { axum::serve(provider, app).await.unwrap() });
        write_spec(dir.path());
        let mut config = managed_config().replace("http://127.0.0.1:1", &origin);
        if fault == "declaration" {
            config = config.replace("[funding]", "[funding]\nbase_confirmations=0");
        }
        if relay_enabled {
            let provider =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/provider.toml");
            config.push_str(&format!(
                "\n[discovery_relay]\nprovider='{}'\nwallet='selected'\nserve=true\n",
                provider.display()
            ));
        }
        config.push_str("\n[wallets.static]\nmode='static'\nprivate_key_env='STATIC'\n[servers.static]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='static'\nsources=['api']\n");
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, config).unwrap();
        let mut env = env();
        env.insert("STATIC".into(), format!("{:064x}", 1));
        match fault {
            "missing_env" => {
                env.remove("BASE");
            }
            "insecure_endpoint" => {
                env.insert("BASE".into(), "http://untrusted.example".into());
            }
            _ => {}
        }
        let mut deployment =
            Deployment::load_for_serving_with_relay(&path, &env, Default::default())
                .await
                .unwrap();
        let strict = deployment
            .initialize_wallets(&env, Default::default(), None, &BTreeSet::new())
            .await;
        assert!(strict.is_err());
        // Inspect the same client binding used by tool/raw/documentation paths.
        let initialized = match deployment.initialized.take() {
            Some(initialized) => initialized,
            None => deployment
                .initialize_wallets_mode(&env, Default::default(), None, &BTreeSet::new(), true)
                .await
                .unwrap(),
        };
        assert!(initialized.managed_pools.is_empty());
        let payer = initialized.wallets["selected"].clone();
        let error = payer.documentation_identity().await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::rotation::error::AdmissionError>(),
            Some(crate::rotation::error::AdmissionError::WalletNotReady(_))
        ));
        let error = payer
            .clone()
            .with_source_identity("dynamic")
            .execute(crate::catalog::RoutedRequest {
                method: "GET".into(),
                url: format!("{origin}/read"),
                query: Default::default(),
                body: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::rotation::error::AdmissionError>(),
            Some(crate::rotation::error::AdmissionError::WalletNotReady(_))
        ));
        drop(payer);
        initialized.shutdown().await.unwrap();
        let running = deployment.bind(&env).await.unwrap();
        assert!(running.managed_pools.is_empty());
        assert!(running.funding_runtime.is_none());
        assert!(running.treasury.is_none());
        assert!(running.store_only.is_some());
        let addresses: BTreeMap<_, _> = running.addresses().into_iter().collect();
        let stop = CancellationToken::new();
        let task = tokio::spawn(running.serve(stop.clone()));
        for (listener, expected) in [("one", "wallet_not_ready"), ("static", "static-ok")] {
            let response = reqwest::Client::new().post(format!("http://{}/mcp",addresses[listener]))
                .bearer_auth("fixture-token").header("accept","application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"api_read","arguments":{}}}))
                .send().await.unwrap().text().await.unwrap();
            assert!(response.contains(expected), "{fault}: {response}");
            assert_eq!(
                requests.load(Ordering::SeqCst),
                usize::from(listener == "static")
            );
        }
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let store = Store::open(&state, &key, &id).unwrap();
        assert_eq!(store.snapshot().unwrap(), before);
        assert!(store.funding_jobs().unwrap().is_empty());
        assert!(
            store
                .status()
                .unwrap()
                .pools
                .iter()
                .all(|p| p.addresses.is_empty())
        );
        server.abort();
    }
}
