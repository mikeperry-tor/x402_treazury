//! Coherent, bounded inventory reads retain exact operation provenance.
use super::*;

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
            b"original bytes",
            Some(TransactionFacts {
                txid: "fixture".into(),
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
            b"original bytes"
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
            b"initial bytes",
            Some(TransactionFacts {
                txid: "fixture".into(),
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
        b"initial bytes"
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
    assert_eq!(store.funding_recovery_count(&old.id).unwrap(), 1);
    assert!(store.status().unwrap().treasury_operations.is_empty());
}

#[test]
fn malformed_facts_retain_the_job_without_failing_independent_inventory() {
    let (_dir, mut store) = fixture();
    allocate(&mut store, "pool");
    let job = store.funding_jobs().unwrap().remove(0);
    // Job decoding does not read this field; paired fact acquisition must fail.
    store
        .db
        .execute(
            "UPDATE funding_progress SET provider_success='invalid' WHERE job_id=?1",
            [&job.id],
        )
        .unwrap();
    assert!(store.funding_job(&job.id).unwrap().is_some());
    let through = store.funding_work_boundary().unwrap();
    let page = store.funding_work_page("", through).unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page.iter().filter(|(_, work)| work.is_err()).count(), 1);
    assert_eq!(
        page.into_iter()
            .find(|(id, _)| id == &job.id)
            .unwrap()
            .1
            .err()
            .unwrap()
            .operation,
        job.operation_id
    );
}

#[test]
fn malformed_page_advances_without_releasing_or_replacing_jobs() {
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
        assert!(first.iter().all(|(_, work)| work.is_err()));
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
    // A payload fault cannot hide mismatched shared source ownership.
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
    assert!(store.funding_work_page("", through).is_err());
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
fn malformed_health_is_scoped_without_normalizing_values_or_hiding_sql_errors() {
    for (table, field, value) in [
        ("funding_health", "started_at", "'bad'"),
        ("funding_health", "started_at", "-1"),
        ("funding_health", "error_streak", "'bad'"),
        ("funding_health", "timed_out", "'bad'"),
        ("funding_progress", "next_poll", "'bad'"),
        ("funding_progress", "next_poll", "-1"),
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
            assert_eq!(page.iter().filter(|(_, entry)| entry.is_ok()).count(), 1);
            let fault = page
                .into_iter()
                .find(|(id, _)| id == &job.id)
                .unwrap()
                .1
                .err()
                .unwrap();
            assert_eq!(fault.operation, job.operation_id);
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
        assert!(
            store
                .funding_work_page("", store.funding_work_boundary().unwrap())
                .is_err()
        );
    }
}
