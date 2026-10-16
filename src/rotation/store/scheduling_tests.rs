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
            assert!(facts.enabled);
            assert!(!facts.accepted_quote);
            assert!(!facts.readiness_recorded);
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
    store.db.execute("INSERT INTO budget_entries(id,pool_id,day,original_day,requested,reserved,consumed) VALUES(?1,NULL,0,0,1,1,0)",[&job.operation_id]).unwrap();
    assert!(store.funding_work_page("", through).is_err());
    store
        .db
        .execute(
            "DELETE FROM budget_entries WHERE id=?1",
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
