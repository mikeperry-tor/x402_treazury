//! Ordinary retirement uses unfunded local RPC proofs and real Store publication.
use crate::rotation::{
    base::{BaseRpc, now},
    config::{ReserveConfig, ReservesConfig},
    manager::{ManagedPool, PaymentCandidateHandle},
    store::{AllocationKind, Authorization, Store, StoreHandle},
};
use alloy_primitives::{U256, keccak256};
use axum::{Json, Router, extract::State, routing::post};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
use tokio::sync::{Notify, Semaphore};

struct RpcState {
    head: u64,
    confirmed: u64,
    latest: u64,
    timestamp: u64,
    used: BTreeSet<u64>,
    scalar_calls: usize,
    scalar_latest: Option<u64>,
    balance_calls: usize,
    nonce_calls: usize,
    block_nonce: bool,
    block_latest_balance: bool,
    fail: bool,
    total_calls: usize,
}
struct Rpc {
    state: Mutex<RpcState>,
    started: Notify,
    release: Semaphore,
}
struct Fixture {
    dir: tempfile::TempDir,
    wallet: String,
    pool: Arc<ManagedPool>,
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    rpc: Arc<Rpc>,
}
impl Fixture {
    async fn new(tolerance: u64, credited: bool, retained: bool, authorizations: usize) -> Self {
        Self::with_predecessor(tolerance, credited, retained, authorizations, false).await
    }
    async fn with_predecessor(
        tolerance: u64,
        credited: bool,
        retained: bool,
        authorizations: usize,
        predecessor: bool,
    ) -> Self {
        Self::with_accepted_source(
            tolerance,
            credited,
            retained,
            authorizations,
            predecessor,
            false,
        )
        .await
    }
    async fn with_accepted_source(
        tolerance: u64,
        credited: bool,
        retained: bool,
        authorizations: usize,
        predecessor: bool,
        accepted_source: bool,
    ) -> Self {
        let (rpc, base, server) = rpc_fixture().await;
        let dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
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
        let reserves = ReservesConfig {
            small: reserve.clone(),
            large: Some(reserve.clone()),
        };
        let group = state
            .ensure_capacity_group("retirement", &reserves, Some(U256::from(100)), tolerance)
            .unwrap();
        let job = state
            .allocate_capacity_kind(&group, AllocationKind::Large, true)
            .unwrap()
            .unwrap();
        let wallet = state.funding_job(&job).unwrap().unwrap().wallet_id;
        if accepted_source {
            let operation = state.funding_job(&job).unwrap().unwrap().operation_id;
            state
                .accept_preparation(&operation, Some(&group), 60, None)
                .unwrap();
        }
        if credited {
            let query = state
                .scoped_chain_query(&group, vec![wallet.clone()], vec![])
                .unwrap();
            state
                .record_verified_credit(&wallet, base.view(query).await.unwrap())
                .unwrap();
        }
        if retained {
            state
                .configure_capacity_group(
                    &group,
                    Some(U256::from(100)),
                    tolerance,
                    &ReservesConfig {
                        small: reserve,
                        large: None,
                    },
                )
                .unwrap();
        }
        for index in 0..authorizations {
            let candidate = PaymentCandidateHandle {
                wallet: wallet.clone(),
                address: state.status().unwrap().pools[0].addresses[0]
                    .address
                    .clone(),
            };
            let admission = state
                .prepare_advisory_payment(&group, U256::from(1), "fixture", &candidate)
                .unwrap();
            state
                .journal_authorization(
                    &admission,
                    Authorization {
                        payer: candidate.address,
                        payee: format!("0x{:040x}", 7),
                        nonce: format!("0x{:064x}", index + 1),
                        valid_after: 0,
                        valid_before: now().unwrap() + 1_000,
                    },
                    None,
                )
                .unwrap();
        }
        if predecessor {
            let original = state.funding_job(&job).unwrap().unwrap();
            state
                .accept_preparation(&original.operation_id, Some(&group), 60, None)
                .unwrap();
            let revision = state.snapshot().unwrap().0;
            state
                .prepare_with_facts(
                    &original.operation_id,
                    revision,
                    b"next",
                    b"private-fixture-signed-bytes",
                    Some(crate::rotation::transaction::TransactionFacts {
                        txid: "fixture".into(),
                        expiry_height: 100,
                        amount_zatoshis: 50,
                        fee_zatoshis: 10,
                        deadline: Some(10_000),
                    }),
                )
                .unwrap();
            let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
            db.execute(
                "UPDATE funding_jobs SET state='COMPLETE' WHERE id=?1",
                [&job],
            )
            .unwrap();
            db.execute("UPDATE funding_progress SET phase='\"COMPLETE\"',last_error='settlement_unverified' WHERE job_id=?1", [&job]).unwrap();
            state.fill_funding_vacancies("retirement").unwrap();
            let still_bound: String = db
                .query_row(
                    "SELECT wallet_id FROM reserve_claims WHERE pool_id=?1 AND slot='large'",
                    [&group],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                still_bound, wallet,
                "pending original source must block reserve replacement"
            );
            state.confirm_spend(&original.operation_id).unwrap();
            state.fill_funding_vacancies("retirement").unwrap();
            let replacement: String = db
                .query_row(
                    "SELECT wallet_id FROM reserve_claims WHERE pool_id=?1 AND slot='large'",
                    [&group],
                    |r| r.get(0),
                )
                .unwrap();
            assert_ne!(replacement, wallet);
            assert_eq!(
                state
                    .prepared_bytes(&original.operation_id)
                    .unwrap()
                    .as_slice(),
                b"private-fixture-signed-bytes"
            );
        }
        // Pausing ordinary funding grants no new transfer authority and must not
        // disable separately configured retirement authority.
        state.pause_new_funding();
        let (store, worker) = StoreHandle::spawn(state);
        let pool = Arc::new(
            ManagedPool::new(
                store.clone(),
                group.clone(),
                base,
                crate::payment::SpendPolicy {
                    max_atomic: Some(U256::from(100)),
                },
            )
            .unwrap(),
        );
        {
            let mut state = rpc.state.lock().unwrap();
            state.head = 200;
            state.confirmed = 0;
            state.latest = 0;
            state.scalar_calls = 0;
            state.balance_calls = 0;
        }
        Self {
            dir,
            wallet,
            pool,
            store,
            worker,
            server,
            rpc,
        }
    }
    async fn empty() -> Self {
        let (rpc, base, server) = rpc_fixture().await;
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
                "retirement",
                &ReservesConfig {
                    small: ReserveConfig {
                        funding_amount_usdc: "0.000001".into(),
                        max_funding_amount_usdc: None,
                    },
                    large: None,
                },
                Some(U256::from(100)),
                1_000,
            )
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let pool = Arc::new(
            ManagedPool::new(
                store.clone(),
                group,
                base,
                crate::payment::SpendPolicy {
                    max_atomic: Some(U256::from(100)),
                },
            )
            .unwrap(),
        );
        Self {
            dir,
            wallet: String::new(),
            pool,
            store,
            worker,
            server,
            rpc,
        }
    }
    async fn reopen(self) -> Self {
        let Self {
            dir,
            wallet,
            pool,
            store,
            worker,
            server,
            rpc,
        } = self;
        let id = store.call(|s| Ok(s.id().to_owned())).await.unwrap();
        let group = pool.pool.clone();
        let base = pool.base.clone();
        drop(pool);
        drop(store);
        worker.await.unwrap();
        let mut state =
            Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        state.pause_new_funding();
        let (store, worker) = StoreHandle::spawn(state);
        let pool = Arc::new(
            ManagedPool::new(
                store.clone(),
                group,
                base,
                crate::payment::SpendPolicy {
                    max_atomic: Some(U256::from(100)),
                },
            )
            .unwrap(),
        );
        Self {
            dir,
            wallet,
            pool,
            store,
            worker,
            server,
            rpc,
        }
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.path().join("state/state.sqlite")).unwrap()
    }
    fn role(&self) -> String {
        self.db()
            .query_row(
                "SELECT role FROM wallets WHERE id=?1",
                [&self.wallet],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn counts(&self) -> (usize, usize, usize) {
        let state = self.rpc.state.lock().unwrap();
        (state.scalar_calls, state.balance_calls, state.nonce_calls)
    }
    async fn close(self) {
        drop(self.pool);
        drop(self.store);
        self.worker.await.unwrap();
        self.server.abort();
    }
}
async fn rpc_fixture() -> (Arc<Rpc>, BaseRpc, tokio::task::JoinHandle<()>) {
    let rpc = Arc::new(Rpc {
        state: Mutex::new(RpcState {
            head: 100,
            confirmed: 5_000,
            latest: 5_000,
            timestamp: now().unwrap(),
            used: BTreeSet::new(),
            scalar_calls: 0,
            scalar_latest: None,
            balance_calls: 0,
            nonce_calls: 0,
            block_nonce: false,
            block_latest_balance: false,
            fail: false,
            total_calls: 0,
        }),
        started: Notify::new(),
        release: Semaphore::new(0),
    });
    let router = Router::new()
        .route("/", post(handle))
        .with_state(rpc.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (rpc, base, server)
}
async fn handle(State(rpc): State<Arc<Rpc>>, Json(request): Json<Value>) -> Json<Value> {
    let fail = {
        let mut state = rpc.state.lock().unwrap();
        state.total_calls += 1;
        state.fail
    };
    if fail {
        return Json(
            json!({"jsonrpc":"2.0", "id":request["id"], "error":{"code":-32000,"message":"synthetic retirement proof unavailable"}}),
        );
    }
    let (result, blocked) = {
        let mut state = rpc.state.lock().unwrap();
        match request["method"].as_str().unwrap() {
            "eth_chainId" => (json!("0x2105"), false),
            "eth_getBlockByNumber" => {
                let tag = request["params"][0].as_str().unwrap();
                let height = if matches!(tag, "latest" | "finalized") {
                    state.head
                } else {
                    u64::from_str_radix(&tag[2..], 16).unwrap()
                };
                (
                    json!({"number":format!("0x{height:x}"), "hash":format!("0x{height:064x}"), "timestamp":format!("0x{:x}",state.timestamp)}),
                    false,
                )
            }
            "eth_call" => {
                let data = request["params"][0]["data"].as_str().unwrap();
                let amount = if data.starts_with("0x70a08231") {
                    if request["params"][1] == "latest" {
                        state.scalar_calls += 1;
                        state.scalar_latest.unwrap_or(state.latest)
                    } else {
                        state.balance_calls += 1;
                        assert_eq!(request["params"][1]["requireCanonical"], true);
                        let hash = request["params"][1]["blockHash"].as_str().unwrap();
                        let height = u64::from_str_radix(&hash[2..], 16).unwrap();
                        if height == state.head {
                            state.latest
                        } else if state.head > 200 && height == 188 {
                            1_001
                        } else {
                            state.confirmed
                        }
                    }
                } else {
                    assert_eq!(
                        &data[..10],
                        format!(
                            "0x{}",
                            alloy_primitives::hex::encode(
                                &keccak256("authorizationState(address,bytes32)")[..4]
                            )
                        ),
                        "only USDC authorizationState proofs are allowed"
                    );
                    state.nonce_calls += 1;
                    let nonce = u64::from_str_radix(&data[data.len() - 16..], 16).unwrap();
                    u64::from(state.used.contains(&nonce))
                };
                let blocked = if data.starts_with("0x70a08231") {
                    state.block_latest_balance
                        && request["params"][1]["blockHash"] == format!("0x{:064x}", state.head)
                } else {
                    state.block_nonce
                };
                (json!(format!("0x{amount:064x}")), blocked)
            }
            method => panic!("unexpected mutation or unrelated RPC: {method}"),
        }
    };
    if blocked {
        rpc.started.notify_one();
        rpc.release.acquire().await.unwrap().forget();
    }
    Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
}

#[tokio::test]
async fn ordinary_reconciliation_retires_at_inclusive_group_threshold_without_funding() {
    for (tolerance, confirmed, latest, retired) in [
        (1_000, 999, 999, true),
        (1_000, 1_000, 1_000, true),
        (1_000, 1_001, 1_000, false),
        (1_000, 1_000, 1_001, false),
        (0, 0, 0, true),
        (0, 0, 1, false),
        (0, 1, 0, false),
    ] {
        let f = Fixture::new(tolerance, true, true, 0).await;
        {
            let mut state = f.rpc.state.lock().unwrap();
            state.confirmed = confirmed;
            state.latest = latest;
        }
        let result = f.pool.reconcile().await;
        if retired {
            result.unwrap();
        }
        assert_eq!(
            f.role(),
            if retired { "RETIRED" } else { "RETAINED" },
            "confirmed={confirmed} latest={latest} tolerance={tolerance}"
        );
        let count: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "retirement cannot allocate a replacement deposit");
        if retired {
            let evidence: (String, String, String) = f.db().query_row("SELECT confirmed_amount,latest_amount,accepted_maximum FROM wallet_retirements WHERE wallet_id=?1", [&f.wallet], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(
                evidence,
                (
                    confirmed.to_string(),
                    latest.to_string(),
                    tolerance.to_string()
                )
            );
        }
        f.close().await;
    }
}

#[tokio::test]
async fn ordinary_retirement_resolves_more_than_one_authorization_page_without_replay() {
    let f = Fixture::new(1_000, true, true, 66).await;
    {
        let mut state = f.rpc.state.lock().unwrap();
        state.timestamp = now().unwrap();
        state.used = (1..=66).filter(|n| n % 2 == 0).collect();
    }
    // Model previously accepted authorizations whose signed intervals are now
    // past. Only the RPC confirmed timestamp can prove their non-use final.
    f.db()
        .execute(
            "UPDATE payment_attempts SET valid_before=?1",
            [i64::try_from(now().unwrap() - 10).unwrap()],
        )
        .unwrap();
    f.pool.reconcile().await.unwrap();
    assert_eq!(f.role(), "RETIRED");
    let rows: (i64, i64) = f
        .db()
        .query_row(
            "SELECT SUM(outcome='USED'),SUM(outcome='EXPIRED_UNUSED') FROM payment_resolutions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(rows, (33, 33));
    assert_eq!(f.counts().2, 66);
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        66
    );
    f.close().await;
}

#[tokio::test]
async fn still_valid_unused_authorization_remains_owned_after_finite_sync() {
    let f = Fixture::new(1_000, true, true, 1).await;
    assert!(
        f.pool.reconcile().await.is_err(),
        "finite sync must report unresolved original authority"
    );
    assert_eq!(f.role(), "RETAINED");
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM payment_resolutions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    f.close().await;
}

#[tokio::test]
async fn retirement_never_probes_uncredited_or_enabled_reserve_wallets() {
    for (credited, retained) in [(false, true), (true, false)] {
        let f = Fixture::new(1_000, credited, retained, 0).await;
        let _ = f.pool.reconcile().await;
        assert_ne!(f.role(), "RETIRED");
        assert_eq!(
            f.counts().1,
            0,
            "ineligible wallet must not request canonical retirement balances"
        );
        assert_eq!(f.counts().2, 0);
        f.close().await;
    }
}

#[tokio::test]
async fn disconnected_sync_waiter_does_not_cancel_accepted_authorization_proof() {
    let f = Fixture::new(1_000, true, true, 1).await;
    {
        let mut state = f.rpc.state.lock().unwrap();
        state.used.insert(1);
        state.block_nonce = true;
    }
    let waiting = {
        let pool = f.pool.clone();
        tokio::spawn(async move { pool.reconcile().await })
    };
    f.rpc.started.notified().await;
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    assert!(
        f.pool.reconcile().await.is_err(),
        "original reconciliation owner must remain held"
    );
    assert_eq!(f.counts().2, 1);
    f.rpc.state.lock().unwrap().block_nonce = false;
    f.rpc.release.add_permits(1);
    while f.role() != "RETIRED" {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        f.counts().2,
        1,
        "disconnected waiter must not duplicate active proof I/O"
    );
    f.close().await;
}

#[tokio::test]
async fn latest_above_tolerance_finishes_low_saved_minimum_work_across_idle_rescans() {
    let f = Fixture::new(1_000, true, true, 0).await;
    {
        let mut rpc = f.rpc.state.lock().unwrap();
        // Credit arrives between the startup scalar and its canonical stage.
        rpc.scalar_latest = Some(0);
        rpc.latest = 1_001;
    }
    let stop = tokio_util::sync::CancellationToken::new();
    let serving = {
        let pool = f.pool.clone();
        let stopped = stop.clone();
        tokio::spawn(async move { pool.run_reconciliation(stopped).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let wallet = f.wallet.clone();
            let ready = f
                .store
                .call(move |s| {
                    Ok(s.status()?.pools[0]
                        .addresses
                        .iter()
                        .find(|row| row.id == wallet)
                        .unwrap()
                        .runtime_balance_evidence
                        .as_ref()
                        .is_some_and(|proof| proof.latest.amount_atomic == "1001"))
                })
                .await
                .unwrap();
            if ready {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.role(), "RETAINED");
    let saved: String = f
        .db()
        .query_row(
            "SELECT balance FROM wallets WHERE id=?1",
            [&f.wallet],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, "0", "canonical minimum semantics remain unchanged");
    let counts = f.counts();
    assert_eq!(counts.0, 1);
    assert!(counts.1 >= 2);
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| {
            let status = s.status()?;
            let endpoint = status.pools[0]
                .addresses
                .iter()
                .find(|row| row.id == wallet)
                .unwrap()
                .runtime_balance_evidence
                .as_ref()
                .unwrap()
                .endpoint
                .clone();
            s.invalidate_endpoint_evidence(&endpoint)?;
            assert!(
                s.status()?.pools[0]
                    .addresses
                    .iter()
                    .find(|row| row.id == wallet)
                    .unwrap()
                    .runtime_balance_evidence
                    .is_none()
            );
            Ok(())
        })
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    for _ in 0..3 {
        let group = f.pool.pool.clone();
        f.store
            .call(move |s| {
                s.observation_page(&group, -1)?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    }
    assert_eq!(
        f.counts(),
        counts,
        "serving, capacity rescans and certificate loss cannot resurrect completed work"
    );
    stop.cancel();
    serving.await.unwrap().unwrap();
    f.close().await;
}

#[tokio::test]
async fn confirmed_above_tolerance_keeps_confirmation_progress_under_existing_cadence() {
    let f = Fixture::new(1_000, true, true, 0).await;
    f.rpc.state.lock().unwrap().confirmed = 1_001;
    assert!(f.pool.reconcile().await.is_err());
    assert_eq!(f.role(), "RETAINED");
    let counts = f.counts();
    assert!(counts.1 >= 2);
    let wallet = f.wallet.clone();
    let due = f
        .store
        .call(move |s| Ok(s.retirement_due(&wallet)))
        .await
        .unwrap()
        .expect("confirmation progress must remain pending");
    assert!(due >= tokio::time::Instant::now() + std::time::Duration::from_secs(59));
    assert!(!f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert_eq!(
        f.counts(),
        counts,
        "another caller cannot shorten the observation floor"
    );
    {
        let mut state = f.rpc.state.lock().unwrap();
        state.head = 300;
        state.confirmed = 0;
    }
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert_eq!(f.role(), "RETIRED");
    f.close().await;
}

#[tokio::test]
async fn disabled_group_and_changed_active_role_stop_ordinary_canonical_work() {
    for disable in [true, false] {
        let f = Fixture::new(1_000, true, true, 0).await;
        if disable {
            f.db().execute("UPDATE pools SET enabled=0", []).unwrap();
        } else {
            f.db()
                .execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1", [&f.wallet])
                .unwrap();
        }
        let _ = f.pool.reconcile().await;
        assert_ne!(f.role(), "RETIRED");
        assert_eq!(f.counts().1, 0);
        f.close().await;
    }
}

#[tokio::test]
async fn canonical_retry_cannot_replace_expired_scalar_owner_outcome_or_cooldown() {
    let f = Fixture::new(1_000, true, true, 0).await;
    f.rpc.state.lock().unwrap().confirmed = 1_001;
    assert!(f.pool.reconcile().await.is_err());
    let before = f.counts();
    assert_eq!(before.0, 1);
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    assert!(!f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert!(
        f.counts().1 > before.1,
        "due confirmation work must acquire another real proof"
    );
    let value = f
        .store
        .observe_balance(f.pool.pool.clone(), f.wallet.clone(), f.pool.base.clone())
        .await
        .unwrap();
    assert_eq!(value, U256::ZERO);
    assert_eq!(
        f.counts().0,
        before.0 + 1,
        "a canonical acquisition cannot manufacture a cached scalar result or delay a due scalar read"
    );
    f.close().await;
}

#[tokio::test]
async fn never_credited_replaced_predecessor_remains_retained_and_late_credit_can_pay() {
    let f = Fixture::with_predecessor(1_000, false, false, 0, true).await;
    assert_eq!(f.role(), "RETAINED");
    let claim: (String, i64) = f
        .db()
        .query_row(
            "SELECT wallet_id,generation FROM reserve_claims WHERE slot='large'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_ne!(claim.0, f.wallet);
    let operations = f
        .db()
        .query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
        .unwrap();
    f.pool.reconcile().await.unwrap();
    assert_eq!(f.role(), "RETAINED");
    assert_eq!(
        f.counts().1,
        0,
        "never-credited predecessor cannot demand useless canonical balance proofs"
    );
    let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
    f.store
        .call(move |s| s.finish_balance(&group, &wallet, Some(U256::from(2_000))))
        .await
        .unwrap();
    let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
    f.store
        .call(move |s| {
            let address = s.status()?.pools[0]
                .addresses
                .iter()
                .find(|a| a.id == wallet)
                .unwrap()
                .address
                .clone();
            s.prepare_advisory_payment(
                &group,
                U256::from(1),
                "late-credit",
                &PaymentCandidateHandle { wallet, address },
            )
            .map(|_| ())
        })
        .await
        .unwrap();
    assert_eq!(f.role(), "RETAINED");
    let after: (String, i64) = f
        .db()
        .query_row(
            "SELECT wallet_id,generation FROM reserve_claims WHERE slot='large'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        after, claim,
        "late credit must not reclaim the replacement's generation"
    );
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        operations
    );
    f.close().await;
}

#[tokio::test]
async fn fresh_scalar_demand_or_active_role_during_nonce_proof_prevents_stale_retirement() {
    for active in [false, true] {
        let f = Fixture::new(1_000, true, true, 1).await;
        {
            let mut state = f.rpc.state.lock().unwrap();
            state.used.insert(1);
            state.block_nonce = true;
        }
        let pending = {
            let pool = f.pool.clone();
            tokio::spawn(async move { pool.reconcile().await })
        };
        f.rpc.started.notified().await;
        if active {
            f.db()
                .execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1", [&f.wallet])
                .unwrap();
        } else {
            let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
            f.store
                .call(move |s| s.finish_balance(&group, &wallet, Some(U256::from(1))))
                .await
                .unwrap();
        }
        f.rpc.state.lock().unwrap().block_nonce = false;
        f.rpc.release.add_permits(1);
        let result = pending.await.unwrap();
        assert_eq!(f.role(), if active { "ACTIVE" } else { "RETAINED" });
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM wallet_retirements", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        if !active {
            assert!(
                result.is_err(),
                "new scalar demand must remain visible to finite sync"
            );
            let wallet = f.wallet.clone();
            assert!(
                f.store
                    .call(move |s| Ok(s.retirement_requested(&wallet)))
                    .await
                    .unwrap()
            );
            let saved: String = f
                .db()
                .query_row(
                    "SELECT balance FROM wallets WHERE id=?1",
                    [&f.wallet],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                saved, "1",
                "stale canonical publication must not overwrite the newer scalar"
            );
        }
        f.close().await;
    }
}

#[tokio::test]
async fn malformed_original_nonce_stops_only_its_retirement_request_without_repeated_proofs() {
    let f = Fixture::new(1_000, true, true, 1).await;
    f.db()
        .execute(
            "UPDATE payment_attempts SET nonce='private-malformed-nonce'",
            [],
        )
        .unwrap();
    assert!(f.pool.reconcile().await.is_err());
    assert_eq!(f.role(), "RETAINED");
    let before = f.counts();
    assert_eq!(before.1, 0);
    assert_eq!(before.2, 0);
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    for _ in 0..3 {
        let group = f.pool.pool.clone();
        f.store
            .call(move |s| {
                let through = s.wallet_boundary(&group)?;
                s.retirement_page(&group, -1, through).map(|_| ())
            })
            .await
            .unwrap();
        assert!(f.pool.retire_wallet(f.wallet.clone()).await.is_err());
    }
    assert_eq!(f.counts(), before);
    assert_eq!(
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    f.close().await;
}

#[test]
fn zero_and_dust_retirement_diagnostics_are_safe_visible_lifecycle_events() {
    if crate::test_log_capture::run_in_child(
        "rotation::manager::retirement::tests::zero_and_dust_retirement_diagnostics_are_safe_visible_lifecycle_events",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(log.reopen().unwrap())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for remainder in [0, 1_000] {
                let f = Fixture::new(1_000, true, true, 0).await;
                {
                    let mut state = f.rpc.state.lock().unwrap();
                    state.latest = remainder;
                    state.confirmed = remainder;
                }
                f.pool.reconcile().await.unwrap();
                assert_eq!(f.role(), "RETIRED");
                f.close().await;
            }
        });
    let logged = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(
        logged.matches("Retained wallet retired").count(),
        2,
        "{logged}"
    );
    for expected in [
        "INFO",
        "remainder_atomic=0",
        "remainder_atomic=1000",
        "tolerance_atomic=1000",
        "remaining USDC stays owned",
        "pool=",
        "wallet=",
    ] {
        assert!(logged.contains(expected), "missing {expected}: {logged}");
    }
    for forbidden in [
        "WARN",
        "ERROR",
        "private-fixture-signed-bytes",
        "private-malformed-nonce",
        "b\"fixture\"",
        "spent",
    ] {
        assert!(
            !logged.contains(forbidden),
            "unexpected {forbidden}: {logged}"
        );
    }
}

#[test]
fn stopped_records_warn_once_with_safe_cause_while_proof_retries_remain_recoverable() {
    if crate::test_log_capture::run_in_child(
        "rotation::manager::retirement::tests::stopped_records_warn_once_with_safe_cause_while_proof_retries_remain_recoverable",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(log.reopen().unwrap())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            // Derived checkpoint damage rebuilds from canonical evidence without
            // an operator stop or warning; immutable original damage below does not.
            let f = Fixture::new(1_000, true, true, 0).await;
            let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
            f.store
                .call(move |s| s.finish_balance(&group, &wallet, Some(U256::ZERO)))
                .await
                .unwrap();
            f.db()
                .execute(
                    "UPDATE wallets SET financial_height=1,financial_hash=NULL WHERE id=?1",
                    [&f.wallet],
                )
                .unwrap();
            assert!(f.pool.reconcile().await.is_ok());
            assert_eq!(f.role(), "RETIRED");
            f.close().await;
            let repaired = std::fs::read_to_string(log.path()).unwrap();
            assert!(!repaired.contains("WARN"), "{repaired}");
            assert!(!repaired.contains("ERROR"), "{repaired}");

            for corrupt in ["nonce", "column", "tolerance"] {
                let f = Fixture::new(
                    1_000,
                    true,
                    true,
                    usize::from(matches!(corrupt, "nonce" | "column")),
                )
                .await;
                if corrupt == "nonce" {
                    f.db()
                        .execute(
                            "UPDATE payment_attempts SET nonce='private-malformed-nonce'",
                            [],
                        )
                        .unwrap();
                } else if corrupt == "column" {
                    f.db()
                        .execute(
                            "UPDATE payment_attempts SET valid_before='private-invalid-column'",
                            [],
                        )
                        .unwrap();
                } else {
                    f.db()
                        .execute(
                            "UPDATE pools SET retirement_tolerance='private-invalid-tolerance'",
                            [],
                        )
                        .unwrap();
                }
                assert!(f.pool.reconcile().await.is_err());
                for _ in 0..3 {
                    assert!(f.pool.retire_wallet(f.wallet.clone()).await.is_err());
                }
                f.close().await;
            }
            for latest in [0, 1_001] {
                let f = Fixture::new(1_000, true, true, 0).await;
                let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
                f.store
                    .call(move |s| s.finish_balance(&group, &wallet, Some(U256::ZERO)))
                    .await
                    .unwrap();
                f.rpc.state.lock().unwrap().fail = true;
                assert!(f.pool.retire_wallet(f.wallet.clone()).await.is_err());
                tokio::time::pause();
                tokio::time::advance(std::time::Duration::from_secs(61)).await;
                assert!(f.pool.retire_wallet(f.wallet.clone()).await.is_err());
                {
                    let mut rpc = f.rpc.state.lock().unwrap();
                    rpc.fail = false;
                    rpc.latest = latest;
                }
                tokio::time::advance(std::time::Duration::from_secs(121)).await;
                assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
                assert_eq!(f.role(), if latest == 0 { "RETIRED" } else { "RETAINED" });
                let calls = f.rpc.state.lock().unwrap().total_calls;
                assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
                assert_eq!(f.rpc.state.lock().unwrap().total_calls, calls);
                let logged = std::fs::read_to_string(log.path()).unwrap();
                let own: Vec<_> = logged
                    .lines()
                    .filter(|line| line.contains(&f.wallet))
                    .collect();
                assert_eq!(
                    own.iter()
                        .filter(|line| line.contains("Wallet retirement is deferred"))
                        .count(),
                    1
                );
                assert_eq!(
                    own.iter()
                        .filter(|line| line.contains("Retained wallet retired"))
                        .count(),
                    usize::from(latest == 0)
                );
                assert_eq!(
                    own.iter()
                        .filter(
                            |line| line.contains("Wallet retirement check completed after a delay")
                        )
                        .count(),
                    usize::from(latest > 0)
                );
                f.close().await;
                tokio::time::resume();
            }
        });
    let logged = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(logged.matches("WARN").count(), 3, "{logged}");
    assert_eq!(
        logged
            .matches("Wallet retirement stopped because stored records need inspection")
            .count(),
        2,
        "{logged}"
    );
    assert_eq!(
        logged.matches("Wallet retirement is deferred").count(),
        2,
        "{logged}"
    );
    assert_eq!(
        logged.matches("Retained wallet retired").count(),
        2,
        "{logged}"
    );
    assert_eq!(
        logged
            .matches("Wallet retirement check completed after a delay")
            .count(),
        1,
        "{logged}"
    );
    for expected in [
        "stored payment authorization",
        "stored retirement tolerance",
        "stored wallet record has an invalid column type",
        "Wallet retirement check completed after a delay",
        "pool=",
        "wallet=",
    ] {
        assert!(logged.contains(expected), "missing {expected}: {logged}");
    }
    for forbidden in [
        "ERROR",
        "private-malformed-nonce",
        "private-invalid-column",
        "private-invalid-tolerance",
        "synthetic retirement proof unavailable",
        "private-fixture-signed-bytes",
        "unclassified validation error",
    ] {
        assert!(
            !logged.contains(forbidden),
            "unexpected {forbidden}: {logged}"
        );
    }
}

#[tokio::test]
async fn empty_serving_start_discovers_later_retained_candidate_without_scalar_demand() {
    let mut f = Fixture::empty().await;
    let started = Arc::new(Notify::new());
    let notify = started.clone();
    Arc::get_mut(&mut f.pool).unwrap().reconciliation_test_hook =
        Some(Arc::new(move || notify.notify_one()));
    let stop = tokio_util::sync::CancellationToken::new();
    let serving = {
        let pool = f.pool.clone();
        let stopped = stop.clone();
        tokio::spawn(async move { pool.run_reconciliation(stopped).await })
    };
    started.notified().await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(f.counts(), (0, 0, 0));
    let group = f.pool.pool.clone();
    let (wallet, query) = f
        .store
        .call(move |s| {
            let reserve = ReserveConfig {
                funding_amount_usdc: "0.000001".into(),
                max_funding_amount_usdc: None,
            };
            s.configure_capacity_group(
                &group,
                Some(U256::from(100)),
                1_000,
                &ReservesConfig {
                    small: reserve.clone(),
                    large: Some(reserve),
                },
            )?;
            let job = s
                .allocate_capacity_kind(&group, AllocationKind::Large, true)?
                .unwrap();
            let wallet = s.funding_job(&job)?.unwrap().wallet_id;
            let query = s.scoped_chain_query(&group, vec![wallet.clone()], vec![])?;
            Ok((wallet, query))
        })
        .await
        .unwrap();
    f.wallet = wallet.clone();
    let view = f.pool.base.view(query).await.unwrap();
    f.store
        .call(move |s| s.record_verified_credit(&wallet, view).map(|_| ()))
        .await
        .unwrap();
    {
        let mut state = f.rpc.state.lock().unwrap();
        state.head = 200;
        state.latest = 0;
        state.confirmed = 0;
    }
    let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
    f.store
        .call(move |s| {
            // Observe the low amount while READY, then release the optional slot.
            // There is no requested scalar refresh to accidentally wake retirement.
            s.finish_balance(&group, &wallet, Some(U256::ZERO))?;
            s.configure_capacity_group(
                &group,
                Some(U256::from(100)),
                1_000,
                &ReservesConfig {
                    small: ReserveConfig {
                        funding_amount_usdc: "0.000001".into(),
                        max_funding_amount_usdc: None,
                    },
                    large: None,
                },
            )
        })
        .await
        .unwrap();
    let flags: (i64, i64) = f
        .db()
        .query_row(
            "SELECT refresh_pending,low_balance FROM wallets WHERE id=?1",
            [&f.wallet],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(flags, (0, 0));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while f.role() != "RETIRED" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.counts().0,
        0,
        "the finished empty startup pass cannot extend into new scalar inventory"
    );
    stop.cancel();
    serving.await.unwrap().unwrap();
    f.close().await;
}

#[tokio::test]
async fn restart_rederives_unfinished_original_authorization_then_preserves_exact_retirement() {
    let f = Fixture::new(1_000, true, true, 1).await;
    assert!(f.pool.reconcile().await.is_err());
    assert_eq!(f.role(), "RETAINED");
    let attempt: (String, String, String, i64) = f
        .db()
        .query_row(
            "SELECT id,payer,nonce,valid_before FROM payment_attempts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    let f = f.reopen().await;
    f.rpc.state.lock().unwrap().used.insert(1);
    f.pool.reconcile().await.unwrap();
    assert_eq!(f.role(), "RETIRED");
    let after: (String, String, String, i64) = f
        .db()
        .query_row(
            "SELECT id,payer,nonce,valid_before FROM payment_attempts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        after, attempt,
        "restart never replays or rewrites the original authority"
    );
    let support: String = f
        .db()
        .query_row(
            "SELECT support FROM wallet_retirements WHERE wallet_id=?1",
            [&f.wallet],
            |r| r.get(0),
        )
        .unwrap();
    let retired: (String,String,String) = f.db().query_row("SELECT accepted_maximum,observed_bound,reason FROM wallet_retirements WHERE wallet_id=?1", [&f.wallet], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let counts = f.counts();
    let f = f.reopen().await;
    f.pool.reconcile().await.unwrap();
    assert_eq!(f.role(), "RETIRED");
    assert_eq!(
        f.counts(),
        counts,
        "retired history creates no scalar or canonical discovery after restart"
    );
    assert_eq!(
        f.db()
            .query_row(
                "SELECT support FROM wallet_retirements WHERE wallet_id=?1",
                [&f.wallet],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        support
    );
    assert_eq!(retired, ("1000".into(), "0".into(), "automatic".into()));
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    f.close().await;
}

#[tokio::test]
async fn accepted_current_and_archived_source_calculations_block_retirement_after_credit() {
    for archived in [false, true] {
        let f = Fixture::with_accepted_source(1_000, true, true, 0, false, true).await;
        let (job, operation): (String, String) = f
            .db()
            .query_row(
                "SELECT job_id,operation_id FROM funding_progress",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        if archived {
            f.db().execute("INSERT INTO funding_recovery(operation_id,job_id,phase) VALUES(?1,?2,'\"COMPLETE\"')", rusqlite::params![operation,job]).unwrap();
            f.db().execute("UPDATE funding_progress SET operation_id='fresh-unstarted-source' WHERE job_id=?1", [&job]).unwrap();
        }
        let accepted: bool = f
            .db()
            .query_row(
                "SELECT preparation IS NOT NULL FROM operation_network WHERE operation_id=?1",
                [&operation],
                |r| r.get(0),
            )
            .unwrap();
        assert!(accepted);
        f.pool.reconcile().await.unwrap();
        assert_eq!(f.role(), "RETAINED");
        assert_eq!(
            f.counts().1,
            0,
            "accepted source input ownership blocks retirement proofs before signed bytes exist"
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM wallet_retirements", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let group = f.pool.pool.clone();
        let op = operation.clone();
        f.store
            .call(move |s| {
                // Complete the exact accepted source calculation; this is not a new
                // funding attempt, even when its job now points at a newer operation.
                let revision = s.snapshot()?.0;
                s.prepare_with_facts(
                    &op,
                    revision,
                    b"source-finished",
                    b"original-source-signed-bytes",
                    Some(crate::rotation::transaction::TransactionFacts {
                        txid: "original-source".into(),
                        expiry_height: 100,
                        amount_zatoshis: 50,
                        fee_zatoshis: 10,
                        deadline: Some(10_000),
                    }),
                )?;
                s.confirm_spend(&op)?;
                let through = s.wallet_boundary(&group)?;
                s.retirement_page(&group, -1, through).map(|_| ())
            })
            .await
            .unwrap();
        assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
        assert_eq!(f.role(), "RETIRED");
        let op = operation.clone();
        assert_eq!(
            f.store
                .call(move |s| Ok(s.prepared_bytes(&op)?.to_vec()))
                .await
                .unwrap(),
            b"original-source-signed-bytes"
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        f.close().await;
    }
}

#[path = "recovery_tests.rs"]
mod recovery_tests;
