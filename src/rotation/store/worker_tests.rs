use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn exact_worker_loss_fails_accepted_waiters_without_replay_and_other_store_progresses() {
    let dir = tempfile::tempdir().unwrap();
    let state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let id = store.call(|s| Ok(s.id().to_owned())).await.unwrap();
    let other_dir = tempfile::tempdir().unwrap();
    let other = Store::create(
        &other_dir.path().join("state"),
        &other_dir.path().join("key"),
        1,
        b"other",
    )
    .unwrap();
    let (other, other_worker) = StoreHandle::spawn(other);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (entered, accepted) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let accepted_store = store.clone();
    let waiter = tokio::spawn(async move {
        accepted_store
            .call::<()>(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
                entered.send(()).unwrap();
                wait.recv().unwrap();
                panic!("fixture worker failure");
            })
            .await
    });
    accepted.await.unwrap();
    waiter.abort();
    release.send(()).unwrap();
    store.stopped().await;
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(worker.await.unwrap_err().is_panic());
    let forbidden = calls.clone();
    let error = store
        .call(move |_| {
            forbidden.fetch_add(100, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert!(error.is::<WorkerStopped>());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).is_err(),
        "a dead worker must not release the lock still owned by accepted consumers"
    );
    assert_eq!(other.call(|_| Ok(7)).await.unwrap(), 7);
    drop(store);
    let reopened = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    drop(reopened);
    drop(other);
    other_worker.await.unwrap();
}

#[tokio::test]
async fn worker_loss_reports_one_safe_capability_failure_and_truthful_live_waiter_error() {
    let ((error, joined), logs) = crate::limits::tests::capture(async {
        let dir = tempfile::tempdir().unwrap();
        let state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let error = store
            .call::<()>(|_| panic!("private fixture panic payload"))
            .await
            .unwrap_err();
        store.stopped().await;
        let joined = worker.await.unwrap_err();
        (error, joined)
    })
    .await;
    assert!(error.is::<WorkerStopped>());
    assert!(joined.is_panic());
    assert!(!joined.to_string().contains("private fixture panic payload"));
    assert_eq!(logs.matches("store_worker_stopped").count(), 1, "{logs}");
    assert!(
        logs.contains("ERROR") && logs.contains("worker_panic"),
        "{logs}"
    );
    assert!(!logs.contains("private fixture panic payload"), "{logs}");
}
