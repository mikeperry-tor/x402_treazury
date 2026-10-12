//! Measure actual production statements, not just their bounded result sizes.
use super::*;
use rusqlite::{Params, StatementStatus, types::Value};

#[test]
fn indexed_lookups_preserve_retirement_null_claims_and_refresh_exclusions() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(SCHEMA).unwrap();
    db.execute("INSERT INTO pools(id,name) VALUES ('pool','pool')", [])
        .unwrap();
    for (sequence, role) in [(1, "ACTIVE"), (2, "RETAINED"), (3, "READY")] {
        db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES (?1,'pool',?2,?1,X'00','1',?3)", params![format!("wallet-{sequence}"), sequence, role]).unwrap();
    }
    // A wallet lies beyond the captured pass boundary; retire another wallet
    // between pages. Neither may leak into the remaining captured page.
    let first = inspect(&db, history::WALLET_PAGE_SQL, params!["pool", -1, 2, 1]).0;
    assert_eq!(first[0][0], Value::Integer(1));
    db.execute("UPDATE wallets SET role='RETIRED' WHERE sequence=2", [])
        .unwrap();
    assert!(
        inspect(&db, history::WALLET_PAGE_SQL, params!["pool", 1, 2, 64])
            .0
            .is_empty()
    );
    assert_eq!(
        inspect(&db, history::WALLET_PAGE_SQL, params!["pool", 1, 3, 64]).0[0][0],
        Value::Integer(3)
    );

    db.execute("INSERT INTO funding_jobs(id,wallet_id,state,target,pool_id,allocation_kind) VALUES ('initial','wallet-1','QUEUED','1','pool','initial')", []).unwrap();
    // Historical initial jobs are recoverable, but never captured reserve claims.
    assert!(
        inspect(
            &db,
            bootstrap::STARTUP_CLAIM_SQL,
            params!["pool", "initial", Option::<i64>::None]
        )
        .0
        .is_empty()
    );
    db.execute("INSERT INTO funding_jobs(id,wallet_id,state,target,pool_id,allocation_kind,claim_generation) VALUES ('reserve','wallet-3','QUEUED','1','pool','small',0)", []).unwrap();
    assert_eq!(
        inspect(
            &db,
            bootstrap::STARTUP_CLAIM_SQL,
            params!["pool", "small", 0]
        )
        .0,
        vec![vec![Value::Text("reserve".into())]]
    );
    assert!(
        inspect(
            &db,
            bootstrap::STARTUP_CLAIM_SQL,
            params!["pool", "small", 1]
        )
        .0
        .is_empty()
    );
    for id in ["legacy", "refresh"] {
        db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase) VALUES (?1,'initial','\"QUOTED\"')", [id]).unwrap();
    }
    db.execute(
        "INSERT INTO funding_quote_refreshes(operation_id) VALUES ('refresh')",
        [],
    )
    .unwrap();
    assert_eq!(
        inspect(&db, funding::RECOVERY_COUNT_SQL, ["initial"]).0,
        vec![vec![Value::Integer(1)]]
    );
}

fn inspect(db: &Connection, sql: &str, params: impl Params) -> (Vec<Vec<Value>>, i32) {
    let mut stmt = db.prepare(sql).unwrap();
    let columns = stmt.column_count();
    let result = stmt
        .query_map(params, |row| (0..columns).map(|n| row.get(n)).collect())
        .unwrap()
        .collect::<rusqlite::Result<Vec<Vec<Value>>>>()
        .unwrap();
    assert_eq!(stmt.get_status(StatementStatus::FullscanStep), 0);
    assert_eq!(stmt.get_status(StatementStatus::Sort), 0);
    (result, stmt.get_status(StatementStatus::VmStep))
}

#[test]
fn scoped_query_work_is_independent_of_unrelated_lifetime_history() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch(SCHEMA).unwrap();
    db.execute("INSERT INTO pools(id,name) VALUES ('pool','pool')", [])
        .unwrap();
    db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES ('live','pool',30000,'live',X'00','1','ACTIVE')", []).unwrap();
    db.execute("INSERT INTO funding_jobs(id,wallet_id,state,target,pool_id,allocation_kind,claim_generation) VALUES ('job','live','QUEUED','1','pool','small',4)", []).unwrap();
    db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase) VALUES ('operation','job','\"QUOTED\"')", []).unwrap();
    let queries = |db: &Connection| {
        [
            inspect(db, history::WALLET_PAGE_SQL, params!["pool", -1, 30000, 64]),
            inspect(db, history::WALLET_PAGE_SQL, params!["pool", -1, 29999, 64]),
            inspect(
                db,
                bootstrap::STARTUP_CLAIM_SQL,
                params!["pool", "small", 4],
            ),
            inspect(
                db,
                bootstrap::STARTUP_CLAIM_SQL,
                params!["pool", "large", 4],
            ),
            inspect(db, funding::RECOVERY_COUNT_SQL, ["job"]),
            inspect(db, funding::RECOVERY_COUNT_SQL, ["missing"]),
        ]
    };
    let before = queries(&db);
    let tx = db.transaction().unwrap();
    for n in 0..20000 {
        let id = format!("history-{n}");
        tx.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES (?1,'pool',?2,?1,X'00','1','RETIRED')", params![id,n]).unwrap();
        tx.execute("INSERT INTO funding_jobs(id,wallet_id,state,target,pool_id,allocation_kind,claim_generation) VALUES (?1,?1,'COMPLETE','1','pool','small',?2)", params![id,n+1000]).unwrap();
        tx.execute(
            "INSERT INTO funding_recovery(operation_id,job_id,phase) VALUES (?1,?1,'\"QUOTED\"')",
            [&id],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    for ((rows, work), (expected, baseline)) in queries(&db).into_iter().zip(&before) {
        assert_eq!(&rows, expected);
        // B-tree boundary branches may add a few opcodes; inventory-sized
        // iteration may not. This fails even for index scans with zero fullscan steps.
        assert!(
            work <= baseline + 8,
            "query work grew from {baseline} to {work}"
        );
    }
    assert_eq!(before[0].0.len(), 1);
    assert!(before[1].0.is_empty());
    assert_eq!(before[2].0, vec![vec![Value::Text("job".into())]]);
    assert!(before[3].0.is_empty());
    assert_eq!(before[4].0, vec![vec![Value::Integer(1)]]);
    assert_eq!(before[5].0, vec![vec![Value::Integer(0)]]);
}
