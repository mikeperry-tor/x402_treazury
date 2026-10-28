//! Exercise both inventory feeds across their page boundary through real RPC reads.
use super::*;
use std::collections::BTreeMap;

const WALLETS: usize = 70;

#[derive(Default)]
struct Reads {
    calls: BTreeMap<String, usize>,
    active: BTreeMap<String, usize>,
}

#[derive(Clone)]
struct InventoryRpc {
    reads: Arc<std::sync::Mutex<Reads>>,
    first_address: String,
    healthy_address: String,
    hold_first: bool,
    hold_all: Arc<AtomicBool>,
    fail: bool,
    events: tokio::sync::mpsc::UnboundedSender<String>,
    release: Arc<tokio::sync::Semaphore>,
}

async fn balance(State(rpc): State<InventoryRpc>, Json(v): Json<Value>) -> Json<Value> {
    assert_eq!(v["method"], "eth_call");
    assert_eq!(v["params"][1], "latest");
    let address = format!("0x{}", &v["params"][0]["data"].as_str().unwrap()[34..]);
    let count = {
        let mut reads = rpc.reads.lock().unwrap();
        let active = reads.active.entry(address.clone()).or_default();
        *active += 1;
        assert_eq!(*active, 1, "duplicate active balance request for {address}");
        let count = reads.calls.entry(address.clone()).or_default();
        *count += 1;
        *count
    };
    rpc.events.send(address.clone()).unwrap();
    if rpc.hold_all.load(Ordering::SeqCst)
        || (rpc.hold_first && address == rpc.first_address && count == 1)
    {
        rpc.release.acquire().await.unwrap().forget();
    }
    *rpc.reads.lock().unwrap().active.get_mut(&address).unwrap() -= 1;
    if rpc.fail && address != rpc.healthy_address {
        Json(
            json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32000,"message":"fixture unavailable"}}),
        )
    } else {
        Json(json!({"jsonrpc":"2.0","id":v["id"],"result":format!("0x{:064x}",17)}))
    }
}

struct Inventory {
    fixture: Fixture,
    rpc: InventoryRpc,
    events: tokio::sync::mpsc::UnboundedReceiver<String>,
    addresses: Vec<String>,
}

impl Inventory {
    async fn new(all_pending: bool, fail: bool, hold_first: bool) -> Self {
        let mut fixture = fixture(false, false).await;
        // Additional retained identities need no funding/signing setup. Reuse an
        // encrypted unfunded fixture key; these rows are observed, never signed.
        let db = rusqlite::Connection::open(fixture.dir.path().join("state/state.sqlite")).unwrap();
        for n in 2..WALLETS {
            db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance) SELECT ?1,pool_id,?2,?3,key,target,'RETAINED','5000000' FROM wallets WHERE id=?4", rusqlite::params![format!("inventory-{n}"),n as i64,format!("0x{:040x}",n+1000),fixture.wallet]).unwrap();
        }
        if all_pending {
            db.execute("UPDATE wallets SET low_balance=1,refresh_pending=1", [])
                .unwrap();
        }
        let addresses = db
            .prepare("SELECT lower(address) FROM wallets ORDER BY sequence")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(addresses.len(), WALLETS);
        drop(db);
        let (events_tx, events) = tokio::sync::mpsc::unbounded_channel();
        let rpc = InventoryRpc {
            reads: Arc::default(),
            first_address: fixture.rpc.address.to_lowercase(),
            healthy_address: addresses.last().unwrap().clone(),
            hold_first,
            hold_all: Arc::new(AtomicBool::new(false)),
            fail,
            events: events_tx,
            release: Arc::new(tokio::sync::Semaphore::new(0)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/rpc", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/rpc", post(balance))
            .with_state(rpc.clone());
        fixture.server.abort();
        fixture.server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        fixture.pool = Arc::new(
            ManagedPool::new(
                fixture.store.clone(),
                fixture.group.clone(),
                BaseRpc::new(&endpoint, 12).unwrap(),
                crate::payment::SpendPolicy {
                    max_atomic: Some(U256::from(100)),
                },
            )
            .unwrap(),
        );
        Self {
            fixture,
            rpc,
            events,
            addresses,
        }
    }

