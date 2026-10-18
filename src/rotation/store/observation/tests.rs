use super::*;

fn cached(failures: u32) -> Owner {
    Arc::new(tokio::sync::Mutex::new(Observation {
        due: Some(Instant::now() + CADENCE),
        failures,
        value: (failures == 0).then_some(U256::ZERO),
    }))
}

fn fixture() -> (
    tempfile::TempDir,
    StoreHandle,
    tokio::task::JoinHandle<()>,
    Vec<String>,
) {
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
        "pool",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(U256::from(100)),
        0
    )
    .unwrap();
    let wallets = state
        .funding_jobs()
        .unwrap()
        .into_iter()
        .map(|job| job.wallet_id)
        .collect();
    let (store, worker) = StoreHandle::spawn(state);
    (dir, store, worker, wallets)
}

#[tokio::test(start_paused = true)]
async fn expired_metadata_reclaims_successes_and_retired_failures_but_keeps_live_backoff() {
    let (_dir, store, worker, wallets) = fixture();
    let (live, retired) = (wallets[0].clone(), wallets[1].clone());
    let id = retired.clone();
    store
        .call(move |s| {
            s.db.execute("UPDATE wallets SET role='RETIRED' WHERE id=?1", [id])?;
            Ok(())
        })
        .await
        .unwrap();
    {
        let mut owners = store.balance_observers.lock().unwrap();
        owners.insert("success".into(), cached(0));
        owners.insert(live.clone(), cached(3));
        owners.insert(retired.clone(), cached(3));
    }
    tokio::time::advance(CADENCE - Duration::from_secs(1)).await;
    drop(store.balance_acquisition_owner("new").await);
    assert_eq!(store.balance_observers.lock().unwrap().len(), 4);
    tokio::time::advance(Duration::from_secs(1)).await;
    drop(store.balance_acquisition_owner("new").await);
    let owner = {
        let owners = store.balance_observers.lock().unwrap();
        assert!(!owners.contains_key("success"));
        assert!(!owners.contains_key(&retired));
        owners[&live].clone()
    };
    assert_eq!(owner.lock().await.failures, 3);
    assert!(Arc::ptr_eq(
        &owner,
        &store.balance_acquisition_owner(&live).await
    ));
    drop(owner);
    drop(store);
    worker.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn expired_metadata_keeps_active_and_waiting_owners() {
    let (_dir, store, worker, _wallets) = fixture();
    let active = cached(0);
    let waiting = cached(0);
    {
        let mut owners = store.balance_observers.lock().unwrap();
        owners.insert("active".into(), active.clone());
        owners.insert("waiting".into(), waiting.clone());
    }
    let guard = active.lock_owned().await;
    tokio::time::advance(CADENCE).await;
    drop(store.balance_acquisition_owner("new").await);
    assert!(
        store
            .balance_observers
            .lock()
            .unwrap()
            .contains_key("active")
    );
    assert!(Arc::ptr_eq(&waiting, &store.balance_owner("waiting")));
    drop(guard);
    drop(waiting);
    drop(store.balance_acquisition_owner("new").await);
    assert!(
        !store
            .balance_observers
            .lock()
            .unwrap()
            .contains_key("active")
    );
    assert!(
        !store
            .balance_observers
            .lock()
            .unwrap()
            .contains_key("waiting")
    );
    drop(store);
    worker.await.unwrap();
}

#[tokio::test]
async fn retirement_cleanup_rechecks_owners_acquired_during_the_store_read() {
    let (_dir, store, worker, wallets) = fixture();
    let retired = wallets[0].clone();
    let id = retired.clone();
    store
        .call(move |s| {
            s.db.execute("UPDATE wallets SET role='RETIRED' WHERE id=?1", [id])?;
            Ok(())
        })
        .await
        .unwrap();
    let owner = cached(2);
    owner.lock().await.due = Some(Instant::now());
    store
        .balance_observers
        .lock()
        .unwrap()
        .insert(retired.clone(), owner);
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let blocked_store = store.clone();
    let blocker = tokio::spawn(async move {
        blocked_store
            .call(move |_| {
                entered.send(()).unwrap();
                blocked.recv().unwrap();
                Ok(())
            })
            .await
            .unwrap();
    });
    ready.await.unwrap();
    let acquired = {
        let cleanup = store.balance_acquisition_owner("new");
        tokio::pin!(cleanup);
        assert!(futures_util::poll!(&mut cleanup).is_pending());
        let acquired = store.balance_owner(&retired);
        release.send(()).unwrap();
        drop(cleanup.await);
        acquired
    };
    blocker.await.unwrap();
    assert!(Arc::ptr_eq(&acquired, &store.balance_owner(&retired)));
    drop(acquired);
    drop(store.balance_acquisition_owner("new").await);
    assert!(
        !store
            .balance_observers
            .lock()
            .unwrap()
            .contains_key(&retired)
    );
    drop(store);
    worker.await.unwrap();
}

#[tokio::test]
async fn cancelled_cleanup_leaves_no_permanent_unused_owner() {
    let (_dir, store, worker, wallets) = fixture();
    let failure = cached(1);
    failure.lock().await.due = Some(Instant::now());
    store
        .balance_observers
        .lock()
        .unwrap()
        .insert(wallets[0].clone(), failure);
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let blocked_store = store.clone();
    let blocker = tokio::spawn(async move {
        blocked_store
            .call(move |_| {
                entered.send(()).unwrap();
                blocked.recv().unwrap();
                Ok(())
            })
            .await
            .unwrap();
    });
    ready.await.unwrap();
    {
        let cleanup = store.balance_acquisition_owner("cancelled");
        tokio::pin!(cleanup);
        assert!(futures_util::poll!(&mut cleanup).is_pending());
    }
    assert!(
        store
            .balance_observers
            .lock()
            .unwrap()
            .contains_key("cancelled")
    );
    release.send(()).unwrap();
    blocker.await.unwrap();
    drop(store.balance_acquisition_owner("new").await);
    assert!(
        !store
            .balance_observers
            .lock()
            .unwrap()
            .contains_key("cancelled")
    );
    drop(store);
    worker.await.unwrap();
}
