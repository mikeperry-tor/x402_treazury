use super::*;

#[tokio::test]
async fn store_only_worker_failure_is_terminal_and_other_store_remains_usable() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let owner = StoreOnlyOwner::new(state);
    let other_dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &other_dir.path().join("state"),
        &other_dir.path().join("key"),
        1,
        b"other",
    )
    .unwrap();
    let other = StoreOnlyOwner::new(state);
    let error = owner
        .store
        .call::<()>(|_| panic!("fixture store-only failure"))
        .await
        .unwrap_err();
    assert!(error.is::<crate::rotation::store::WorkerStopped>());
    owner.store.stopped().await;
    assert_eq!(other.store.call(|_| Ok(11)).await.unwrap(), 11);
    assert!(owner.close().await.is_err());
    other.close().await.unwrap();
}