    fn assert_no_financial_side_effects(&self) {
        let db =
            rusqlite::Connection::open(self.fixture.dir.path().join("state/state.sqlite")).unwrap();
        for table in [
            "payment_attempts",
            "outgoing",
            "funding_recovery",
            "funding_refunds",
        ] {
            let count: i64 = db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "unexpected {table} mutation");
        }
        let jobs: i64 = db
            .query_row("SELECT count(*) FROM funding_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(jobs, 2, "observation allocated new funding work");
    }

    fn assert_calls(&self, first: usize, rest: usize) {
        let reads = self.rpc.reads.lock().unwrap();
        assert_eq!(reads.calls.len(), WALLETS);
        assert!(reads.active.values().all(|active| *active == 0));
        for address in &self.addresses {
            assert_eq!(
                reads.calls[address],
                if address == &self.rpc.first_address {
                    first
                } else {
                    rest
                },
                "requests for {address}"
            );
        }
    }
}

#[tokio::test]
async fn recurrent_pending_failures_reach_later_pages_without_duplicate_active_reads() {
    let f = Inventory::new(true, true, false).await;
    let mut previous = BTreeMap::<String, usize>::new();
    for pass in 0..2 {
        if pass > 0 {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(60)).await;
        }
        tokio::time::timeout(Duration::from_secs(15), f.fixture.pool.reconcile())
            .await
            .unwrap()
            .unwrap_err();
        {
            let reads = f.rpc.reads.lock().unwrap();
            assert_eq!(reads.calls.len(), WALLETS);
            assert!(reads.active.values().all(|active| *active == 0));
            for address in &f.addresses {
                assert!(reads.calls[address] > previous.get(address).copied().unwrap_or(0));
            }
            previous.clone_from(&reads.calls);
        }
        let db =
            rusqlite::Connection::open(f.fixture.dir.path().join("state/state.sqlite")).unwrap();
        let preserved: i64 = db.query_row("SELECT count(*) FROM wallets WHERE balance='5000000' AND low_balance=1 AND balance_refresh_error='observation_unavailable'", [], |r| r.get(0)).unwrap();
        assert_eq!(preserved, WALLETS as i64 - 1);
        let healthy: (String, bool, Option<String>) = db.query_row("SELECT balance,low_balance,balance_refresh_error FROM wallets WHERE lower(address)=?1", [&f.rpc.healthy_address], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(healthy, ("17".into(), false, None));
        f.assert_no_financial_side_effects();
    }
    f.fixture.close().await;
}

#[tokio::test]
async fn held_pending_read_allows_full_live_inventory_and_retains_a_new_failure() {
    let mut f = Inventory::new(false, false, true).await;
    let pool = f.fixture.pool.clone();
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let running = tokio::spawn(async move { pool.run_reconciliation(stopped).await });
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut observed = std::collections::BTreeSet::new();
        while observed.len() < WALLETS {
            let address = f.events.recv().await.unwrap();
            assert!(
                observed.insert(address),
                "duplicate read before first release"
            );
        }
    })
    .await
    .unwrap();
    // The first wallet is still in flight in both feeds, while every sibling,
    // including page two, has independently reached the real RPC server.
    f.fixture
        .pool
        .note_insufficient(f.fixture.wallet.clone())
        .await
        .unwrap();
    assert!(!running.is_finished());
    assert_eq!(f.rpc.reads.lock().unwrap().calls[&f.rpc.first_address], 1);
    f.rpc.release.add_permits(1);
    while f
        .rpc
        .reads
        .lock()
        .unwrap()
        .active
        .values()
        .any(|active| *active > 0)
    {
        tokio::task::yield_now().await;
    }
    f.fixture.wait_completion().await;
    f.fixture.finish_observations(U256::from(17)).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(60)).await;
    loop {
        let address = f.events.recv().await.unwrap();
        if address == f.rpc.first_address {
            break;
        }
    }
    while f
        .rpc
        .reads
        .lock()
        .unwrap()
        .active
        .values()
        .any(|active| *active > 0)
    {
        tokio::task::yield_now().await;
    }
    f.fixture.wait_completion().await;
    stop.cancel();
    running.await.unwrap().unwrap();
    f.assert_calls(2, 1);
    // These observation-only synthetic rows deliberately have no funding jobs;
    // full financial status requires that unrelated provenance for every wallet.
    let db = rusqlite::Connection::open(f.fixture.dir.path().join("state/state.sqlite")).unwrap();
    let observed: (String, bool, bool, Option<String>) = db
        .query_row(
            "SELECT balance,low_balance,refresh_pending,balance_refresh_error FROM wallets WHERE id=?1",
            [&f.fixture.wallet],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(observed, ("17".into(), false, false, None));
    drop(db);
    f.assert_no_financial_side_effects();
    f.fixture.close().await;
}

