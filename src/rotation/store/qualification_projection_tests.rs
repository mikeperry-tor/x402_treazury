//! Independent baseline fields and real SQL snapshot/required-failure boundaries.
use super::*;
use std::collections::BTreeSet;

// Unfunded reporting-only rows: no signing, provider calls or qualification authority.
fn fixture() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        123,
        b"fixture",
    )
    .unwrap();
    for name in ["zeta", "alpha"] {
        crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            name,
            "0.000005",
            Some(U256::from(10)),
            0,
        )
        .unwrap();
    }
    let wallets: Vec<String> = store
        .db
        .prepare(
            "SELECT w.id FROM wallets w JOIN pools p ON p.id=w.pool_id ORDER BY p.name,w.sequence",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    for (index, role) in ["ACTIVE", "READY", "RETAINED", "RETIRED"]
        .into_iter()
        .enumerate()
    {
        store
            .db
            .execute(
                "UPDATE wallets SET role=?2,balance=?3 WHERE id=?1",
                params![wallets[index], role, (index + 7).to_string()],
            )
            .unwrap();
    }
    store.db.execute("UPDATE wallets SET low_balance=1,balance_refresh_error='fixture refresh',evidence_repair='fixture repair',block_height=7,block_hash='fixture block' WHERE id=?1", [&wallets[0]]).unwrap();
    let jobs: Vec<String> = store.db.prepare("SELECT f.id FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id ORDER BY p.name,w.sequence").unwrap().query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    for (job, phase) in jobs.iter().zip([
        funding::FundingPhase::Complete,
        funding::FundingPhase::DepositPending,
        funding::FundingPhase::RecoveryRequired,
        funding::FundingPhase::RefundPending,
    ]) {
        store
            .db
            .execute(
                "UPDATE funding_progress SET phase=?2 WHERE job_id=?1",
                params![job, serde_json::to_string(&phase).unwrap()],
            )
            .unwrap();
    }
    store.db.execute("INSERT INTO funding_health(job_id,started_at,error_streak,timed_out) VALUES(?1,10,2,1)", [&jobs[1]]).unwrap();
    store.db.execute("INSERT INTO wallet_retirements(wallet_id,pool_id,confirmed_amount,latest_amount,confirmed_height,confirmed_hash,latest_height,latest_hash,accepted_maximum,observed_bound,support,endpoint,reason,recorded_at,disputed) SELECT id,pool_id,'7','11',3,'confirmed',4,'latest','12','11','{}','fixture','automatic',99,'fixture dispute' FROM wallets WHERE id=?1", [&wallets[3]]).unwrap();
    store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) SELECT 'damaged',pool_id,id,0,'not-an-amount','fixture','POSSIBLY_SUBMITTED' FROM wallets WHERE id=?1", [&wallets[0]]).unwrap();
    for (id, expiry) in [("z-source", 200), ("a-source", 300)] {
        store.accept_preparation(id, None, 10, None).unwrap();
        store
            .prepare_with_facts(
                id,
                store.snapshot().unwrap().0,
                b"prepared",
                crate::rotation::store::test_support::transaction_raw(expiry).as_slice(),
                Some(TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(expiry),
                    expiry_height: expiry,
                    amount_zatoshis: 8,
                    fee_zatoshis: 2,
                    deadline: Some(1000),
                }),
            )
            .unwrap();
        // Authored historical disposition lets the next fixture owner prepare.
        if id == "z-source" {
            store.confirm_spend(id).unwrap();
        }
    }
    store
        .db
        .execute(
            "INSERT INTO expired_operations VALUES('z-source',201,?1)",
            [store.snapshot().unwrap().0],
        )
        .unwrap();
    (dir, store)
}

