//! Production initial-credit reads are advisory and consume a claim once.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    store::{Store, StoreHandle},
};
use alloy_primitives::U256;
use axum::{Json, Router, routing::post};
use futures_util::FutureExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[tokio::test]
async fn startup_and_funding_share_credit_after_cancellation_without_losing_new_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let key = dir.path().join("key");
    let mut state = Store::create(&path, &key, 1, b"fixture").unwrap();
    let group = state
        .ensure_capacity_group(
            "shared_credit",
            &crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(100)),
            0,
        )
        .unwrap();
    state
        .allocate_capacity_kind(&group, crate::rotation::store::AllocationKind::Small, true)
        .unwrap();
    let job = state.funding_jobs().unwrap().remove(0);
    let calls = Arc::new(AtomicU64::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (count, began, released) = (calls.clone(), started.clone(), release.clone());
    let app = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let (count, began, released) = (count.clone(), began.clone(), released.clone());
            async move {
                assert_eq!(request["method"], "eth_call");
                assert_eq!(request["params"][1], "latest");
                let call = count.fetch_add(1, Ordering::SeqCst) + 1;
                if call == 1 {
                    let released = released.notified();
                    tokio::pin!(released);
                    released.as_mut().enable();
                    began.notify_one();
                    released.await;
                }
                let value = if call == 1 { 17 } else { 33 };
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":format!("0x{value:064x}")}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (store, owner) = StoreHandle::spawn(state);
    let pool = super::super::manager::ManagedPool::new(
        store.clone(),
        group.clone(),
        base.clone(),
        crate::payment::SpendPolicy {
            max_atomic: Some(U256::from(100)),
        },
    )
    .unwrap();
    let observer = Observations {
        store: store.clone(),
        near: super::super::near::NearClient::new(None).unwrap(),
        base,
    };
    let startup_pool = pool.clone();
    let startup = tokio::spawn(async move { startup_pool.reconcile().await });
    started.notified().await;
    {
        let credit = observer.credit(job.clone());
        tokio::pin!(credit);
        // Poll the actual funding consumer while the startup read owns the RPC.
        assert!(credit.as_mut().now_or_never().is_none());
        startup.abort();
        assert!(startup.await.unwrap_err().is_cancelled());
        let (pending_pool, pending_wallet) = (group.clone(), job.wallet_id.clone());
        store
            .call(move |s| s.low_balance(&pending_pool, &pending_wallet))
            .await
            .unwrap();
        release.notify_one();
        credit.await.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let db = rusqlite::Connection::open(path.join("state.sqlite")).unwrap();
    let status = || {
        db.query_row(
            "SELECT balance,refresh_pending,low_balance FROM wallets WHERE id=?1",
            [&job.wallet_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            },
        )
        .unwrap()
    };
    // Publication is durable after caller cancellation, but cannot clear a
    // request accepted after this acquisition began.
    assert_eq!(status(), ("17".into(), true, true));
    observer.credit(job.clone()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(status(), ("17".into(), true, true));

    let active = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    observer.credit(job.clone()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(status(), ("33".into(), false, false));
    assert_eq!(
        db.query_row(
            "SELECT readiness_requirement FROM funding_jobs WHERE id=?1",
            [&job.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "17"
    );
    for (query, expected) in [
        ("SELECT count(*) FROM wallets", 1),
        ("SELECT count(*) FROM funding_jobs", 1),
        (
            "SELECT count(*) FROM funding_progress WHERE phase!='\"ALLOCATED\"'",
            0,
        ),
        ("SELECT count(*) FROM outgoing", 0),
        (
            "SELECT count(*) FROM operation_network WHERE preparation IS NOT NULL",
            0,
        ),
    ] {
        assert_eq!(
            db.query_row(query, [], |r| r.get::<_, i64>(0)).unwrap(),
            expected
        );
    }
    drop(db);
    drop(observer);
    drop(pool);
    drop(store);
    owner.await.unwrap();
    active.abort();
    server.abort();
}

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
    let base = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
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
    // The first zero observation owns the minute cooldown; observing new
    // external credit requires a genuinely later acquisition.
    let active = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
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
        tokio::time::advance(std::time::Duration::from_secs(60)).await;
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
        db.query_row(
            "SELECT count(*) FROM operation_network WHERE preparation IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
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
    active.abort();
}

struct CreditOnlyBackend(Observations);
impl FundingBackend for CreditOnlyBackend {
    type Observations = Observations;
    fn observations(&self) -> Observations {
        self.0.clone()
    }
    async fn quote_prerequisite(&mut self, _: &FundingJob) -> Result<QuoteRequest> {
        anyhow::bail!("disabled fixture must not request a quote")
    }
    async fn prepare(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        anyhow::bail!("disabled fixture must not prepare")
    }
    async fn submit(&mut self, _: &FundingJob) -> Result<()> {
        anyhow::bail!("disabled fixture must not submit")
    }
    async fn reconcile(&mut self, _: &FundingJob) -> Result<bool> {
        Ok(false)
    }
    fn max_attempts(&self, _: &FundingJob) -> u32 {
        3
    }
}

#[tokio::test]
async fn production_credit_uses_shared_failure_schedule_without_recounting_cached_errors() {
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
            "shared_schedule",
            &crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(100)),
            0,
        )
        .unwrap();
    state
        .allocate_capacity_kind(&group, crate::rotation::store::AllocationKind::Small, true)
        .unwrap();
    let job = state.funding_jobs().unwrap().remove(0);
    let quote = Quote {
        request: json!({"amount":job.target,"recipient":job.recipient}),
        response: json!({}),
        input: 50,
        deadline: u64::MAX,
        deposit: Some("fixture-deposit".into()),
    };
    state
        .save_funding_quote(&job.id, &serde_json::to_vec(&quote).unwrap())
        .unwrap();
    state
        .require_preparation_authority(
            &job.operation_id,
            Some(&super::super::transaction::ManagedFundingAuthority {
                job_id: job.id.clone(),
                auto_fund: true,
                max_attempts: 3,
            }),
        )
        .unwrap();
    state
        .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
        .unwrap();
    state.disable_pool("shared_schedule").unwrap();
    let calls = Arc::new(AtomicU64::new(0));
    let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (count, failure) = (calls.clone(), failing.clone());
    let app = Router::new().route("/", post(move |Json(request): Json<Value>| {
        let (count, failure) = (count.clone(), failure.clone());
        async move {
            assert_eq!(request["method"], "eth_call");
            let acquisition = count.fetch_add(1, Ordering::SeqCst);
            if acquisition == 1 {
                // This acquisition completes within a fractional scheduler tick.
                // Its shared deadline must round from the root clock anchor.
                tokio::time::advance(std::time::Duration::from_millis(550)).await;
            }
            if failure.load(Ordering::SeqCst) {
                Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"unavailable"}}))
            } else {
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":format!("0x{:064x}", 0)}))
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (store, owner) = StoreHandle::spawn(state);
    let reads = Observations {
        store: store.clone(),
        near: super::super::near::NearClient::new(None).unwrap(),
        base,
    };
    // A separate discovery consumer already owns the failure and its cooldown.
    let initial = reads.credit_observation(job.clone()).await;
    assert!(initial.result.is_err());
    let due = initial.next_due.unwrap();
    let cached = reads.credit_observation(job.clone()).await;
    assert!(cached.result.is_err());
    assert_eq!(cached.next_due, Some(due));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let worker = FundingWorker::new(store.clone(), CreditOnlyBackend(reads), 5);
    // Keep paused-time network fixtures from automatically advancing to timers.
    let active = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    worker.tick(1000).await.unwrap();
    let observed_job = || {
        let store = store.clone();
        let id = job.id.clone();
        async move {
            store
                .call(move |s| s.funding_job(&id))
                .await
                .unwrap()
                .unwrap()
        }
    };
    assert_eq!(observed_job().await.error_streak, 1);
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    worker.tick(1061).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(observed_job().await.error_streak, 2);
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    worker.tick(1122).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        observed_job().await.error_streak,
        2,
        "cached second failure is not a third observation"
    );
    tokio::time::advance(std::time::Duration::from_millis(58_450)).await;
    worker.tick(1181).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        observed_job().await.error_streak,
        2,
        "a fractional deadline cannot recount the same failed acquisition"
    );
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    worker.tick(1183).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(observed_job().await.error_streak, 3);
    failing.store(false, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(241)).await;
    worker.tick(1424).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(observed_job().await.error_streak, 0);
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let forbidden = (
        db.query_row("SELECT count(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        db.query_row(
            "SELECT count(*) FROM funding_jobs WHERE readiness_requirement IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
    );
    drop(db);
    assert_eq!(
        forbidden,
        (0, 0),
        "observations neither submit source work nor invent positive credit"
    );
    drop(worker);
    drop(store);
    owner.await.unwrap();
    active.abort();
    server.abort();
}
