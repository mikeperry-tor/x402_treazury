//! Actual public direct-payment ownership, using the admission HTTP fixture.
use super::*;

#[tokio::test]
async fn unsigned_candidate_neither_publishes_roles_nor_waits_for_the_payment_gate() {
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
        "unsigned",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(1_000_000)),
        0
    )
    .unwrap();
    let jobs = state.funding_jobs().unwrap();
    for job in &jobs {
        state
            .record_credit(&job.wallet_id, "1", "fixture", 100)
            .unwrap();
    }
    let expected = jobs[0].wallet_id.clone();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute(
        "UPDATE wallets SET role='RETAINED' WHERE id=?1",
        [&expected],
    )
    .unwrap();
    db.execute_batch("CREATE TRIGGER no_unsigned_role_write BEFORE UPDATE ON wallets BEGIN SELECT RAISE(ABORT,'unsigned role mutation'); END;
        CREATE TRIGGER no_unsigned_claim_write BEFORE UPDATE ON reserve_claims BEGIN SELECT RAISE(ABORT,'unsigned claim mutation'); END;
        CREATE TRIGGER no_unsigned_authorization BEFORE INSERT ON payment_attempts BEGIN SELECT RAISE(ABORT,'unsigned authorization'); END;").unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let manager = ManagedPool::new(
        store.clone(),
        pool,
        BaseRpc::new("http://127.0.0.1:1", 12).unwrap(),
        SpendPolicy::dollars("1").unwrap(),
    )
    .unwrap();
    assert_eq!(manager.candidate().await.unwrap().wallet, expected);
    let gate = manager.gate.clone().lock_owned().await;
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), manager.candidate())
            .await
            .unwrap()
            .unwrap()
            .wallet,
        expected
    );
    assert_eq!(
        db.query_row("SELECT role FROM wallets WHERE id=?1", [&expected], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "RETAINED"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM payment_attempts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(gate);
    drop(manager);
    drop(store);
    worker.await.unwrap();
}
#[tokio::test]
async fn canceled_direct_waiters_retain_accepted_work_before_and_after_journaling() {
    for stage in 0..3 {
        let mut f = fixture().await;
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        f.pool.test_admission_hook = Some(Arc::new({
            let (entered, release) = (entered.clone(), release.clone());
            move |observed| {
                let (entered, release) = (entered.clone(), release.clone());
                Box::pin(async move {
                    if matches!(
                        (stage, observed),
                        (0, AdmissionTestStage::Acquired)
                            | (1, AdmissionTestStage::Signed { .. })
                            | (2, AdmissionTestStage::Journaled)
                    ) {
                        entered.notify_one();
                        release.acquire().await.unwrap().forget();
                    }
                })
            }
        }));
        let http = crate::network::NetworkContext::new(Default::default())
            .unwrap()
            .http(
                &crate::network::IsolationId::evm(&f.payer.address).unwrap(),
                &f.url,
                None,
            )
            .unwrap();
        let retry = http.get(&f.url).build().unwrap();
        let challenge = http.execute(retry.try_clone().unwrap()).await.unwrap();
        let pool = f.pool.clone();
        let payer = f.payer.clone();
        let waiter = tokio::spawn(async move { pool.pay(&http, payer, retry, challenge).await });
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        let count = || {
            f.db()
                .query_row(
                    "SELECT count(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        };
        assert_eq!(count(), i64::from(stage == 2));
        assert_eq!(f.signed.load(Ordering::SeqCst), 0);
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        release.add_permits(1);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            f.pool.drain_direct_payments(),
        )
        .await
        .unwrap();
        assert_eq!(f.signed.load(Ordering::SeqCst), 1);
        assert_eq!(
            count(),
            1,
            "cancellation cannot abandon or duplicate the authorization"
        );
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT count(DISTINCT nonce) FROM payment_attempts",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        f.close().await;
    }
}
