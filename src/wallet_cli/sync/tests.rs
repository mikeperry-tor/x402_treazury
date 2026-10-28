use super::*;
use crate::rotation::{
    store::{AllocationKind, Store},
    test_capacity::small_reserve,
};
use alloy_primitives::U256;
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

fn config() -> MetaConfig {
    serde_json::from_value(json!({"version":1,"servers":{},"sources":{"unused":{"spec":"/missing/provider.toml","api_key_env":"UNRELATED_MISSING_KEY"}},
        "wallets":{"configured":{"mode":"zcash_rotation","max_api_payment_usdc":"1","max_conversion_overhead_percent":5,"reserves":{"small":{"funding_amount_usdc":"2"}}},
            "static":{"mode":"static","private_key_env":"MISSING_STATIC_KEY"}},
        "funding":{"near_api_key_env":"MISSING_NEAR_KEY","near_user_session_env":"MISSING_NEAR_SESSION","base_rpc_fallback_url_envs":[]}
    })).unwrap()
}
fn allocate(store: &mut Store, name: &str) -> String {
    let pool = store
        .ensure_capacity_group(name, &small_reserve("2"), Some(U256::from(1_000_000)), 0)
        .unwrap();
    store
        .allocate_capacity_kind(&pool, AllocationKind::Small, true)
        .unwrap();
    pool
}
fn state(dir: &std::path::Path) -> Store {
    Store::create(&dir.join("state"), &dir.join("key"), 1, b"unfunded-fixture").unwrap()
}

