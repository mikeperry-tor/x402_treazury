//! Store-level fixtures exercise encryption AAD and independently constructed schemas.
use super::*;
use crate::test_funding_quote as quote_fixture;

fn populated(dir: &Path) -> Store {
    let mut s = Store::create(&dir.join("state"), &dir.join("key"), 1, b"initial").unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        s,
        "fixture",
        crate::rotation::test_capacity::small_reserve("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let jobs = s.funding_jobs().unwrap();
    for (i, job) in jobs.iter().enumerate() {
        let revision = s.snapshot().unwrap().0;
        s.save_refund_address(
            &job.id,
            &format!("refund-{i}"),
            revision,
            b"refund snapshot",
        )
        .unwrap();
        s.save_funding_quote(
            &job.id,
            &quote_fixture::bound(&job.target, &job.recipient, &format!("quote-{i}")),
        )
        .unwrap();
        s.advance_funding(
            &job.id,
            funding::FundingPhase::Quoted,
            funding::FundingPhase::Preparing,
        )
        .unwrap();
        s.reserve(&job.operation_id, Some(&pool), 1, 100).unwrap();
        let revision = s.snapshot().unwrap().0;
        s.prepare_with_facts(
            &job.operation_id,
            revision,
            format!("prepared-{i}").as_bytes(),
            format!("signed-{i}").as_bytes(),
            Some(TransactionFacts {
                txid: format!("tx-{i}"),
                expiry_height: 100,
                amount_zatoshis: 80,
                fee_zatoshis: 20,
                deadline: 1000,
            }),
        )
        .unwrap();
        s.request_broadcast(&job.operation_id, 1, 1, false).unwrap();
        if i == 0 {
            s.confirm_spend(&job.operation_id, 100, 1).unwrap();
            s.record_refund(refunds::RefundStatus {
                operation_id: job.operation_id.clone(),
                txid: "refund-tx".into(),
                output_index: 0,
                amount: 10,
                height: 10,
            })
            .unwrap();
        }
        s.record_credit(&job.wallet_id, "5000000", "base-block", 100)
            .unwrap();
    }
    let wallet = &s.status().unwrap().pools[0].addresses[0];
    s.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence) VALUES ('payment',?1,?2,0,'7','hash','POSSIBLY_SUBMITTED',?3,?3,?4,0,1000,1)",params![pool,wallet.id,wallet.address,format!("0x{:064x}",1)]).unwrap();
    evidence::add_exposure(&s.db, &wallet.id, U256::from(7)).unwrap();
    s.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote,refund) SELECT 'prior-operation',p.job_id,p.phase,p.quote,r.address FROM funding_progress p JOIN funding_refunds r ON r.job_id=p.job_id ORDER BY p.rowid LIMIT 1",[]).unwrap();
    s
}
// Frozen historical schemas still contain these fields. Populate them explicitly
// in legacy donors instead of making current runtime schemas depend on them.
fn add_obsolete_metadata(store: &Store) {
    store.db.execute_batch("CREATE TABLE payment_anchors(pool_id TEXT PRIMARY KEY,height INTEGER NOT NULL,hash TEXT NOT NULL); ALTER TABLE funding_progress ADD COLUMN turn INTEGER NOT NULL DEFAULT 0; CREATE INDEX funding_schedule_turn ON funding_progress(turn); INSERT INTO payment_anchors SELECT id,100,'base-block' FROM pools;").unwrap();
}

