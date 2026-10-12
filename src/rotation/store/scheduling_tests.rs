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
        for FundingWork { job, facts } in page {
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
fn malformed_facts_fail_the_page_instead_of_hiding_a_job() {
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
    assert!(store.funding_work_page("", through).is_err());
}
