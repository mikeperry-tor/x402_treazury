//! Observe payer selection at transport construction, before any network I/O.
use super::*;
use crate::{
    catalog::{Config, build_tools},
    network::{IsolationId, Mode, NetworkPolicy},
    payment::SpendPolicy,
    pricing::runtime::received,
    rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{Store, StoreHandle},
    },
};
use alloy_primitives::U256;
use serde_json::json;
use std::sync::Mutex;

#[test]
fn advertised_prices_select_the_initial_payer_without_unsigned_discovery() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "catalog_state::initial_price_tests::initial_price_child",
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

#[tokio::test]
#[ignore = "isolated network observer; invoked by parent"]
async fn initial_price_child() {
    crate::network::install(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some("127.0.0.1:9150".parse().unwrap()),
        ..Default::default()
    })
    .unwrap();
    let selected = Arc::new(Mutex::new(Vec::new()));
    let captured = selected.clone();
    crate::network::install_http_scope_observer(move |observation| {
        captured.lock().unwrap().push(observation.canonical.clone());
        anyhow::bail!("fixture stops before unsigned request")
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "initial",
        crate::rotation::test_capacity::two_reserves("0.001"),
        Some(U256::from(1000)),
        0
    )
    .unwrap();
    let jobs = state.funding_jobs().unwrap();
    assert_eq!(jobs.len(), 2);
    for (job, balance) in jobs.iter().zip(["10", "1000"]) {
        state
            .record_credit(&job.wallet_id, balance, "fixture", 100)
            .unwrap();
    }
    let wallets = state.status().unwrap().pools.remove(0).addresses;
    let credentials: Vec<_> = wallets
        .iter()
        .map(|w| crate::network::global().credentials(&IsolationId::evm(&w.address).unwrap()))
        .collect();
    let (store, worker) = StoreHandle::spawn(state);
    let client = PaidClient::managed(Arc::new(
        ManagedPool::new(
            store.clone(),
            pool,
            BaseRpc::new("https://rpc.example.com").unwrap(),
            SpendPolicy::dollars("0.001").unwrap(),
        )
        .unwrap(),
    ));
    let cfg: Config = toml::from_str(
        r#"
        [pricing]
        source = "catalog"
        format = "payment-info"
        unit = "call"
        usd_as_usdc = true
    "#,
    )
    .unwrap();
    let root = json!({"paths":{"/items":{"get":{
        "summary":"Read items.",
        "x-payment-info":{"price":{"mode":"fixed","currency":"USD","amount":"0.0001"}}
    }}}});
    let tool = build_tools(&cfg, &root, "priced").unwrap().remove(0);
    assert_eq!(tool.advertised_pricing.amount(), Some(U256::from(100)));
    assert!(
        tool.catalog_hints.is_none(),
        "local catalog has no observed cache"
    );
    let mut bound = bind(vec![(
        tool,
        client.clone(),
        "https://api.example.com".into(),
    )])
    .remove(0);
    // A cloned client default must always be overwritten by the tool's metadata.
    bound.client = bound.client.with_initial_price(Some(U256::from(1_000_000)));
    let hints = crate::pricing::runtime::fixture(dir.path()).await;
    let key = hints.operation("GET", "/items");
    for (case, expected) in [
        ("local", 1),
        ("observed", 0),
        ("expired-observed", 1),
        ("above-cap", 0),
        ("unknown", 0),
        ("explicit-url", 1),
        ("zero", 0),
    ] {
        match case {
            "observed" => {
                bound.tool.catalog_hints = Some(hints.clone());
                key.learn(U256::from(5), received().unwrap());
            }
            "expired-observed" => hints.invalidate(),
            "above-cap" => {
                let mut root = root.clone();
                root["paths"]["/items"]["get"]["x-payment-info"]["price"]["amount"] = json!("2");
                bound.tool = build_tools(&cfg, &root, "priced").unwrap().remove(0);
            }
            "unknown" => {
                bound.tool = build_tools(
                    &Config::default(),
                    &json!({"paths":{"/items":{"get":{}}}}),
                    "unknown",
                )
                .unwrap()
                .remove(0);
            }
            "explicit-url" => {
                bound.tool = build_tools(&cfg, &root, "priced").unwrap().remove(0);
                bound.tool.request_url = Some("https://direct.example.com/items".into());
            }
            "zero" => {
                let mut root = root.clone();
                root["paths"]["/items"]["get"]["x-payment-info"]["price"]["amount"] = json!("0");
                bound.tool = build_tools(&cfg, &root, "priced").unwrap().remove(0);
            }
            _ => {}
        }
        let before = selected.lock().unwrap().len();
        let error = bound.invoke_output(&Default::default()).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("fixture stops before unsigned request"),
            "{case}: {error:#}"
        );
        let observed = selected.lock().unwrap();
        assert_eq!(observed.len(), before + 1, "{case}: no retry or extra I/O");
        assert_eq!(observed[before], credentials[expected], "{case}");
    }
    let before = selected.lock().unwrap().len();
    let relay = client
        .clone()
        .with_initial_price(Some(U256::from(100)))
        .for_relay();
    assert!(
        relay
            .execute_response_scoped(
                bound
                    .tool
                    .route("https://relay.example.com", &Default::default())
                    .unwrap(),
                &crate::payment::SubmissionProgress::default(),
                None,
            )
            .await
            .is_err()
    );
    assert_eq!(selected.lock().unwrap().len(), before + 1);
    assert_eq!(
        selected.lock().unwrap()[before],
        credentials[0],
        "relay fee selection cannot inherit a target API price"
    );
    drop(relay);
    // No RPC, SOCKS connection, unsigned response or signature was needed. The
    // observer rejected construction before all of those paths could execute.
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM funding_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(db);
    drop(bound);
    drop(client);
    drop(store);
    worker.await.unwrap();
}
