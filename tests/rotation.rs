use std::path::Path;
use x402_treazure::rotation::store::{Store, StoreHandle, status};
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
    // Simulate a database produced by the offline foundation, before admission tables.
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch(
        "DROP TABLE payment_attempts; DROP TABLE payment_anchors; DROP TABLE treasury_operations; DROP TABLE treasury_sync; PRAGMA user_version=0;",
    )
    .unwrap();
    drop(db);
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
    let managed = std::collections::BTreeSet::from(["a".into()]);
    let statics = std::collections::BTreeSet::from(["b".into()]);
    assert!(s.configure_profiles(&managed, &statics).is_err());
    assert!(s.status().unwrap().pools[1].enabled);
    s.configure_profiles(&managed, &Default::default()).unwrap();
    assert!(!s.status().unwrap().pools[1].enabled);
    assert_eq!(s.ensure_pool("b", "10").unwrap(), b);
    assert_eq!(s.status().unwrap().pools[1].addresses.len(), 2);
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

#[test]
fn sync_checkpoints_are_atomic_and_readiness_is_revision_and_time_bound() {
    use x402_treazure::rotation::store::{SyncObservation, SyncPhase};
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
        height: Some(2_000_000),
        confirmations: 3,
        max_age_seconds: 10,
        confirmed_pool_balances_zatoshis: None,
        confirmed_shielded_zatoshis: 1_000,
        spendable_shielded_zatoshis: 500,
    };
    assert!(s.require_spend_ready(100, 1).is_err());
    let revision = s
        .save_sync_snapshot(1, b"synced-wallet", Some(ready.clone()))
        .unwrap();
    assert_eq!(revision, 2);
    s.require_spend_ready(110, 500).unwrap();
    for (now, amount) in [(111, 1), (99, 1), (100, 501), (100, 0)] {
        assert!(s.require_spend_ready(now, amount).is_err());
    }
    let mut stale = ready.clone();
    stale.phase = SyncPhase::Failed;
    assert!(
        s.save_sync_snapshot(1, b"stale-wallet", Some(stale))
            .is_err()
    );
    assert_eq!(s.snapshot().unwrap().1.as_slice(), b"synced-wallet");
    assert_eq!(s.status().unwrap().sync.unwrap().phase, SyncPhase::Ready);
    s.save_snapshot(2, b"derived-address").unwrap();
    assert!(s.require_spend_ready(100, 1).is_err());
    s.save_sync_snapshot(3, b"resynced", Some(ready.clone()))
        .unwrap();
    s.reserve("outgoing", None, 1, 100, 1_000).unwrap();
    s.prepare("outgoing", 4, b"calculated", b"signed-transaction")
        .unwrap();
    s.save_sync_snapshot(5, b"still-pending", Some(ready))
        .unwrap();
    assert_eq!(
        s.require_spend_ready(100, 1).unwrap_err().to_string(),
        "treasury_send_pending"
    );
    s.confirm_spend("outgoing", 100, 1).unwrap();
    s.set_sync_phase(SyncPhase::Offline).unwrap();
    assert!(s.require_spend_ready(100, 1).is_err());
    let snapshot = s.snapshot().unwrap();
    let id = s.id().to_owned();
    drop(s);
    let s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(s.snapshot().unwrap().0, snapshot.0);
    assert_eq!(s.snapshot().unwrap().1, snapshot.1);
    assert!(s.require_spend_ready(100, 1).is_err());
}

#[test]
fn sync_schema_migrates_admission_state_without_changing_wallet() {
    let dir = tempfile::tempdir().unwrap();
    let s = create(dir.path());
    let id = s.id().to_owned();
    drop(s);
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch(
        "DROP TABLE treasury_operations; DROP TABLE treasury_sync; PRAGMA user_version=1;",
    )
    .unwrap();
    drop(db);
    assert!(status(&dir.path().join("state")).unwrap().sync.is_none());
    let s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert_eq!(s.snapshot().unwrap().0, 1);
    assert!(s.status().unwrap().sync.is_none());
}

#[test]
fn submission_contract_only_accepts_durable_pending_bytes() {
    use x402_treazure::rotation::transaction::PreparedTransaction;
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    assert!(PreparedTransaction::load(&s, "op").is_err());
    s.reserve("op", None, 1, 100, 1000).unwrap();
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
    s.confirm_spend("op", 100, 1).unwrap();
    assert!(PreparedTransaction::load(&s, "op").is_err());
}

