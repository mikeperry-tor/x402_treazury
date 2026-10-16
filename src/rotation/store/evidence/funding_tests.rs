use super::*;
use crate::rotation::base::{BaseRpc, now};
use axum::{Json, Router, routing::post};
use futures_util::FutureExt;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn readiness_support_survives_spend_and_requires_exact_historical_endpoint_reverification() {
    disputed_readiness_allows_fresh_capacity(88).await;
}

#[tokio::test]
async fn latest_only_readiness_dispute_allows_fresh_capacity_after_restart() {
    disputed_readiness_allows_fresh_capacity(100).await;
}

async fn disputed_readiness_allows_fresh_capacity(conflict_height: u64) {
    // head, wrong chain, changed historical amount/hash, current capacity
    let state = Arc::new(Mutex::new((100u64, false, false, false, 0u64)));
    let rpc_state = state.clone();
    let app=Router::new().route("/",post(move |Json(v):Json<Value>| {
        let state=rpc_state.clone();
        async move {
            let (head,wrong,amount_conflict,hash_conflict,current)=*state.lock().unwrap();
            let result=match v["method"].as_str().unwrap() {
                "eth_chainId"=>json!(if wrong {"0x1"}else{"0x2105"}),
                "eth_getBlockByNumber"=>{
                    let tag=v["params"][0].as_str().unwrap();
                    let height=if tag=="latest"{head}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                    let hash=if hash_conflict && height==conflict_height {999}else{height};
                    let timestamp=if head>100 && height<=100 {now().unwrap()-500}else{now().unwrap()};
                    json!({"number":format!("0x{height:x}"),"hash":format!("0x{hash:064x}"),"timestamp":format!("0x{timestamp:x}")})
                },
                "eth_call"=>{
                    assert!(v["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231"));
                    assert_eq!(v["params"][1]["requireCanonical"],true);
                    let hash=v["params"][1]["blockHash"].as_str().unwrap();
                    let height=u64::from_str_radix(&hash[2..],16).unwrap();
                    let amount=if height<=100 {5_000_000u64+u64::from(amount_conflict)}else{current};
                    json!(format!("0x{amount:064x}"))
                },
                method=>panic!("unexpected historical support mutation {method}"),
            };
            Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc = BaseRpc::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        12,
        120,
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = crate::rotation::store::test_support::declare_two_reserves(
        &mut store,
        "readiness",
        "5",
        Some(alloy_primitives::U256::from(100_000_000u64)),
        0,
    )
    .unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    let query = store
        .scoped_chain_query(&pool, vec![job.wallet_id.clone()], vec![])
        .unwrap();
    let support = store
        .record_verified_credit(&job.wallet_id, rpc.view(query).await.unwrap())
        .unwrap();
    let original = serde_json::to_string(&support).unwrap();
    state.lock().unwrap().0 = 200;
    let query = store
        .scoped_chain_query(&pool, vec![job.wallet_id.clone()], vec![])
        .unwrap();
    store
        .record_verified_credit(&job.wallet_id, rpc.view(query).await.unwrap())
        .unwrap();
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT balance FROM wallets WHERE id=?1",
                [&job.wallet_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "0"
    );
    state.lock().unwrap().1 = true;
    let query = store
        .scoped_chain_query(&pool, vec![job.wallet_id.clone()], vec![])
        .unwrap();
    let error = rpc.view(query).await.err().unwrap();
    store.record_base_failure(&error).unwrap();
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT readiness_disputed FROM funding_jobs WHERE id=?1",
                [&job.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "endpoint_reverification_required"
    );
    state.lock().unwrap().1 = false;
    let request = store
        .funding_readiness_reverification(&job.id, &job.operation_id)
        .unwrap()
        .unwrap();
    for reason in ["chain_repair_required", "evidence_repair_required"] {
        let blocked = rpc
            .verify_historical_balance(request.clone())
            .await
            .unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET evidence_repair=?2 WHERE id=?1",
                params![job.wallet_id, reason],
            )
            .unwrap();
        assert!(
            store
                .restore_funding_readiness_support(&job.id, &job.operation_id, blocked)
                .is_err()
        );
        let dispute: String = store
            .db
            .query_row(
                "SELECT readiness_disputed FROM funding_jobs WHERE id=?1",
                [&job.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dispute, "endpoint_reverification_required");
    }
    // Restore only this fixture's injected gate before testing ordinary repair.
    store
        .db
        .execute(
            "UPDATE wallets SET evidence_repair=NULL WHERE id=?1",
            [&job.wallet_id],
        )
        .unwrap();
    let verified = rpc.verify_historical_balance(request).await.unwrap();
    let changes = store.capacity_changes(&pool);
    let unrelated = store.capacity_changes("unrelated");
    let changed = changes.notified();
    let other_changed = unrelated.notified();
    tokio::pin!(changed, other_changed);
    changed.as_mut().enable();
    other_changed.as_mut().enable();
    store
        .restore_funding_readiness_support(&job.id, &job.operation_id, verified)
        .unwrap();
    assert!(changed.as_mut().now_or_never().is_some());
    assert!(other_changed.as_mut().now_or_never().is_none());
    let (saved, dispute): (String, Option<String>) = store
        .db
        .query_row(
            "SELECT readiness_support,readiness_disputed FROM funding_jobs WHERE id=?1",
            [&job.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, original);
    assert!(dispute.is_none());
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT balance FROM wallets WHERE id=?1",
                [&job.wallet_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "0",
        "historical repair cannot refill admission balance"
    );

    // An exact-hash amount contradiction remains detectable without a cache,
    // and disputes readiness without inventing a nonce/debit dependency.
    let other = store
        .funding_jobs()
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.id != job.id)
        .unwrap();
    state.lock().unwrap().0 = 100;
    let query = store
        .scoped_chain_query(&pool, vec![other.wallet_id.clone()], vec![])
        .unwrap();
    store
        .record_verified_credit(&other.wallet_id, rpc.view(query).await.unwrap())
        .unwrap();
    store.evidence_cache.clear();
    state.lock().unwrap().2 = true;
    let query = store
        .scoped_chain_query(&pool, vec![other.wallet_id.clone()], vec![])
        .unwrap();
    let view = rpc.view(query).await.unwrap();
    assert_eq!(
        store
            .record_verified_credit(&other.wallet_id, view)
            .unwrap_err()
            .downcast_ref::<EvidenceFailure>(),
        Some(&EvidenceFailure::AmountContradiction)
    );
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT readiness_disputed FROM funding_jobs WHERE id=?1",
                [&other.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "evidence_repair_required"
    );
    let repair: Option<String> = store
        .db
        .query_row(
            "SELECT evidence_repair FROM wallets WHERE id=?1",
            [&other.wallet_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(repair.is_none());
    state.lock().unwrap().2 = false;
    state.lock().unwrap().0 = 200;

    // Reopen preserves archival accounting support. Explicit historical
    // revalidation, rather than current balance acquisition, checks its anchors.
    let id = store.id().to_owned();
    drop(store);
    let mut store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    state.lock().unwrap().3 = true;
    let request = crate::rotation::base::historical::HistoricalBalanceRequest {
        capture: store.capture_evidence(&job.wallet_id).unwrap(),
        observation: support.observation.clone(),
        checkpoint: support.checkpoint.clone(),
    };
    let error = rpc.verify_historical_balance(request).await.err().unwrap();
    assert!(error.is::<crate::rotation::base::EvidenceConflict>());
    store.record_base_failure(&error).unwrap();
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT readiness_disputed FROM funding_jobs WHERE id=?1",
                [&job.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "chain_repair_required"
    );
    assert!(
        store
            .funding_readiness_reverification(&job.id, &job.operation_id)
            .is_err()
    );
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT readiness_support FROM funding_jobs WHERE id=?1",
                [&job.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        original
    );
    // Neither a confirmed nor a latest-only readiness discrepancy is a
    // financial checkpoint. Persist the dispute across another restart, then
    // acquire independent current capacity without rewriting historical D.
    drop(store);
    let mut store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    state.lock().unwrap().4 = 6_000_000;
    store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES('outstanding',?1,?2,0,'2000000','fixture','POSSIBLY_SUBMITTED')",params![pool,job.wallet_id]).unwrap();
    add_exposure(&store.db, &job.wallet_id, U256::from(2_000_000)).unwrap();
    let query = store
        .scoped_chain_query(&pool, vec![job.wallet_id.clone()], vec![])
        .unwrap();
    let view = rpc.view(query).await.unwrap();
    store
        .publish_candidate_evidence(&pool, &job.wallet_id, view)
        .unwrap();
    assert!(
        store
            .cached_payment_candidate(&pool, U256::from(4_000_000), None)
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .cached_payment_candidate(&pool, U256::from(4_000_001), None)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        exposure(&store.db, &job.wallet_id).unwrap(),
        U256::from(2_000_000)
    );
    let (saved, dispute, repair): (String, String, Option<String>) = store.db.query_row(
        "SELECT f.readiness_support,f.readiness_disputed,w.evidence_repair FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id WHERE f.id=?1",
        [&job.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(saved, original);
    assert_eq!(dispute, "chain_repair_required");
    assert!(repair.is_none());

    // An actual financial dependency at that same historical anchor must still
    // block acquisition and mark durable repair; fresh balances cannot clear it.
    store
        .db
        .execute(
            "UPDATE wallets SET financial_height=?2,financial_hash=?3 WHERE id=?1",
            params![
                job.wallet_id,
                i64::try_from(conflict_height).unwrap(),
                format!("0x{conflict_height:064x}")
            ],
        )
        .unwrap();
    let query = store
        .scoped_chain_query(&pool, vec![job.wallet_id.clone()], vec![])
        .unwrap();
    let error = rpc.view(query).await.err().unwrap();
    store.record_base_failure(&error).unwrap();
    assert!(
        store
            .cached_payment_view(&pool, &job.wallet_id)
            .unwrap()
            .is_none()
    );
    let repair: String = store
        .db
        .query_row(
            "SELECT evidence_repair FROM wallets WHERE id=?1",
            [&job.wallet_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(repair, "chain_repair_required");
    assert_eq!(
        exposure(&store.db, &job.wallet_id).unwrap(),
        U256::from(2_000_000)
    );
    server.abort();
}
