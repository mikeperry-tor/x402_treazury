//! Retirement retains exact historical support across cache loss and disputes.
use super::*;
use crate::rotation::{
    base::{BaseRpc, now},
    config::{ReserveConfig, ReservesConfig},
    store::AllocationKind,
};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct FundingOnlyPermits;
impl crate::rotation::restriction::FundingPermits for FundingOnlyPermits {
    fn validate_treasury(&self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn validate_state(&self, _: &serde_json::Value) -> anyhow::Result<()> {
        Ok(())
    }
    fn reserve_allocations(&self, _: &str, _: &str, _: &[i64]) -> anyhow::Result<Vec<String>> {
        panic!("retirement must not allocate")
    }
    fn check_preparation(&self, _: &str, _: &str, _: u64) -> anyhow::Result<()> {
        panic!("retirement must not prepare")
    }
    fn reserve_preparation(&self, _: &str, _: &str, _: &str, _: u64) -> anyhow::Result<()> {
        panic!("retirement must not prepare")
    }
    fn retire_unprepared(&self, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
        panic!("retirement must not reset source work")
    }
}

#[tokio::test]
async fn retirement_keeps_exact_support_after_reopen_and_never_reactivates_on_dispute() {
    // Fresh funding head, fresh draining head; later changes deliberately contradict
    // one exact retired observation, never fabricate a financial certificate.
    let state = Arc::new(Mutex::new((100u64, false, false, false)));
    let rpc_state = state.clone();
    let app = Router::new().route("/", post(move |Json(v): Json<Value>| {
        let state = rpc_state.clone();
        async move {
            let (head, wrong_chain, amount_conflict, hash_conflict) = *state.lock().unwrap();
            let result = match v["method"].as_str().unwrap() {
                "eth_chainId" => json!(if wrong_chain { "0x1" } else { "0x2105" }),
                "eth_getBlockByNumber" => {
                    let tag = v["params"][0].as_str().unwrap();
                    let height = if tag == "latest" { head } else { u64::from_str_radix(&tag[2..], 16).unwrap() };
                    let hash = if hash_conflict && height == 188 { 999 } else { height };
                    json!({"number":format!("0x{height:x}"),"hash":format!("0x{hash:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
                },
                "eth_call" => {
                    assert!(v["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231"));
                    let hash = v["params"][1]["blockHash"].as_str().unwrap();
                    let height = u64::from_str_radix(&hash[2..],16).unwrap();
                    let amount = if height <= 100 { 100 } else { 1 + u64::from(amount_conflict) };
                    json!(format!("0x{amount:064x}"))
                },
                method => panic!("unexpected retirement RPC {method}"),
            };
            Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let reserve = ReserveConfig {
        funding_amount_usdc: "0.000001".into(),
        max_funding_amount_usdc: None,
    };
    let config = ReservesConfig {
        small: reserve.clone(),
        large: Some(reserve.clone()),
    };
    let pool = store
        .ensure_capacity_group("retirement", &config, Some(U256::from(100)), 1)
        .unwrap();
    let job = store
        .allocate_capacity_kind(&pool, AllocationKind::Large, true)
        .unwrap()
        .unwrap();
    let wallet = store.funding_job(&job).unwrap().unwrap().wallet_id;
    let query = store
        .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
        .unwrap();
    store
        .record_verified_credit(&wallet, rpc.view(query).await.unwrap())
        .unwrap();
    // Disabling a funded optional slot is the real retained-role transition.
    store
        .configure_capacity_group(
            &pool,
            Some(U256::from(100)),
            1,
            &ReservesConfig {
                small: reserve,
                large: None,
            },
        )
        .unwrap();
    assert_eq!(role(&store, &wallet), "RETAINED");
    state.lock().unwrap().0 = 200;
    store.deny_new_funding();
    for scoped in [true, false] {
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let view = rpc.view(query).await.unwrap();
        if scoped {
            store.publish_scoped_evidence(&pool, view).unwrap();
        } else {
            store.reconcile_pool(&pool, view).unwrap();
        }
        assert_eq!(
            role(&store, &wallet),
            "RETAINED",
            "qualification denial blocks retirement without blocking verified balance publication"
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM wallet_retirements", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let treasury = store.id().to_owned();
    drop(store);
    let mut store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury,
    )
    .unwrap();
    store
        .install_funding_permits(Arc::new(FundingOnlyPermits))
        .unwrap();
    for scoped in [true, false] {
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let view = rpc.view(query).await.unwrap();
        if scoped {
            store.publish_scoped_evidence(&pool, view).unwrap();
        } else {
            store.reconcile_pool(&pool, view).unwrap();
        }
        assert_eq!(
            role(&store, &wallet),
            "RETAINED",
            "funding permits grant no retirement mutation authority"
        );
    }
    drop(store);
    let mut store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury,
    )
    .unwrap();
    store.pause_new_funding();
    let query = store
        .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
        .unwrap();
    store
        .publish_scoped_evidence(&pool, rpc.view(query).await.unwrap())
        .unwrap();
    assert_eq!(role(&store, &wallet), "RETIRED");
    let original: String = store
        .db
        .query_row(
            "SELECT support FROM wallet_retirements WHERE wallet_id=?1",
            [&wallet],
            |r| r.get(0),
        )
        .unwrap();
    let treasury = store.id().to_owned();
    drop(store);
    let mut store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury,
    )
    .unwrap();
    let capture = store.capture_evidence(&wallet).unwrap();
    assert!(
        capture
            .durable_balance_anchors
            .iter()
            .any(|anchor| anchor.height == 188)
    );
    assert!(
        capture
            .durable_balance_anchors
            .iter()
            .any(|anchor| anchor.height == 200)
    );
    state.lock().unwrap().1 = true;
    let query = store
        .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
        .unwrap();
    let error = rpc.view(query).await.err().unwrap();
    store.record_base_failure(&error).unwrap();
    assert_eq!(role(&store, &wallet), "RETIRED");
    assert!(
        store
            .db
            .query_row(
                "SELECT disputed IS NOT NULL FROM wallet_retirements WHERE wallet_id=?1",
                [&wallet],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    state.lock().unwrap().1 = false;
    state.lock().unwrap().3 = true;
    state.lock().unwrap().0 = 300;
    // The already-disputed retirement remains immutable. Independent current
    // evidence may be acquired, but can neither clear the dispute nor reactivate.
    let query = store
        .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
        .unwrap();
    let view = rpc.view(query).await.unwrap();
    store.publish_scoped_evidence(&pool, view).unwrap();
    assert!(
        store
            .advisory_candidate(&pool, U256::from(1), Some(&wallet), i64::MAX, &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT disputed FROM wallet_retirements WHERE wallet_id=?1",
                [&wallet],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "evidence_repair_required"
    );
    assert_eq!(role(&store, &wallet), "RETIRED");
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT support FROM wallet_retirements WHERE wallet_id=?1",
                [&wallet],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        original
    );
    assert_eq!(store.funding_jobs().unwrap().len(), 1);
    server.abort();
}
fn role(store: &Store, wallet: &str) -> String {
    store
        .db
        .query_row("SELECT role FROM wallets WHERE id=?1", [wallet], |r| {
            r.get(0)
        })
        .unwrap()
}
