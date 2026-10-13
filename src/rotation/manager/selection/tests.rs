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
    calls: Arc<StdMutex<BTreeMap<String, usize>>>,
    head: Arc<AtomicUsize>,
    blocked: Arc<StdMutex<BTreeSet<String>>>,
    started: Arc<StdMutex<BTreeSet<String>>>,
    unavailable: Arc<AtomicBool>,
    release: Arc<tokio::sync::Notify>,
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
        .allocate_capacity_kind(&pool, AllocationKind::Small, true)
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
    // Seed an already-consumed small claim for selection tests over established
    // inventory. Exact readiness below still comes from the real RPC fixture.
    db.execute(
        "UPDATE wallets SET role='ACTIVE' WHERE id=?1",
        [wallet_for(&initial)],
    )
    .unwrap();
    db.execute("UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1 WHERE pool_id=?1 AND slot='small'", [&pool]).unwrap();
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
    };
    drop(db);
    let balances = Arc::new(StdMutex::new(BTreeMap::<String, u64>::new()));
    let blocked = Arc::new(StdMutex::new(BTreeSet::<String>::new()));
    let started = Arc::new(StdMutex::new(BTreeSet::<String>::new()));
    let calls = Arc::new(StdMutex::new(BTreeMap::<String, usize>::new()));
    let head = Arc::new(AtomicUsize::new(200));
    let release = Arc::new(tokio::sync::Notify::new());
    let unavailable = Arc::new(AtomicBool::new(false));
    let initializing = Arc::new(AtomicBool::new(true));
    let timestamp = now().unwrap();
    let app=Router::new().route("/",post({
        let (balances,blocked,started,release,unavailable,initializing,calls,head)=(balances.clone(),blocked.clone(),started.clone(),release.clone(),unavailable.clone(),initializing.clone(),calls.clone(),head.clone());
        move |Json(v):Json<Value>| {
            let (balances,blocked,started,release,unavailable,initializing,calls,head)=(balances.clone(),blocked.clone(),started.clone(),release.clone(),unavailable.clone(),initializing.clone(),calls.clone(),head.clone());
            async move {
                if unavailable.load(Ordering::SeqCst) { return Json(json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32602,"message":"fixture unavailable"}})); }
                let result=match v["method"].as_str().unwrap() {
                    "eth_chainId"=>json!("0x2105"),
                    "eth_getBlockByNumber"=>{
                        let tag=v["params"][0].as_str().unwrap();
                        let height=if tag=="latest"{if initializing.load(Ordering::SeqCst){100}else{head.load(Ordering::SeqCst) as u64}}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{timestamp:x}")})
                    },
                    "eth_call"=>{
                        let data=v["params"][0]["data"].as_str().unwrap();
                        assert!(data.starts_with("0x70a08231"),"selection must not start historical scans or writes");
                        let address=format!("0x{}",&data[data.len()-40..]).to_lowercase();
                        started.lock().unwrap().insert(address.clone());
                        *calls.lock().unwrap().entry(address.clone()).or_default() += 1;
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
    calls.lock().unwrap().clear();
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
        calls,
        head,
        blocked,
        started,
        unavailable,
        release,
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
            (CandidateEvidenceState::Usable, _)
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
            (CandidateEvidenceState::Usable, _)
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
        (CandidateEvidenceState::Usable, _)
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
        (CandidateEvidenceState::Usable, _)
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

#[tokio::test]
async fn independent_publication_wakes_all_blocked_passes_without_new_acquisition() {
    let mut f = fixture(2).await;
    for wallet in [&f.bound.wallet, &f.small, &f.large, &f.retained[0]] {
        f.block(wallet);
    }
    let target = f.retained[1].clone();
    f.balance(&target, 10);
    let waiting = Arc::new(tokio::sync::Semaphore::new(0));
    let evaluated = Arc::new(AtomicUsize::new(0));
    f.manager.test_admission_hook = Some(Arc::new({
        let (waiting, evaluated) = (waiting.clone(), evaluated.clone());
        move |stage| {
            if matches!(stage, AdmissionTestStage::CandidateWaiting) {
                waiting.add_permits(1);
            }
            if matches!(stage, AdmissionTestStage::CandidateEvaluated) {
                evaluated.fetch_add(1, Ordering::SeqCst);
            }
            std::future::ready(()).boxed()
        }
    }));
    let launch = || {
        let (manager, bound) = (f.manager.clone(), f.bound.clone());
        tokio::spawn(async move {
            let CandidatePassResult::Selected(selected) = manager
                .acquire_candidate(&bound, U256::from(5))
                .await
                .unwrap()
            else {
                panic!("published candidate must wake selection")
            };
            selected.candidate.wallet.clone()
        })
    };
    let first = launch();
    let second = launch();
    tokio::time::timeout(std::time::Duration::from_secs(5), waiting.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    // Both passes have reached the wait; publication must also be safe if it
    // races the transition from the last scan into polling the notification.
    assert!(matches!(
        f.manager
            .acquire_candidate_wallet(target.clone())
            .await
            .unwrap(),
        (CandidateEvidenceState::Usable, _)
    ));
    for task in [first, second] {
        let wallet = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(wallet, target);
    }
    assert_eq!(evaluated.load(Ordering::SeqCst), 2 * READS);
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
async fn completed_sufficient_candidate_survives_cache_churn() {
    let mut f = fixture(260).await;
    f.balance(&f.bound.wallet, 10);
    let owner = f.manager.clone();
    let bound = f.bound.wallet.clone();
    let others: Vec<_> = [&f.small, &f.large]
        .into_iter()
        .chain(&f.retained)
        .cloned()
        .collect();
    let scans = Arc::new(AtomicUsize::new(0));
    f.manager.test_admission_hook = Some(Arc::new(move |stage| {
        let (owner, bound, others, scans) =
            (owner.clone(), bound.clone(), others.clone(), scans.clone());
        async move {
            if matches!(stage, AdmissionTestStage::CandidateScan)
                && scans.fetch_add(1, Ordering::SeqCst) == 1
            {
                assert!(matches!(
                    owner.acquire_candidate_wallet(bound.clone()).await.unwrap(),
                    (CandidateEvidenceState::Usable, _)
                ));
                for wallet in others {
                    owner.acquire_candidate_wallet(wallet).await.unwrap();
                }
                let pool = owner.pool.clone();
                assert!(matches!(
                    owner
                        .store
                        .call(move |s| s.candidate_evidence(&pool, &bound).map(|result| result.0))
                        .await
                        .unwrap(),
                    CandidateEvidenceState::Missing
                ));
            }
        }
        .boxed()
    }));
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        f.manager.acquire_candidate(&f.bound, U256::from(5)),
    )
    .await
    .unwrap()
    .unwrap();
    let selected = matches!(result, CandidatePassResult::Selected(_));
    let negative = matches!(result, CandidatePassResult::CompleteNegative);
    drop(result);
    f.close().await;
    assert!(
        selected,
        "sufficient completed observation was lost; complete_negative={negative}"
    );
}

#[tokio::test]
async fn cached_candidate_survives_eviction_between_evaluation_and_batched_selection() {
    let mut f = fixture(260).await;
    f.balance(&f.large, 10);
    f.block(&f.bound.wallet);
    let owner = f.manager.clone();
    let target = f.large.clone();
    let others = f.retained.clone();
    let evaluated = Arc::new(AtomicUsize::new(0));
    f.manager.test_admission_hook = Some(Arc::new(move |stage| {
        let (owner, target, others, evaluated) = (
            owner.clone(),
            target.clone(),
            others.clone(),
            evaluated.clone(),
        );
        async move {
            if matches!(stage, AdmissionTestStage::CandidateEvaluated) {
                match evaluated.fetch_add(1, Ordering::SeqCst) {
                    // Publish the next priority wallet after the initial scan.
                    0 => {
                        owner.acquire_candidate_wallet(target).await.unwrap();
                    }
                    // Its cached proof has now been evaluated and must stay owned.
                    1 => {
                        for wallet in others {
                            owner.acquire_candidate_wallet(wallet).await.unwrap();
                        }
                        let pool = owner.pool.clone();
                        assert!(matches!(
                            owner
                                .store
                                .call(move |s| s
                                    .candidate_evidence(&pool, &target)
                                    .map(|result| result.0))
                                .await
                                .unwrap(),
                            CandidateEvidenceState::Missing
                        ));
                    }
                    _ => {}
                }
            }
        }
        .boxed()
    }));
    let CandidatePassResult::Selected(selected) = select(&f).await else {
        panic!("cached completed candidate was lost to churn")
    };
    assert_eq!(selected.candidate.wallet, f.large);
    assert_eq!(
        f.calls.lock().unwrap()[&f.address(&f.large)],
        2,
        "one exact confirmed/latest acquisition, without rereading after eviction"
    );
    drop(selected);
    f.close().await;
}

async fn select_owned(f: &Fixture, view: ChainView, cost: u64) -> Option<SelectedCandidate> {
    f.manager
        .select_completed_candidate(&f.bound.wallet, U256::from(cost), i64::MAX, &mut vec![view])
        .await
        .unwrap()
}

#[tokio::test]
async fn evicted_owned_proof_keeps_expiry_and_rechecks_liabilities_and_invalidation() {
    let f = fixture(260).await;
    f.balance(&f.bound.wallet, 10);
    let (_, view) = f
        .manager
        .acquire_candidate_wallet(f.bound.wallet.clone())
        .await
        .unwrap();
    let view = view.unwrap();
    for wallet in &f.retained {
        f.manager
            .acquire_candidate_wallet(wallet.clone())
            .await
            .unwrap();
    }
    let (pool, wallet) = (f.manager.pool.clone(), f.bound.wallet.clone());
    assert!(matches!(
        f.manager
            .store
            .call(move |s| s.candidate_evidence(&pool, &wallet).map(|result| result.0))
            .await
            .unwrap(),
        CandidateEvidenceState::Missing
    ));
    let calls = f.calls.lock().unwrap().clone();
    let selected = select_owned(&f, view.clone(), 5).await.unwrap();
    assert_eq!(
        selected.view.admission_valid_until,
        view.admission_valid_until
    );
    assert_eq!(selected.candidate.wallet, f.bound.wallet);
    assert!(f.manager.gate.try_lock().is_err());
    drop(selected);
    let mut expired = view.clone();
    expired
        .balance_certificates
        .get_mut(&f.bound.wallet)
        .unwrap()
        .restrict_validity(0);
    assert!(select_owned(&f, expired, 5).await.is_none());
    f.db()
        .execute(
            "UPDATE wallets SET unresolved_exposure='7' WHERE id=?1",
            [&f.bound.wallet],
        )
        .unwrap();
    assert!(select_owned(&f, view.clone(), 5).await.is_none());
    f.db()
        .execute(
            "UPDATE wallets SET unresolved_exposure='0' WHERE id=?1",
            [&f.bound.wallet],
        )
        .unwrap();
    let endpoint = view.observations[&f.bound.wallet].endpoint.clone();
    f.manager
        .store
        .call(move |s| s.invalidate_endpoint_evidence(&endpoint))
        .await
        .unwrap();
    assert!(select_owned(&f, view, 5).await.is_none());
    assert_eq!(
        *f.calls.lock().unwrap(),
        calls,
        "local revalidation must not acquire again"
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
async fn chain_order_wins_over_cache_publication_order() {
    let f = fixture(260).await;
    f.head.store(300, Ordering::SeqCst);
    f.balance(&f.bound.wallet, 10);
    let (_, view) = f
        .manager
        .acquire_candidate_wallet(f.bound.wallet.clone())
        .await
        .unwrap();
    for wallet in &f.retained {
        f.manager
            .acquire_candidate_wallet(wallet.clone())
            .await
            .unwrap();
    }
    // A slower independent acquisition from an older head completes after churn.
    f.head.store(200, Ordering::SeqCst);
    f.balance(&f.bound.wallet, 3);
    f.manager
        .acquire_candidate_wallet(f.bound.wallet.clone())
        .await
        .unwrap();
    let view = view.unwrap();
    let selected = select_owned(&f, view.clone(), 5).await;
    assert!(
        selected.is_some(),
        "older retained-cache entry must not suppress the newer completed proof"
    );
    drop(selected);
    // The inverse matters financially: a newer insufficient cache observation
    // must supersede our formerly sufficient proof, regardless of ownership.
    f.head.store(400, Ordering::SeqCst);
    f.manager
        .acquire_candidate_wallet(f.bound.wallet.clone())
        .await
        .unwrap();
    let calls = f.calls.lock().unwrap().clone();
    assert!(select_owned(&f, view, 5).await.is_none());
    assert_eq!(*f.calls.lock().unwrap(), calls);
    f.close().await;
}

#[tokio::test]
async fn wallet_local_repair_preserves_healthy_selection_and_negative_diagnostic() {
    use crate::rotation::base::evidence::EvidenceFailure;
    for sufficient in [false, true] {
        let mut f = fixture(0).await;
        if sufficient {
            f.balance(&f.small, 10);
        }
        f.db()
            .execute(
                "UPDATE wallets SET evidence_repair='chain_repair_required' WHERE id=?1",
                [&f.bound.wallet],
            )
            .unwrap();
        let finished = Arc::new(tokio::sync::Notify::new());
        let first = Arc::new(AtomicBool::new(true));
        let bound = f.bound.wallet.clone();
        f.manager.test_admission_hook = Some(Arc::new(move |stage| {
            let (finished, first, bound) = (finished.clone(), first.clone(), bound.clone());
            Box::pin(async move {
                match stage {
                    AdmissionTestStage::CandidateReadCompleted { wallet } if wallet == bound => {
                        finished.notify_one()
                    }
                    AdmissionTestStage::CandidateEvaluated
                        if first.swap(false, Ordering::SeqCst) =>
                    {
                        finished.notified().await
                    }
                    _ => {}
                }
            })
        }));
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            f.manager.acquire_candidate(&f.bound, U256::from(5)),
        )
        .await
        .unwrap();
        match result {
            Ok(CandidatePassResult::Selected(selected)) if sufficient => {
                assert_eq!(selected.candidate.wallet, f.small);
                drop(selected);
            }
            Err(error) if !sufficient => assert_eq!(
                error.downcast_ref::<EvidenceFailure>(),
                Some(&EvidenceFailure::ChainRepairRequired)
            ),
            _ => panic!("wallet-local failure escaped its scope or lost its diagnostic"),
        }
        let db = f.db();
        assert_eq!(
            db.query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [&f.bound.wallet],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "chain_repair_required"
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM reserve_claims WHERE wallet_id IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        drop(db);
        f.close().await;
    }
}

#[test]
fn endpoint_failure_still_aborts_after_wallet_local_failure() {
    use crate::rotation::base::{EndpointMismatch, evidence::EvidenceFailure};
    let mut completed = Vec::new();
    let mut failure = None;
    assert!(
        completion_is_incomplete(
            Err(EvidenceFailure::ChainRepairRequired.into()),
            &mut completed,
            &mut failure
        )
        .unwrap()
    );
    let error = completion_is_incomplete(
        Err(EndpointMismatch {
            endpoint: "a".repeat(64),
        }
        .into()),
        &mut completed,
        &mut failure,
    )
    .unwrap_err();
    assert!(error.is::<EndpointMismatch>());
    assert!(failure.unwrap().is::<EvidenceFailure>());
    assert!(completed.is_empty());
}

#[tokio::test]
async fn healthy_selection_does_not_wait_for_late_wallet_local_failure() {
    let mut f = fixture(0).await;
    f.balance(&f.small, 10);
    f.db()
        .execute(
            "UPDATE wallets SET evidence_repair='chain_repair_required' WHERE id=?1",
            [&f.bound.wallet],
        )
        .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let first = Arc::new(AtomicBool::new(true));
    let bound = f.bound.wallet.clone();
    f.manager.test_admission_hook = Some(Arc::new(move |stage| {
        let (entered, first, bound) = (entered.clone(), first.clone(), bound.clone());
        Box::pin(async move {
            match stage {
                AdmissionTestStage::CandidateReadCompleted { wallet } if wallet == bound => {
                    entered.notify_one();
                    std::future::pending::<()>().await;
                }
                AdmissionTestStage::CandidateEvaluated if first.swap(false, Ordering::SeqCst) => {
                    entered.notified().await
                }
                _ => {}
            }
        })
    }));
    let CandidatePassResult::Selected(selected) = select(&f).await else {
        panic!("healthy reserve must not wait for unrelated rejected evidence");
    };
    assert_eq!(selected.candidate.wallet, f.small);
    drop(selected);
    f.close().await;
}

#[tokio::test]
async fn malformed_readiness_is_wallet_local_in_cold_and_cached_selection() {
    use crate::rotation::base::evidence::EvidenceFailure;
    for cached in [false, true] {
        for sufficient in [false, true] {
            for support in ["{}", "tuple", "null"] {
                let f = fixture(0).await;
                if sufficient {
                    f.balance(&f.small, 10);
                }
                if cached {
                    for wallet in [&f.bound.wallet, &f.small, &f.large] {
                        f.manager
                            .acquire_candidate_wallet(wallet.clone())
                            .await
                            .unwrap();
                    }
                }
                if support == "null" {
                    f.db()
                        .execute(
                            "UPDATE funding_jobs SET readiness_support=NULL WHERE wallet_id=?1",
                            [&f.large],
                        )
                        .unwrap();
                } else if support == "tuple" {
                    f.db()
                        .execute(
                            "UPDATE funding_jobs SET readiness_capacity='999' WHERE wallet_id=?1",
                            [&f.large],
                        )
                        .unwrap();
                } else {
                    f.db()
                        .execute(
                            "UPDATE funding_jobs SET readiness_support=?1 WHERE wallet_id=?2",
                            rusqlite::params![support, f.large],
                        )
                        .unwrap();
                }
                if cached && sufficient {
                    let hinted = f.manager.candidate_for(Some(U256::from(5))).await.unwrap();
                    assert_eq!(hinted.wallet, f.small);
                }
                let result = f.manager.acquire_candidate(&f.bound, U256::from(5)).await;
                match result {
                    Ok(CandidatePassResult::Selected(selected)) if sufficient => {
                        assert_eq!(selected.candidate.wallet, f.small);
                        drop(selected);
                    }
                    Err(error) if !sufficient => assert_eq!(
                        error.downcast_ref::<EvidenceFailure>(),
                        Some(&EvidenceFailure::MalformedSupport)
                    ),
                    _ => {
                        panic!("malformed sibling vetoed healthy selection or lost its diagnostic")
                    }
                }
                assert_eq!(
                    f.db()
                        .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
                assert_eq!(
                    f.db()
                        .query_row(
                            "SELECT COUNT(*) FROM reserve_claims WHERE wallet_id IS NOT NULL",
                            [],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    2
                );
                assert_eq!(
                    f.db()
                        .query_row("SELECT role FROM wallets WHERE id=?1", [&f.large], |r| {
                            r.get::<_, String>(0)
                        })
                        .unwrap(),
                    "READY"
                );
                f.close().await;
            }
        }
    }
}

#[tokio::test]
async fn selection_does_not_swallow_database_failures() {
    let f = fixture(0).await;
    f.db()
        .execute("ALTER TABLE funding_jobs RENAME TO unavailable_jobs", [])
        .unwrap();
    let error = f
        .manager
        .acquire_candidate(&f.bound, U256::from(5))
        .await
        .err()
        .unwrap();
    assert!(error.is::<rusqlite::Error>());
    assert!(!error.is::<crate::rotation::base::evidence::EvidenceFailure>());
    assert!(
        f.manager
            .candidate_for(None)
            .await
            .unwrap_err()
            .is::<rusqlite::Error>()
    );
    assert!(
        f.manager
            .store
            .call(|s| s.startup_groups(&["selection".into()]))
            .await
            .unwrap_err()
            .is::<rusqlite::Error>()
    );
    assert!(f.calls.lock().unwrap().is_empty());
    f.close().await;
}

#[tokio::test]
async fn initial_identity_and_startup_isolate_malformed_wallet_readiness() {
    use crate::rotation::base::evidence::EvidenceFailure;
    for corrupt in ["active", "small", "large", "active_and_small", "all"] {
        let f = fixture(0).await;
        for (name, wallet) in [
            ("active", &f.bound.wallet),
            ("small", &f.small),
            ("large", &f.large),
        ] {
            if corrupt == "all"
                || corrupt == name
                || (corrupt == "active_and_small" && name != "large")
            {
                f.db()
                    .execute(
                        "UPDATE funding_jobs SET readiness_support='{}' WHERE wallet_id=?1",
                        [wallet],
                    )
                    .unwrap();
            }
        }
        let before: Vec<(String, String)> = f
            .db()
            .prepare("SELECT id,role FROM wallets ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let candidate = f.manager.candidate_for(None).await;
        let startup = f
            .manager
            .store
            .call(|s| s.startup_groups(&["selection".into()]))
            .await;
        if corrupt == "all" {
            assert_eq!(
                candidate.unwrap_err().downcast_ref::<EvidenceFailure>(),
                Some(&EvidenceFailure::MalformedSupport)
            );
            assert_eq!(
                startup.unwrap_err().downcast_ref::<EvidenceFailure>(),
                Some(&EvidenceFailure::MalformedSupport)
            );
        } else {
            let expected = match corrupt {
                "active" => &f.small,
                "active_and_small" => &f.large,
                _ => &f.bound.wallet,
            };
            assert_eq!(candidate.unwrap().wallet, *expected);
            let groups = startup.unwrap();
            assert!(groups[0].serving_ready);
            assert_eq!(groups[0].inventory_ready, corrupt == "active");
        }
        let after: Vec<(String, String)> = f
            .db()
            .prepare("SELECT id,role FROM wallets ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT COUNT(*) FROM reserve_claims WHERE wallet_id IS NOT NULL",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(f.calls.lock().unwrap().is_empty());
        f.close().await;
    }
}

#[tokio::test]
async fn initial_identity_skips_malformed_retained_and_preserves_active_role() {
    let f = fixture(2).await;
    for wallet in [&f.bound.wallet, &f.retained[0]] {
        f.db()
            .execute(
                "UPDATE funding_jobs SET readiness_support=NULL WHERE wallet_id=?1",
                [wallet],
            )
            .unwrap();
    }
    let candidate = f.manager.candidate_for(None).await.unwrap();
    assert_eq!(candidate.wallet, f.retained[1]);
    assert_eq!(
        f.db()
            .query_row(
                "SELECT role FROM wallets WHERE id=?1",
                [&f.retained[1]],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "RETAINED"
    );
    // No balance RPC is needed even for a real cold-cache free provider call.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/free", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/free", axum::routing::get(|| async { "free" })),
        )
        .await
        .unwrap();
    });
    let client = crate::payment::PaidClient::managed(Arc::new(f.manager.clone()));
    assert_eq!(
        client
            .execute(crate::catalog::RoutedRequest {
                method: "GET".into(),
                url,
                query: Default::default(),
                body: None
            })
            .await
            .unwrap(),
        "free"
    );
    client.drain().await;
    drop(client);
    server.abort();
    assert!(f.calls.lock().unwrap().is_empty());
    f.close().await;
}

#[tokio::test]
async fn retirement_and_payment_serialize_both_orders_for_the_same_retained_wallet() {
    for payment_first in [true, false] {
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            retirement_payment_order(payment_first),
        )
        .await
        .unwrap();
    }
}

async fn retirement_payment_order(payment_first: bool) {
    let mut f = fixture(1).await;
    let wallet = f.retained[0].clone();
    f.balance(&wallet, 5);
    f.db()
        .execute("UPDATE pools SET retirement_tolerance='5'", [])
        .unwrap();
    let candidate = PaymentCandidateHandle {
        wallet: wallet.clone(),
        address: f
            .db()
            .query_row("SELECT address FROM wallets WHERE id=?1", [&wallet], |r| {
                r.get(0)
            })
            .unwrap(),
    };
    // Acquire the retirement observation before either contender takes
    // the gate. Publication must recheck liabilities added afterwards.
    let (pool, id) = (f.manager.pool.clone(), wallet.clone());
    let query = f
        .manager
        .store
        .call(move |s| s.scoped_chain_query(&pool, vec![id], vec![]))
        .await
        .unwrap();
    let view = f.manager.base.view(query).await.unwrap();
    let acquired = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let signed = Arc::new(AtomicUsize::new(0));
    f.manager.test_admission_hook = Some(Arc::new({
        let (acquired, resume, signed) = (acquired.clone(), resume.clone(), signed.clone());
        move |stage| {
            let (acquired, resume, signed) = (acquired.clone(), resume.clone(), signed.clone());
            async move {
                match stage {
                    AdmissionTestStage::Acquired if payment_first => {
                        acquired.notify_one();
                        resume.notified().await;
                    }
                    AdmissionTestStage::Signed { .. } => {
                        signed.fetch_add(1, Ordering::SeqCst);
                    }
                    _ => {}
                }
            }
            .boxed()
        }
    }));
    let sent = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/pay", listener.local_addr().unwrap());
    let app = Router::new().route("/pay", axum::routing::get({
                let sent = sent.clone();
                let db = f.dir.path().join("state/state.sqlite");
                move |headers: axum::http::HeaderMap| {
                    let (sent, db) = (sent.clone(), db.clone());
                    async move {
                        use axum::response::IntoResponse;
                        if let Some(signature) = headers.get("payment-signature") {
                            let payload: Value = serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
                            let nonce = payload["payload"]["authorization"]["nonce"].as_str().unwrap();
                            let db = rusqlite::Connection::open(db).unwrap();
                            let count: i64 = db.query_row("SELECT COUNT(*) FROM payment_attempts WHERE nonce=?1 AND state='POSSIBLY_SUBMITTED'", [nonce], |r| r.get(0)).unwrap();
                            assert_eq!(count, 1, "journal must precede signed dispatch");
                            sent.fetch_add(1, Ordering::SeqCst);
                            return "paid".into_response();
                        }
                        let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/pay"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                        (axum::http::StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(challenge.to_string()))]).into_response()
                    }
                }
            }));
    let seller = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = crate::network::discovery(&url, std::time::Duration::from_secs(60)).unwrap();
    let response = http.get(&url).send().await.unwrap();
    let retry = http.get(&url).build().unwrap();
    {
        let manager = f.manager.clone();
        let progress = crate::payment::SubmissionProgress::default();
        let mut recovery = RecoveryAllowance::default();
        // Fixed payer is the production path after an identity handoff:
        // another wallet cannot hide a failure to reject this retired payer.
        let payment = manager.pay_with_cover_observed(
            &http,
            candidate,
            retry,
            response,
            None,
            AdmissionObservation {
                fixed_payer: true,
                progress: &progress,
                recovery: &mut recovery,
                hint: None,
            },
        );
        tokio::pin!(payment);
        let pool = f.manager.pool.clone();
        let result = if payment_first {
            tokio::select! {
                _ = acquired.notified() => {}
                result = &mut payment => panic!("payment completed before gate checkpoint: {}", result.is_ok()),
            }
            let retirement = async {
                let guard = Arc::new(f.manager.gate.clone().lock_owned().await);
                f.manager
                    .store
                    .call_with_guard(guard, move |s| s.publish_scoped_evidence(&pool, view))
                    .await
            };
            tokio::pin!(retirement);
            assert!(
                retirement.as_mut().now_or_never().is_none(),
                "retirement must wait for signing/journaling ownership"
            );
            resume.notify_one();
            let (result, retired) = tokio::join!(&mut payment, retirement);
            retired.unwrap();
            result
        } else {
            let guard = Arc::new(f.manager.gate.clone().lock_owned().await);
            assert!(
                payment.as_mut().now_or_never().is_none(),
                "payment must wait for retirement ownership"
            );
            f.manager
                .store
                .call_with_guard(guard.clone(), move |s| {
                    s.publish_scoped_evidence(&pool, view)
                })
                .await
                .unwrap();
            drop(guard);
            payment.as_mut().await
        };
        assert_eq!(result.is_ok(), payment_first, "{result:?}");
        assert_eq!(signed.load(Ordering::SeqCst), usize::from(payment_first));
        assert_eq!(sent.load(Ordering::SeqCst), usize::from(payment_first));
        let db = f.db();
        let (role, exposure): (String, String) = db
            .query_row(
                "SELECT role,unresolved_exposure FROM wallets WHERE id=?1",
                [&wallet],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(role, if payment_first { "RETAINED" } else { "RETIRED" });
        assert_eq!(exposure, if payment_first { "1" } else { "0" });
        for (sql, expected) in [
            (
                "SELECT COUNT(*) FROM payment_attempts",
                i64::from(payment_first),
            ),
            (
                "SELECT COUNT(*) FROM wallet_retirements",
                i64::from(!payment_first),
            ),
            ("SELECT COUNT(*) FROM funding_jobs", 4),
            (
                "SELECT COUNT(*) FROM reserve_claims WHERE wallet_id IS NOT NULL",
                2,
            ),
        ] {
            assert_eq!(
                db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap(),
                expected,
                "{sql}"
            );
        }
        drop(db);
        seller.abort();
    }
    f.close().await;
}

#[tokio::test]
async fn cold_negative_pass_reads_each_captured_wallet_once_across_pages() {
    let f = fixture(PAGE_SIZE + 9).await;
    assert!(matches!(
        select(&f).await,
        CandidatePassResult::CompleteNegative
    ));
    // One acquisition reads each wallet at both confirmed and latest anchors.
    let expected: BTreeMap<_, _> = [&f.bound.wallet, &f.small, &f.large]
        .into_iter()
        .chain(&f.retained)
        .map(|wallet| (f.address(wallet), 2))
        .collect();
    assert_eq!(*f.calls.lock().unwrap(), expected);
    f.close().await;
}

#[tokio::test]
async fn cancelled_pass_drops_its_reads_but_preserves_independent_publication() {
    let f = fixture(2).await;
    let owned = [&f.bound.wallet, &f.small, &f.large, &f.retained[0]];
    for wallet in owned {
        f.block(wallet);
    }
    let independent = f.retained[1].clone();
    f.block(&independent);
    f.balance(&independent, 10);
    let jobs_before: i64 = f
        .db()
        .query_row("SELECT count(*) FROM funding_jobs", [], |r| r.get(0))
        .unwrap();
    let roles: Vec<(String, String)> = f
        .db()
        .prepare("SELECT id,role FROM wallets ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let (manager, bound) = (f.manager.clone(), f.bound.clone());
    let pass = tokio::spawn(async move { manager.acquire_candidate(&bound, U256::from(5)).await });
    let manager = f.manager.clone();
    let wallet = independent.clone();
    let maintenance = tokio::spawn(async move { manager.acquire_candidate_wallet(wallet).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while f.started.lock().unwrap().len() != READS + 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    pass.abort();
    assert!(matches!(pass.await, Err(error) if error.is_cancelled()));
    f.blocked.lock().unwrap().clear();
    f.release.notify_waiters();
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), maintenance)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        (CandidateEvidenceState::Usable, _)
    ));
    for wallet in owned {
        let (pool, wallet) = (f.manager.pool.clone(), wallet.clone());
        let state = f
            .manager
            .store
            .call(move |s| s.candidate_evidence(&pool, &wallet))
            .await
            .unwrap();
        assert!(
            matches!(state, (CandidateEvidenceState::Missing, None)),
            "canceled read must not publish evidence"
        );
    }
    let CandidatePassResult::Selected(selected) = select(&f).await else {
        panic!("independent publication remains usable after cancellation")
    };
    assert_eq!(selected.candidate.wallet, independent);
    drop(selected);
    assert!(f.manager.gate.try_lock().is_ok());
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let after: Vec<(String, String)> = f
        .db()
        .prepare("SELECT id,role FROM wallets ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(after, roles);
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM funding_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        jobs_before
    );
    f.close().await;
}

#[tokio::test]
async fn captured_pass_excludes_newly_allocated_wallet_even_with_published_capacity() {
    let mut f = fixture(1).await;
    for wallet in [&f.bound.wallet, &f.small, &f.large, &f.retained[0]] {
        f.block(wallet);
    }
    let (paused, resume, _) = pause_after_evaluation(&mut f, READS);
    let (manager, bound) = (f.manager.clone(), f.bound.clone());
    let pass = tokio::spawn(async move { manager.acquire_candidate(&bound, U256::from(5)).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), paused.notified())
        .await
        .unwrap();
    let pool = f.manager.pool.clone();
    f.db().execute("UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1 WHERE pool_id=?1 AND slot='small'", [&pool]).unwrap();
    let job = f
        .manager
        .store
        .call(move |s| s.allocate_capacity_kind(&pool, AllocationKind::Small, true))
        .await
        .unwrap()
        .unwrap();
    let new_wallet: String = f
        .db()
        .query_row(
            "SELECT wallet_id FROM funding_jobs WHERE id=?1",
            [&job],
            |r| r.get(0),
        )
        .unwrap();
    f.db()
        .execute(
            "UPDATE wallets SET role='RETAINED' WHERE id=?1",
            [&new_wallet],
        )
        .unwrap();
    f.balance(&new_wallet, 10);
    let (pool, wallet) = (f.manager.pool.clone(), new_wallet.clone());
    let query = f
        .manager
        .store
        .call(move |s| s.scoped_chain_query(&pool, vec![wallet], vec![]))
        .await
        .unwrap();
    let view = f.manager.base.view(query).await.unwrap();
    let wallet = new_wallet.clone();
    f.manager
        .store
        .call(move |s| s.record_verified_credit(&wallet, view))
        .await
        .unwrap();
    assert!(matches!(
        f.manager
            .acquire_candidate_wallet(new_wallet.clone())
            .await
            .unwrap(),
        (CandidateEvidenceState::Usable, _)
    ));
    let calls_before = f
        .calls
        .lock()
        .unwrap()
        .get(&f.address(&new_wallet))
        .copied()
        .unwrap();
    f.blocked.lock().unwrap().clear();
    f.release.notify_waiters();
    resume.notify_one();
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), pass)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        CandidatePassResult::CompleteNegative
    ));
    assert_eq!(
        f.calls.lock().unwrap().get(&f.address(&new_wallet)),
        Some(&calls_before)
    );
    let CandidatePassResult::Selected(selected) = select(&f).await else {
        panic!("the next captured pass must see new capacity")
    };
    assert_eq!(selected.candidate.wallet, new_wallet);
    drop(selected);
    f.close().await;
}
