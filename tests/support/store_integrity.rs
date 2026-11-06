//! Store-level fixtures exercise encryption AAD and independently constructed schemas.
use super::*;
use crate::test_funding_quote as quote_fixture;

#[test]
fn existing_wallet_directory_errors_do_not_suggest_initialization() {
    let tmp = tempfile::tempdir().unwrap();
    // An existing empty directory is not a new initialization destination.
    let error = treasury_id(tmp.path()).unwrap_err();
    assert!(!format!("{error:#}").contains("wallet init"));
    // A file in place of the directory must retain the existing validation error.
    let file = tmp.path().join("file");
    fs::write(&file, b"").unwrap();
    let error = treasury_id(&file).unwrap_err();
    assert!(!format!("{error:#}").contains("wallet init"));
    assert!(directory(&file).is_err());
}

fn populated(dir: &Path) -> Store {
    let mut s = Store::create(&dir.join("state"), &dir.join("key"), 1, b"initial").unwrap();
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        s,
        "fixture",
        crate::rotation::test_capacity::two_reserves("5"),
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
        s.accept_preparation(&job.operation_id, Some(&pool), 100, None)
            .unwrap();
        let revision = s.snapshot().unwrap().0;
        s.prepare_with_facts(
            &job.operation_id,
            revision,
            format!("prepared-{i}").as_bytes(),
            crate::rotation::store::test_support::transaction_raw(100 + i as u32).as_slice(),
            Some(TransactionFacts {
                txid: crate::rotation::store::test_support::transaction_txid(100 + i as u32),
                expiry_height: 100 + i as u32,
                amount_zatoshis: 80,
                fee_zatoshis: 20,
                deadline: Some(1000),
            }),
        )
        .unwrap();
        s.request_broadcast(&job.operation_id, 1, 1, false).unwrap();
        if i == 0 {
            s.confirm_spend(&job.operation_id).unwrap();
        }
        s.record_credit(&job.wallet_id, "5000000", "base-block", 100)
            .unwrap();
    }
    let wallet = &s.status().unwrap().pools[0].addresses[0];
    // This donor models an established payer, including its consumed reserve claim.
    let tx = s.db.transaction().unwrap();
    capacity::consume_reserve(&tx, &pool, &wallet.id).unwrap();
    tx.commit().unwrap();
    s.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence) VALUES ('payment',?1,?2,0,'7','hash','POSSIBLY_SUBMITTED',?3,?3,?4,0,1000,1)",params![pool,wallet.id,wallet.address,format!("0x{:064x}",1)]).unwrap();
    evidence::add_exposure(&s.db, &wallet.id, U256::from(7)).unwrap();
    s.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote,refund) SELECT 'prior-operation',p.job_id,p.phase,p.quote,r.address FROM funding_progress p JOIN funding_refunds r ON r.job_id=p.job_id ORDER BY p.rowid LIMIT 1",[]).unwrap();
    s
}
#[test]
fn unsupported_schema_boundaries_reject_without_mutating_accepted_work() {
    for version in [22, 24] {
        let dir = tempfile::tempdir().unwrap();
        let store = populated(dir.path());
        let id = store.id().to_owned();
        store
            .db
            .pragma_update(None, "user_version", version)
            .unwrap();
        drop(store);
        let state = dir.path().join("state");
        let contents = || {
            fs::read_dir(&state)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (entry.file_name(), fs::read(entry.path()).unwrap())
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let before = contents();
        assert!(Store::open(&state, &dir.path().join("key"), &id).is_err());
        let after_owner = contents();
        assert!(
            after_owner == before,
            "owner rejection changed state directory entries: {:?}",
            after_owner.keys().collect::<Vec<_>>()
        );
        assert!(status(&state).is_err());
        assert!(treasury_id(&state).is_err());
        assert!(static_ownership(&state).is_err());
        assert!(inspect_wallet_snapshot(&state, &dir.path().join("key"), Some(&id)).is_err());
        let after = contents();
        // Live read-only WAL readers may create SQLite coordination files.
        // Existing database and financial journals must remain exact.
        for (name, bytes) in &before {
            if name != "state.sqlite-shm" {
                assert!(
                    after.get(name) == Some(bytes),
                    "inspection changed {name:?}"
                );
            }
        }
        for name in after.keys().filter(|name| !before.contains_key(*name)) {
            assert!(
                name == "state.sqlite-wal" || name == "state.sqlite-shm",
                "inspection created {name:?}"
            );
            if name == "state.sqlite-wal" {
                assert!(
                    after[name].is_empty(),
                    "inspection wrote a new financial journal"
                );
            }
        }
    }
}

#[test]
fn invalid_or_ambiguous_runtime_identity_is_rejected_without_repair() {
    for statement in [
        "UPDATE instance SET id='invalid-uuid'",
        "INSERT INTO instance SELECT '00000000-0000-0000-0000-000000000001',version,birthday,network,account FROM instance",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = populated(dir.path());
        store.db.execute_batch(statement).unwrap();
        drop(store);
        let state = dir.path().join("state");
        let before = fs::read(state.join("state.sqlite")).unwrap();
        assert!(treasury_id(&state).is_err());
        assert!(static_ownership(&state).is_err());
        assert_eq!(fs::read(state.join("state.sqlite")).unwrap(), before);
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
                s.db.query_row("SELECT COUNT(*) FROM outgoing WHERE state='PREPARED' AND preparation IS NOT NULL", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
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
            crate::rotation::store::test_support::transaction_raw(100 + i as u32).as_slice()
        );
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
        db.query_row(
            "SELECT COUNT(*) FROM outgoing WHERE state='PREPARED' AND preparation IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn preparation_write_failures_roll_back_through_reopen() {
    for statement in [
        "CREATE TRIGGER injected BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'injected'); END;",
        "CREATE TRIGGER injected BEFORE INSERT ON treasury_operations BEGIN SELECT RAISE(ABORT,'injected'); END;",
        "CREATE TRIGGER injected BEFORE UPDATE OF preparation ON operation_network BEGIN SELECT RAISE(ABORT,'injected'); END;",
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
            crate::rotation::test_capacity::two_reserves("5"),
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
        s.accept_preparation(&job.operation_id, Some(&pool), 90, None)
            .unwrap();
        let original = serde_json::to_value(s.status().unwrap()).unwrap();
        s.db.execute_batch(statement).unwrap();
        assert!(
            s.prepare_with_facts(
                &job.operation_id,
                1,
                b"calculated",
                crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                Some(TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(100),
                    expiry_height: 100,
                    amount_zatoshis: 70,
                    fee_zatoshis: 20,
                    deadline: Some(1000)
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
            crate::rotation::store::test_support::transaction_raw(100).as_slice(),
            Some(TransactionFacts {
                txid: crate::rotation::store::test_support::transaction_txid(100),
                expiry_height: 100,
                amount_zatoshis: 70,
                fee_zatoshis: 20,
                deadline: Some(1000),
            }),
        )
        .unwrap();
        s.confirm_spend(&job.operation_id).unwrap();
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
        crate::rotation::test_capacity::two_reserves("5"),
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
    store
        .admit_for(&pool, U256::from(1), "offer", view, Some(&candidate))
        .unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    let after = store.payment_candidate(&pool).unwrap();
    assert_eq!(after.wallet, candidate.wallet);
    assert_eq!(after.address, candidate.address);
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
fn unsigned_queries_preserve_current_journals() {
    let dir = tempfile::tempdir().unwrap();
    let store = populated(dir.path());
    let pool = store.status().unwrap().pools.remove(0).id;
    let before = store.db.total_changes();
    store.chain_query(&pool).unwrap();
    store.payment_chain_query(&pool).unwrap();
    let after = store.db.total_changes();
    assert_eq!(before, after);
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
}

#[test]
fn legacy_pool_generation_is_report_metadata_not_admission_authority() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path());
    let prepared = prepared_payment(&mut store);
    store
        .db
        .execute(
            "UPDATE pools SET generation=7 WHERE id=?1",
            [&prepared.pool],
        )
        .unwrap();
    store
        .journal_authorization(&prepared, prepared_authorization(&prepared), None)
        .unwrap();
    let id = store.id().to_owned();
    for reopened in [false, true] {
        if reopened {
            drop(store);
            store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        }
        assert_atomic_counts(&store, 2, 1);
        assert_eq!(store.status().unwrap().pools[0].generation, 7);
        for (attempt, expected) in [("payment", 0), (prepared.id.as_str(), 7)] {
            let generation: i64 = store
                .db
                .query_row(
                    "SELECT generation FROM payment_attempts WHERE id=?1",
                    [attempt],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(generation, expected);
        }
    }
}

#[test]
fn atomic_payment_preparation_is_unreserved_and_commit_rechecks_authority() {
    for failure in ["none", "evidence", "signature_expiry", "ownership", "cap"] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = populated(dir.path());
        let prepared = prepared_payment(&mut store);
        assert_atomic_counts(&store, 1, 0);
        store.chain_query(&prepared.pool).unwrap();
        store.payment_chain_query(&prepared.pool).unwrap();
        let mut auth = prepared_authorization(&prepared);
        match failure {
            "evidence" => store.db.execute_batch("CREATE TRIGGER fail_evidence BEFORE INSERT ON payment_admission_evidence BEGIN SELECT RAISE(ABORT,'injected support failure'); END;").unwrap(),
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
            let saved: (String, Option<String>) = store
                .db
                .query_row(
                    "SELECT balance,reserved_before FROM payment_admission_evidence",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(saved, ("5000000".into(), None));
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
        assert!(result.unwrap_err().is::<WorkerStopped>());
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

        balances: Default::default(),
        observations: Default::default(),
        balance_certificates: Default::default(),
        resolution_certificates: Default::default(),
        released: vec![],
        resolutions: Default::default(),
    };
    store.publish_scoped_evidence(&pool, view()).unwrap();
    drop(store);
    let mut reopened =
        Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    reopened.publish_scoped_evidence(&pool, view()).unwrap();
    assert_eq!(reopened.status().unwrap().state_schema_version, 23);
}