#[test]
fn populated_historical_schemas_require_explicit_conversion_without_mutation() {
    let donor = tempfile::tempdir().unwrap();
    let s = populated(donor.path());
    add_obsolete_metadata(&s);
    let id = s.id().to_owned();
    drop(s);
    for version in 0..=12 {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        fs::create_dir(&state).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::copy(donor.path().join("key"), dir.path().join("key")).unwrap();
        private_file(&state.join("owner.lock"), true).unwrap();
        private_file(&state.join("state.sqlite"), true).unwrap();
        let db = Connection::open(state.join("state.sqlite")).unwrap();
        let mut section = 0;
        for line in include_str!("../fixtures/state/historical.sql").lines() {
            if let Some(n) = line.strip_prefix("-- version ") {
                section = n.parse::<u32>().unwrap();
            } else if !line.starts_with("--") && section <= version {
                db.execute_batch(line).unwrap();
            }
        }
        db.execute(
            "ATTACH DATABASE ?1 AS donor",
            [donor.path().join("state/state.sqlite").to_str().unwrap()],
        )
        .unwrap();
        let tables = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY rowid")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        db.execute_batch("BEGIN;").unwrap();
        for table in tables {
            let columns = db
                .prepare(&format!("PRAGMA main.table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join(",");
            let projection = columns
                .split(',')
                .map(|column| match (table.as_str(), column) {
                    ("pools", "target") => "'5000000' AS target",
                    ("pools", "bootstrapped") => "1 AS bootstrapped",
                    _ => column,
                })
                .collect::<Vec<_>>()
                .join(",");
            db.execute_batch(&format!(
                "INSERT INTO main.{table}({columns}) SELECT {projection} FROM donor.{table};"
            ))
            .unwrap();
        }
        let violations = db
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(
            violations.is_empty(),
            "historical version {version}: {violations:?}"
        );
        db.execute_batch(&format!(
            "PRAGMA user_version={version}; COMMIT; DETACH DATABASE donor;"
        ))
        .unwrap_or_else(|e| panic!("version {version}: {e}"));
        drop(db);
        let before = fs::read(state.join("state.sqlite")).unwrap();
        for _ in 0..2 {
            let error = Store::open(&state, &dir.path().join("key"), &id)
                .err()
                .expect("occupied legacy state must require explicit conversion");
            assert!(
                error
                    .to_string()
                    .contains("stopped-service conversion required"),
                "version {version}: {error}"
            );
            assert_eq!(fs::read(state.join("state.sqlite")).unwrap(), before);
            let db = Connection::open_with_flags(
                state.join("state.sqlite"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            assert_eq!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                version
            );
            assert_eq!(
                db.query_row("SELECT SUM(reserved) FROM budget_entries", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                100
            );
        }
    }
}

#[test]
fn encrypted_record_damage_and_substitution_reject_at_real_store_boundaries() {
    for field in ["snapshot", "wallet", "quote", "outgoing", "refund"] {
        for mutation in [
            "truncate",
            "nonce",
            "ciphertext",
            "tag",
            "swap",
            "cross_type",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let s = populated(dir.path());
            let id = s.id().to_owned();
            let status = s.status().unwrap();
            let jobs = &status.funding_jobs;
            let wallet = &status.pools[0].addresses[1];
            let (table, column, key, target, source) = match field {
                "snapshot" => (
                    "snapshots",
                    "bytes",
                    "revision",
                    s.snapshot().unwrap().0.to_string(),
                    s.db.query_row("SELECT MIN(revision) FROM snapshots", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap()
                    .to_string(),
                ),
                "wallet" => (
                    "wallets",
                    "key",
                    "id",
                    wallet.id.clone(),
                    status.pools[0].addresses[0].id.clone(),
                ),
                "quote" => (
                    "funding_progress",
                    "quote",
                    "job_id",
                    jobs[1].id.clone(),
                    jobs[0].id.clone(),
                ),
                "outgoing" => (
                    "outgoing",
                    "raw",
                    "id",
                    jobs[1].operation_id.clone(),
                    jobs[0].operation_id.clone(),
                ),
                _ => (
                    "funding_refunds",
                    "address",
                    "job_id",
                    jobs[1].id.clone(),
                    jobs[0].id.clone(),
                ),
            };
            let from = if mutation == "swap" { &source } else { &target };
            let mut bytes: Vec<u8> =
                s.db.query_row(
                    &format!("SELECT {column} FROM {table} WHERE {key}=?1"),
                    [from],
                    |r| r.get(0),
                )
                .unwrap();
            if mutation == "cross_type" {
                bytes = if field == "outgoing" {
                    s.db.query_row(
                        "SELECT bytes FROM snapshots ORDER BY revision DESC LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap()
                } else {
                    s.db.query_row("SELECT raw FROM outgoing ORDER BY rowid LIMIT 1", [], |r| {
                        r.get(0)
                    })
                    .unwrap()
                };
            }
            match mutation {
                "truncate" => bytes.truncate(27),
                "nonce" => bytes[0] ^= 1,
                "ciphertext" => bytes[12] ^= 1,
                "tag" => {
                    let n = bytes.len();
                    bytes[n - 1] ^= 1;
                }
                _ => {}
            }
            s.db.execute(
                &format!("UPDATE {table} SET {column}=?1 WHERE {key}=?2"),
                params![bytes, target],
            )
            .unwrap();
            drop(s);
            let opened = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id);
            if field == "snapshot" {
                assert!(opened.is_err(), "{field} {mutation}");
                continue;
            }
            let s = opened.unwrap();
            let rejected = match field {
                "wallet" => s.wallet_secret(&status.pools[0].id, &wallet.id).is_err(),
                "quote" => s.funding_quote(&jobs[1].id).is_err(),
                "outgoing" => s.prepared_bytes(&jobs[1].operation_id).is_err(),
                _ => s.refund_address(&jobs[1].id).is_err(),
            };
            assert!(rejected, "{field} {mutation}");
            assert_eq!(
                s.db.query_row("SELECT SUM(reserved) FROM budget_entries", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                100
            );
        }
    }
}

#[test]
fn same_key_different_treasury_cannot_exchange_snapshot_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let s = populated(dir.path());
    let id = s.id().to_owned();
    let revision = s.snapshot().unwrap().0;
    let other_dir = tempfile::tempdir().unwrap();
    let mut other = Store::create(
        &other_dir.path().join("state"),
        &other_dir.path().join("key"),
        1,
        b"other",
    )
    .unwrap();
    other.key = Zeroizing::new(*s.key);
    fs::write(other_dir.path().join("key"), other.key.as_slice()).unwrap();
    for n in 1..revision {
        other.save_snapshot(n, b"foreign snapshot").unwrap();
    }
    assert_eq!(other.snapshot().unwrap().1.as_slice(), b"foreign snapshot");
    let cipher: Vec<u8> = other
        .db
        .query_row(
            "SELECT bytes FROM snapshots WHERE revision=?1",
            [revision],
            |r| r.get(0),
        )
        .unwrap();
    s.db.execute(
        "UPDATE snapshots SET bytes=?1 WHERE revision=?2",
        params![cipher, revision],
    )
    .unwrap();
    drop(s);
    drop(other);
    assert!(Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).is_err());
}

#[test]
fn future_schema_is_refused_and_referenced_snapshots_survive_repeated_saves() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = populated(dir.path());
    let id = s.id().to_owned();
    let jobs = s.status().unwrap().funding_jobs;
    let pinned =
        s.db.prepare("SELECT revision FROM outgoing ORDER BY revision")
            .unwrap()
            .query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
    let mut revision = s.snapshot().unwrap().0;
    for _ in 0..20 {
        revision = s.save_snapshot(revision, b"fresh checkpoint").unwrap();
    }
    let snapshots =
        s.db.prepare("SELECT revision FROM snapshots ORDER BY revision")
            .unwrap()
            .query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
    let mut expected = pinned;
    expected.push(revision);
    assert_eq!(snapshots, expected);
    drop(s);
    let s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    for (i, job) in jobs.iter().enumerate() {
        assert_eq!(
            s.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            format!("signed-{i}").as_bytes()
        );
        #[cfg(feature = "zcash")]
        assert_eq!(
            s.prepared_snapshot(&job.operation_id).unwrap().as_slice(),
            format!("prepared-{i}").as_bytes()
        );
    }
    s.db.execute_batch("PRAGMA user_version=999;").unwrap();
    drop(s);
    assert!(Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).is_err());
    let db = Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        999
    );
    assert_eq!(
        db.query_row("SELECT SUM(reserved) FROM budget_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        100
    );
}

#[test]
fn preparation_and_refund_write_failures_roll_back_through_reopen() {
    for statement in [
        "CREATE TRIGGER injected BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'injected'); END;",
        "CREATE TRIGGER injected BEFORE INSERT ON treasury_operations BEGIN SELECT RAISE(ABORT,'injected'); END;",
        "CREATE TRIGGER injected BEFORE UPDATE ON budget_entries BEGIN SELECT RAISE(ABORT,'injected'); END;",
        "CREATE TRIGGER injected BEFORE UPDATE ON funding_progress BEGIN SELECT RAISE(ABORT,'injected'); END;",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state");
        let key = tmp.path().join("key");
        let mut s = Store::create(&dir, &key, 1, b"initial").unwrap();
        let id = s.id.clone();
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            s,
            "test",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let job = s.funding_jobs().unwrap().remove(0);
        s.save_funding_quote(
            &job.id,
            &quote_fixture::bound(&job.target, &job.recipient, "quote"),
        )
        .unwrap();
        s.advance_funding(
            &job.id,
            funding::FundingPhase::Quoted,
            funding::FundingPhase::Preparing,
        )
        .unwrap();
        s.reserve(&job.operation_id, Some(&pool), 1, 100).unwrap();
        let original = serde_json::to_value(s.status().unwrap()).unwrap();
        s.db.execute_batch(statement).unwrap();
        assert!(
            s.prepare_with_facts(
                &job.operation_id,
                1,
                b"calculated",
                b"signed",
                Some(TransactionFacts {
                    txid: "tx".into(),
                    expiry_height: 100,
                    amount_zatoshis: 70,
                    fee_zatoshis: 20,
                    deadline: 1000
                })
            )
            .is_err()
        );
        drop(s);
        let mut s = Store::open(&dir, &key, &id).unwrap();
        assert_eq!(serde_json::to_value(s.status().unwrap()).unwrap(), original);
        assert_eq!(s.snapshot().unwrap().1.as_slice(), b"initial");
        assert!(s.prepared_bytes(&job.operation_id).is_err());
        assert!(s.operation(&job.operation_id).is_err());
        s.db.execute_batch("DROP TRIGGER injected").unwrap();
        s.prepare_with_facts(
            &job.operation_id,
            1,
            b"calculated",
            b"signed",
            Some(TransactionFacts {
                txid: "tx".into(),
                expiry_height: 100,
                amount_zatoshis: 70,
                fee_zatoshis: 20,
                deadline: 1000,
            }),
        )
        .unwrap();
        s.confirm_spend(&job.operation_id, 90, 1).unwrap();
        let original = serde_json::to_value(s.status().unwrap()).unwrap();
        s.db.execute_batch("CREATE TRIGGER injected AFTER INSERT ON refund_outputs BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        let refund = refunds::RefundStatus {
            txid: "refund".into(),
            output_index: 0,
            operation_id: job.operation_id,
            amount: 20,
            height: 100,
        };
        assert!(s.record_refund(refund.clone()).is_err());
        drop(s);
        let mut s = Store::open(&dir, &key, &id).unwrap();
        assert_eq!(serde_json::to_value(s.status().unwrap()).unwrap(), original);
        s.db.execute_batch("DROP TRIGGER injected").unwrap();
        s.record_refund(refund.clone()).unwrap();
        s.record_refund(refund).unwrap();
        assert_eq!(s.status().unwrap().refunds.len(), 1);
    }
}

#[test]
fn expired_admission_evidence_preserves_prior_balance_without_role_change_or_reservation() {
    use crate::rotation::base::{Anchor, ChainView};
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let id = store.id().to_owned();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        store,
        "freshness",
        crate::rotation::test_capacity::small_reserve("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let jobs = store.funding_jobs().unwrap();
    for job in &jobs {
        store
            .record_credit(&job.wallet_id, "5000000", "old", 10)
            .unwrap();
    }
    let candidate = store.payment_candidate(&pool).unwrap();
    let view = ChainView {
        observations: Default::default(),
        balance_certificates: Default::default(),
        resolution_certificates: Default::default(),
        admission_valid_until: 0,
        anchor: Anchor {
            height: 20,
            hash: "historical".into(),
        },
        balances: [
            (jobs[0].wallet_id.clone(), U256::ZERO),
            (jobs[1].wallet_id.clone(), U256::from(5_000_000)),
        ]
        .into(),
        released: vec![],
        resolutions: Default::default(),
    };
    let error = store
        .admit_for(&pool, U256::from(1), "offer", view, Some(&candidate))
        .err()
        .unwrap();
    assert!(error.to_string().contains("fresh Base balances required"));
    drop(store);
    let mut store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    let after = store.payment_candidate(&pool).unwrap();
    assert_eq!(after.wallet, candidate.wallet);
    assert_eq!(after.generation, candidate.generation);
    assert_eq!(store.funding_jobs().unwrap().len(), 2);
    let attempts: i64 = store
        .db
        .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(attempts, 0);
    let (balance, height): (String, i64) = store
        .db
        .query_row(
            "SELECT balance,block_height FROM wallets WHERE id=?1",
            [&candidate.wallet],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((balance.as_str(), height), ("5000000", 10));
}

#[test]
fn admission_skips_retired_balances_but_retains_their_authorizations() {
    let dir = tempfile::tempdir().unwrap();
    let store = populated(dir.path());
    let pool = store.status().unwrap().pools[0].id.clone();
    let wallet: String = store
        .db
        .query_row(
            "SELECT wallet_id FROM payment_attempts WHERE id='payment'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    store
        .db
        .execute("UPDATE wallets SET role='RETIRED' WHERE id=?1", [&wallet])
        .unwrap();
    assert!(
        store
            .chain_query(&pool)
            .unwrap()
            .wallets
            .iter()
            .any(|(id, _)| id == &wallet)
    );
    let query = store.payment_chain_query(&pool).unwrap();
    assert!(!query.wallets.iter().any(|(id, _)| id == &wallet));
    assert!(query.pending.iter().any(|auth| auth.wallet == wallet));
}

#[test]
fn unsigned_queries_and_rejected_legacy_reopen_preserve_all_journals() {
    let donor = tempfile::tempdir().unwrap();
    let store = populated(donor.path());
    add_obsolete_metadata(&store);
    let id = store.id().to_owned();
    let pool = store.status().unwrap().pools.remove(0).id;
    store.chain_query(&pool).unwrap();
    store.payment_chain_query(&pool).unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(store);

    // Construct an actual frozen legacy database. Fresh schema16 deliberately
    // cannot admit unsigned rows; never relax its CHECK to manufacture this case.
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    fs::create_dir(&state).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::copy(donor.path().join("key"), dir.path().join("key")).unwrap();
    private_file(&state.join("owner.lock"), true).unwrap();
    let database = state.join("state.sqlite");
    private_file(&database, true).unwrap();
    let db = Connection::open(&database).unwrap();
    db.execute_batch(include_str!("../fixtures/state/historical.sql"))
        .unwrap();
    db.execute(
        "ATTACH DATABASE ?1 AS donor",
        [donor.path().join("state/state.sqlite").to_str().unwrap()],
    )
    .unwrap();
    let tables = db
        .prepare("SELECT name FROM main.sqlite_master WHERE type='table' ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for table in tables {
        let columns = db
            .prepare(&format!("PRAGMA main.table_info({table})"))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let projection = columns
            .iter()
            .map(|column| match (table.as_str(), column.as_str()) {
                ("pools", "target") => "'5000000' AS target",
                ("pools", "bootstrapped") => "1 AS bootstrapped",
                _ => column.as_str(),
            })
            .collect::<Vec<_>>()
            .join(",");
        db.execute_batch(&format!(
            "INSERT INTO main.{table}({}) SELECT {projection} FROM donor.{table}",
            columns.join(",")
        ))
        .unwrap();
    }
    db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) SELECT 'unsigned',pool_id,wallet_id,generation,amount,requirements_hash,'ADMITTED' FROM payment_attempts WHERE id='payment'", []).unwrap();
    db.execute_batch("PRAGMA user_version=12; DETACH DATABASE donor;")
        .unwrap();
    drop(db);
    let before = fs::read(&database).unwrap();
    let error = Store::open(&state, &dir.path().join("key"), &id)
        .err()
        .expect("legacy state requires explicit conversion");
    assert!(
        error
            .to_string()
            .contains("stopped-service conversion required")
    );
    assert_eq!(fs::read(&database).unwrap(), before);
    let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let rows: Vec<(String, String)> = db
        .prepare("SELECT id,state FROM payment_attempts ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("payment".into(), "POSSIBLY_SUBMITTED".into()),
            ("unsigned".into(), "ADMITTED".into())
        ]
    );
}

#[tokio::test]
async fn accepted_group_job_keeps_ownership_after_waiter_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let state = populated(dir.path());
    let pool = state.status().unwrap().pools.remove(0).id;
    let (store, worker) = StoreHandle::spawn(state);
    let gate = store.group_gate(&pool).unwrap();
    let guard = std::sync::Arc::new(gate.clone().lock_owned().await);
    let (started, accepted) = tokio::sync::oneshot::channel();
    let (release, paused) = std::sync::mpsc::channel();
    let waiter = tokio::spawn({
        let store = store.clone();
        async move {
            store
                .call_with_guard(guard, move |s| {
                    started.send(()).unwrap();
                    paused.recv().unwrap();
                    s.chain_query(&pool).map(|_| ())
                })
                .await
        }
    });
    accepted.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(
        gate.try_lock().is_err(),
        "accepted work still owns the group"
    );
    release.send(()).unwrap();
    store.call(|_| Ok(())).await.unwrap();
    assert!(gate.try_lock().is_ok(), "finished work releases the group");
    drop(store);
    worker.await.unwrap();
}

fn prepared_payment(store: &mut Store) -> PreparedPayment {
    let pool = store.status().unwrap().pools.remove(0);
    let view = crate::rotation::base::ChainView {
        observations: Default::default(),
        balance_certificates: Default::default(),
        resolution_certificates: Default::default(),
        admission_valid_until: crate::rotation::base::now().unwrap() + 120,
        anchor: crate::rotation::base::Anchor {
            height: 100,
            hash: "base-block".into(),
        },
        balances: pool
            .addresses
            .iter()
            .map(|w| (w.id.clone(), U256::from(5_000_000)))
            .collect(),
        released: vec![],
        resolutions: Default::default(),
    };
    store
        .prepare_payment(&pool.id, U256::from(11), "atomic-offer", view, None)
        .unwrap()
}
fn prepared_authorization(prepared: &PreparedPayment) -> Authorization {
    Authorization {
        payer: prepared.address.clone(),
        payee: format!("0x{:040x}", 2),
        nonce: format!("0x{:064x}", 2),
        valid_after: 0,
        valid_before: crate::rotation::base::now().unwrap() + 100,
    }
}
fn assert_atomic_counts(store: &Store, signed: i64, evidence: i64) {
    let count: i64 = store
        .db
        .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, signed);
    let unsigned: i64 = store
        .db
        .query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='ADMITTED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unsigned, 0);
    let count: i64 = store
        .db
        .query_row("SELECT COUNT(*) FROM payment_admission_evidence", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, evidence);
    let wallets = store
        .db
        .prepare("SELECT id,unresolved_exposure FROM wallets")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for (wallet, aggregate) in wallets {
        let journal = store
            .db
            .prepare("SELECT amount FROM payment_attempts WHERE wallet_id=?1 AND state!='RESOLVED'")
            .unwrap()
            .query_map([wallet], |row| row.get::<_, String>(0))
            .unwrap()
            .fold(U256::ZERO, |sum, value| {
                sum.checked_add(amount(&value.unwrap()).unwrap()).unwrap()
            });
        assert_eq!(amount(&aggregate).unwrap(), journal);
    }
}

#[test]
fn atomic_payment_preparation_is_unreserved_and_commit_rechecks_authority() {
    for failure in [
        "none",
        "evidence",
        "expiry",
        "signature_expiry",
        "ownership",
        "capacity",
        "cap",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = populated(dir.path());
        let mut prepared = prepared_payment(&mut store);
        assert_atomic_counts(&store, 1, 0);
        store.chain_query(&prepared.pool).unwrap();
        store.payment_chain_query(&prepared.pool).unwrap();
        let mut auth = prepared_authorization(&prepared);
        match failure {
            "evidence" => store.db.execute_batch("CREATE TRIGGER fail_evidence BEFORE INSERT ON payment_admission_evidence BEGIN SELECT RAISE(ABORT,'injected support failure'); END;").unwrap(),
            "expiry" => prepared.valid_until = 0,
            "signature_expiry" => auth.valid_before = 1,
            "ownership" => store.disable_pool("fixture").unwrap(),
            "capacity" => {
                let tx = store.db.transaction().unwrap();
                tx.execute("UPDATE payment_attempts SET amount='4999999' WHERE id='payment'", []).unwrap();
                evidence::add_exposure(&tx, &prepared.wallet, U256::from(4_999_992)).unwrap();
                tx.commit().unwrap();
            },
            "cap" => { store.db.execute("UPDATE pools SET payment_limit='10' WHERE id=?1", [&prepared.pool]).unwrap(); },
            _ => {}
        }
        let result = store.journal_authorization(&prepared, auth, None);
        if failure == "none" {
            result.unwrap();
            assert_atomic_counts(&store, 2, 1);
            let saved: (String, String) = store
                .db
                .query_row(
                    "SELECT balance,reserved_before FROM payment_admission_evidence",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(saved, ("5000000".into(), "7".into()));
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                error.is::<AdmissionExpired>(),
                matches!(failure, "expiry" | "signature_expiry")
            );
            assert_atomic_counts(&store, 1, 0);
        }
        let id = store.id().to_owned();
        drop(store);
        let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        assert_atomic_counts(
            &store,
            if failure == "none" { 2 } else { 1 },
            i64::from(failure == "none"),
        );
    }
}

#[tokio::test]
async fn cancelled_atomic_commit_keeps_complete_signed_liability_and_support() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = populated(dir.path());
    let id = state.id().to_owned();
    let prepared = prepared_payment(&mut state);
    let auth = prepared_authorization(&prepared);
    let pool = prepared.pool.clone();
    let (store, worker) = StoreHandle::spawn(state);
    let gate = store.group_gate(&pool).unwrap();
    let guard = std::sync::Arc::new(gate.clone().lock_owned().await);
    let (started, accepted) = tokio::sync::oneshot::channel();
    let (release, paused) = std::sync::mpsc::channel();
    let waiter = tokio::spawn({
        let store = store.clone();
        async move {
            store
                .call_with_guard(guard, move |s| {
                    started.send(()).unwrap();
                    paused.recv().unwrap();
                    s.journal_authorization(&prepared, auth, None)
                })
                .await
        }
    });
    accepted.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(gate.try_lock().is_err());
    release.send(()).unwrap();
    store
        .call(|s| {
            assert_atomic_counts(s, 2, 1);
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    worker.await.unwrap();
    let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_atomic_counts(&store, 2, 1);
}

#[test]
#[ignore = "subprocess helper invoked by atomic_payment_survives_process_exit"]
fn atomic_payment_process_child() {
    let dir = std::path::PathBuf::from(std::env::var_os("X402_ATOMIC_FIXTURE_DIR").unwrap());
    let id = std::env::var("X402_ATOMIC_FIXTURE_ID").unwrap();
    let mut store = Store::open(&dir.join("state"), &dir.join("key"), &id).unwrap();
    let prepared = prepared_payment(&mut store);
    if std::env::var("X402_ATOMIC_FIXTURE_PHASE").unwrap() == "committed" {
        store
            .journal_authorization(&prepared, prepared_authorization(&prepared), None)
            .unwrap();
    }
    // Neither SQLite nor the owning store/gate receives a destructor here.
    std::process::exit(0);
}

#[test]
fn atomic_payment_survives_process_exit_before_and_after_commit() {
    for (phase, committed) in [("prepared", false), ("committed", true)] {
        let dir = tempfile::tempdir().unwrap();
        let store = populated(dir.path());
        let id = store.id().to_owned();
        drop(store);
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "rotation::store::integrity_tests::atomic_payment_process_child",
                "--ignored",
            ])
            .env("X402_ATOMIC_FIXTURE_DIR", dir.path())
            .env("X402_ATOMIC_FIXTURE_ID", &id)
            .env("X402_ATOMIC_FIXTURE_PHASE", phase)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        assert_atomic_counts(&store, 1 + i64::from(committed), i64::from(committed));
    }
}

