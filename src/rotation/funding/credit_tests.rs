//! Production initial-credit reads are advisory and consume a claim once.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    store::{Store, StoreHandle},
};
use alloy_primitives::U256;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[tokio::test]
async fn partial_credit_consumes_initial_claim_and_late_reads_never_reopen_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let key = dir.path().join("key");
    let mut state = Store::create(&path, &key, 1, b"fixture").unwrap();
    let id = state.id().to_owned();
    let group = crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "initial",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(100)),
        0
    )
    .unwrap();
    let job = state.funding_jobs().unwrap().remove(0);
    let balance = Arc::new(AtomicU64::new(0));
    let value = balance.clone();
    let app=Router::new().route("/",post(move |Json(v):Json<Value>|{
        let value=value.clone();
        async move {
            assert_eq!(v["method"],"eth_call");assert_eq!(v["params"][1],"latest");
            Json(json!({"jsonrpc":"2.0","id":v["id"],"result":format!("0x{:064x}",value.load(Ordering::SeqCst))}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = BaseRpc::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        12,
        120,
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (store, owner) = StoreHandle::spawn(state);
    let observer = Observations {
        store: store.clone(),
        near: super::super::near::NearClient::new(None).unwrap(),
        base,
    };
    observer.credit(job.clone()).await.unwrap();
    let current = job.clone();
    store
        .call(move |s| {
            s.current_funding_claim(&current.id)?;
            Ok(())
        })
        .await
        .unwrap();
    // Credit must commit even when an independent source outcome is uncertain.
    let db = rusqlite::Connection::open(path.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE funding_progress SET phase='\"PREPARING\"' WHERE job_id=?1",
        [&job.id],
    )
    .unwrap();
    balance.store(7, Ordering::SeqCst);
    observer.credit(job.clone()).await.unwrap();
    assert_eq!(
        db.query_row(
            "SELECT readiness_requirement FROM funding_jobs WHERE id=?1",
            [&job.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "7"
    );
    assert_eq!(
        db.query_row(
            "SELECT role FROM wallets WHERE id=?1",
            [&job.wallet_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "READY"
    );
    assert_eq!(
        db.query_row(
            "SELECT phase FROM funding_progress WHERE job_id=?1",
            [&job.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "\"PREPARING\""
    );
    let current = job.clone();
    store
        .call(move |s| {
            assert!(s.current_funding_claim(&current.id).is_err());
            Ok(())
        })
        .await
        .unwrap();
    db.execute("UPDATE funding_jobs SET readiness_support='malformed',readiness_disputed='old' WHERE id=?1",[&job.id]).unwrap();
    let pool = group.clone();
    store
        .call(move |s| s.disable_pool("initial").map(|_| pool))
        .await
        .unwrap();
    for value in [0, 5_000_000] {
        balance.store(value, Ordering::SeqCst);
        observer.credit(job.clone()).await.unwrap();
    }
    assert_eq!(
        db.query_row(
            "SELECT readiness_requirement FROM funding_jobs WHERE id=?1",
            [&job.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "7"
    );
    assert_eq!(
        db.query_row(
            "SELECT balance FROM wallets WHERE id=?1",
            [&job.wallet_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "5000000"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM funding_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM budget_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(observer);
    drop(store);
    owner.await.unwrap();
    let state = Store::open(&path, &key, &id).unwrap();
    assert!(state.current_funding_claim(&job.id).is_err());
    assert_eq!(
        state.funding_job(&job.id).unwrap().unwrap().operation_id,
        job.operation_id
    );
    server.abort();
}
