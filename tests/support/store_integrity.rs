//! Store-level fixtures exercise encryption AAD and independently constructed schemas.
use super::*;
use crate::rotation::transaction::PreparedTransaction;

fn populated(dir: &Path) -> Store {
    let mut s = Store::create(&dir.join("state"), &dir.join("key"), 1, b"initial").unwrap();
    let pool = s.ensure_pool("fixture", "5").unwrap();
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
        s.save_funding_quote(&job.id, format!("quote-{i}").as_bytes())
            .unwrap();
        s.advance_funding(
            &job.id,
            funding::FundingPhase::Quoted,
            funding::FundingPhase::Preparing,
        )
        .unwrap();
        s.reserve(&job.operation_id, Some(&pool), 1, 100, 1000)
            .unwrap();
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
    s.db.execute("INSERT INTO payment_attempts VALUES ('payment',?1,?2,0,'7','hash','POSSIBLY_SUBMITTED',?3,?3,?4,0,1000)",params![pool,wallet.id,wallet.address,format!("0x{:064x}",1)]).unwrap();
    s.db.execute(
        "INSERT INTO payment_anchors VALUES (?1,100,'base-block')",
        [&pool],
    )
    .unwrap();
    s.db.execute("INSERT INTO funding_recovery SELECT 'prior-operation',p.job_id,p.phase,p.quote,r.address FROM funding_progress p JOIN funding_refunds r ON r.job_id=p.job_id ORDER BY p.rowid LIMIT 1",[]).unwrap();
    s
}
#[test]
fn populated_historical_schemas_preserve_liability_and_backfill_identity_rules() {
    let donor = tempfile::tempdir().unwrap();
    let s = populated(donor.path());
    let id = s.id().to_owned();
    let jobs = s.status().unwrap().funding_jobs;
    let revision = s.snapshot().unwrap().0;
    drop(s);
    for version in 0..=10 {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        fs::create_dir(&state).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::copy(donor.path().join("key"), dir.path().join("key")).unwrap();
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
            db.execute_batch(&format!(
                "INSERT INTO main.{table} SELECT * FROM donor.{table};"
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
        let s = Store::open(&state, &dir.path().join("key"), &id).unwrap();
        assert_eq!(
            s.db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            10
        );
        assert_eq!(s.snapshot().unwrap().0, revision);
        assert_eq!(
            s.prepared_bytes(&jobs[1].operation_id).unwrap().as_slice(),
            b"signed-1"
        );
        assert_eq!(
            s.db.query_row("SELECT SUM(reserved) FROM budget_entries", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            100
        );
        if version >= 1 {
            assert_eq!(
                s.db.query_row(
                    "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
        if version >= 4 {
            assert_eq!(s.funding_quote(&jobs[1].id).unwrap().as_slice(), b"quote-1");
            assert_eq!(
                PreparedTransaction::load(&s, &jobs[1].operation_id)
                    .unwrap()
                    .network_identity()
                    .unwrap(),
                crate::network::IsolationId::evm(&jobs[1].recipient).unwrap()
            );
            assert!(
                s.db.query_row(
                    "SELECT started_at FROM funding_health WHERE job_id=?1",
                    [&jobs[1].id],
                    |r| r.get::<_, Option<i64>>(0)
                )
                .unwrap()
                .is_some()
            );
        } else {
            assert!(
                PreparedTransaction::load(&s, &jobs[1].operation_id)
                    .unwrap()
                    .network_identity()
                    .is_err()
            );
        }
        if version >= 5 {
            assert_eq!(
                s.refund_address(&jobs[1].id).unwrap().as_deref(),
                Some("refund-1")
            );
        }
        if version >= 6 {
            assert_eq!(
                s.operation_recipient("prior-operation").unwrap(),
                Some(jobs[0].recipient.clone())
            );
        }
        if version >= 7 {
            assert_eq!(s.status().unwrap().refunds.len(), 1);
        }
        drop(s);
        let reopened = Store::open(&state, &dir.path().join("key"), &id).unwrap();
        assert!(reopened.operation_pending(&jobs[1].operation_id).unwrap());
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
    s.db.execute_batch("PRAGMA user_version=11;").unwrap();
    drop(s);
    assert!(Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).is_err());
    let db = Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        11
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
        let pool = s.ensure_pool("test", "5").unwrap();
        let job = s.funding_jobs().unwrap().remove(0);
        s.save_funding_quote(&job.id, b"quote").unwrap();
        s.advance_funding(
            &job.id,
            funding::FundingPhase::Quoted,
            funding::FundingPhase::Preparing,
        )
        .unwrap();
        s.reserve(&job.operation_id, Some(&pool), 1, 100, 1000)
            .unwrap();
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