// Independent copy of the PRE-Q2 published field contract, deliberately not using
// the new row adapters/serializer/hash stream. Small fixed synthetic states only.
fn baseline(db: &Connection, status: &Status) -> Value {
    use sha2::{Digest, Sha256};
    let jobs: Vec<_> = status.funding_jobs.iter().map(|job| json!({"id":job.id,"pool_id":job.pool_id,"pool_name":job.pool_name,"wallet_id":job.wallet_id,"recipient":job.recipient,"target":job.target,"operation_id":job.operation_id,"phase":job.phase})).collect();
    let operations: Vec<_> = status.treasury_operations.iter().map(|operation| {
        let raw: Vec<u8> = db.query_row("SELECT raw FROM outgoing WHERE id=?1", [&operation.operation_id], |row| row.get(0)).unwrap();
        json!({"operation_id":operation.operation_id,"submission":operation.submission,"attempts":operation.attempts,"facts":operation.facts,"prepared_record_sha256":alloy_primitives::hex::encode(Sha256::digest(raw))})
    }).collect();
    json!({"schema_version":2,"state_schema_version":status.state_schema_version,"treasury_id":status.treasury_id,"pools":status.pools,"funding_jobs":jobs,"source_operations":operations})
}
fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn complete_lifecycle_matches_independent_baseline_fields_order_nulls_and_identities() {
    let (_dir, store) = fixture();
    let status = read_status(&store.db).unwrap();
    let expected = baseline(&store.db, &status);
    let before = store.db.total_changes();
    let actual = store.qualification_lifecycle().unwrap();
    assert_eq!(actual, expected);
    assert_eq!(store.qualification_state().unwrap()["lifecycle"], expected);
    assert_eq!(store.db.total_changes(), before);
    assert_eq!(
        keys(&actual),
        BTreeSet::from([
            "schema_version",
            "state_schema_version",
            "treasury_id",
            "pools",
            "funding_jobs",
            "source_operations"
        ])
    );
    assert_eq!(actual["pools"][0]["name"], "alpha");
    assert_eq!(actual["pools"][1]["name"], "zeta");
    assert_eq!(actual["funding_jobs"].as_array().unwrap().len(), 4);
    assert_eq!(
        actual["funding_jobs"][0]["pool_name"], "zeta",
        "job row order is independent of pool sort order"
    );
    assert_eq!(actual["source_operations"][0]["operation_id"], "a-source");
    assert_eq!(actual["source_operations"][1]["operation_id"], "z-source");
    assert_eq!(actual["source_operations"][1]["submission"], "EXPIRED");
    for pool in actual["pools"].as_array().unwrap() {
        assert_eq!(
            keys(pool),
            BTreeSet::from([
                "id",
                "name",
                "generation",
                "enabled",
                "payment_limit_atomic",
                "retirement_tolerance_atomic",
                "retirement_accounting",
                "serving_ready",
                "inventory_ready",
                "reserve_claims",
                "retained_wallets",
                "retired_wallets",
                "funding_degraded",
                "addresses"
            ])
        );
        assert!(pool["funding_degraded"].as_bool().unwrap());
        for claim in pool["reserve_claims"].as_array().unwrap() {
            assert_eq!(
                keys(claim),
                BTreeSet::from([
                    "slot",
                    "enabled",
                    "generation",
                    "wallet_id",
                    "target_atomic",
                    "maximum_output_atomic",
                    "destination_ready",
                    "blocked_funding_job_id"
                ])
            );
        }
        for wallet in pool["addresses"].as_array().unwrap() {
            assert_eq!(
                keys(wallet),
                BTreeSet::from([
                    "id",
                    "address",
                    "role",
                    "target",
                    "balance_estimate_atomic",
                    "low_balance",
                    "balance_refresh_error",
                    "confirmed_height",
                    "confirmed_hash",
                    "signed_exposure_atomic",
                    "financial_repair",
                    "destination_recorded",
                    "destination_disputed",
                    "retirement",
                    "runtime_balance_evidence"
                ])
            );
            assert!(wallet["runtime_balance_evidence"].is_null());
        }
    }
    let active = &actual["pools"][0]["addresses"][0];
    assert_eq!(active["role"], "ACTIVE");
    assert_eq!(active["balance_estimate_atomic"], "7");
    assert_eq!(active["low_balance"], true);
    assert_eq!(active["balance_refresh_error"], "fixture refresh");
    assert_eq!(active["financial_repair"], "fixture repair");
    assert!(active["signed_exposure_atomic"].is_null());
    assert_eq!(
        actual["pools"][1]["addresses"][1]["retirement"],
        json!({"confirmed_amount":"7","latest_amount":"11","accepted_maximum":"12","observed_bound":"11","confirmed_height":3,"confirmed_hash":"confirmed","latest_height":4,"latest_hash":"latest","disputed":"fixture dispute","recorded_at":99})
    );
    assert_eq!(
        actual["pools"][1]["retirement_accounting"],
        json!({"automatic_recorded_confirmed_atomic":"7","automatic_count":1,"manual_recorded_confirmed_atomic":"0","manual_count":0,"historical_unknown_count":0})
    );
    for job in actual["funding_jobs"].as_array().unwrap() {
        assert_eq!(
            keys(job),
            BTreeSet::from([
                "id",
                "pool_id",
                "pool_name",
                "wallet_id",
                "recipient",
                "target",
                "operation_id",
                "phase"
            ])
        );
    }
    for operation in actual["source_operations"].as_array().unwrap() {
        assert_eq!(
            keys(operation),
            BTreeSet::from([
                "operation_id",
                "submission",
                "attempts",
                "facts",
                "prepared_record_sha256"
            ])
        );
    }
}