#[tokio::test]
async fn advisory_capacity_reports_while_complete_inventory_still_drains() {
    if crate::test_log_capture::run_in_child(
        "rotation::manager::maintenance::tests::inventory::advisory_capacity_reports_while_complete_inventory_still_drains",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let f = Inventory::new(true, true, false).await;
    f.fixture
        .pool
        .maintenance(CancellationToken::new(), true)
        .await
        .unwrap_err();
    assert_eq!(f.rpc.reads.lock().unwrap().calls.len(), WALLETS);
    f.assert_no_financial_side_effects();
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(logs.lines().any(|line| line.contains("DEBUG")
        && line.contains("Advisory balance observation capacity reached")));
    assert!(logs.contains("limit_reads=4"));
    assert!(!logs.contains(&f.rpc.first_address));
    f.fixture.close().await;
}

#[tokio::test]
async fn advisory_capacity_looks_past_coalesced_rows_and_keeps_coalescing_silent() {
    if crate::test_log_capture::run_in_child(
        "rotation::manager::maintenance::tests::inventory::advisory_capacity_looks_past_coalesced_rows_and_keeps_coalescing_silent",
    ) {
        return;
    }
    use tracing::instrument::WithSubscriber;
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(log.reopen().unwrap())
        .finish();
    let mut f = Inventory::new(true, false, false).await;
    let db = rusqlite::Connection::open(f.fixture.dir.path().join("state/state.sqlite")).unwrap();
    db.execute("DELETE FROM wallets WHERE sequence >= 4", [])
        .unwrap();
    f.rpc.hold_all.store(true, Ordering::SeqCst);
    let pool = f.fixture.pool.clone();
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let running = tokio::spawn(
        async move { pool.maintenance(stopped, false).await }.with_subscriber(subscriber),
    );
    tokio::time::timeout(Duration::from_secs(15), async {
        for _ in 0..BALANCE_READS {
            f.events.recv().await.unwrap();
        }
        // A Store round trip gives the pending inventory pass a chance to finish
        // after all four reads have been accepted and retained.
        f.fixture.store.call(|_| Ok(())).await.unwrap();
        tokio::task::yield_now().await;
    })
    .await
    .unwrap();
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(!logs.contains("Advisory balance observation capacity reached"));
    db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance,low_balance,refresh_pending) SELECT 'later-pending',pool_id,4,'0x000000000000000000000000000000000000ffff',key,target,'RETAINED','5000000',0,0 FROM wallets WHERE id=?1", [&f.fixture.wallet]).unwrap();
    f.fixture
        .pool
        .note_insufficient("later-pending".into())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let logs = std::fs::read_to_string(log.path()).unwrap();
            if logs.lines().any(|line| {
                line.contains("DEBUG")
                    && line.contains("Advisory balance observation capacity reached")
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.rpc.reads.lock().unwrap().calls.len(), BALANCE_READS);
    let pending: bool = db
        .query_row(
            "SELECT refresh_pending FROM wallets WHERE id='later-pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(pending);
    f.assert_no_financial_side_effects();
    stop.cancel();
    running.await.unwrap().unwrap();
    f.rpc.hold_all.store(false, Ordering::SeqCst);
    f.rpc.release.add_permits(BALANCE_READS);
    drop(db);
    f.fixture.close().await;
}
