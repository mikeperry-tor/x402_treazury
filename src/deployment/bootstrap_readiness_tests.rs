use super::*;
use crate::rotation::store::Store;

fn allocated_readiness_fixture() -> (tempfile::TempDir, Store) {
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
    (dir, state)
}

#[tokio::test]
async fn captured_completion_survives_current_claim_generation_advancement() {
    let (dir, state) = allocated_readiness_fixture();
    let jobs = state.funding_jobs().unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let names = BTreeSet::from(["web".to_owned()]);
    let attempts = BTreeMap::from([("web".to_owned(), 3)]);
    let captured = capture_startup(&store, &names).await.unwrap();
    assert!(
        !check_startup(&store, &names, &attempts, Some(&captured), false)
            .await
            .unwrap()
    );
    store
        .call(move |s| {
            for job in jobs {
                s.record_credit(&job.wallet_id, &job.target, "fixture", 100)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    // Advance the current reserve slots while retaining completed historical
    // jobs. This creates no replacement allocation or new spending authority.
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.execute(
            "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1",
            []
        )
        .unwrap(),
        2
    );
    let current = capture_startup(&store, &names).await.unwrap();
    for (old, new) in captured["web"].iter().zip(&current["web"]) {
        assert_eq!(old.slot, new.slot);
        assert_eq!(old.generation + 1, new.generation);
    }
    for progress in [false, true] {
        assert!(
            check_startup(&store, &names, &attempts, Some(&captured), progress)
                .await
                .unwrap()
        );
        assert!(
            !check_startup(&store, &names, &attempts, Some(&current), progress)
                .await
                .unwrap()
        );
    }
    drop(store);
    worker.await.unwrap();
}

#[tokio::test]
async fn startup_recovery_limit_uses_durable_count_and_inclusive_maximum() {
    let (dir, state) = allocated_readiness_fixture();
    let jobs = state.funding_jobs().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    for job in &jobs {
        for index in 0..3 {
            db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase) VALUES (?1,?2,'\"ALLOCATED\"')", rusqlite::params![format!("history-{}-{index}", job.id), job.id]).unwrap();
        }
        assert_eq!(state.funding_recovery_count(&job.id).unwrap(), 3);
    }
    let (store, worker) = StoreHandle::spawn(state);
    let names = BTreeSet::from(["web".to_owned()]);
    let captured = capture_startup(&store, &names).await.unwrap();
    for progress in [false, true] {
        for maximum in [3, 4] {
            let attempts = BTreeMap::from([("web".to_owned(), maximum)]);
            assert!(
                !check_startup(&store, &names, &attempts, Some(&captured), progress)
                    .await
                    .unwrap()
            );
        }
        let attempts = BTreeMap::from([("web".to_owned(), 2)]);
        let error = check_startup(&store, &names, &attempts, Some(&captured), progress)
            .await
            .unwrap_err();
        assert!(error.is::<StartupObstructed>());
        let message = error.to_string();
        assert!(
            message.contains("recovery_attempt_limit_reached"),
            "{message}"
        );
        for job in &jobs {
            assert!(message.contains(&job.id));
            assert!(!message.contains(&job.recipient));
        }
    }
    drop(store);
    worker.await.unwrap();
}

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

#[tokio::test]
async fn concurrent_group_completion_is_scoped_paged_and_stable_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state");
    let key = dir.path().join("key");
    let mut state = Store::create(&state_path, &key, 1, b"fixture").unwrap();
    let id = state.id().to_owned();
    let names: BTreeSet<_> = (0..GROUP_PAGE + 1)
        .map(|i| format!("group_{i:02}"))
        .collect();
    for name in names.iter().map(String::as_str).chain(["unselected"]) {
        crate::rotation::test_capacity::declare_and_allocate!(
            state,
            name,
            crate::rotation::test_capacity::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
    }
    let selected_pools: BTreeSet<_> = state
        .status()
        .unwrap()
        .pools
        .into_iter()
        .filter(|pool| names.contains(&pool.name))
        .map(|pool| pool.id)
        .collect();
    let mut jobs: Vec<_> = state
        .funding_jobs()
        .unwrap()
        .into_iter()
        .filter(|j| selected_pools.contains(&j.pool_id))
        .collect();
    assert_eq!(jobs.len(), names.len() * 2);
    // Keep a claim in the final page unfinished while all its siblings complete.
    let last_pool = state
        .startup_groups(&[names.last().unwrap().clone()])
        .unwrap()
        .remove(0)
        .pool;
    let last_index = jobs.iter().position(|j| j.pool_id == last_pool).unwrap();
    let last = jobs.remove(last_index);
    let (store, worker) = StoreHandle::spawn(state);
    let attempts = names.iter().map(|n| (n.clone(), 3)).collect();
    let captured = capture_startup(&store, &names).await.unwrap();
    let mut completions = Vec::new();
    for job in jobs.into_iter().rev() {
        let store = store.clone();
        completions.push(tokio::spawn(async move {
            store
                .call(move |s| s.record_credit(&job.wallet_id, &job.target, "fixture", 100))
                .await
                .unwrap();
        }));
    }
    for task in completions {
        task.await.unwrap();
    }
    assert!(
        !check_startup(&store, &names, &attempts, Some(&captured), false)
            .await
            .unwrap(),
        "a complete first page cannot hide an unfinished final-page claim"
    );
    store
        .call(move |s| s.record_credit(&last.wallet_id, &last.target, "fixture", 100))
        .await
        .unwrap();
    assert!(
        check_startup(&store, &names, &attempts, Some(&captured), true)
            .await
            .unwrap()
    );
    let summary = bootstrap_summary(&store, &names, &captured).await.unwrap();
    assert_eq!(
        summary.captured_completed_groups,
        names.iter().cloned().collect::<Vec<_>>()
    );
    assert_eq!(
        summary.inventory_ready_groups,
        summary.captured_completed_groups
    );
    assert_eq!(summary.confirmed_balances.len(), names.len() * 2);
    assert!(
        summary
            .confirmed_balances
            .iter()
            .all(|b| b.confirmed_balance_usdc == "5.000000")
    );
    let expected = serde_json::to_value(summary).unwrap();
    let before = store.call(|s| Ok(s.funding_jobs()?.len())).await.unwrap();
    drop(store);
    worker.await.unwrap();
    let state = Store::open(&state_path, &key, &id).unwrap();
    assert_eq!(state.funding_jobs().unwrap().len(), before);
    let (store, worker) = StoreHandle::spawn(state);
    assert!(
        check_startup(&store, &names, &attempts, Some(&captured), false)
            .await
            .unwrap()
    );
    assert_eq!(
        serde_json::to_value(bootstrap_summary(&store, &names, &captured).await.unwrap()).unwrap(),
        expected
    );
    let other = BTreeSet::from(["unselected".to_owned()]);
    let other_claims = capture_startup(&store, &other).await.unwrap();
    assert!(
        check_startup(
            &store,
            &other,
            &BTreeMap::from([("unselected".into(), 3)]),
            None,
            false
        )
        .await
        .unwrap(),
        "an unrelated allocated identity is available without completing its claims"
    );
    assert!(
        !check_startup(
            &store,
            &other,
            &BTreeMap::from([("unselected".into(), 3)]),
            Some(&other_claims),
            false
        )
        .await
        .unwrap(),
        "completion of selected groups must not complete unrelated claims"
    );
    assert_eq!(
        store.call(|s| Ok(s.funding_jobs()?.len())).await.unwrap(),
        before
    );
    drop(store);
    worker.await.unwrap();
}

#[tokio::test]
async fn reopened_shortages_are_rechecked_for_bootstrap_and_serve() {
    let (dir, mut state) = allocated_readiness_fixture();
    let jobs = state.funding_jobs().unwrap();
    let identity = state.id().to_owned();
    let shortage = "treasury_insufficient_spendable_funds; previous owner";
    for job in &jobs {
        state
            .defer_funding(&job.id, u64::MAX / 2, Some(shortage), false)
            .unwrap();
    }
    drop(state);
    let state = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &identity,
    )
    .unwrap();
    assert!(
        state
            .funding_jobs()
            .unwrap()
            .iter()
            .all(|job| job.last_error.as_deref() == Some(shortage))
    );
    let (store, worker) = StoreHandle::spawn(state);
    let names = BTreeSet::from(["web".to_owned()]);
    let attempts = BTreeMap::from([("web".to_owned(), 3)]);
    let captured = capture_startup(&store, &names).await.unwrap();
    // Identity availability does not certify funding completion. Captured claims
    // keep waiting on historical diagnostics, even with old persisted backoff.
    assert!(
        check_startup(&store, &names, &attempts, None, false)
            .await
            .unwrap(),
        "historical funding shortage cannot remove the allocated serving identity"
    );
    assert!(
        !check_startup(&store, &names, &attempts, Some(&captured), false)
            .await
            .unwrap()
    );
    for (index, job) in jobs.iter().enumerate() {
        let job = job.clone();
        store
            .call(move |s| {
                // Archived callbacks cannot certify a current shortage.
                s.record_funding_observation_health(
                    &job.id,
                    "archived-operation",
                    0,
                    1,
                    Some(shortage),
                )?;
                s.observe_funding_condition(
                    &job.id,
                    "archived-operation",
                    Some(crate::rotation::error::FundingCondition::InsufficientFunds),
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            !check_startup(&store, &names, &attempts, Some(&captured), false)
                .await
                .unwrap()
        );
        let job = jobs[index].clone();
        store
            .call(move |s| {
                s.record_funding_observation_health(
                    &job.id,
                    &job.operation_id,
                    0,
                    1,
                    Some(shortage),
                )?;
                s.observe_funding_condition(
                    &job.id,
                    &job.operation_id,
                    Some(crate::rotation::error::FundingCondition::InsufficientFunds),
                )
            })
            .await
            .unwrap();
    }
    let error = check_startup(&store, &names, &attempts, Some(&captured), false)
        .await
        .unwrap_err();
    assert!(
        error.is::<StartupObstructed>(),
        "a newly reproduced shortage still stops startup"
    );
    store
        .call(move |s| {
            for job in jobs {
                let current = s.funding_job(&job.id)?.unwrap();
                assert_eq!(current.operation_id, job.operation_id);
                assert_eq!(current.attempts, job.attempts);
                assert_eq!(s.funding_recovery_count(&job.id)?, 0);
                s.record_funding_observation_health(&job.id, &job.operation_id, 0, 0, None)?;
                s.record_credit(&job.wallet_id, &job.target, "fixture", 100)?;
            }
            assert!(s.status()?.treasury_operations.is_empty());
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    for scope in [None, Some(&captured)] {
        assert!(
            check_startup(&store, &names, &attempts, scope, false)
                .await
                .unwrap()
        );
    }
    drop(store);
    worker.await.unwrap();
}
