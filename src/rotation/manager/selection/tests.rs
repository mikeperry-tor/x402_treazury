//! Actual RPC certificates over temporary, unfunded role inventory. No fabricated
//! financial certificates or production fixture bypasses are used.
use super::*;
use crate::rotation::{
    config::{ReserveConfig, ReservesConfig},
    store::{AllocationKind, Store},
};
use axum::{Json, Router, routing::post};
use futures_util::FutureExt;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct Fixture {
    dir: tempfile::TempDir,
    manager: ManagedPool,
    bound: PaymentCandidateHandle,
    small: String,
    large: String,
    retained: Vec<String>,
    balances: Arc<StdMutex<BTreeMap<String, u64>>>,
    blocked: Arc<StdMutex<BTreeSet<String>>>,
    started: Arc<StdMutex<BTreeSet<String>>>,
    unavailable: Arc<AtomicBool>,
    worker: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.path().join("state/state.sqlite")).unwrap()
    }
    fn address(&self, wallet: &str) -> String {
        self.db()
            .query_row("SELECT address FROM wallets WHERE id=?1", [wallet], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
            .to_lowercase()
    }
    fn balance(&self, wallet: &str, amount: u64) {
        self.balances
            .lock()
            .unwrap()
            .insert(self.address(wallet), amount);
    }
    fn block(&self, wallet: &str) {
        self.blocked.lock().unwrap().insert(self.address(wallet));
    }
    async fn close(self) {
        self.server.abort();
        drop(self.manager);
        self.worker.await.unwrap();
    }
}
async fn fixture(retained_count: usize) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let reserves = ReservesConfig {
        small: ReserveConfig {
            funding_amount_usdc: "0.000001".into(),
            max_funding_amount_usdc: None,
        },
        large: Some(ReserveConfig {
            funding_amount_usdc: "0.000010".into(),
            max_funding_amount_usdc: None,
        }),
    };
    let pool = store
        .ensure_capacity_group("selection", &reserves, Some(U256::from(100_000_000u64)), 0)
        .unwrap();
    let initial = store
        .allocate_capacity_kind(&pool, AllocationKind::Initial, true)
        .unwrap()
        .unwrap();
    let large_job = store
        .allocate_capacity_kind(&pool, AllocationKind::Large, true)
        .unwrap()
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let wallet_for = |job: &str| {
        db.query_row(
            "SELECT wallet_id FROM funding_jobs WHERE id=?1",
            [job],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
    };
    let mut retained = Vec::new();
    for _ in 0..retained_count {
        let job = store
            .allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let wallet = wallet_for(&job);
        db.execute("UPDATE wallets SET role='RETAINED' WHERE id=?1", [&wallet])
            .unwrap();
        retained.push(wallet);
        // Seed consumed historical claim ownership, never financial evidence.
        // All wallet keys/jobs were allocated normally and D comes from RPC below.
        db.execute("UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1 WHERE pool_id=?1 AND slot='small'",[&pool]).unwrap();
    }
    let small_job = store
        .allocate_capacity_kind(&pool, AllocationKind::Small, true)
        .unwrap()
        .unwrap();
    let wallets = [
        wallet_for(&initial),
        wallet_for(&small_job),
        wallet_for(&large_job),
    ];
    let bound = PaymentCandidateHandle {
        wallet: wallets[0].clone(),
        address: db
            .query_row(
                "SELECT address FROM wallets WHERE id=?1",
                [&wallets[0]],
                |r| r.get(0),
            )
            .unwrap(),
        generation: 0,
    };
    drop(db);
    let balances = Arc::new(StdMutex::new(BTreeMap::<String, u64>::new()));
    let blocked = Arc::new(StdMutex::new(BTreeSet::<String>::new()));
    let started = Arc::new(StdMutex::new(BTreeSet::<String>::new()));
    let release = Arc::new(tokio::sync::Notify::new());
    let unavailable = Arc::new(AtomicBool::new(false));
    let initializing = Arc::new(AtomicBool::new(true));
    let app=Router::new().route("/",post({
        let (balances,blocked,started,release,unavailable,initializing)=(balances.clone(),blocked.clone(),started.clone(),release.clone(),unavailable.clone(),initializing.clone());
        move |Json(v):Json<Value>| {
            let (balances,blocked,started,release,unavailable,initializing)=(balances.clone(),blocked.clone(),started.clone(),release.clone(),unavailable.clone(),initializing.clone());
            async move {
                if unavailable.load(Ordering::SeqCst) { return Json(json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32602,"message":"fixture unavailable"}})); }
                let result=match v["method"].as_str().unwrap() {
                    "eth_chainId"=>json!("0x2105"),
                    "eth_getBlockByNumber"=>{
                        let tag=v["params"][0].as_str().unwrap();
                        let height=if tag=="latest"{if initializing.load(Ordering::SeqCst){100}else{200}}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
                    },
                    "eth_call"=>{
                        let data=v["params"][0]["data"].as_str().unwrap();
                        assert!(data.starts_with("0x70a08231"),"selection must not start historical scans or writes");
                        let address=format!("0x{}",&data[data.len()-40..]).to_lowercase();
                        started.lock().unwrap().insert(address.clone());
                        while blocked.lock().unwrap().contains(&address) { release.notified().await; }
                        // Hold fast reads until the blocked lanes have actually
                        // started, making independence an observed assertion.
                        loop {
                            let all_started = blocked.lock().unwrap().is_subset(&started.lock().unwrap());
                            if all_started { break; }
                            tokio::task::yield_now().await;
                        }
                        let value=if initializing.load(Ordering::SeqCst){100}else{*balances.lock().unwrap().get(&address).unwrap_or(&0)};
                        json!(format!("0x{value:064x}"))
                    },
                    method=>panic!("unexpected selection RPC {method}"),
                };
                Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
            }
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
    for wallet in wallets.iter().chain(&retained) {
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let view = base.view(query).await.unwrap();
        store.record_verified_credit(wallet, view).unwrap();
    }
    let treasury = store.id().to_owned();
    drop(store);
    let store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury,
    )
    .unwrap();
    initializing.store(false, Ordering::SeqCst);
    started.lock().unwrap().clear();
    let (store, worker) = StoreHandle::spawn(store);
    let manager =
        ManagedPool::new(store, pool, base, SpendPolicy::dollars("100").unwrap()).unwrap();
    Fixture {
        dir,
        manager,
        bound,
        small: wallets[1].clone(),
        large: wallets[2].clone(),
        retained,
        balances,
        blocked,
        started,
        unavailable,
        worker,
        server,
    }
}

