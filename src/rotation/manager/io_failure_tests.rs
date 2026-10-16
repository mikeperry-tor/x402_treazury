//! Actual WAL errors after SDK signing must never authorize provider dispatch.
use super::*;
use crate::rotation::store::{
    Store,
    test_vfs::{self, Failure},
};

#[tokio::test]
async fn journal_io_failure_sends_no_signature_and_reopen_keeps_atomic_exposure() {
    for kind in [Failure::Write, Failure::Sync] {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let id = state.id().to_owned();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut state,
            "payer",
            "5",
            Some(U256::from(5_000_000)),
            0,
        )
        .unwrap();
        for job in state.funding_jobs().unwrap() {
            state
                .record_credit(&job.wallet_id, "5000000", &format!("0x{:064x}", 88), 88)
                .unwrap();
        }
        test_vfs::install(&mut state);
        let fixture = Fixture {
            holding: 5_000_000,
            debit: 1,
            price: Arc::new(AtomicU64::new(1)),
            head: Arc::new(AtomicU64::new(100)),
            calls: Arc::default(),
            unsigned: Arc::default(),
            sent: Arc::default(),
            used: Arc::default(),
            timestamps: Arc::new(StdMutex::new(
                (1..=100).map(|h| (h, now().unwrap())).collect(),
            )),
            db: dir.path().join("state/state.sqlite"),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/rpc", post(rpc))
            .route("/pay", get(seller))
            .with_state(fixture.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (store, worker) = StoreHandle::spawn(state);
        let mut manager = ManagedPool::new(
            store.clone(),
            pool,
            BaseRpc::new(&format!("{base}/rpc"), 12, 120).unwrap(),
            SpendPolicy::dollars("5").unwrap(),
        )
        .unwrap();
        let fault = Arc::new(StdMutex::new(None));
        let wal = dir.path().join("state/state.sqlite-wal");
        manager.test_admission_hook = Some(Arc::new({
            let fault = fault.clone();
            move |stage| {
                if matches!(stage, AdmissionTestStage::Signed { .. }) {
                    let previous = fault.lock().unwrap().replace(test_vfs::arm(&wal, kind));
                    assert!(
                        previous.is_none(),
                        "storage failure must not replace a signature"
                    );
                }
                async {}.boxed()
            }
        }));
        let client = crate::payment::PaidClient::managed(Arc::new(manager));
        let error = client
            .execute(crate::catalog::RoutedRequest {
                method: "GET".into(),
                url: format!("{base}/pay"),
                query: Default::default(),
                body: None,
            })
            .await
            .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<rusqlite::Error>()
                .unwrap_or_else(|| panic!("unexpected fixture error: {error:#}"))
                .sqlite_error_code(),
            Some(rusqlite::ErrorCode::SystemIoFailure)
        );
        assert!(fault.lock().unwrap().as_ref().unwrap().fired());
        assert_eq!(fixture.unsigned.load(Ordering::SeqCst), 1);
        assert!(fixture.sent.lock().unwrap().is_empty());
        drop(fault.lock().unwrap().take());
        drop(client);
        drop(store);
        worker.await.unwrap();
        let reopened =
            Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        let db = rusqlite::Connection::open(&fixture.db).unwrap();
        let pending: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(pending == 0 || pending == 1);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM payment_admission_evidence", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            pending
        );
        assert_eq!(
            db.query_row(
                "SELECT SUM(CAST((SELECT CAST(COALESCE(SUM(CAST(a.amount AS INTEGER)),0) AS TEXT) FROM payment_attempts a WHERE a.wallet_id=wallets.id AND a.state='POSSIBLY_SUBMITTED') AS INTEGER)) FROM wallets",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            pending
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM payment_resolutions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            fixture.sent.lock().unwrap().is_empty(),
            "reopen cannot send an uncertain authorization"
        );
        drop(reopened);
        server.abort();
    }
}