#[tokio::test]
async fn owner_panic_never_acknowledges_or_loses_an_atomic_payment() {
    for committed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut state = populated(dir.path());
        let id = state.id().to_owned();
        let prepared = prepared_payment(&mut state);
        let auth = prepared_authorization(&prepared);
        let (store, worker) = StoreHandle::spawn(state);
        let gate = store.group_gate(&prepared.pool).unwrap();
        let guard = std::sync::Arc::new(gate.clone().lock_owned().await);
        let result: Result<()> = store
            .call_with_guard(guard, move |s| {
                if committed {
                    s.journal_authorization(&prepared, auth, None)?;
                }
                panic!("injected atomic payment owner failure");
            })
            .await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("store worker stopped")
        );
        assert!(worker.await.unwrap_err().is_panic());
        assert!(gate.try_lock().is_ok());
        drop(store);
        let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        assert_atomic_counts(&store, 1 + i64::from(committed), i64::from(committed));
    }
}

#[test]
fn obsolete_capacity_metadata_is_optional_and_never_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path());
    let pool = store.status().unwrap().pools[0].id.clone();
    let id = store.id().to_owned();
    let obsolete_count: i64 = store.db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('payment_anchors','funding_schedule_turn')",[],|r|r.get(0)).unwrap();
    assert_eq!(obsolete_count, 0);
    let has_turn: bool = store
        .db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('funding_progress') WHERE name='turn')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!has_turn);
    let view = || super::super::base::ChainView {
        anchor: super::super::base::Anchor {
            height: 200,
            hash: "new-observation".into(),
        },
        admission_valid_until: u64::MAX,
        balances: Default::default(),
        observations: Default::default(),
        balance_certificates: Default::default(),
        resolution_certificates: Default::default(),
        released: vec![],
        resolutions: Default::default(),
    };
    store.publish_scoped_evidence(&pool, view()).unwrap();
    // Reproduce the old schema16 layout with retained metadata. Opening and
    // publishing new evidence must neither require nor rewrite these old fields.
    store.db.execute_batch("CREATE TABLE payment_anchors(pool_id TEXT PRIMARY KEY,height INTEGER NOT NULL,hash TEXT NOT NULL); ALTER TABLE funding_progress ADD COLUMN turn INTEGER NOT NULL DEFAULT 0; CREATE INDEX funding_schedule_turn ON funding_progress(turn); UPDATE funding_progress SET turn=7;").unwrap();
    store
        .db
        .execute(
            "INSERT INTO payment_anchors VALUES (?1,100,'original')",
            [&pool],
        )
        .unwrap();
    drop(store);
    let mut reopened =
        Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    reopened.publish_scoped_evidence(&pool, view()).unwrap();
    let anchor: (i64, String) = reopened
        .db
        .query_row(
            "SELECT height,hash FROM payment_anchors WHERE pool_id=?1",
            [&pool],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(anchor, (100, "original".into()));
    assert_eq!(
        reopened
            .db
            .query_row("SELECT MIN(turn) FROM funding_progress", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        7
    );
}
