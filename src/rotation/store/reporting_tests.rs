//! Current read-only projections preserve journals and expose damaged evidence.
use super::*;
use serde_json::json;

fn current() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        123,
        b"initial",
    )
    .unwrap();
    for name in ["zeta", "alpha"] {
        super::super::test_support::declare_two_reserves(
            &mut store,
            name,
            "0.000005",
            Some(U256::from(10)),
            0,
        )
        .unwrap();
    }
    store
        .accept_preparation("operation", None, 10, None)
        .unwrap();
    store
        .prepare_with_facts(
            "operation",
            store.snapshot().unwrap().0,
            b"prepared",
            super::super::test_support::transaction_raw(100).as_slice(),
            Some(TransactionFacts {
                txid: super::super::test_support::transaction_txid(100),
                expiry_height: 100,
                amount_zatoshis: 8,
                fee_zatoshis: 2,
                deadline: Some(1000),
            }),
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT INTO expired_operations VALUES ('operation',101,?1)",
            [store.snapshot().unwrap().0],
        )
        .unwrap();
    (dir, store)
}

#[test]
fn current_status_projects_expiry_without_mutating_journals() {
    let (_dir, store) = current();
    let raw: Vec<u8> = store
        .db
        .query_row("SELECT raw FROM outgoing WHERE id='operation'", [], |row| {
            row.get(0)
        })
        .unwrap();
    store.db.execute_batch("PRAGMA query_only=ON").unwrap();
    let changes = store.db.total_changes();
    let report = read_status(&store.db).unwrap();
    assert_eq!(report.schema_version, 6);
    assert_eq!(report.state_schema_version, 23);
    assert_eq!(report.treasury_id, store.id());
    assert_eq!(report.birthday, 123);
    assert_eq!(report.snapshot_revision, 2);
    assert!(report.outgoing_pending);
    assert!(report.sync.is_none());
    assert!(!report.sync_complete);
    assert_eq!(
        report
            .pools
            .iter()
            .map(|pool| pool.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert_eq!(report.funding_jobs.len(), 4);
    assert_eq!(report.treasury_operations.len(), 1);
    let operation = &report.treasury_operations[0];
    assert_eq!(operation.operation_id, "operation");
    assert_eq!(operation.submission, "EXPIRED");
    assert_eq!(operation.facts.amount_zatoshis, 8);
    let saved: (String, Vec<u8>) = store.db.query_row(
        "SELECT t.submission,o.raw FROM treasury_operations t JOIN outgoing o ON o.id=t.id WHERE t.id='operation'",
        [], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!(saved, ("PREPARED".into(), raw));
    assert_eq!(
        serde_json::to_value(read_status(&store.db).unwrap()).unwrap(),
        serde_json::to_value(report).unwrap()
    );
    assert_eq!(store.db.total_changes(), changes);
}

#[test]
fn malformed_status_observations_and_operation_facts_fail_without_repair() {
    let (_dir, store) = current();
    for corrupt_sync in [true, false] {
        let query = if corrupt_sync {
            store
                .db
                .execute("INSERT INTO treasury_sync VALUES (1,'not-json')", [])
                .unwrap();
            "SELECT observation FROM treasury_sync"
        } else {
            store.db.execute("DELETE FROM treasury_sync", []).unwrap();
            store
                .db
                .execute("UPDATE treasury_operations SET facts='not-json'", [])
                .unwrap();
            "SELECT facts FROM treasury_operations"
        };
        store.db.execute_batch("PRAGMA query_only=ON").unwrap();
        let changes = store.db.total_changes();
        let error = read_status(&store.db)
            .err()
            .expect("invalid persisted JSON cannot become partial status");
        assert!(error.is::<serde_json::Error>());
        assert_eq!(
            store
                .db
                .query_row(query, [], |row| row.get::<_, String>(0))
                .unwrap(),
            "not-json"
        );
        assert_eq!(store.db.total_changes(), changes);
        store.db.execute_batch("PRAGMA query_only=OFF").unwrap();
    }
}

#[test]
fn malformed_exposure_is_unknown_only_for_affected_wallet_without_repair() {
    let (_dir, store) = current();
    let report = read_status(&store.db).unwrap();
    let pool = &report.pools[0];
    let damaged = &pool.addresses[0];
    let sibling = &pool.addresses[1];
    store.db.execute(
        "INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES ('damaged',?1,?2,0,'not-an-amount','fixture','POSSIBLY_SUBMITTED')",
        params![pool.id,damaged.id],
    ).unwrap();
    store.db.execute_batch("PRAGMA query_only=ON").unwrap();
    let changes = store.db.total_changes();
    let observed = read_status(&store.db).unwrap();
    let wallets = &observed.pools[0].addresses;
    assert_eq!(wallets.len(), 2);
    assert_eq!(wallets[0].id, damaged.id);
    assert!(wallets[0].signed_exposure_atomic.is_none());
    assert_eq!(wallets[1].id, sibling.id);
    assert_eq!(wallets[1].signed_exposure_atomic.as_deref(), Some("0"));
    assert_eq!(observed.pools.len(), 2);
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT amount FROM payment_attempts WHERE id='damaged'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "not-an-amount"
    );
    assert_eq!(store.db.total_changes(), changes);
    let encoded = serde_json::to_value(observed).unwrap();
    assert_eq!(
        encoded["pools"][0]["addresses"][0]["signed_exposure_atomic"],
        json!(null)
    );
}
