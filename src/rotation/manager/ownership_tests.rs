//! Reuse the actual local Base and journal-checking seller fixture.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
enum CancelAt {
    Acquired,
    Signed,
    Journaled,
}
#[tokio::test]
async fn canceled_direct_waiter_does_not_suppress_acknowledged_signed_dispatch() {
    cancel_at(CancelAt::Journaled).await;
}
#[tokio::test]
async fn canceled_direct_waiter_before_preparation_retains_admission_owner() {
    cancel_at(CancelAt::Acquired).await;
}
#[tokio::test]
async fn canceled_direct_waiter_with_private_signature_finishes_exact_authorization() {
    cancel_at(CancelAt::Signed).await;
}
async fn cancel_at(cancel_at: CancelAt) {
    use crate::rotation::store::Store;
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "direct",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(crate::rotation::config::positive_usdc("5").unwrap()),
        0
    )
    .unwrap();
    for job in state.funding_jobs().unwrap() {
        state
            .record_credit(&job.wallet_id, "5000000", &format!("0x{:064x}", 88), 88)
            .unwrap();
    }
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
    let journaled = Arc::new(tokio::sync::Notify::new());
    let signatures = Arc::new(StdMutex::new(Vec::new()));
    let release = Arc::new(tokio::sync::Notify::new());
    manager.test_admission_hook = Some(Arc::new({
        let journaled = journaled.clone();
        let release = release.clone();
        let signatures = signatures.clone();
        move |stage| {
            let journaled = journaled.clone();
            let release = release.clone();
            let signatures = signatures.clone();
            async move {
                if let AdmissionTestStage::Signed { nonce, .. } = &stage {
                    signatures.lock().unwrap().push(nonce.clone());
                }
                let stop = matches!(
                    (cancel_at, stage),
                    (CancelAt::Acquired, AdmissionTestStage::Acquired)
                        | (CancelAt::Signed, AdmissionTestStage::Signed { .. })
                        | (CancelAt::Journaled, AdmissionTestStage::Journaled)
                );
                if stop {
                    journaled.notify_one();
                    release.notified().await;
                }
            }
            .boxed()
        }
    }));
    let expected = manager.candidate().await.unwrap();
    let expected_address = expected.address.clone();
    let http = crate::network::NetworkContext::new(Default::default())
        .unwrap()
        .discovery(&base, None)
        .unwrap();
    let retry = http.get(format!("{base}/pay")).build().unwrap();
    let response = http.execute(retry.try_clone().unwrap()).await.unwrap();
    let waiter = tokio::spawn({
        let manager = manager.clone();
        async move { manager.pay(&http, expected, retry, response).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), journaled.notified())
        .await
        .unwrap();
    assert!(fixture.sent.lock().unwrap().is_empty());
    let db = rusqlite::Connection::open(&fixture.db).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, i64::from(cancel_at == CancelAt::Journaled));
    assert_eq!(
        manager.gate.try_lock().is_err(),
        cancel_at != CancelAt::Journaled,
        "acquired/private-signature admission retains the gate; acknowledged dispatch releases it"
    );
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert_eq!(
        manager.gate.try_lock().is_err(),
        cancel_at != CancelAt::Journaled,
        "waiter cancellation cannot release the accepted owner's gate"
    );
    release.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        manager.drain_direct_payments(),
    )
    .await
    .unwrap();
    assert_eq!(fixture.sent.lock().unwrap().len(), 1);
    assert_eq!(fixture.unsigned.load(Ordering::SeqCst), 1);
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "seller success cannot release managed liability");
    let signatures = signatures.lock().unwrap().clone();
    assert_eq!(
        signatures.len(),
        1,
        "cancellation cannot trigger replacement signing"
    );
    assert_eq!(*fixture.sent.lock().unwrap(), signatures);
    let (payer, nonce, amount): (String, String, String) = db
        .query_row(
            "SELECT payer,nonce,amount FROM payment_attempts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(payer.to_lowercase(), expected_address.to_lowercase());
    assert_eq!(nonce, signatures[0]);
    assert_eq!(amount, "1");
    let exposure: String = db
        .query_row(
            "SELECT unresolved_exposure FROM wallets WHERE lower(address)=lower(?1)",
            [&expected_address],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(exposure, "1");
    assert!(
        manager.gate.try_lock().is_ok(),
        "completed ownership releases the gate"
    );
    drop(manager);
    drop(store);
    worker.await.unwrap();
    server.abort();
}
