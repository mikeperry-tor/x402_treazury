//! Coherent, bounded inventory reads retain exact operation provenance.
use super::*;
use crate::rotation::store::funding::FundingPhase;

fn fixture() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    (dir, store)
}

fn allocate(store: &mut Store, name: &str) {
    super::super::test_support::declare_two_reserves(
        store,
        name,
        "5",
        Some(U256::from(1_000_000)),
        0,
    )
    .unwrap();
}

#[test]
fn jobless_prepared_owner_stays_observable_when_new_submission_is_disabled() {
    let (_dir, mut store) = fixture();
    let id = uuid::Uuid::new_v4().to_string();
    store.accept_preparation(&id, None, 60, None).unwrap();
    store
        .bind_operation_recipient(&id, &format!("0x{:040x}", 1))
        .unwrap();
    let revision = store.snapshot().unwrap().0;
    store
        .prepare_with_facts(
            &id,
            revision,
            b"next",
            crate::rotation::store::test_support::transaction_raw(100).as_slice(),
            Some(TransactionFacts {
                txid: crate::rotation::store::test_support::transaction_txid(100),
                expiry_height: 100,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: Some(10000),
            }),
        )
        .unwrap();
    for restriction in [
        super::super::super::restriction::FundingRestriction::Paused,
        super::super::super::restriction::FundingRestriction::DenyNewFunding,
    ] {
        store.funding_restriction = restriction;
        assert!(store.request_broadcast(&id, 1, 1, false).is_err());
        assert_eq!(
            store.jobless_pending_operation().unwrap().as_deref(),
            Some(id.as_str())
        );
        assert_eq!(
            store.prepared_bytes(&id).unwrap().as_slice(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
        assert_eq!(store.operation(&id).unwrap().attempts, 0);
    }
}

#[test]
fn partial_credit_after_preparation_forbids_first_deposit_and_retains_incoming_owner() {
    let (_dir, mut store) = fixture();
    allocate(&mut store, "initial");
    let job = store.funding_jobs().unwrap().remove(0);
    store
        .save_funding_quote(
            &job.id,
            &serde_json::to_vec(
                &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
            )
            .unwrap(),
        )
        .unwrap();
    store
        .require_preparation_authority(
            &job.operation_id,
            Some(&crate::rotation::transaction::ManagedFundingAuthority {
                job_id: job.id.clone(),
                auto_fund: true,
                max_attempts: 3,
            }),
        )
        .unwrap();
    store
        .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
        .unwrap();
    let revision = store.snapshot().unwrap().0;
    store
        .prepare_with_facts(
            &job.operation_id,
            revision,
            b"next",
            crate::rotation::store::test_support::transaction_raw(100).as_slice(),
            Some(TransactionFacts {
                txid: crate::rotation::store::test_support::transaction_txid(100),
                expiry_height: 100,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: Some(10000),
            }),
        )
        .unwrap();
    store
        .record_funding_balance(
            &job.id,
            &job.operation_id,
            &job.wallet_id,
            &job.recipient,
            U256::from(7),
        )
        .unwrap();
    assert!(
        store
            .request_broadcast(&job.operation_id, 1, 1, false)
            .is_err()
    );
    assert!(funding::destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
    assert!(store.operation_pending(&job.operation_id).unwrap());
    assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 0);
    assert_eq!(
        store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
        crate::rotation::store::test_support::transaction_raw(100).as_slice()
    );
}

#[test]
fn pages_keep_bounds_order_and_matching_facts_without_mutation() {
    let (_dir, mut store) = fixture();
    for i in 0..35 {
        allocate(&mut store, &format!("pool{i}"));
    }
    let through = store.funding_work_boundary().unwrap();
    let expected: Vec<_> = store
        .funding_jobs()
        .unwrap()
        .into_iter()
        .map(|j| j.id)
        .collect();
    allocate(&mut store, "arrival");
    let before = serde_json::to_value(store.status().unwrap()).unwrap();
    let mut after = String::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut pages = 0;
    loop {
        let page = store.funding_work_page(&after, through).unwrap();
        if page.is_empty() {
            break;
        }
        pages += 1;
        assert!(page.len() <= PAGE_SIZE);
        for (_, entry) in page {
            let FundingWork { job, facts } = entry.unwrap();
            assert!(job.id > after);
            assert!(expected.contains(&job.id));
            assert!(seen.insert(job.id.clone()));
            assert_eq!(job.phase, funding::FundingPhase::Allocated);
            assert!(facts.new_funding_allowed);
            assert!(!facts.accepted_quote);
            assert!(!facts.initial_credit_recorded);
            assert!(!facts.archived);
            assert!(facts.source_submission.is_none());
            assert_eq!(facts.source_attempts, 0);
            after = job.id;
        }
    }
    assert_eq!(pages, 2);
    assert_eq!(seen.len(), expected.len());
    assert_eq!(
        serde_json::to_value(store.status().unwrap()).unwrap(),
        before
    );
}

#[test]
fn captured_phase_and_facts_survive_rollover_and_reopen() {
    let (dir, mut store) = fixture();
    allocate(&mut store, "pool");
    let old = store.funding_jobs().unwrap().remove(0);
    let quote = serde_json::to_vec(&serde_json::json!({
        "request": {"amount":old.target,"recipient":old.recipient}
    }))
    .unwrap();
    store.save_funding_quote(&old.id, &quote).unwrap();
    let through = store.funding_work_boundary().unwrap();
    let captured = store
        .funding_work_page("", through)
        .unwrap()
        .into_iter()
        .map(|(_, work)| work.unwrap())
        .find(|work| work.job.id == old.id)
        .unwrap();
    assert_eq!(captured.job.phase, funding::FundingPhase::Quoted);
    assert!(captured.facts.accepted_quote);
    store
        .close_unprepared_funding_for_operation(&old.id, &old.operation_id)
        .unwrap();
    store
        .reauthorize_closed_funding_for_operation(&old.id, &old.operation_id, 3)
        .unwrap();
    // This was the scheduler's fatal second read. A captured page no longer
    // performs it, and still carries the facts belonging to its old phase.
    assert!(store.funding_facts(&old.id, &old.operation_id).is_err());
    assert_eq!(captured.job.operation_id, old.operation_id);
    assert!(captured.facts.accepted_quote);
    let treasury = store.id().to_owned();
    drop(store);
    let store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury,
    )
    .unwrap();
    let replacement = store
        .funding_work_page("", through)
        .unwrap()
        .into_iter()
        .map(|(_, work)| work.unwrap())
        .find(|work| work.job.id == old.id)
        .unwrap();
    assert_ne!(replacement.job.operation_id, old.operation_id);
    assert_eq!(replacement.job.phase, funding::FundingPhase::Allocated);
    assert!(!replacement.facts.accepted_quote);
    assert!(!replacement.facts.archived);
    assert!(replacement.facts.source_submission.is_none());
    assert!(
        store
            .scope_funding_completion_failure(
                &old.id,
                &old.operation_id,
                FundingDecodeFailure.into()
            )
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .scope_funding_completion_failure(
                &old.id,
                &old.operation_id,
                anyhow::anyhow!("shared storage failure")
            )
            .is_err()
    );
    assert_eq!(
        store
            .scope_funding_completion_failure(
                &old.id,
                &replacement.job.operation_id,
                FundingDecodeFailure.into()
            )
            .unwrap()
            .unwrap()
            .operation,
        replacement.job.operation_id
    );
    assert_eq!(store.funding_recovery_count(&old.id).unwrap(), 0);
    assert!(store.status().unwrap().treasury_operations.is_empty());
}

#[test]
fn optional_provider_and_disclosure_metadata_do_not_block_inventory() {
    for required_source in [false, true] {
        let (_dir, mut store) = fixture();
        allocate(&mut store, "pool");
        let job = store.funding_jobs().unwrap().remove(0);
        if required_source {
            store
                .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                .unwrap();
            store
                .prepare_with_facts(
                    &job.operation_id,
                    1,
                    b"next",
                    crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                    Some(TransactionFacts {
                        txid: crate::rotation::store::test_support::transaction_txid(100),
                        expiry_height: 100,
                        amount_zatoshis: 50,
                        fee_zatoshis: 10,
                        deadline: None,
                    }),
                )
                .unwrap();
            // A damaged local sequence is possibly disclosed; preserve observation
            // of the authenticated original while withholding first-send authority.
            store
                .db
                .execute(
                    "UPDATE treasury_operations SET attempts='invalid' WHERE id=?1",
                    [&job.operation_id],
                )
                .unwrap();
        } else {
            // Optional provider hints are unknown when malformed, not authority
            // to close an original or suppress independently authorized work.
            store
                .db
                .execute(
                    "UPDATE funding_progress SET provider_success='invalid' WHERE job_id=?1",
                    [&job.id],
                )
                .unwrap();
        }
        assert!(store.funding_job(&job.id).unwrap().is_some());
        let through = store.funding_work_boundary().unwrap();
        let page = store.funding_work_page("", through).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page.iter().filter(|(_, work)| work.is_err()).count(), 0);
        let result = page.into_iter().find(|(id, _)| id == &job.id).unwrap().1;
        if required_source {
            let work = result.unwrap();
            assert_eq!(work.facts.source_submission.as_deref(), Some("UNKNOWN"));
            assert!(work.facts.source_attempts >= 1);
            assert!(!work.facts.new_funding_allowed);
            assert!(store.operation(&job.operation_id).is_err());
            assert_eq!(
                store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
                crate::rotation::store::test_support::transaction_raw(100).as_slice()
            );
            assert!(store.operation_pending(&job.operation_id).unwrap());
        } else {
            let work = result.unwrap();
            assert!(!work.facts.provider_success);
            assert!(work.facts.new_funding_allowed);
            assert!(store.status().unwrap().treasury_operations.is_empty());
        }
    }
}

