//! Real WAL failures after SDK signing; no signature may reach HTTP.
use super::*;
use crate::rotation::store::{
    Store,
    test_vfs::{self, Failure},
};

#[tokio::test]
async fn journal_io_failure_sends_no_signature_and_reopen_keeps_atomic_estimate() {
    for kind in [Failure::Write, Failure::Sync] {
        let mut f = fixture().await;
        f.db()
            .execute(
                "UPDATE wallets SET balance='20' WHERE id=?1",
                [&f.payer.wallet],
            )
            .unwrap();
        let id = f.pool.store.call(|s| Ok(s.id().to_owned())).await.unwrap();
        f.pool
            .store
            .call(|s| {
                test_vfs::install(s);
                Ok(())
            })
            .await
            .unwrap();
        let fault = Arc::new(std::sync::Mutex::new(None));
        let wal = f.dir.path().join("state/state.sqlite-wal");
        f.pool.test_admission_hook = Some(Arc::new({
            let fault = fault.clone();
            move |stage| {
                if matches!(stage, AdmissionTestStage::Signed { .. }) {
                    assert!(
                        fault
                            .lock()
                            .unwrap()
                            .replace(test_vfs::arm(&wal, kind))
                            .is_none(),
                        "a journal failure cannot authorize replacement signing"
                    );
                }
                Box::pin(async {})
            }
        }));
        let error = f.pay().await.unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<rusqlite::Error>()
                .unwrap_or_else(|| panic!("{error:#}"))
                .sqlite_error_code(),
            Some(rusqlite::ErrorCode::SystemIoFailure)
        );
        assert!(fault.lock().unwrap().as_ref().unwrap().fired());
        assert_eq!(f.signed.load(Ordering::SeqCst), 0);
        drop(fault.lock().unwrap().take());
        f.pool.drain_direct_payments().await;
        let Fixture {
            dir,
            pool,
            payer,
            signed,
            worker,
            server,
            ..
        } = f;
        drop(pool);
        worker.await.unwrap();
        let reopened =
            Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        let (pending,evidence,balance): (i64,i64,String) = db.query_row("SELECT (SELECT count(*) FROM payment_attempts),(SELECT count(*) FROM payment_admission_evidence),balance FROM wallets WHERE id=?1", [&payer.wallet], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert!(
            pending == 0 || pending == 1,
            "a failed fsync may still have committed"
        );
        assert_eq!(evidence, pending);
        assert_eq!(balance, if pending == 1 { "13" } else { "20" });
        assert_eq!(
            db.query_row("SELECT count(*) FROM payment_resolutions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            signed.load(Ordering::SeqCst),
            0,
            "reopen cannot replay an uncertain signature"
        );
        drop(reopened);
        server.abort();
    }
}
