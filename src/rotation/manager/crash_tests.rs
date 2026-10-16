//! Kill the actual signing process at financial boundaries; only loopback peers
//! and temporary, unfunded wallets participate. No destructor simulates recovery.
use super::*;
use crate::{payment::PaidClient, rotation::store::Store};
use std::{path::Path, process::Stdio, time::Duration};

const CHILD: &str = "rotation::manager::admission_tests::crash_tests::payment_crash_child";

#[tokio::test]
#[ignore = "private subprocess fixture invoked by crash-boundary test"]
async fn payment_crash_child() {
    let root = std::env::var_os("X402_CRASH_FIXTURE_ROOT").expect("explicit fixture only");
    let root = Path::new(&root);
    let base = std::env::var("X402_CRASH_FIXTURE_URL").unwrap();
    let stage = std::env::var("X402_CRASH_FIXTURE_STAGE").unwrap();
    let id = crate::rotation::store::status(&root.join("state"))
        .unwrap()
        .treasury_id;
    let state = Store::open(&root.join("state"), &root.join("key"), &id).unwrap();
    let pool = state
        .status()
        .unwrap()
        .pools
        .into_iter()
        .find(|p| p.name == "payer")
        .unwrap()
        .id;
    let (store, _worker) = StoreHandle::spawn(state);
    let mut manager = ManagedPool::new(
        store,
        pool,
        BaseRpc::new(&format!("{base}/rpc"), 12, 120).unwrap(),
        SpendPolicy::dollars("5").unwrap(),
    )
    .unwrap();
    let marker = root.join("boundary");
    manager.test_admission_hook = Some(Arc::new(move |at| {
        let marker = marker.clone();
        let stage = stage.clone();
        async move {
            if matches!(
                (&*stage, at),
                ("signed", AdmissionTestStage::Signed { .. })
                    | ("journaled", AdmissionTestStage::Journaled)
            ) {
                std::fs::write(marker, b"ready").unwrap();
                std::future::pending::<()>().await;
            }
        }
        .boxed()
    }));
    PaidClient::managed(Arc::new(manager))
        .execute(route(&base))
        .await
        .unwrap();
    panic!("parent must kill this process before response completion");
}

fn route(base: &str) -> crate::catalog::RoutedRequest {
    crate::catalog::RoutedRequest {
        method: "GET".into(),
        url: format!("{base}/pay"),
        query: Default::default(),
        body: None,
    }
}

fn attempts(path: &Path) -> Vec<(String, String, String, String, String)> {
    let db = rusqlite::Connection::open(path).unwrap();
    db.prepare("SELECT id,wallet_id,nonce,amount,state FROM payment_attempts ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[tokio::test]
async fn process_loss_preserves_authorizations_without_replay_and_peer_can_pay() {
    for stage in ["signed", "journaled", "dispatched"] {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let id = state.id().to_owned();
        let mut pools = Vec::new();
        for name in ["payer", "peer"] {
            pools.push(
                crate::rotation::store::test_support::declare_two_reserves(
                    &mut state,
                    name,
                    "5",
                    Some(U256::from(5_000_000)),
                    0,
                )
                .unwrap(),
            );
        }
        for job in state.funding_jobs().unwrap() {
            state
                .record_credit(&job.wallet_id, "5000000", &format!("0x{:064x}", 88), 88)
                .unwrap();
        }
        drop(state);
        let fixture = Fixture {
            holding: 5_000_000,
            debit: 1,
            price: Arc::new(AtomicU64::new(1)),
            head: Arc::new(AtomicU64::new(100)),
            calls: Arc::default(),
            unsigned: Arc::default(),
            sent: Arc::default(),
            timestamps: Arc::new(StdMutex::new(
                (1..=100).map(|h| (h, now().unwrap())).collect(),
            )),
            used: Arc::default(),
            db: dir.path().join("state/state.sqlite"),
        };
        let received = Arc::new(tokio::sync::Notify::new());
        let signal = received.clone();
        let hold = stage == "dispatched";
        let app = Router::new()
            .route("/rpc", post(rpc))
            .route(
                "/pay",
                get(move |State(f): State<Fixture>, headers: HeaderMap| {
                    let signal = signal.clone();
                    async move {
                        let signed = headers.contains_key("payment-signature");
                        let response = seller(State(f.clone()), headers).await;
                        if signed && hold && f.sent.lock().unwrap().len() == 1 {
                            signal.notify_one();
                            std::future::pending::<()>().await;
                        }
                        response
                    }
                }),
            )
            .with_state(fixture.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", CHILD, "--nocapture"])
            .env("X402_CRASH_FIXTURE_ROOT", dir.path())
            .env("X402_CRASH_FIXTURE_URL", &base)
            .env("X402_CRASH_FIXTURE_STAGE", stage)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            if hold {
                received.notified().await;
            } else {
                while !dir.path().join("boundary").exists() {
                    assert!(
                        child.try_wait().unwrap().is_none(),
                        "child exited before {stage}"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        })
        .await
        .unwrap();
        child.kill().await.unwrap();
        let status = child.wait().await.unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        let saved = attempts(&fixture.db);
        assert_eq!(saved.len(), usize::from(stage != "signed"));
        assert_eq!(fixture.sent.lock().unwrap().len(), usize::from(hold));
        let state = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        assert_eq!(attempts(&fixture.db), saved);
        let (store, worker) = StoreHandle::spawn(state);
        let manager = |pool: String| {
            Arc::new(
                ManagedPool::new(
                    store.clone(),
                    pool,
                    BaseRpc::new(&format!("{base}/rpc"), 12, 120).unwrap(),
                    SpendPolicy::dollars("5").unwrap(),
                )
                .unwrap(),
            )
        };
        let payer = manager(pools[0].clone());
        payer.reconcile().await.unwrap();
        assert_eq!(
            attempts(&fixture.db),
            saved,
            "absence at a pre-expiry chain anchor cannot release liability"
        );
        assert_eq!(
            fixture.sent.lock().unwrap().len(),
            usize::from(hold),
            "reopen/reconciliation must not replay"
        );
        if let Some((_, wallet, _, amount, state)) = saved.first() {
            assert_eq!(state, "POSSIBLY_SUBMITTED");
            let db = rusqlite::Connection::open(&fixture.db).unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT (SELECT CAST(COALESCE(SUM(CAST(a.amount AS INTEGER)),0) AS TEXT) FROM payment_attempts a WHERE a.wallet_id=wallets.id AND a.state='POSSIBLY_SUBMITTED') FROM wallets WHERE id=?1",
                    [wallet],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                *amount
            );
        }
        let peer = PaidClient::managed(manager(pools[1].clone()));
        assert_eq!(peer.execute(route(&base)).await.unwrap(), "paid");
        assert_eq!(fixture.sent.lock().unwrap().len(), usize::from(hold) + 1);
        assert_eq!(attempts(&fixture.db).len(), saved.len() + 1);
        drop(peer);
        drop(payer);
        drop(store);
        worker.await.unwrap();
        server.abort();
    }
}