#[test]
fn durable_submission_attempts_preserve_ambiguous_exposure_and_exact_bytes() {
    use x402_treazure::rotation::transaction::{PreparedTransaction, TransactionFacts};
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let facts = TransactionFacts {
        txid: "a".repeat(64),
        expiry_height: 500,
        amount_zatoshis: 50,
        fee_zatoshis: 10,
        deadline: 1000,
    };
    s.reserve("op", None, 0, 100, 100).unwrap();
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
    // Preparation shrinks the conservative reservation to actual input plus fee.
    s.reserve("other", None, 0, 40, 100).unwrap();
    assert!(s.abandon_unprepared("op").is_err());
    s.abandon_unprepared("other").unwrap();
    assert!(s.request_broadcast("op", 1000, 499, false).is_err());
    assert!(s.request_broadcast("op", 701, 499, false).is_err());
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
    s.confirm_spend("op", 60, 0).unwrap();
    assert_eq!(s.operation("op").unwrap().submission, "CONFIRMED");
    assert!(s.request_broadcast("op", 103, 499, true).is_err());
}

#[test]
fn funding_journal_keeps_quote_identity_and_schedules_fairly_after_restart() {
    use x402_treazure::rotation::store::funding::FundingPhase;
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let id = s.id().to_owned();
    s.ensure_pool("a", "5").unwrap();
    s.ensure_pool("b", "7").unwrap();
    let mut picked = std::collections::BTreeSet::new();
    for _ in 0..4 {
        picked.insert(s.next_funding_job(0).unwrap().unwrap().id);
    }
    assert_eq!(picked.len(), 4);
    let job = s.next_funding_job(0).unwrap().unwrap();
    s.save_funding_quote(&job.id, b"sensitive-bound-quote")
        .unwrap();
    assert!(s.save_funding_quote(&job.id, b"other quote").is_err());
    assert!(
        s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Complete)
            .is_err()
    );
    s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Preparing)
        .unwrap();
    s.defer_funding(&job.id, 42, Some("pending"), false)
        .unwrap();
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
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
        b"sensitive-bound-quote"
    );
    for _ in 0..4 {
        assert_ne!(s.next_funding_job(41).unwrap().unwrap().id, job.id);
    }
    assert_eq!(s.status().unwrap().funding_jobs.len(), 4);
}

#[test]
fn complete_backup_restores_keys_quotes_and_pending_source_accounting() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let id = s.id().to_owned();
    let pool = s.ensure_pool("a", "5").unwrap();
    let jobs = s.funding_jobs().unwrap();
    s.save_funding_quote(&jobs[0].id, b"bound-quote").unwrap();
    s.reserve("source", Some(&pool), 1, 100, 1000).unwrap();
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
        b"bound-quote"
    );
    assert!(backup.join("backup.json").is_file());
}

#[test]
fn unprepared_recovery_changes_operation_but_never_releases_signed_liability() {
    use x402_treazure::rotation::store::funding::FundingPhase;
    let dir = tempfile::tempdir().unwrap();
    let mut s = create(dir.path());
    let pool = s.ensure_pool("a", "5").unwrap();
    let job = s.funding_jobs().unwrap().remove(0);
    s.save_funding_quote(&job.id, b"old-quote").unwrap();
    s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Preparing)
        .unwrap();
    s.reserve(&job.operation_id, Some(&pool), 1, 100, 100)
        .unwrap();
    s.recover_unprepared_funding(&job.id).unwrap();
    let next = s.funding_jobs().unwrap().remove(0);
    assert_ne!(next.operation_id, job.operation_id);
    assert_eq!(next.recipient, job.recipient);
    assert_eq!(next.phase, FundingPhase::Allocated);
    assert!(s.funding_quote(&job.id).is_err());
    s.save_funding_quote(&job.id, b"new-quote").unwrap();
    s.advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Preparing)
        .unwrap();
    s.reserve(&next.operation_id, Some(&pool), 1, 100, 100)
        .unwrap();
    s.prepare(&next.operation_id, 1, b"next-snapshot", b"signed")
        .unwrap();
    assert!(s.recover_unprepared_funding(&job.id).is_err());
    assert!(s.operation_pending(&next.operation_id).unwrap());
}
