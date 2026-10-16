//! Read-only status uses the historical schema's actual available columns.
use super::*;
use serde_json::json;

fn historical(version: u32) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    let mut section = 0;
    for line in include_str!("../../../tests/fixtures/state/historical.sql").lines() {
        if let Some(number) = line.strip_prefix("-- version ") {
            section = number.parse::<u32>().unwrap();
        } else if !line.starts_with("--") && section <= version {
            db.execute_batch(line).unwrap();
        }
    }
    db.execute_batch(&format!(
        "PRAGMA user_version={version}; PRAGMA foreign_keys=ON;
        INSERT INTO instance VALUES ('fixture',1,123,'mainnet',0);
        INSERT INTO snapshots VALUES (1,X'00');
        INSERT INTO pools(id,name,target) VALUES ('z','zeta','5'),('a','alpha','5');
        INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance) VALUES
        ('late','a',2,'late-address',X'00','5','RETIRED','7'),
        ('first','a',1,'first-address',X'00','5','READY','5');
        INSERT INTO budget_entries VALUES ('operation','a',1,1,10,10,0);
        INSERT INTO outgoing VALUES ('operation','PREPARED',X'00',1);"
    ))
    .unwrap();
    if version >= 3 {
        let facts = json!({"txid":"fixture-tx","expiry_height":100,"amount_zatoshis":8,"fee_zatoshis":2,"deadline":1000});
        db.execute(
            "INSERT INTO treasury_operations VALUES ('operation',?1,'BROADCAST_REQUESTED',2)",
            [facts.to_string()],
        )
        .unwrap();
    }
    if version >= 4 {
        db.execute_batch("INSERT INTO funding_jobs VALUES ('job','first','QUEUED','5');
            INSERT INTO funding_progress(job_id,operation_id,phase) VALUES ('job','operation','\"RECOVERY_REQUIRED\"');").unwrap();
    }
    if version >= 7 {
        db.execute_batch("INSERT INTO refund_outputs VALUES ('refund',0,'operation',3,50,0);")
            .unwrap();
    }
    if version >= 8 {
        db.execute_batch("INSERT INTO expired_operations VALUES ('operation',101,1);")
            .unwrap();
    }
    db
}