#[test]
fn malformed_phase_routes_recovery_without_releasing_or_replacing_jobs() {
    let (dir, mut store) = fixture();
    for i in 0..35 {
        allocate(&mut store, &format!("pool{i}"));
    }
    let mut jobs = store.funding_jobs().unwrap();
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    for job in &jobs[..PAGE_SIZE] {
        store
            .db
            .execute(
                "UPDATE funding_progress SET phase='invalid' WHERE job_id=?1",
                [&job.id],
            )
            .unwrap();
    }
    let through = store.funding_work_boundary().unwrap();
    let treasury = store.id().to_owned();
    for reopened in [false, true] {
        if reopened {
            drop(store);
            store = Store::open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &treasury,
            )
            .unwrap();
        }
        let first = store.funding_work_page("", through).unwrap();
        assert_eq!(first.len(), PAGE_SIZE);
        assert!(first.iter().all(|(_, work)| {
            work.as_ref()
                .is_ok_and(|work| work.job.phase == FundingPhase::RecoveryRequired)
        }));
        let second = store
            .funding_work_page(&first.last().unwrap().0, through)
            .unwrap();
        assert_eq!(second.len(), 6);
        assert!(second.iter().all(|(_, work)| work.is_ok()));
        for job in &jobs {
            let operation: String = store
                .db
                .query_row(
                    "SELECT operation_id FROM funding_progress WHERE job_id=?1",
                    [&job.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(operation, job.operation_id);
            assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 0);
        }
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM treasury_operations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let job = &jobs[0];
    // An unpublished intent is preserved audit data, not a global inventory
    // fence. Its existing accepted identity still cannot authorize recalculation.
    store
        .accept_preparation(&job.operation_id, None, 1, None)
        .unwrap_err();
    let bad = seal(
        &store.key,
        &format!(
            "v1:{}:{}:preparation:{}",
            store.id,
            store.network.name(),
            job.operation_id
        ),
        br#"{"pool":null,"cost_zatoshis":1,"calculation":null}"#,
    )
    .unwrap();
    store
        .db
        .execute(
            "INSERT INTO operation_network(operation_id,preparation) VALUES(?1,?2)",
            params![job.operation_id, bad],
        )
        .unwrap();
    let work = store
        .funding_work_page("", through)
        .unwrap()
        .into_iter()
        .find(|(id, _)| id == &job.id)
        .unwrap()
        .1
        .unwrap();
    assert!(work.facts.preparation_accepted);
    assert_eq!(work.job.operation_id, job.operation_id);
    assert!(
        store
            .require_unstarted_preparation(&job.operation_id)
            .is_err()
    );
    assert!(
        store
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 1, None)
            .is_err(),
        "an existing accepted pool identity cannot be replaced"
    );
    assert!(
        store
            .scope_funding_decode_failure(&job.id, FundingDecodeFailure.into())
            .is_err(),
        "scoped required-payload failures still validate original source ownership"
    );
    let preserved: Vec<u8> = store
        .db
        .query_row(
            "SELECT preparation FROM operation_network WHERE operation_id=?1",
            [&job.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(preserved, bad);
    store
        .db
        .execute(
            "DELETE FROM operation_network WHERE operation_id=?1",
            [&job.operation_id],
        )
        .unwrap();
    store.db.execute("PRAGMA foreign_keys=OFF", []).unwrap();
    store
        .db
        .execute(
            "UPDATE wallets SET pool_id='unknown' WHERE id=?1",
            [&job.wallet_id],
        )
        .unwrap();
    assert!(store.funding_work_page("", through).is_err());
}

#[test]
fn malformed_health_is_advisory_without_normalizing_values_or_hiding_sql_errors() {
    for (table, field, value) in [
        ("funding_health", "started_at", "'bad'"),
        ("funding_health", "started_at", "-1"),
        ("funding_health", "error_streak", "'bad'"),
        ("funding_health", "timed_out", "'bad'"),
        ("funding_progress", "next_poll", "'bad'"),
        ("funding_progress", "next_poll", "-1"),
        ("funding_progress", "attempts", "'bad'"),
        ("funding_progress", "attempts", "-1"),
        ("funding_progress", "last_error", "x'00'"),
        ("funding_progress", "phase", "'invalid'"),
        ("funding_progress", "phase", "x'00'"),
    ] {
        let (dir, mut store) = fixture();
        allocate(&mut store, "pool");
        let job = store.funding_jobs().unwrap().remove(0);
        let treasury = store.id().to_owned();
        store
            .db
            .execute(
                "INSERT OR IGNORE INTO funding_health(job_id) VALUES(?1)",
                [&job.id],
            )
            .unwrap();
        store
            .db
            .execute(
                &format!("UPDATE {table} SET {field}={value} WHERE job_id=?1"),
                [&job.id],
            )
            .unwrap();
        let before: rusqlite::types::Value = store
            .db
            .query_row(
                &format!("SELECT {field} FROM {table} WHERE job_id=?1"),
                [&job.id],
                |r| r.get(0),
            )
            .unwrap();
        for reopen in [false, true] {
            if reopen {
                drop(store);
                store = Store::open(
                    &dir.path().join("state"),
                    &dir.path().join("key"),
                    &treasury,
                )
                .unwrap();
            }
            let page = store
                .funding_work_page("", store.funding_work_boundary().unwrap())
                .unwrap();
            assert_eq!(page.len(), 2);
            assert!(page.iter().all(|(_, entry)| entry.is_ok()));
            let work = page
                .into_iter()
                .find(|(id, _)| id == &job.id)
                .unwrap()
                .1
                .unwrap();
            assert_eq!(work.job.operation_id, job.operation_id);
            if field == "phase" {
                assert_eq!(work.job.phase, FundingPhase::RecoveryRequired);
                assert!(
                    store
                        .refresh_unprepared_quote_for_operation(&job.id, &job.operation_id)
                        .is_err(),
                    "routing fallback cannot satisfy stored Quoted CAS"
                );
            }
            assert!(store.funding_job(&job.id).unwrap().is_some());
            assert_eq!(store.funding_jobs().unwrap().len(), 2);
            assert!(
                crate::rotation::store::funding::read_jobs(&store.db).is_err(),
                "strict reporting preserves malformed-data visibility"
            );
            assert!(
                store
                    .request_broadcast(&job.operation_id, 1, 1, false)
                    .is_err(),
                "advisory health grants no submission authority"
            );
            let after: rusqlite::types::Value = store
                .db
                .query_row(
                    &format!("SELECT {field} FROM {table} WHERE job_id=?1"),
                    [&job.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(after, before);
            assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 0);
        }
        store.db.execute("DROP TABLE funding_health", []).unwrap();
        let error = store
            .funding_work_page("", store.funding_work_boundary().unwrap())
            .err()
            .unwrap();
        assert!(
            error.is::<rusqlite::Error>(),
            "SQL execution failures must remain visible: {error:#}"
        );
    }
}

#[test]
fn damaged_disclosure_sequence_is_observe_only_and_never_rewritten() {
    use rusqlite::types::Value;
    for (submission, attempts) in [
        (
            Value::Text("PREPARED".into()),
            Value::Text("invalid".into()),
        ),
        (Value::Text("PREPARED".into()), Value::Blob(vec![0])),
        (Value::Text("PREPARED".into()), Value::Integer(-1)),
        (Value::Text("UNKNOWN".into()), Value::Integer(0)),
        (Value::Blob(vec![0]), Value::Integer(0)),
    ] {
        let (_dir, mut store) = fixture();
        allocate(&mut store, "observer");
        let job = store.funding_jobs().unwrap().remove(0);
        store
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
            .unwrap();
        store
            .prepare_with_facts(
                &job.operation_id,
                1,
                b"prepared SDK",
                crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                Some(TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(100),
                    expiry_height: 100,
                    amount_zatoshis: 50,
                    fee_zatoshis: 10,
                    deadline: None,
                }),
            )
            .unwrap();
        store
            .db
            .execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        store
            .db
            .execute(
                "UPDATE treasury_operations SET submission=?2,attempts=?3 WHERE id=?1",
                params![job.operation_id, submission, attempts],
            )
            .unwrap();
        store
            .db
            .execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let observed = store.observation_operation(&job.operation_id).unwrap();
        assert_eq!(observed.submission, "UNKNOWN");
        assert!(observed.attempts >= 1);
        let facts = store.funding_facts(&job.id, &job.operation_id).unwrap();
        assert_eq!(facts.source_submission.as_deref(), Some("UNKNOWN"));
        assert!(facts.source_attempts >= 1 && facts.source_reconciliation_needed());
        assert!(!facts.new_funding_allowed);
        assert!(
            store
                .request_broadcast(&job.operation_id, 1, 1, false)
                .is_err()
        );
        // Even explicit retry cannot invent the missing counter/history.
        assert!(
            store
                .request_broadcast(&job.operation_id, 1, 1, true)
                .is_err()
        );
        let saved: (Value, Value) = store
            .db
            .query_row(
                "SELECT submission,attempts FROM treasury_operations WHERE id=?1",
                [&job.operation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(saved, (submission, attempts.clone()));
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
        store.confirm_spend(&job.operation_id).unwrap();
        let confirmed = store.observation_operation(&job.operation_id).unwrap();
        assert_eq!(confirmed.submission, "CONFIRMED");
        assert!(!store.operation_pending(&job.operation_id).unwrap());
        let saved: Value = store
            .db
            .query_row(
                "SELECT attempts FROM treasury_operations WHERE id=?1",
                [&job.operation_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            saved, attempts,
            "canonical closure never repairs the local counter"
        );
    }
    // NULL cannot be written through the current NOT NULL schema, but a damaged
    // row must still differ from an absent LEFT JOIN when projecting observations.
    let db = Connection::open_in_memory().unwrap();
    let observed = db
        .query_row("SELECT NULL,0", [], |r| {
            observation_disclosure(r, 0, 1, true, false)
        })
        .unwrap();
    assert_eq!(observed, (Some("UNKNOWN".into()), 1));
    let absent = db
        .query_row("SELECT NULL,0", [], |r| {
            observation_disclosure(r, 0, 1, false, false)
        })
        .unwrap();
    assert_eq!(absent, (None, 0));
}

#[test]
fn expiry_label_preserves_native_disclosure_evidence() {
    let db = Connection::open_in_memory().unwrap();
    for (status, count, expected_status, expected_count) in [
        ("PREPARED", 0, "EXPIRED", 0),
        ("UNKNOWN", 0, "EXPIRED", 1),
        ("CONFIRMED", 0, "CONFIRMED", 1),
        ("broken", 0, "EXPIRED", 1),
    ] {
        let expired = expected_status == "EXPIRED";
        let observed = db
            .query_row("SELECT ?1,?2", params![status, count], |r| {
                observation_disclosure(r, 0, 1, true, expired)
            })
            .unwrap();
        assert_eq!(observed, (Some(expected_status.into()), expected_count));
    }
}

#[test]
fn expired_signed_original_cannot_gain_first_disclosure_from_stale_projection() {
    for projected_expiry in [0, 10_000] {
        let (_dir, mut store) = fixture();
        let id = "expired-first-disclosure";
        store
            .bind_operation_recipient(id, &format!("0x{:040x}", 1))
            .unwrap();
        store.accept_preparation(id, None, 60, None).unwrap();
        let raw = crate::rotation::store::test_support::transaction_raw(100);
        store
            .prepare_with_facts(
                id,
                1,
                b"calculated",
                &raw,
                Some(TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(100),
                    expiry_height: 100,
                    amount_zatoshis: 50,
                    fee_zatoshis: 10,
                    deadline: Some(10_000),
                }),
            )
            .unwrap();
        store.db.execute(
            "UPDATE treasury_operations SET facts=json_set(facts,'$.expiry_height',?2,'$.txid','stale projection') WHERE id=?1",
            params![id, projected_expiry],
        ).unwrap();
        let encrypted: Vec<u8> = store
            .db
            .query_row("SELECT raw FROM outgoing WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        let error = crate::rotation::transaction::BroadcastTransaction::request(
            &mut store, id, 1, 100, false,
        )
        .err()
        .expect("expired original cannot mint a submission capability");
        assert!(error.to_string().contains("transaction_expired"));
        assert_eq!(store.operation(id).unwrap().attempts, 0);
        assert_eq!(store.operation(id).unwrap().submission, "PREPARED");
        assert!(store.operation_pending(id).unwrap());
        assert_eq!(store.prepared_bytes(id).unwrap().as_slice(), raw);
        let retained: Vec<u8> = store
            .db
            .query_row("SELECT raw FROM outgoing WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(retained, encrypted);
        assert_eq!(store.db.query_row("SELECT json_extract(facts,'$.expiry_height') FROM treasury_operations WHERE id=?1", [id], |row| row.get::<_,u32>(0)).unwrap(), projected_expiry);
    }
}

#[test]
fn authoritative_raw_and_required_facts_failures_stay_scoped_to_the_original() {
    for damage in ["ciphertext", "required_object", "required_amount"] {
        let (_dir, mut store) = fixture();
        let raw = crate::rotation::store::test_support::transaction_raw(100);
        for id in ["damaged", "sibling"] {
            store
                .bind_operation_recipient(id, &format!("0x{:040x}", 1))
                .unwrap();
            store.accept_preparation(id, None, 60, None).unwrap();
            store
                .prepare_with_facts(
                    id,
                    store.snapshot().unwrap().0,
                    b"calculated",
                    &raw,
                    Some(TransactionFacts {
                        txid: crate::rotation::store::test_support::transaction_txid(100),
                        expiry_height: 100,
                        amount_zatoshis: 50,
                        fee_zatoshis: 10,
                        deadline: Some(10_000),
                    }),
                )
                .unwrap();
            if id == "damaged" {
                // Finish this original before preparing the sibling. Later
                // damage to its retained evidence must not veto sibling work.
                store.confirm_spend(id).unwrap();
            }
        }
        match damage {
            "ciphertext" => {
                store
                    .db
                    .execute("UPDATE outgoing SET raw=x'00' WHERE id='damaged'", [])
                    .unwrap();
            }
            "required_object" => {
                store
                    .db
                    .execute(
                        "UPDATE treasury_operations SET facts='[]' WHERE id='damaged'",
                        [],
                    )
                    .unwrap();
            }
            "required_amount" => {
                store.db.execute("UPDATE treasury_operations SET facts=json_set(facts,'$.amount_zatoshis','invalid') WHERE id='damaged'", []).unwrap();
            }
            _ => unreachable!(),
        }
        let retained: Vec<u8> = store
            .db
            .query_row("SELECT raw FROM outgoing WHERE id='damaged'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let error = store.observation_operation("damaged").err().unwrap();
        if damage != "ciphertext" {
            assert!(error.is::<super::scheduling::FundingDecodeFailure>());
        }
        assert!(store.request_broadcast("damaged", 1, 1, false).is_err());
        let sibling =
            crate::rotation::transaction::PreparedTransaction::load(&store, "sibling").unwrap();
        assert_eq!(sibling.bytes(), raw);
        assert_eq!(sibling.facts().unwrap().expiry_height, 100);
        store.confirm_spend("sibling").unwrap();
        assert!(!store.operation_pending("sibling").unwrap());
        assert!(!store.operation_pending("damaged").unwrap());
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT attempts FROM treasury_operations WHERE id='damaged'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT raw FROM outgoing WHERE id='damaged'", [], |row| row
                    .get::<_, Vec<u8>>(0))
                .unwrap(),
            retained
        );
    }
}