#[tokio::test]
async fn scope_ignores_unrelated_validation_and_retains_generated_and_accepted_recipients() {
    for (assignment_scope, generated_name) in [
        ("deployment", "auto_v1_deployment"),
        ("server", "auto_v1_server_agent"),
        ("source", "auto_v1_source_unused"),
        ("binding", "auto_v1_binding_5_agent_unused"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state(dir.path());
        let configured = allocate(&mut state, "configured");
        let accepted = allocate(&mut state, "removed");
        allocate(&mut state, "unaccepted_removed");
        allocate(&mut state, "static");
        let generated = allocate(&mut state, generated_name);
        let job = state
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_id == accepted)
            .unwrap();
        state
            .save_funding_quote(
                &job.id,
                &serde_json::to_vec(
                    &json!({"request":{"amount":job.target,"recipient":job.recipient}}),
                )
                .unwrap(),
            )
            .unwrap();
        state
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
            .unwrap();
        state.disable_pool("removed").unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let mut value = serde_json::to_value(config()).unwrap();
        let mut template = value["wallets"]["configured"].clone();
        template["max_api_payment_usdc"] = json!("invalid");
        template["max_funding_spend_zec"] = json!("invalid");
        value["wallets"]["not_allocated"] = template.clone();
        value["wallets"]["not_allocated"]["max_retirement_remainder_usdc"] =
            json!("invalid-unused-tolerance");
        value["wallet_templates"] = json!({"default":template,"unused":template});
        value["wallets"]["configured"]["reserves"]["small"]["funding_amount_usdc"] =
            json!("invalid");
        value["wallet_assignment"] = json!({"scope":assignment_scope,"template":"default"});
        value["sources"]["broken_binding"] =
            json!({"spec":"/missing/provider.toml","wallet":"unknown"});
        value["sources"]["static_binding"] =
            json!({"spec":"/missing/provider.toml","wallet":"static"});
        value["servers"] = json!({
            "agent":{"listen":"127.0.0.1:8080","sources":["unused"]},
            "unrelated":{"listen":"127.0.0.1:8081","sources":["missing","broken_binding","static_binding"]},
            "bound":{"listen":"127.0.0.1:8082","sources":["unused"],"wallet":"unknown"}
        });
        let mut config: MetaConfig = serde_json::from_value(value).unwrap();
        assert!(crate::rotation::assignment::resolve(&config).is_err());
        let (observed, malformed) = scope(&config, &store).await.unwrap();
        assert_eq!(malformed, 0);
        assert_eq!(observed.len(), 3);
        assert_eq!(
            observed
                .keys()
                .map(|(pool, _)| pool.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([configured, accepted, generated])
        );
        // Observation neither declares missing configured pools nor allocates.
        assert_eq!(
            store.call(|s| Ok(s.funding_jobs()?.len())).await.unwrap(),
            5
        );
        config.wallet_templates.clear();
        let (observed, malformed) = scope(&config, &store).await.unwrap();
        assert_eq!(malformed, 0);
        assert_eq!(observed.len(), 2);
        assert!(observed.contains_key(&(job.pool_id, job.wallet_id)));
        drop(store);
        worker.await.unwrap();
    }
}

#[tokio::test]
async fn treasury_only_sync_has_no_base_configuration_requirement() {
    let dir = tempfile::tempdir().unwrap();
    let (store, worker) = StoreHandle::spawn(state(dir.path()));
    let config = serde_json::from_value(json!({"version":1,"servers":{}})).unwrap();
    let result = observe_usdc(&config, store.clone(), &CancellationToken::new()).await;
    assert_eq!(result.status, "inapplicable");
    assert_eq!(
        (result.attempted, result.completed, result.failed),
        (0, 0, 0)
    );
    drop(store);
    worker.await.unwrap();
}

#[tokio::test]
async fn one_pass_refreshes_independent_wallets_without_near_credentials_or_source_side_effects() {
    use axum::{Json, Router, extract::State, routing::post};
    let dir = tempfile::tempdir().unwrap();
    let mut state = state(dir.path());
    let pool = allocate(&mut state, "configured");
    let job = state.funding_jobs().unwrap().remove(0);
    state
        .record_credit(&job.wallet_id, "7", "fixture", 1)
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let bad = "0x0000000000000000000000000000000000000001";
    db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance) SELECT 'failing',pool_id,sequence+1,?1,key,target,'RETAINED','99' FROM wallets WHERE id=?2",rusqlite::params![bad,job.wallet_id]).unwrap();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/rpc", listener.local_addr().unwrap());
    let app = Router::new().route("/rpc",post(|State(calls):State<Arc<Mutex<Vec<String>>>>,Json(request):Json<serde_json::Value>| async move {
        assert_eq!(request["method"],"eth_call");
        assert_eq!(request["params"][1],"latest");
        let address = format!("0x{}",&request["params"][0]["data"].as_str().unwrap()[34..]);
        calls.lock().unwrap().push(address.clone());
        if address == "0x0000000000000000000000000000000000000001" {
            Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"private upstream prose"}}))
        } else {
            Json(json!({"jsonrpc":"2.0","id":request["id"],"result":format!("0x{:064x}",17)}))
        }
    })).with_state(seen);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // Resolve configured Base settings through the production API without
    // touching process-global environment in parallel tests.
    let base = BaseRpc::new(&endpoint, 12).unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let (wallets, malformed) = scope(&config(), &store).await.unwrap();
    assert_eq!(malformed, 0);
    assert_eq!(wallets.len(), 2);
    let report = AssetObservation {
        status: "inapplicable",
        attempted: 0,
        completed: 0,
        failed: 0,
        error: None,
    };
    let report = refresh_usdc(
        store.clone(),
        wallets,
        report,
        base,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(report.status, "failed");
    assert_eq!(
        (report.attempted, report.completed, report.failed),
        (2, 1, 1)
    );
    assert_eq!(report.error, Some("usdc_balance_unavailable"));
    assert_eq!(calls.lock().unwrap().len(), 2);
    let balances = db
        .prepare("SELECT id,balance FROM wallets WHERE pool_id=?1 ORDER BY sequence")
        .unwrap()
        .query_map([pool], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        balances,
        vec![
            (job.wallet_id, "17".into()),
            ("failing".into(), "99".into())
        ]
    );
    for table in [
        "outgoing",
        "treasury_operations",
        "operation_network",
        "payment_attempts",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM funding_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    drop(store);
    worker.await.unwrap();
    server.abort();
}

#[tokio::test]
async fn finite_sync_applies_group_tolerance_and_retires_without_a_funding_owner() {
    use crate::rotation::config::{ReserveConfig, ReservesConfig};
    use axum::{Json, Router, routing::post};
    for tolerance in ["0.001", "0"] {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state(dir.path());
        let reserve = ReserveConfig {
            funding_amount_usdc: "2".into(),
            max_funding_amount_usdc: None,
        };
        let pool = state
            .ensure_capacity_group(
                "configured",
                &ReservesConfig {
                    small: reserve.clone(),
                    large: Some(reserve.clone()),
                },
                Some(U256::from(1_000_000)),
                0,
            )
            .unwrap();
        let job = state
            .allocate_capacity_kind(&pool, AllocationKind::Large, true)
            .unwrap()
            .unwrap();
        let wallet = state.funding_job(&job).unwrap().unwrap().wallet_id;
        let head = Arc::new(std::sync::atomic::AtomicU64::new(100));
        let current = head.clone();
        let timestamp = crate::rotation::base::now().unwrap();
        let app = Router::new().route("/", post(move |Json(request):Json<serde_json::Value>| {
            let head = current.load(std::sync::atomic::Ordering::SeqCst);
            async move {
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let height = if matches!(tag,"latest"|"finalized") {head} else {u64::from_str_radix(&tag[2..],16).unwrap()};
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{timestamp:x}")})
                    },
                    "eth_call" => json!(format!("0x{:064x}", if head==100 {2_000_000} else {1_000})),
                    method => panic!("unexpected mutation {method}"),
                };
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base =
            BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let query = state
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        state
            .record_verified_credit(&wallet, base.view(query).await.unwrap())
            .unwrap();
        state
            .configure_capacity_group(
                &pool,
                Some(U256::from(1_000_000)),
                0,
                &ReservesConfig {
                    small: reserve,
                    large: None,
                },
            )
            .unwrap();
        state.pause_new_funding();
        head.store(200, std::sync::atomic::Ordering::SeqCst);
        let (store, worker) = StoreHandle::spawn(state);
        let mut settings = config();
        if let crate::rotation::config::WalletConfig::ZcashRotation {
            max_retirement_remainder_usdc,
            ..
        } = settings.wallets.get_mut("configured").unwrap()
        {
            *max_retirement_remainder_usdc = tolerance.into();
        }
        let (wallets, malformed) = scope(&settings, &store).await.unwrap();
        assert_eq!(malformed, 0);
        let report = refresh_usdc(
            store.clone(),
            wallets,
            AssetObservation {
                status: "inapplicable",
                attempted: 0,
                completed: 0,
                failed: 0,
                error: None,
            },
            base,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(report.status, "completed");
        assert_eq!(
            (report.attempted, report.completed, report.failed),
            (1, 1, 0)
        );
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        let role: String = db
            .query_row("SELECT role FROM wallets WHERE id=?1", [wallet], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            role,
            if tolerance == "0.001" {
                "RETIRED"
            } else {
                "RETAINED"
            }
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM treasury_operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(db);
        drop(store);
        worker.await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn invalid_existing_retirement_policy_keeps_scalar_sync_and_saved_authority() {
    use axum::{Json, Router, routing::post};
    let dir = tempfile::tempdir().unwrap();
    let mut state = state(dir.path());
    let pool = allocate(&mut state, "configured");
    let job = state.funding_jobs().unwrap().remove(0);
    state
        .record_credit(&job.wallet_id, "7", "fixture", 1)
        .unwrap();
    let mut settings = config();
    if let crate::rotation::config::WalletConfig::ZcashRotation {
        max_retirement_remainder_usdc,
        ..
    } = settings.wallets.get_mut("configured").unwrap()
    {
        *max_retirement_remainder_usdc = "invalid".into();
    }
    let (store, worker) = StoreHandle::spawn(state);
    let (wallets, malformed) = scope(&settings, &store).await.unwrap();
    assert_eq!(malformed, 1);
    assert_eq!(wallets.get(&(pool, job.wallet_id)), Some(&false));
    let app = Router::new().route(
        "/",
        post(|Json(request): Json<serde_json::Value>| async move {
            assert_eq!(request["method"], "eth_call");
            assert_eq!(request["params"][1], "latest");
            Json(json!({"jsonrpc":"2.0","id":request["id"],"result":format!("0x{:064x}",17)}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let report = refresh_usdc(
        store.clone(),
        wallets,
        AssetObservation::inventory(malformed),
        base,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(report.status, "failed");
    assert_eq!(
        (report.attempted, report.completed, report.failed),
        (2, 1, 1)
    );
    assert_eq!(report.error, Some("usdc_inventory_unavailable"));
    let status = store.call(|s| s.status()).await.unwrap();
    assert_eq!(status.pools[0].addresses[0].confirmed_balance, "17");
    assert_eq!(
        status.pools[0].retirement_tolerance_atomic.as_deref(),
        Some("0")
    );
    drop(store);
    worker.await.unwrap();
    server.abort();
}