#[test]
fn narrow_lifecycle_ignores_only_unexported_diagnostics_and_keeps_required_health_strict() {
    let (_dir, store) = fixture();
    let expected = store.qualification_lifecycle().unwrap();
    store
        .db
        .execute("INSERT INTO treasury_sync VALUES(1,'not-json')", [])
        .unwrap();
    store
        .db
        .execute(
            "UPDATE funding_progress SET next_poll='bad',last_error=X'00'",
            [],
        )
        .unwrap();
    store
        .db
        .execute(
            "UPDATE funding_health SET started_at='bad',error_streak='bad'",
            [],
        )
        .unwrap();
    assert_eq!(store.qualification_lifecycle().unwrap(), expected);
    assert!(
        store.status().is_err(),
        "full Status still validates its own diagnostics"
    );
    assert!(
        store.qualification_state().is_err(),
        "full export still requires complete Status"
    );
    store
        .db
        .execute("UPDATE funding_health SET timed_out='bad'", [])
        .unwrap();
    assert!(
        store.qualification_lifecycle().is_err(),
        "required funding degradation cannot default invalid health to healthy"
    );
    store.db.execute("DELETE FROM funding_health", []).unwrap();
    assert!(
        store.qualification_lifecycle().is_ok(),
        "absent optional health retains its original default"
    );
    store
        .db
        .execute("UPDATE wallet_retirements SET confirmed_amount='bad'", [])
        .unwrap();
    assert!(
        store.qualification_lifecycle().is_err(),
        "required retirement accounting remains strict"
    );
    store
        .db
        .execute("UPDATE wallet_retirements SET confirmed_amount='7'", [])
        .unwrap();
    store
        .db
        .execute("UPDATE funding_progress SET phase='not-json'", [])
        .unwrap();
    assert!(
        store.qualification_lifecycle().is_err(),
        "required phase cannot disappear with scheduling diagnostics"
    );
}

#[test]
fn raw_record_hashes_and_expiry_are_exact_without_exporting_or_validating_signed_bytes() {
    use sha2::{Digest, Sha256};
    let (dir, store) = fixture();
    let id = store.id().to_owned();
    let before = store.qualification_lifecycle().unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(store.qualification_lifecycle().unwrap(), before);
    store.db.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE treasury_operations SET submission=X'00' WHERE id='z-source'").unwrap();
    assert!(
        store.qualification_lifecycle().is_err(),
        "expiry projection must not hide an invalid original submission column"
    );
    assert!(store.status().is_err());
    store.db.execute_batch("UPDATE treasury_operations SET submission='CONFIRMED' WHERE id='z-source'; PRAGMA ignore_check_constraints=OFF").unwrap();
    store
        .db
        .execute("UPDATE outgoing SET raw=X'00' WHERE id='a-source'", [])
        .unwrap();
    store
        .db
        .execute(
            "UPDATE treasury_operations SET attempts=-1 WHERE id='a-source'",
            [],
        )
        .unwrap();
    let changed = store.qualification_lifecycle().unwrap();
    assert_eq!(
        changed["source_operations"][0]["prepared_record_sha256"],
        alloy_primitives::hex::encode(Sha256::digest([0u8]))
    );
    assert_eq!(
        changed["source_operations"][0]["attempts"], -1,
        "reporting preserves its existing signed integer domain"
    );
    assert_ne!(
        changed["source_operations"][0]["prepared_record_sha256"],
        before["source_operations"][0]["prepared_record_sha256"]
    );
    assert_eq!(
        changed["source_operations"][1],
        before["source_operations"][1]
    );
    assert!(
        !changed.to_string().contains(&alloy_primitives::hex::encode(
            crate::rotation::store::test_support::transaction_raw(300)
        ))
    );
    store
        .db
        .execute_batch("PRAGMA foreign_keys=OFF; DELETE FROM outgoing WHERE id='a-source'")
        .unwrap();
    assert!(
        store.qualification_lifecycle().is_err(),
        "LEFT JOIN must expose the missing required record"
    );
}

