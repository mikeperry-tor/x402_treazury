#[path = "support/capacity.rs"]
mod capacity_fixture;
#[path = "support/funding_quote.rs"]
mod quote_fixture;
use std::path::Path;
#[path = "support/consume_reserve.rs"]
mod consume_reserve;
#[path = "support/rpc_evidence.rs"]
mod rpc_evidence;
use x402_treazury::rotation::store::{Store, StoreHandle, status};
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
    crate::capacity_fixture::declare_and_allocate!(
        store,
        "research",
        crate::capacity_fixture::two_reserves("5.00"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
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
fn groups_consume_reserves_independently_and_keep_allocation_targets() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let a = crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5.00"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let b = crate::capacity_fixture::declare_and_allocate!(
        s,
        "b",
        crate::capacity_fixture::two_reserves("10.00"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let wallets = s.status().unwrap().pools;
    for pool in &wallets {
        assert_eq!(pool.addresses.len(), 2);
        assert!(!pool.inventory_ready);
    }
    let first = &wallets[0].addresses[0];
    rpc_evidence::credit(&mut s, &first.id, "4999999", 13).unwrap();
    assert!(!s.status().unwrap().pools[0].inventory_ready);
    for address in &wallets[0].addresses {
        rpc_evidence::credit(&mut s, &address.id, "5000000", 14).unwrap();
    }
    let now = s.status().unwrap();
    assert!(now.pools[0].inventory_ready);
    assert!(!now.pools[1].inventory_ready);
    assert_eq!(
        crate::capacity_fixture::declare_and_allocate!(
            s,
            "a",
            crate::capacity_fixture::two_reserves("7.00"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap(),
        a
    );
    s = consume_reserve::consume_and_replace(
        s,
        &dir.path().join("state"),
        &dir.path().join("key"),
        &a,
    );
    assert!(
        s.allocate_capacity_kind(
            &a,
            x402_treazury::rotation::store::AllocationKind::Small,
            true
        )
        .unwrap()
        .is_none()
    );
    let now = s.status().unwrap();
    assert_eq!(now.pools[0].addresses.len(), 3);
    assert_eq!(now.pools[0].addresses[0].role, "ACTIVE");
    assert_eq!(now.pools[0].addresses[1].role, "READY");
    assert_eq!(now.pools[0].addresses[1].target, "5000000");
    assert_eq!(now.pools[0].addresses[2].target, "7000000");
    assert_eq!(now.pools[1].addresses.len(), 2);
    s.accept_preparation("pending", Some(&a), 10, None).unwrap();
    s.disable_pool("a").unwrap();
    assert!(
        s.allocate_capacity_kind(
            &a,
            x402_treazury::rotation::store::AllocationKind::Small,
            true
        )
        .is_err()
    );
    assert!(s.accept_preparation("new", Some(&a), 10, None).is_err());
    s.prepare("pending", 1, b"snapshot", b"raw").unwrap();
    assert_eq!(s.prepared_bytes("pending").unwrap().as_slice(), b"raw");
    assert_eq!(s.snapshot().unwrap().0, 2);
    let id = s.id().to_owned();
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(
        crate::capacity_fixture::declare_and_allocate!(
            s,
            "a",
            crate::capacity_fixture::two_reserves("7"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap(),
        a
    );
    assert_eq!(s.status().unwrap().pools[0].addresses.len(), 3);
    let managed = std::collections::BTreeSet::from(["a".into()]);
    let statics = std::collections::BTreeSet::from(["b".into()]);
    assert!(s.configure_profiles(&managed, &statics).is_err());
    assert!(s.status().unwrap().pools[1].enabled);
    s.configure_profiles(&managed, &Default::default()).unwrap();
    assert!(!s.status().unwrap().pools[1].enabled);
    assert_eq!(
        crate::capacity_fixture::declare_and_allocate!(
            s,
            "b",
            crate::capacity_fixture::two_reserves("10"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap(),
        b
    );
    assert_eq!(s.status().unwrap().pools[1].addresses.len(), 2);
}
#[test]
fn source_accounting_and_prepared_send_survive_restart_without_duplicate_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let id = s.id().to_owned();
    let a = crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let b = crate::capacity_fixture::declare_and_allocate!(
        s,
        "b",
        crate::capacity_fixture::two_reserves("10"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    s.accept_preparation("op-a", Some(&a), 60, None).unwrap();
    s.accept_preparation("op-a", Some(&a), 60, None).unwrap();
    assert!(s.accept_preparation("op-a", Some(&b), 60, None).is_err());
    s.accept_preparation("independent", Some(&b), 50, None)
        .unwrap();
    s.accept_preparation("op-b", Some(&b), 40, None).unwrap();
    assert!(s.require_unstarted_preparation("op-a").is_err());
    assert!(s.require_unstarted_preparation("op-b").is_err());
    assert_eq!(
        s.prepare("op-a", 1, b"post-calc", b"exact-transaction")
            .unwrap(),
        2
    );
    assert!(s.prepare("op-b", 2, b"other", b"other-tx").is_err());
    assert!(
        s.accept_preparation("blocked-after-raw", Some(&b), 40, None)
            .is_err()
    );
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
    assert!(s.require_unstarted_preparation("op-a").is_err());
    assert!(s.require_unstarted_preparation("op-b").is_err());
    s.confirm_spend("op-a").unwrap();
    s.confirm_spend("op-a").unwrap();
    s.accept_preparation("op-a", Some(&a), 60, None).unwrap();
    s.accept_preparation("op-b", Some(&b), 40, None).unwrap();
    assert_eq!(s.prepare("op-b", 2, b"post-second", b"tx-b").unwrap(), 3);
}
#[tokio::test]
async fn concurrent_pool_requests_and_source_result_publications_serialize() {
    let dir = tempfile::tempdir().unwrap();
    let s = create(dir.path());
    let (handle, worker) = StoreHandle::spawn(s);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let h = handle.clone();
        tasks.push(tokio::spawn(async move {
            h.call(|s| {
                crate::capacity_fixture::declare_and_allocate!(
                    s,
                    "same",
                    crate::capacity_fixture::two_reserves("5"),
                    Some(alloy_primitives::U256::from(1_000_000u64)),
                    0
                )
            })
            .await
            .unwrap()
        }));
    }
    let mut ids = std::collections::BTreeSet::new();
    for t in tasks {
        ids.insert(t.await.unwrap());
    }
    assert_eq!(ids.len(), 1);
    let h = handle.clone();
    let pool = ids.into_iter().next().unwrap();
    let other = handle
        .call(|s| {
            crate::capacity_fixture::declare_and_allocate!(
                s,
                "other",
                crate::capacity_fixture::two_reserves("5"),
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0
            )
        })
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        handle.call(move |s| s.accept_preparation("a", Some(&pool), 60, None)),
        h.call(move |s| s.accept_preparation("b", Some(&other), 60, None))
    );
    a.unwrap();
    b.unwrap();
    handle
        .call(|s| {
            assert!(s.require_unstarted_preparation("a").is_err());
            assert!(s.require_unstarted_preparation("b").is_err());
            assert_eq!(s.snapshot()?.0, 1);
            Ok(())
        })
        .await
        .unwrap();
    // These are durable result publications, not concurrent SDK calculations.
    // Accepted intents are audit-only; actual raw ownership and snapshot CAS
    // prevent a second result from replacing the committed wallet state.
    let (a, b) = tokio::join!(
        handle.call(|s| s.prepare("a", 1, b"post-a", b"raw-a")),
        h.call(|s| s.prepare("b", 1, b"post-b", b"raw-b")),
    );
    assert_ne!(
        a.is_ok(),
        b.is_ok(),
        "only one original raw result is committed"
    );
    let (winner, loser, raw) = if a.is_ok() {
        ("a", "b", b"raw-a")
    } else {
        ("b", "a", b"raw-b")
    };
    handle
        .call(move |s| {
            assert_eq!(s.snapshot()?.0, 2);
            assert_eq!(s.prepared_bytes(winner)?.as_slice(), raw);
            assert!(s.prepared_bytes(loser).is_err());
            assert!(s.operation_pending(winner)?);
            assert!(s.accept_preparation("third", None, 1, None).is_err());
            Ok(())
        })
        .await
        .unwrap();
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
fn failed_refill_preserves_committed_payment_roles_and_keys_restore() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let pool = crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let wallets = s.status().unwrap().pools.remove(0).addresses;
    let secret = s.wallet_secret(&pool, &wallets[0].id).unwrap();
    for w in &wallets {
        rpc_evidence::credit(&mut s, &w.id, "5000000", 13).unwrap();
    }
    s = consume_reserve::consume_only(s, &dir.path().join("state"), &dir.path().join("key"), &pool);
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_job BEFORE INSERT ON funding_jobs BEGIN SELECT RAISE(ABORT,'injected disk-write failure'); END;").unwrap();
    assert!(
        s.allocate_capacity_kind(
            &pool,
            x402_treazury::rotation::store::AllocationKind::Small,
            true
        )
        .is_err()
    );
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
    assert!(
        s.allocate_capacity_kind(
            &pool,
            x402_treazury::rotation::store::AllocationKind::Small,
            true
        )
        .unwrap()
        .is_some()
    );
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
            crate::capacity_fixture::declare_and_allocate!(
                s,
                "survives",
                crate::capacity_fixture::two_reserves("5"),
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0
            )
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
    let pool = crate::capacity_fixture::declare_and_allocate!(
        s,
        "crash",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    s.accept_preparation("saved", Some(&pool), 100, None)
        .unwrap();
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
    assert_eq!(
        crate::capacity_fixture::declare_and_allocate!(
            s,
            "crash",
            crate::capacity_fixture::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap(),
        persisted.pools[0].id
    );
    assert_eq!(s.status().unwrap().pools[0].addresses.len(), 2);
    assert!(s.accept_preparation("duplicate", None, 1, None).is_err());
    let revision = s.snapshot().unwrap().0;
    assert!(
        s.prepare(
            "duplicate",
            revision,
            b"duplicate snapshot",
            b"duplicate bytes"
        )
        .is_err()
    );
    assert_eq!(
        s.prepared_bytes("saved").unwrap().as_slice(),
        b"same-transaction-bytes"
    );
}

#[test]
fn sync_checkpoints_are_atomic_without_a_local_capacity_gate() {
    use x402_treazury::rotation::store::{SyncObservation, SyncPhase};
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let ready = SyncObservation {
        phase: SyncPhase::Ready,
        last_error: None,
        snapshot_revision: 0,
        checked_at: Some(100),
        checkpoint_at: 0,
        scanned_blocks: 1,
        target_height: Some(2_000_000),
        observed_tip_height: None,
        height: Some(2_000_000),
        confirmations: 3,
        confirmed_pool_balances_zatoshis: None,
        confirmed_shielded_zatoshis: 1_000,
        spendable_shielded_zatoshis: 500,
    };
    assert!(s.require_preparation_available().is_ok());
    let revision = s
        .save_sync_snapshot(1, b"synced-wallet", Some(ready.clone()))
        .unwrap();
    assert_eq!(revision, 2);
    s.require_preparation_available().unwrap();
    s.require_preparation_available().unwrap();
    s.require_preparation_available().unwrap();
    assert!(s.require_preparation_available().is_ok());
    let mut stale = ready.clone();
    stale.phase = SyncPhase::Failed;
    assert!(
        s.save_sync_snapshot(1, b"stale-wallet", Some(stale))
            .is_err()
    );
    assert_eq!(s.snapshot().unwrap().1.as_slice(), b"synced-wallet");
    assert_eq!(s.status().unwrap().sync.unwrap().phase, SyncPhase::Ready);
    s.save_snapshot(2, b"derived-address").unwrap();
    assert!(s.require_preparation_available().is_ok());
    s.save_sync_snapshot(3, b"resynced", Some(ready.clone()))
        .unwrap();
    s.accept_preparation("outgoing", None, 100, None).unwrap();
    s.prepare("outgoing", 4, b"calculated", b"signed-transaction")
        .unwrap();
    s.save_sync_snapshot(5, b"still-pending", Some(ready))
        .unwrap();
    assert_eq!(
        s.require_preparation_available().unwrap_err().to_string(),
        "treasury_send_pending"
    );
    s.confirm_spend("outgoing").unwrap();
    s.set_sync_phase(SyncPhase::Offline).unwrap();
    assert!(s.require_preparation_available().is_ok());
    let snapshot = s.snapshot().unwrap();
    let id = s.id().to_owned();
    drop(s);
    let s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(s.snapshot().unwrap().0, snapshot.0);
    assert_eq!(s.snapshot().unwrap().1, snapshot.1);
    assert!(s.require_preparation_available().is_ok());
}

#[test]
fn legacy_sync_schema_is_rejected_without_changing_wallet() {
    let dir = tempfile::tempdir().unwrap();
    let s = create(dir.path());
    let id = s.id().to_owned();
    drop(s);
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    // Build an actual empty version-1 schema while retaining the encrypted identity
    // and snapshot. Merely lowering user_version on schema16 is not a legacy fixture.
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT IN ('instance','snapshots')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for table in tables {
        db.execute_batch(&format!("DROP TABLE \"{}\";", table.replace('"', "\"\"")))
            .unwrap();
    }
    let mut version = 0;
    for line in include_str!("fixtures/state/historical.sql").lines() {
        if let Some(value) = line.strip_prefix("-- version ") {
            version = value.parse::<u32>().unwrap();
        } else if version <= 1
            && line.starts_with("CREATE ")
            && !line.starts_with("CREATE TABLE instance(")
            && !line.starts_with("CREATE TABLE snapshots(")
        {
            db.execute_batch(line).unwrap();
        }
    }
    db.execute_batch("PRAGMA user_version=1;").unwrap();
    drop(db);
    assert!(status(&dir.path().join("state")).unwrap().sync.is_none());
    let saved = std::fs::read(dir.path().join("state/state.sqlite")).unwrap();
    let error = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id)
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("stopped-service conversion required")
    );
    assert_eq!(
        std::fs::read(dir.path().join("state/state.sqlite")).unwrap(),
        saved
    );
    assert!(status(&dir.path().join("state")).unwrap().sync.is_none());
}

#[test]
fn submission_contract_only_accepts_durable_pending_bytes() {
    use x402_treazury::rotation::transaction::PreparedTransaction;
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    assert!(PreparedTransaction::load(&s, "op").is_err());
    s.accept_preparation("op", None, 100, None).unwrap();
    assert!(PreparedTransaction::load(&s, "op").is_err());
    s.prepare("op", 1, b"calculated", b"exact-bytes").unwrap();
    // Later sync checkpoints must preserve the snapshot referenced by this send.
    s.save_snapshot(2, b"synced-after-calculation").unwrap();
    assert_eq!(
        s.prepare("op", 1, b"calculated", b"exact-bytes").unwrap(),
        2
    );
    let id = s.id().to_owned();
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    let durable = PreparedTransaction::load(&s, "op").unwrap();
    assert_eq!(durable.operation_id(), "op");
    assert_eq!(durable.bytes(), b"exact-bytes");
    s.confirm_spend("op").unwrap();
    assert!(PreparedTransaction::load(&s, "op").is_err());
}

#[test]
fn durable_submission_attempts_preserve_ambiguous_exposure_and_exact_bytes() {
    use x402_treazury::rotation::transaction::{PreparedTransaction, TransactionFacts};
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let facts = TransactionFacts {
        txid: "a".repeat(64),
        expiry_height: 500,
        amount_zatoshis: 50,
        fee_zatoshis: 10,
        deadline: Some(1000),
    };
    s.accept_preparation("op", None, 60, None).unwrap();
    let mut too_large = facts.clone();
    too_large.fee_zatoshis = 60;
    assert!(
        s.prepare_with_facts("op", 1, b"post-calculation", b"signed", Some(too_large))
            .is_err()
    );
    assert_eq!(s.snapshot().unwrap().0, 1);
    assert!(!s.operation_pending("op").unwrap());
    s.prepare_with_facts("op", 1, b"post-calculation", b"signed", Some(facts.clone()))
        .unwrap();
    assert_eq!(s.operation("op").unwrap().facts, facts);
    assert!(s.accept_preparation("other", None, 40, None).is_err());
    assert!(s.abandon_unprepared("op").is_err());
    s.abandon_unprepared("other").unwrap();
    assert!(s.request_broadcast("op", 1000, 499, false).is_err());

    assert!(s.request_broadcast("op", 100, 500, false).is_err());
    assert_eq!(s.request_broadcast("op", 100, 499, false).unwrap(), 1);
    let id = s.id().to_owned();
    drop(s); // crash before a submission result
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(s.operation("op").unwrap().submission, "BROADCAST_REQUESTED");
    assert!(s.request_broadcast("op", 101, 499, false).is_err());
    let saved = PreparedTransaction::load(&s, "op").unwrap();
    assert_eq!(saved.bytes(), b"signed");
    assert_eq!(s.request_broadcast("op", 101, 499, true).unwrap(), 2);
    assert!(s.broadcast_result("op", 1, true).is_err()); // stale response
    s.broadcast_result("op", 2, false).unwrap();
    assert_eq!(s.operation("op").unwrap().submission, "UNKNOWN");
    assert!(s.operation_pending("op").unwrap());
    let attempt = s.request_broadcast("op", 102, 499, true).unwrap();
    s.broadcast_result("op", attempt, true).unwrap();
    assert_eq!(s.operation("op").unwrap().submission, "BROADCAST");
    assert!(s.operation_pending("op").unwrap()); // accepted is not confirmed
    s.confirm_spend("op").unwrap();
    assert_eq!(s.operation("op").unwrap().submission, "CONFIRMED");
    assert!(s.request_broadcast("op", 103, 499, true).is_err());
}

#[test]
fn funding_journal_keeps_quote_identity_and_observation_health_after_restart() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let id = s.id().to_owned();
    crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    crate::capacity_fixture::declare_and_allocate!(
        s,
        "b",
        crate::capacity_fixture::two_reserves("7"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let jobs = s.funding_jobs().unwrap();
    assert_eq!(jobs.len(), 4);
    assert_eq!(
        jobs.iter()
            .map(|job| &job.operation_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
    let job = &jobs[0];
    s.save_funding_quote(
        &job.id,
        &quote_fixture::bound(&job.target, &job.recipient, "sensitive-bound-quote"),
    )
    .unwrap();
    assert!(
        s.save_funding_quote(
            &job.id,
            &quote_fixture::bound(&job.target, &job.recipient, "other quote")
        )
        .is_err()
    );
    assert!(
        s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Complete)
            .is_err()
    );
    s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Preparing)
        .unwrap();
    s.defer_funding(&job.id, 42, Some("pending"), false)
        .unwrap();
    drop(s);
    let s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    let restored = s
        .funding_jobs()
        .unwrap()
        .into_iter()
        .find(|j| j.id == job.id)
        .unwrap();
    assert_eq!(restored.operation_id, job.operation_id);
    assert_eq!(restored.phase, FundingPhase::Preparing);
    assert_eq!(
        s.funding_quote(&job.id).unwrap().as_slice(),
        quote_fixture::bound(&job.target, &job.recipient, "sensitive-bound-quote").as_slice()
    );
    assert_eq!(restored.next_poll, 42);
    assert_eq!(restored.last_error.as_deref(), Some("pending"));
    for expected in &jobs {
        assert_eq!(
            s.funding_job(&expected.id).unwrap().unwrap().operation_id,
            expected.operation_id
        );
    }
    assert_eq!(s.status().unwrap().funding_jobs.len(), 4);
}

#[test]
fn complete_backup_restores_keys_quotes_and_pending_source_accounting() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let id = s.id().to_owned();
    let pool = crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let jobs = s.funding_jobs().unwrap();
    s.save_funding_quote(
        &jobs[0].id,
        &quote_fixture::bound(&jobs[0].target, &jobs[0].recipient, "bound-quote"),
    )
    .unwrap();
    s.accept_preparation("source", Some(&pool), 100, None)
        .unwrap();
    s.prepare("source", 1, b"prepared-snapshot", b"durable-signed-bytes")
        .unwrap();
    let original = serde_json::to_value(s.status().unwrap()).unwrap();
    let secret = s.wallet_secret(&pool, &jobs[0].wallet_id).unwrap();
    let backup = dir.path().join("backup");
    s.backup(&backup).unwrap();
    assert!(s.backup(&backup).is_err());
    let restored = Store::open(&backup, &backup.join("key"), &id).unwrap();
    assert_eq!(
        serde_json::to_value(restored.status().unwrap()).unwrap(),
        original
    );
    assert_eq!(
        restored
            .wallet_secret(&pool, &jobs[0].wallet_id)
            .unwrap()
            .as_slice(),
        secret.as_slice()
    );
    assert_eq!(
        restored.prepared_bytes("source").unwrap().as_slice(),
        b"durable-signed-bytes"
    );
    assert_eq!(
        restored.funding_quote(&jobs[0].id).unwrap().as_slice(),
        quote_fixture::bound(&jobs[0].target, &jobs[0].recipient, "bound-quote").as_slice()
    );
    assert!(backup.join("backup.json").is_file());
}

#[test]
fn unprepared_recovery_changes_operation_but_never_releases_signed_liability() {
    use x402_treazury::rotation::store::funding::FundingPhase;
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let pool = crate::capacity_fixture::declare_and_allocate!(
        s,
        "a",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let job = s.funding_jobs().unwrap().remove(0);
    s.save_funding_quote(
        &job.id,
        &quote_fixture::bound(&job.target, &job.recipient, "old-quote"),
    )
    .unwrap();
    s.recover_unprepared_funding(&job.id).unwrap();
    assert_eq!(s.funding_jobs().unwrap()[0].operation_id, job.operation_id);
    s.reauthorize_closed_funding(&job.id, 1).unwrap();
    let next = s.funding_jobs().unwrap().remove(0);
    assert_ne!(next.operation_id, job.operation_id);
    assert_eq!(next.recipient, job.recipient);
    assert_eq!(next.phase, FundingPhase::Allocated);
    assert!(s.funding_quote(&job.id).is_err());
    s.save_funding_quote(
        &job.id,
        &quote_fixture::bound(&job.target, &job.recipient, "new-quote"),
    )
    .unwrap();
    s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Preparing)
        .unwrap();
    s.accept_preparation(&next.operation_id, Some(&pool), 100, None)
        .unwrap();
    assert!(s.recover_unprepared_funding(&job.id).is_err());
    assert_eq!(s.funding_jobs().unwrap()[0].operation_id, next.operation_id);
    s.prepare(&next.operation_id, 1, b"next-snapshot", b"signed")
        .unwrap();
    assert!(s.recover_unprepared_funding(&job.id).is_err());
    assert!(s.operation_pending(&next.operation_id).unwrap());
}

#[test]
fn operation_and_refund_identity_survive_promotion_and_reopen() {
    use x402_treazury::{
        network::IsolationId,
        rotation::{
            store::funding::FundingPhase,
            transaction::{PreparedTransaction, TransactionFacts},
        },
    };
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let treasury_id = s.id().to_owned();
    let pool = crate::capacity_fixture::declare_and_allocate!(
        s,
        "identity",
        crate::capacity_fixture::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let job = s.funding_jobs().unwrap().remove(0);
    s.save_refund_address(&job.id, "fixture-refund-address", 1, b"refund snapshot")
        .unwrap();
    s.save_funding_quote(
        &job.id,
        &quote_fixture::bound(&job.target, &job.recipient, "immutable quote"),
    )
    .unwrap();
    s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Preparing)
        .unwrap();
    s.accept_preparation(&job.operation_id, Some(&pool), 100, None)
        .unwrap();
    s.prepare_with_facts(
        &job.operation_id,
        2,
        b"prepared snapshot",
        b"immutable signed bytes",
        Some(TransactionFacts {
            txid: "fixture".into(),
            expiry_height: 100,
            amount_zatoshis: 80,
            fee_zatoshis: 20,
            deadline: Some(1000),
        }),
    )
    .unwrap();
    let expected = IsolationId::evm(&job.recipient).unwrap();
    assert_eq!(
        PreparedTransaction::load(&s, &job.operation_id)
            .unwrap()
            .network_identity()
            .unwrap(),
        expected
    );
    for address in s.status().unwrap().pools[0]
        .addresses
        .iter()
        .map(|a| a.id.clone())
        .collect::<Vec<_>>()
    {
        rpc_evidence::credit(&mut s, &address, "5000000", 13).unwrap();
    }
    s = consume_reserve::consume_and_replace(
        s,
        &dir.path().join("state"),
        &dir.path().join("key"),
        &pool,
    );
    assert_ne!(
        s.status().unwrap().pools[0].addresses[1].address,
        job.recipient
    );
    drop(s);
    let s = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury_id,
    )
    .unwrap();
    let original = PreparedTransaction::load(&s, &job.operation_id).unwrap();
    assert_eq!(original.network_identity().unwrap(), expected);
    assert_eq!(original.bytes(), b"immutable signed bytes");
    assert_eq!(
        s.refund_address(&job.id).unwrap().as_deref(),
        Some("fixture-refund-address")
    );
    assert_eq!(
        s.funding_quote(&job.id).unwrap().as_slice(),
        quote_fixture::bound(&job.target, &job.recipient, "immutable quote").as_slice()
    );
}

#[test]
fn accepted_calculation_survives_reopen_without_local_balance_or_day_policy() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let id = s.id().to_owned();
    assert!(s.require_preparation_available().is_ok());
    s.accept_preparation("pending", None, 90, None).unwrap();
    assert!(s.require_preparation_available().is_ok());
    assert!(s.abandon_unprepared("pending").is_err());
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert!(!s.status().unwrap().outgoing_pending);
    assert!(s.accept_preparation("another", None, 1, None).is_ok());
    assert!(s.accept_preparation("pending", None, 91, None).is_err());
    assert!(s.abandon_unprepared("pending").is_err());
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('budget_entries','refund_outputs')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn bridge_target_and_quote_commit_atomically_and_survive_restart() {
    use x402_treazury::rotation::near::Quote;
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let id = s.id().to_owned();
    crate::capacity_fixture::declare_and_allocate!(
        s,
        "minimum",
        x402_treazury::rotation::config::ReservesConfig {
            small: x402_treazury::rotation::config::ReserveConfig {
                funding_amount_usdc: "2".into(),
                max_funding_amount_usdc: Some("2.5".into())
            },
            large: None
        },
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let job = s.funding_jobs().unwrap()[0].clone();
    let quote = |target: &str| {
        serde_json::to_vec(&Quote {
            request: serde_json::json!({"amount":target,"recipient":job.recipient}),
            response: serde_json::json!({}),
            input: 100000,
            deadline: 2000000000,
            deposit: Some("fixture".into()),
        })
        .unwrap()
    };
    s.select_quote_terms(&job.id, &job.operation_id, "2000000", "2500000")
        .unwrap();
    assert!(
        s.save_funding_quote_with_target(&job.id, &quote("1000000"), "1000000")
            .is_err()
    );
    assert!(
        s.save_funding_quote_with_target(&job.id, &quote("2500000"), "3000000")
            .is_err()
    );
    // Inject a failure at acceptance; immutable intent and absence of accepted terms remain.
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_quote BEFORE UPDATE OF quote ON funding_progress BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        s.save_funding_quote_with_target(&job.id, &quote("2500000"), "2500000")
            .is_err()
    );
    assert_eq!(s.funding_jobs().unwrap()[0].target, "2000000");
    db.execute_batch("DROP TRIGGER reject_quote;").unwrap();
    s.save_funding_quote_with_target(&job.id, &quote("2500000"), "2500000")
        .unwrap();
    s.save_funding_quote_with_target(&job.id, &quote("2500000"), "2500000")
        .unwrap();
    assert!(
        s.save_funding_quote_with_target(&job.id, &quote("3000000"), "3000000")
            .is_err()
    );
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    crate::capacity_fixture::declare_and_allocate!(
        s,
        "minimum",
        crate::capacity_fixture::two_reserves("2"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let state = s.status().unwrap();
    assert_eq!(
        state.pools[0]
            .addresses
            .iter()
            .find(|a| a.id == job.wallet_id)
            .unwrap()
            .target,
        "2000000"
    );
    assert_eq!(s.funding_jobs().unwrap()[0].target, "2000000");
    assert_eq!(
        s.destination_requirement(&job.id).unwrap().amount,
        "2500000"
    );
    rpc_evidence::credit(&mut s, &job.wallet_id, "2000000", 13).unwrap();
    let initial = s.destination_requirement(&job.id).unwrap();
    assert!(
        initial.recorded,
        "partial credit consumes the unique initial claim"
    );
    assert_eq!(initial.amount, "2000000");
    assert_eq!(s.funding_jobs().unwrap()[0].target, "2000000");
    assert!(s.status().unwrap().treasury_operations.is_empty());
}

#[tokio::test]
async fn canonical_depletion_is_debug_without_an_sdk_balance_admission_gate() {
    // Isolate tracing's callsite interest cache from concurrent tests that emit
    // the same event without a subscriber.
    const CHILD: &str = "TREAZURY_TEST_TREASURY_WARNING_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "canonical_depletion_is_debug_without_an_sdk_balance_admission_gate",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    use std::sync::{Arc, Mutex};
    use x402_treazury::rotation::store::{SyncObservation, SyncPhase};
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let capture = Capture(Arc::default());
    let sink = capture.clone();
    use tracing_subscriber::prelude::*;
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .without_time()
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                metadata.target() == "x402_treazury::rotation::store"
            })),
    );
    let _guard = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let mut store = create(dir.path());
    let observation = SyncObservation {
        phase: SyncPhase::Ready,
        last_error: None,
        snapshot_revision: 0,
        checked_at: Some(100),
        checkpoint_at: 0,
        scanned_blocks: 1,
        target_height: Some(2_000_000),
        observed_tip_height: None,
        height: Some(2_000_000),
        confirmations: 3,
        confirmed_pool_balances_zatoshis: None,
        confirmed_shielded_zatoshis: 0,
        spendable_shielded_zatoshis: 0,
    };
    store
        .save_sync_snapshot(1, b"empty", Some(observation))
        .unwrap();
    assert!(store.require_preparation_available().is_ok());
    assert!(
        capture.0.lock().unwrap().is_empty(),
        "saved SDK balances are diagnostic only"
    );
    assert!(store.status().unwrap().treasury_operations.is_empty());
    capture.0.lock().unwrap().clear();
    let pool = crate::capacity_fixture::declare_and_allocate!(
        store,
        "drain",
        crate::capacity_fixture::two_reserves("1"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let mut status = store.status().unwrap();
    let wallets = status.pools.remove(0).addresses;
    let amounts = |first: u64, second: u64| {
        [
            (wallets[0].id.clone(), first),
            (wallets[1].id.clone(), second),
        ]
        .into()
    };
    // Initial zeroes are unfunded allocations, not exhaustion.
    let view = rpc_evidence::acquire(store.chain_query(&pool).unwrap(), amounts(0, 0), 100).await;
    store.reconcile_pool(&pool, view).unwrap();
    let view = rpc_evidence::acquire(
        store.chain_query(&pool).unwrap(),
        amounts(1_000_000, 1_000_000),
        200,
    )
    .await;
    store.reconcile_pool(&pool, view).unwrap();
    let view = rpc_evidence::acquire(
        store.chain_query(&pool).unwrap(),
        amounts(1, 1_000_000),
        300,
    )
    .await;
    store.reconcile_pool(&pool, view).unwrap();
    assert!(capture.0.lock().unwrap().is_empty());
    // An incomplete view must roll back without reporting depletion.
    let mut incomplete = rpc_evidence::acquire(
        store.chain_query(&pool).unwrap(),
        amounts(0, 1_000_000),
        400,
    )
    .await;
    incomplete.balances.remove(&wallets[1].id);
    assert!(store.reconcile_pool(&pool, incomplete).is_err());
    assert!(capture.0.lock().unwrap().is_empty());
    let view = rpc_evidence::acquire(
        store.chain_query(&pool).unwrap(),
        amounts(0, 1_000_000),
        500,
    )
    .await;
    store.reconcile_pool(&pool, view).unwrap();
    let view = rpc_evidence::acquire(
        store.chain_query(&pool).unwrap(),
        amounts(0, 1_000_000),
        600,
    )
    .await;
    store.reconcile_pool(&pool, view).unwrap();
    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logs.matches("evm_wallet_depleted").count(), 1);
    assert!(logs.contains("DEBUG") && !logs.contains("WARN") && !logs.contains("ERROR"));
    assert!(logs.contains(&wallets[0].id));
    assert!(logs.contains(&pool));
    capture.0.lock().unwrap().clear();
    let view = rpc_evidence::acquire(store.chain_query(&pool).unwrap(), amounts(0, 0), 700).await;
    store.reconcile_pool(&pool, view).unwrap();
    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logs.matches("evm_wallet_depleted").count(), 1);
    assert!(logs.contains("DEBUG") && !logs.contains("WARN") && !logs.contains("ERROR"));
    assert!(logs.contains(&wallets[1].id));
}
