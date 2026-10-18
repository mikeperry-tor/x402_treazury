//! Exercise SQLite's real rollback/recovery after write, fsync and capacity
//! errors. A failed checkpoint must not erase committed financial obligations.
use super::*;
use test_vfs::{Failure, arm, install};

fn reopen(dir: &Path, id: &str) -> Store {
    Store::open(&dir.join("state"), &dir.join("key"), id).unwrap()
}
fn assert_consistent(store: &Store) {
    assert_eq!(
        store
            .db
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}
fn reservation(store: &Store, id: &str) -> i64 {
    let resolved: bool = store
        .db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1 AND state='RESOLVED')",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    if resolved {
        0
    } else {
        store.preparation_intent(id).unwrap().cost_zatoshis as i64
    }
}

#[test]
fn failed_wal_write_or_sync_never_partially_prepares_or_releases_reservation() {
    for kind in [Failure::Write, Failure::Sync] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"initial",
        )
        .unwrap();
        let id = store.id().to_owned();
        install(&mut store);
        store
            .accept_preparation("operation", None, 100, None)
            .unwrap();
        let before = store.snapshot().unwrap().0;
        let fault = arm(&dir.path().join("state/state.sqlite-wal"), kind);
        let error = store
            .prepare(
                "operation",
                before,
                b"prepared-wallet",
                b"exact-signed-bytes",
            )
            .unwrap_err();
        assert!(fault.fired());
        assert_eq!(
            error
                .downcast_ref::<rusqlite::Error>()
                .unwrap()
                .sqlite_error_code(),
            Some(rusqlite::ErrorCode::SystemIoFailure)
        );
        drop(fault);
        drop(store);
        let mut store = reopen(dir.path(), &id);
        assert_consistent(&store);
        assert_eq!(reservation(&store, "operation"), 100);
        // A sync error may leave a complete WAL commit readable. Either outcome
        // must be atomic; never interpret the error as proof of no preparation.
        let pending = store.operation_pending("operation").unwrap();
        assert_eq!(store.snapshot().unwrap().0, before + i64::from(pending));
        if pending {
            assert_eq!(
                &*store.prepared_bytes("operation").unwrap(),
                b"exact-signed-bytes"
            );
            assert_eq!(&*store.snapshot().unwrap().1, b"prepared-wallet");
            assert!(
                store
                    .prepare("operation", before, b"other", b"replacement")
                    .is_err()
            );
        } else {
            assert_eq!(&*store.snapshot().unwrap().1, b"initial");
        }
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT COALESCE(SUM(attempts),0) FROM treasury_operations",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[test]
fn failed_checkpoint_retains_committed_bytes_and_liability_after_reopen() {
    for kind in [Failure::Write, Failure::Sync] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"initial",
        )
        .unwrap();
        let id = store.id().to_owned();
        install(&mut store);
        store
            .db
            .execute_batch("PRAGMA wal_autocheckpoint=0")
            .unwrap();
        store
            .accept_preparation("operation", None, 100, None)
            .unwrap();
        store
            .prepare("operation", 1, b"prepared-wallet", b"exact-signed-bytes")
            .unwrap();
        let fault = arm(&dir.path().join("state/state.sqlite"), kind);
        assert!(
            store
                .db
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .is_err()
        );
        assert!(fault.fired());
        drop(fault);
        drop(store);
        let store = reopen(dir.path(), &id);
        assert_consistent(&store);
        assert_eq!(reservation(&store, "operation"), 100);
        assert_eq!(
            &*store.prepared_bytes("operation").unwrap(),
            b"exact-signed-bytes"
        );
        assert_eq!(&*store.snapshot().unwrap().1, b"prepared-wallet");
    }
}

#[test]
fn sqlite_full_rolls_back_preparation_without_releasing_existing_funds() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"initial",
    )
    .unwrap();
    let id = store.id().to_owned();
    store
        .accept_preparation("operation", None, 100, None)
        .unwrap();
    let pages: i64 = store
        .db
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    store
        .db
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    let error = store
        .prepare("operation", 1, &vec![3; 1024 * 1024], b"signed")
        .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<rusqlite::Error>()
            .unwrap()
            .sqlite_error_code(),
        Some(rusqlite::ErrorCode::DiskFull)
    );
    drop(store);
    let store = reopen(dir.path(), &id);
    assert_consistent(&store);
    assert_eq!(reservation(&store, "operation"), 100);
    assert!(!store.operation_pending("operation").unwrap());
    assert_eq!(&*store.snapshot().unwrap().1, b"initial");
}