#[test]
fn keyed_admission_uses_existing_transaction_and_optional_lifecycle_never_hides_bad_keyed_facts() {
    let (_dir, store) = fixture();
    let status = read_status(&store.db).unwrap();
    let pool = &status.pools[0];
    let wallet = &pool.addresses[0];
    let generation = pool.reserve_claims[0].generation;
    let tx = store.db.unchecked_transaction().unwrap();
    tx.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES('new',?1,?2,0,'1','fixture','POSSIBLY_SUBMITTED')", params![pool.id,wallet.id]).unwrap();
    tx.execute("INSERT INTO payment_admission_evidence(attempt_id,balance,payer_role) VALUES('new','7','ACTIVE')", []).unwrap();
    tx.execute(
        "UPDATE reserve_claims SET generation=generation+1 WHERE pool_id=?1 AND slot='small'",
        [&pool.id],
    )
    .unwrap();
    let payment = payment_evidence(&tx, "new").unwrap();
    assert_eq!(payment["attempt_id"], "new");
    assert_eq!(
        payment["lifecycle"]["pools"][0]["reserve_claims"][0]["generation"],
        generation + 1
    );
    assert_eq!(
        payment["balance_evidence"],
        json!({"schema_version":3,"estimate_atomic":"7"})
    );
    tx.execute("UPDATE treasury_operations SET facts='not-json'", [])
        .unwrap();
    let optional = payment_evidence(&tx, "new").unwrap();
    assert_eq!(
        optional["lifecycle"],
        json!({"available":false,"reason":"status_observation_unavailable"})
    );
    assert_eq!(optional["amount"], payment["amount"]);
    assert_eq!(optional["balance_evidence"], payment["balance_evidence"]);
    tx.execute("UPDATE payment_attempts SET amount='0' WHERE id='new'", [])
        .unwrap();
    assert!(
        payment_evidence(&tx, "new").is_err(),
        "invalid original admission is required even when lifecycle is unavailable"
    );
    drop(tx);
    assert!(store.qualification_payment("new").is_err());
    assert_eq!(
        store.qualification_lifecycle().unwrap()["pools"][0]["reserve_claims"][0]["generation"],
        generation
    );
}

#[test]
fn all_collections_use_the_passed_pinned_sql_snapshot_after_another_writer_commits() {
    let (dir, store) = fixture();
    let mut writer = Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let tx = store.db.unchecked_transaction().unwrap();
    // Actual first projection collection pins this exact financial read transaction.
    let pools = reporting::read_pools(&tx).unwrap();
    let wallet = pools[0].addresses[0].id.clone();
    let job: String = tx
        .query_row(
            "SELECT id FROM funding_jobs WHERE wallet_id=?1",
            [&wallet],
            |row| row.get(0),
        )
        .unwrap();
    let before = baseline(&tx, &read_status(&tx).unwrap());
    {
        let change = writer.transaction().unwrap();
        change
            .execute("UPDATE wallets SET balance='31' WHERE id=?1", [&wallet])
            .unwrap();
        change
            .execute(
                "UPDATE funding_progress SET phase=?2 WHERE job_id=?1",
                params![
                    job,
                    serde_json::to_string(&funding::FundingPhase::Swapping).unwrap()
                ],
            )
            .unwrap();
        change
            .execute(
                "UPDATE treasury_operations SET attempts=5 WHERE id='a-source'",
                [],
            )
            .unwrap();
        change.commit().unwrap();
    }
    assert_eq!(
        read_lifecycle(&tx, None).unwrap(),
        before,
        "collection reads cannot mix old wallets with new jobs/operations"
    );
    tx.commit().unwrap();
    let after = store.qualification_lifecycle().unwrap();
    assert_eq!(
        after["pools"][0]["addresses"][0]["balance_estimate_atomic"],
        "31"
    );
    assert_eq!(
        after["funding_jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == job)
            .unwrap()["phase"],
        "SWAPPING"
    );
    assert_eq!(after["source_operations"][0]["attempts"], 5);
}