#[test]
fn historical_status_projects_available_fields_without_mutating_journals() {
    for version in 0..=12 {
        let db = historical(version);
        db.execute_batch("PRAGMA query_only=ON").unwrap();
        let changes = db.total_changes();
        let report =
            read_status(&db).unwrap_or_else(|error| panic!("version {version}: {error:#}"));
        assert_eq!(report.state_schema_version, version);
        assert_eq!(report.schema_version, 2);
        assert_eq!(report.treasury_id, "fixture");
        assert_eq!(report.birthday, 123);
        assert_eq!(report.snapshot_revision, 1);
        assert!(report.outgoing_pending);
        assert!(report.sync.is_none());
        assert!(!report.sync_fresh);
        assert_eq!(
            report
                .pools
                .iter()
                .map(|pool| pool.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "zeta"]
        );
        let pool = &report.pools[0];
        assert_eq!(
            pool.addresses
                .iter()
                .map(|wallet| wallet.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "late"]
        );
        assert!(!pool.serving_ready && !pool.inventory_ready);
        assert!(pool.reserve_claims.is_empty());
        assert!(pool.payment_limit_atomic.is_none());
        assert!(pool.retirement_tolerance_atomic.is_none());
        assert_eq!(pool.funding_degraded, version >= 4);
        assert!(!report.pools[1].funding_degraded);
        assert_eq!(pool.retirement_accounting.historical_unknown_count, 1);
        assert!(pool.retirement_accounting.automatic_count.is_none());
        for wallet in &pool.addresses {
            assert!(wallet.signed_exposure_atomic.is_none());
            assert!(!wallet.destination_recorded);
            assert!(wallet.runtime_balance_evidence.is_none());
        }
        assert_eq!(report.funding_jobs.len(), usize::from(version >= 4));
        assert_eq!(report.refunds.len(), usize::from(version >= 7));
        assert_eq!(report.treasury_operations.len(), usize::from(version >= 3));
        if version >= 3 {
            let operation = &report.treasury_operations[0];
            assert_eq!(operation.operation_id, "operation");
            assert_eq!(operation.attempts, 2);
            assert_eq!(operation.facts.amount_zatoshis, 8);
            assert_eq!(
                operation.submission,
                if version >= 8 {
                    "EXPIRED"
                } else {
                    "BROADCAST_REQUESTED"
                }
            );
            let saved: String = db
                .query_row("SELECT submission FROM treasury_operations", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(saved, "BROADCAST_REQUESTED", "expiry is projection only");
        }
        if version >= 7 {
            assert_eq!(report.refunds[0].amount, 3);
            assert_eq!(report.refunds[0].operation_id, "operation");
        }
        assert_eq!(
            serde_json::to_value(read_status(&db).unwrap()).unwrap(),
            serde_json::to_value(report).unwrap()
        );
        assert_eq!(db.total_changes(), changes);
    }
}

#[test]
fn malformed_status_observations_and_operation_facts_fail_without_repair() {
    for corrupt_sync in [true, false] {
        let db = historical(3);
        let query = if corrupt_sync {
            db.execute("INSERT INTO treasury_sync VALUES (1,'not-json')", [])
                .unwrap();
            "SELECT observation FROM treasury_sync"
        } else {
            db.execute("UPDATE treasury_operations SET facts='not-json'", [])
                .unwrap();
            "SELECT facts FROM treasury_operations"
        };
        db.execute_batch("PRAGMA query_only=ON").unwrap();
        let changes = db.total_changes();
        let error = read_status(&db)
            .err()
            .expect("invalid persisted JSON cannot become a partial status");
        assert!(error.is::<serde_json::Error>());
        assert_eq!(
            db.query_row(query, [], |row| row.get::<_, String>(0))
                .unwrap(),
            "not-json"
        );
        assert_eq!(db.total_changes(), changes);
    }
}

#[test]
fn historical_completed_status_does_not_require_current_obligation_columns() {
    for version in 4..=12 {
        let db = historical(version);
        db.execute("UPDATE funding_jobs SET state='COMPLETE'", [])
            .unwrap();
        db.execute(
            "UPDATE funding_progress SET last_error='funding_quote_failed; historical'",
            [],
        )
        .unwrap();
        db.execute_batch("PRAGMA query_only=ON").unwrap();
        let changes = db.total_changes();
        let report =
            read_status(&db).unwrap_or_else(|error| panic!("version {version}: {error:#}"));
        assert_eq!(
            report.funding_jobs[0].phase,
            funding::FundingPhase::Complete
        );
        assert!(report.funding_jobs[0].last_error.is_none());
        assert!(report.outgoing_pending);
        assert_eq!(db.total_changes(), changes);
    }
}

#[test]
fn schema16_retirement_accounting_preserves_recorded_amounts_without_current_admission() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool =
        test_support::declare_two_reserves(&mut store, "historical", "1", Some(U256::from(100)), 0)
            .unwrap();
    let wallet: String = store
        .db
        .query_row(
            "SELECT id FROM wallets WHERE pool_id=?1 LIMIT 1",
            [&pool],
            |r| r.get(0),
        )
        .unwrap();
    store
        .db
        .execute("UPDATE wallets SET role='RETIRED' WHERE id=?1", [&wallet])
        .unwrap();
    store.db.execute("INSERT INTO wallet_retirements(wallet_id,pool_id,confirmed_amount,latest_amount,confirmed_height,confirmed_hash,latest_height,latest_hash,accepted_maximum,observed_bound,support,endpoint,reason,recorded_at) VALUES(?1,?2,'27','30',10,'old-confirmed',12,'old-latest','100','100','historical-support','historical-endpoint','automatic',100)", params![wallet,pool]).unwrap();
    // Remove the new field/index, reproducing the actual old layout rather
    // than asking a fresh-schema report to pretend it lacks the field.
    store.db.execute_batch("DROP INDEX payment_disputed_wallet; DROP INDEX payment_undisputed_wallet; ALTER TABLE payment_attempts DROP COLUMN resolution_disputed; PRAGMA user_version=16; PRAGMA query_only=ON;").unwrap();
    let before = store.db.total_changes();
    let report = read_status(&store.db).unwrap();
    let pool = &report.pools[0];
    assert_eq!(
        pool.retirement_accounting
            .automatic_recorded_confirmed_atomic
            .as_deref(),
        Some("27")
    );
    assert_eq!(pool.retirement_accounting.automatic_count, Some(1));
    assert_eq!(pool.retirement_accounting.historical_unknown_count, 0);
    assert_eq!(pool.retired_wallets, 1);
    let wallet = pool.addresses.iter().find(|w| w.id == wallet).unwrap();
    assert_eq!(wallet.retirement.as_ref().unwrap().latest_amount, "30");
    assert!(wallet.signed_exposure_atomic.is_none());
    assert!(!pool.serving_ready);
    assert_eq!(store.db.total_changes(), before);
}
