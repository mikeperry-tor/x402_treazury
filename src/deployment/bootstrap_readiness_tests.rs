use super::*;
use crate::rotation::store::Store;

#[tokio::test]
async fn paged_vacancies_remain_incomplete_without_mutation_or_missing_policy_shortcut() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let names: BTreeSet<_> = (0..GROUP_PAGE + 1)
        .map(|i| format!("group_{i:02}"))
        .collect();
    for name in &names {
        state
            .ensure_capacity_group(
                name,
                &crate::rotation::test_capacity::two_reserves("5"),
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
    }
    let observer = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let version = || {
        observer
            .query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    let before = version();
    let (store, worker) = StoreHandle::spawn(state);
    let mut attempts: BTreeMap<_, _> = names.iter().map(|n| (n.clone(), 3)).collect();
    let last = names.last().unwrap().clone();
    attempts.remove(&last);
    assert_eq!(
        check_startup(&store, &names, &attempts, None, false)
            .await
            .unwrap_err()
            .to_string(),
        "startup funding policy missing"
    );
    attempts.insert(last, 3);
    assert!(
        !check_startup(&store, &names, &attempts, None, true)
            .await
            .unwrap()
    );
    let captured = capture_startup(&store, &names).await.unwrap();
    assert_eq!(captured.len(), GROUP_PAGE + 1);
    assert!(
        !check_startup(&store, &names, &attempts, Some(&captured), true)
            .await
            .unwrap()
    );
    assert_eq!(
        before,
        version(),
        "readiness assessment committed a mutation"
    );
    assert_eq!(
        observer
            .query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(store);
    worker.await.unwrap();
}

#[tokio::test]
async fn serving_readiness_does_not_substitute_for_captured_claim_completion() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "web",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let jobs = state.funding_jobs().unwrap();
    state
        .record_credit(&jobs[0].wallet_id, &jobs[0].target, "fixture", 100)
        .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let names = BTreeSet::from(["web".to_owned()]);
    let attempts = BTreeMap::from([("web".to_owned(), 3)]);
    let captured = capture_startup(&store, &names).await.unwrap();
    // Ordinary serving returns before requiring an attempts policy for a ready group.
    assert!(
        check_startup(&store, &names, &BTreeMap::new(), None, false)
            .await
            .unwrap()
    );
    assert_eq!(
        check_startup(&store, &names, &BTreeMap::new(), Some(&captured), false)
            .await
            .unwrap_err()
            .to_string(),
        "startup funding policy missing"
    );
    assert!(
        !check_startup(&store, &names, &attempts, Some(&captured), false)
            .await
            .unwrap()
    );
    assert!(
        !check_startup(&store, &names, &attempts, Some(&BTreeMap::new()), false)
            .await
            .unwrap(),
        "missing captured group must fall back to current claims"
    );
    let mut missing = captured.clone();
    for claim in missing.get_mut("web").unwrap() {
        claim.generation += 100;
    }
    assert!(
        !check_startup(&store, &names, &attempts, Some(&missing), false)
            .await
            .unwrap(),
        "missing historical paths cannot borrow current serving readiness"
    );
    let remaining = jobs[1].clone();
    store
        .call(move |s| s.record_credit(&remaining.wallet_id, &remaining.target, "fixture", 100))
        .await
        .unwrap();
    assert!(
        check_startup(&store, &names, &attempts, Some(&captured), true)
            .await
            .unwrap()
    );
    let observer = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    observer
        .execute("UPDATE pools SET enabled=0 WHERE name='web'", [])
        .unwrap();
    assert_eq!(
        check_startup(&store, &names, &BTreeMap::new(), None, false)
            .await
            .unwrap_err()
            .to_string(),
        "bootstrap group disabled"
    );
    drop(store);
    worker.await.unwrap();
}
