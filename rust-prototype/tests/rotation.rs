use std::path::Path;
use x402_mcp_prototype::rotation::store::{Store, StoreHandle, status};
fn create(dir: &Path) -> Store {
    Store::create(
        &dir.join("state"),
        &dir.join("key"),
        2_000_000,
        b"secret-wallet-snapshot",
    )
    .unwrap()
}
#[test]
fn state_is_encrypted_exclusive_and_recovers_identity_and_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = create(dir.path());
    let id = store.id().to_owned();
    assert!(Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).is_err());
    store.ensure_pool("research", "5.00").unwrap();
    assert_eq!(status(&dir.path().join("state")).unwrap().pools.len(), 1);
    assert_eq!(store.save_snapshot(1, b"next-secret-snapshot").unwrap(), 2);
    assert!(store.save_snapshot(1, b"stale").is_err());
    for path in std::fs::read_dir(dir.path().join("state")).unwrap() {
        let bytes = std::fs::read(path.unwrap().path()).unwrap();
        for needle in [
            b"secret-wallet-snapshot".as_slice(),
            b"next-secret-snapshot".as_slice(),
        ] {
            assert!(!bytes.windows(needle.len()).any(|w| w == needle));
        }
    }
    drop(store);
    assert!(Store::open(&dir.path().join("state"), &dir.path().join("key"), "wrong").is_err());
    let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(
        store.snapshot().unwrap().1.as_slice(),
        b"next-secret-snapshot"
    );
    drop(store);
    std::fs::write(dir.path().join("key"), [0u8; 32]).unwrap();
    assert!(Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).is_err());
    assert!(Store::create(&dir.path().join("state"), &dir.path().join("key"), 1, b"x").is_err());
}
#[test]
fn pools_bootstrap_rotate_independently_and_keep_allocation_targets() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let a = s.ensure_pool("a", "5.00").unwrap();
    let b = s.ensure_pool("b", "10.00").unwrap();
    let wallets = s.status().unwrap().pools;
    for pool in &wallets {
        assert_eq!(pool.addresses.len(), 2);
        assert!(!pool.bootstrapped);
    }
    let first = &wallets[0].addresses[0];
    assert!(s.record_credit(&first.id, "4999999", "block", 1).is_err());
    for address in &wallets[0].addresses {
        s.record_credit(&address.id, "5000000", "block", 1).unwrap();
    }
    let now = s.status().unwrap();
    assert!(now.pools[0].bootstrapped);
    assert!(!now.pools[1].bootstrapped);
    assert_eq!(s.ensure_pool("a", "7.00").unwrap(), a);
    assert_eq!(s.promote(&a, 0).unwrap(), 1);
    assert!(s.promote(&a, 0).is_err());
    assert!(s.promote(&a, 1).is_err());
    assert!(s.promote(&b, 0).is_err());
    let now = s.status().unwrap();
    assert_eq!(now.pools[0].addresses.len(), 3);
    assert_eq!(now.pools[0].addresses[0].role, "RETIRED");
    assert_eq!(now.pools[0].addresses[1].role, "ACTIVE");
    assert_eq!(now.pools[0].addresses[1].target, "5000000");
    assert_eq!(now.pools[0].addresses[2].target, "7000000");
    assert_eq!(now.pools[1].addresses.len(), 2);
    s.reserve("pending", Some(&a), 1, 10, 100).unwrap();
    s.disable_pool("a").unwrap();
    assert!(s.promote(&a, 1).is_err());
    assert!(s.reserve("new", Some(&a), 1, 10, 100).is_err());
    assert!(s.prepare("pending", 1, b"snapshot", b"raw").is_err());
    assert_eq!(s.snapshot().unwrap().0, 1);
    let id = s.id().to_owned();
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(s.ensure_pool("a", "7").unwrap(), a);
    assert_eq!(s.status().unwrap().pools[0].addresses.len(), 3);
}
#[test]
fn shared_budget_and_prepared_send_survive_restart_without_duplicate_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let id = s.id().to_owned();
    let a = s.ensure_pool("a", "5").unwrap();
    let b = s.ensure_pool("b", "10").unwrap();
    s.reserve("op-a", Some(&a), 10, 60, 100).unwrap();
    s.reserve("op-a", Some(&a), 10, 60, 100).unwrap();
    assert!(s.reserve("op-a", Some(&b), 10, 60, 100).is_err());
    assert!(s.reserve("op-b", Some(&b), 11, 50, 100).is_err());
    s.reserve("op-b", Some(&b), 11, 40, 100).unwrap();
    assert_eq!(
        s.prepare("op-a", 1, b"post-calc", b"exact-transaction")
            .unwrap(),
        2
    );
    assert!(s.prepare("op-b", 2, b"other", b"other-tx").is_err());
    assert_eq!(s.snapshot().unwrap().0, 2);
    assert!(
        s.prepare("op-a", 1, b"changed", b"exact-transaction")
            .is_err()
    );
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(
        s.prepared_bytes("op-a").unwrap().as_slice(),
        b"exact-transaction"
    );
    assert_eq!(
        s.prepare("op-a", 1, b"post-calc", b"exact-transaction")
            .unwrap(),
        2
    );
    assert!(s.confirm_spend("op-a", 61, 11).is_err());
    s.confirm_spend("op-a", 55, 11).unwrap();
    s.confirm_spend("op-a", 55, 11).unwrap();
    assert!(s.confirm_spend("op-a", 54, 11).is_err());
    s.reserve("op-a", Some(&a), 10, 60, 100).unwrap();
    assert!(s.reserve("too-much", Some(&b), 11, 6, 100).is_err());
    assert_eq!(s.prepare("op-b", 2, b"post-second", b"tx-b").unwrap(), 3);
}
#[tokio::test]
async fn concurrent_pool_requests_and_budget_reservations_serialize() {
    let dir = tempfile::tempdir().unwrap();
    let s = create(dir.path());
    let (handle, worker) = StoreHandle::spawn(s);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let h = handle.clone();
        tasks.push(tokio::spawn(async move {
            h.call(|s| s.ensure_pool("same", "5")).await.unwrap()
        }));
    }
    let mut ids = std::collections::BTreeSet::new();
    for t in tasks {
        ids.insert(t.await.unwrap());
    }
    assert_eq!(ids.len(), 1);
    let h = handle.clone();
    let pool = ids.into_iter().next().unwrap();
    let other = pool.clone();
    let (a, b) = tokio::join!(
        handle.call(move |s| s.reserve("a", Some(&pool), 1, 60, 100)),
        h.call(move |s| s.reserve("b", Some(&other), 1, 60, 100))
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        handle.call(|s| s.status()).await.unwrap().pools[0]
            .addresses
            .len(),
        2
    );
    drop(h);
    drop(handle);
    worker.await.unwrap();
}