async fn select(f: &Fixture) -> CandidatePassResult {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.manager.acquire_candidate(&f.bound, U256::from(5)),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn blocked_bound_and_large_do_not_delay_small_or_retained() {
    for retained_pays in [false, true] {
        let f = fixture(1).await;
        f.block(&f.bound.wallet);
        f.block(&f.large);
        let expected = if retained_pays {
            &f.retained[0]
        } else {
            &f.small
        };
        f.balance(expected, 10);
        let CandidatePassResult::Selected(selected) = select(&f).await else {
            panic!("independent candidate must complete")
        };
        assert_eq!(&selected.candidate.wallet, expected);
        assert!(
            f.started
                .lock()
                .unwrap()
                .contains(&f.bound.address.to_lowercase())
        );
        assert!(f.started.lock().unwrap().contains(&f.address(&f.large)));
        drop(selected);
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        f.close().await;
    }
}

#[tokio::test]
async fn pass_crosses_keyset_pages_without_inventory_limit() {
    let f = fixture(PAGE_SIZE + 9).await;
    let last = f.retained.last().unwrap();
    f.balance(last, 10);
    let CandidatePassResult::Selected(selected) = select(&f).await else {
        panic!("last page must be visited")
    };
    assert_eq!(&selected.candidate.wallet, last);
    drop(selected);
    f.close().await;
}

#[tokio::test]
async fn unavailable_pass_is_not_a_complete_negative() {
    let f = fixture(2).await;
    f.unavailable.store(true, Ordering::SeqCst);
    assert!(matches!(select(&f).await, CandidatePassResult::Incomplete));
    f.unavailable.store(false, Ordering::SeqCst);
    assert!(matches!(
        select(&f).await,
        CandidatePassResult::CompleteNegative
    ));
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.close().await;
}

#[tokio::test]
async fn gated_selection_rechecks_current_liabilities_and_all_completed_candidates() {
    let f = fixture(1).await;
    f.balance(&f.bound.wallet, 10);
    f.balance(&f.small, 8);
    f.balance(&f.retained[0], 6);
    // Acquire actual opaque proofs first; a concurrent accepted liability then
    // makes the previously sufficient bound payer and best retained fit unusable.
    for wallet in [&f.bound.wallet, &f.small, &f.retained[0]] {
        assert!(matches!(
            f.manager
                .acquire_candidate_wallet(wallet.clone())
                .await
                .unwrap(),
            CandidateEvidenceState::Usable
        ));
    }
    let held = f.manager.gate.clone().lock_owned().await;
    let manager = f.manager.clone();
    let bound = f.bound.clone();
    let task = tokio::spawn(async move { manager.acquire_candidate(&bound, U256::from(5)).await });
    f.db()
        .execute(
            "UPDATE wallets SET unresolved_exposure='7' WHERE id=?1 OR id=?2",
            rusqlite::params![f.bound.wallet, f.retained[0]],
        )
        .unwrap();
    drop(held);
    let CandidatePassResult::Selected(selected) = task.await.unwrap().unwrap() else {
        panic!("ready candidate remains sufficient")
    };
    assert_eq!(selected.candidate.wallet, f.small);
    assert!(
        f.manager.gate.try_lock().is_err(),
        "selection must retain signing ownership"
    );
    drop(selected);
    f.close().await;
}

async fn cache_balances(f: &Fixture, skip: Option<&str>) {
    for wallet in [&f.bound.wallet, &f.small, &f.large]
        .into_iter()
        .chain(&f.retained)
        .filter(|wallet| Some(wallet.as_str()) != skip)
    {
        assert!(matches!(
            f.manager
                .acquire_candidate_wallet(wallet.clone())
                .await
                .unwrap(),
            CandidateEvidenceState::Usable
        ));
    }
    f.started.lock().unwrap().clear();
}

#[tokio::test]
async fn cached_negative_pass_batches_scans_without_reacquiring_balances() {
    let mut f = fixture(PAGE_SIZE + 9).await;
    cache_balances(&f, None).await;
    let scans = Arc::new(AtomicUsize::new(0));
    let evaluated = Arc::new(AtomicUsize::new(0));
    f.manager.test_admission_hook = Some(Arc::new({
        let (scans, evaluated) = (scans.clone(), evaluated.clone());
        move |stage| {
            match stage {
                AdmissionTestStage::CandidateScan => {
                    scans.fetch_add(1, Ordering::SeqCst);
                }
                AdmissionTestStage::CandidateEvaluated => {
                    evaluated.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
            std::future::ready(()).boxed()
        }
    }));
    assert!(matches!(
        select(&f).await,
        CandidatePassResult::CompleteNegative
    ));
    let wallets = f.retained.len() + 3;
    assert_eq!(evaluated.load(Ordering::SeqCst), wallets);
    assert!(scans.load(Ordering::SeqCst) <= 2 + wallets / PAGE_SIZE);
    assert!(
        f.started.lock().unwrap().is_empty(),
        "cached insufficient evidence must not trigger new RPC reads"
    );
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.close().await;
}

#[tokio::test]
async fn batched_scan_sees_independent_publication_before_reaching_that_wallet() {
    let mut f = fixture(PAGE_SIZE + 9).await;
    let target = f.retained.last().unwrap().clone();
    cache_balances(&f, Some(&target)).await;
    f.balance(&target, 10);
    let (paused, resume, evaluated) = pause_after_evaluation(&mut f, 1);
    let (manager, bound) = (f.manager.clone(), f.bound.clone());
    let task = tokio::spawn(async move { manager.acquire_candidate(&bound, U256::from(5)).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), paused.notified())
        .await
        .unwrap();
    // A different owner publishes while the foreground walks cached rows.
    assert!(matches!(
        f.manager
            .acquire_candidate_wallet(target.clone())
            .await
            .unwrap(),
        CandidateEvidenceState::Usable
    ));
    resume.notify_one();
    let CandidatePassResult::Selected(selected) =
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    else {
        panic!("independent completed evidence must be considered during traversal")
    };
    assert_eq!(selected.candidate.wallet, target);
    assert!(
        evaluated.load(Ordering::SeqCst) <= PAGE_SIZE,
        "do not wait until the target's inventory row or pass exhaustion"
    );
    assert!(f.manager.gate.try_lock().is_err());
    drop(selected);
    assert_eq!(
        *f.started.lock().unwrap(),
        BTreeSet::from([f.address(&target)])
    );
    f.close().await;
}

fn pause_after_evaluation(
    f: &mut Fixture,
    at: usize,
) -> (
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    Arc<AtomicUsize>,
) {
    let paused = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let evaluated = Arc::new(AtomicUsize::new(0));
    f.manager.test_admission_hook = Some(Arc::new({
        let (paused, resume, evaluated) = (paused.clone(), resume.clone(), evaluated.clone());
        move |stage| {
            let (paused, resume, evaluated) = (paused.clone(), resume.clone(), evaluated.clone());
            async move {
                if matches!(stage, AdmissionTestStage::CandidateEvaluated)
                    && evaluated.fetch_add(1, Ordering::SeqCst) + 1 == at
                {
                    paused.notify_one();
                    resume.notified().await;
                }
            }
            .boxed()
        }
    }));
    (paused, resume, evaluated)
}

#[tokio::test]
async fn independent_publication_is_selected_before_waiting_for_blocked_reads() {
    let mut f = fixture(2).await;
    for wallet in [&f.bound.wallet, &f.small, &f.large, &f.retained[0]] {
        f.block(wallet);
    }
    let target = f.retained[1].clone();
    f.balance(&target, 10);
    let (paused, resume, evaluated) = pause_after_evaluation(&mut f, READS);
    let (manager, bound) = (f.manager.clone(), f.bound.clone());
    let task = tokio::spawn(async move { manager.acquire_candidate(&bound, U256::from(5)).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), paused.notified())
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            f.manager.acquire_candidate_wallet(target.clone())
        )
        .await
        .unwrap()
        .unwrap(),
        CandidateEvidenceState::Usable
    ));
    resume.notify_one();
    let CandidatePassResult::Selected(selected) =
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    else {
        panic!("independent capacity must not wait for the four stalled acquisitions")
    };
    assert_eq!(selected.candidate.wallet, target);
    assert_eq!(evaluated.load(Ordering::SeqCst), READS);
    assert!(
        f.blocked
            .lock()
            .unwrap()
            .is_subset(&f.started.lock().unwrap())
    );
    drop(selected);
    f.close().await;
}