#[test]
fn failed_promotion_rolls_back_every_role_and_job_and_keys_restore() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let pool = s.ensure_pool("a", "5").unwrap();
    let wallets = s.status().unwrap().pools.remove(0).addresses;
    let secret = s.wallet_secret(&pool, &wallets[0].id).unwrap();
    for w in &wallets {
        s.record_credit(&w.id, "5000000", "block", 1).unwrap();
    }
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_job BEFORE INSERT ON funding_jobs BEGIN SELECT RAISE(ABORT,'injected disk-write failure'); END;").unwrap();
    assert!(s.promote(&pool, 0).is_err());
    let now = s.status().unwrap();
    assert_eq!(now.pools[0].generation, 0);
    assert_eq!(now.pools[0].addresses.len(), 2);
    assert_eq!(now.pools[0].addresses[0].role, "ACTIVE");
    assert_eq!(now.pools[0].addresses[1].role, "READY");
    db.execute_batch("DROP TRIGGER fail_job").unwrap();
    drop(db);
    let id = s.id().to_owned();
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(
        s.wallet_secret(&pool, &wallets[0].id).unwrap().as_slice(),
        secret.as_slice()
    );
    assert!(s.wallet_secret("other-pool", &wallets[0].id).is_err());
    s.promote(&pool, 0).unwrap();
}

#[tokio::test]
async fn cancelling_a_waiter_does_not_cancel_an_accepted_store_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let s = create(dir.path());
    let (handle, worker) = StoreHandle::spawn(s);
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let h = handle.clone();
    let caller = tokio::spawn(async move {
        h.call(move |s| {
            let _ = entered.send(());
            gate.recv().unwrap();
            s.ensure_pool("survives", "5")
        })
        .await
    });
    started.await.unwrap();
    caller.abort();
    release.send(()).unwrap();
    assert_eq!(
        handle.call(|s| s.status()).await.unwrap().pools[0].name,
        "survives"
    );
    drop(handle);
    worker.await.unwrap();
}

#[test]
#[ignore = "subprocess helper for the crash-recovery test"]
fn committed_crash_child() {
    let root = std::env::var_os("X402_TEST_COMMITTED_CRASH_DIR").expect("explicit subprocess only");
    let mut s = create(Path::new(&root));
    let pool = s.ensure_pool("crash", "5").unwrap();
    s.reserve("saved", Some(&pool), 1, 100, 100).unwrap();
    s.prepare(
        "saved",
        1,
        b"post-crash-calculation",
        b"same-transaction-bytes",
    )
    .unwrap();
    // Exit without Rust destructors: recovery must use the committed WAL.
    std::process::exit(17);
}

#[test]
fn committed_state_recovers_after_process_exit_without_destructors() {
    let dir = tempfile::tempdir().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "committed_crash_child"])
        .env("X402_TEST_COMMITTED_CRASH_DIR", dir.path())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(17));
    let persisted = status(&dir.path().join("state")).unwrap();
    let mut s = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &persisted.treasury_id,
    )
    .unwrap();
    assert_eq!(
        s.prepared_bytes("saved").unwrap().as_slice(),
        b"same-transaction-bytes"
    );
    assert_eq!(
        s.snapshot().unwrap().1.as_slice(),
        b"post-crash-calculation"
    );
    assert_eq!(s.ensure_pool("crash", "5").unwrap(), persisted.pools[0].id);
    assert_eq!(s.status().unwrap().pools[0].addresses.len(), 2);
    assert!(s.reserve("duplicate", None, 2, 1, 100).is_err());
}
