//! The restriction is enforced at store boundaries, independently of worker settings.
use super::*;
use std::sync::{Arc, Mutex};
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn fixture() -> (tempfile::TempDir, Store, String) {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = s.ensure_pool("fixture", "5").unwrap();
    (dir, s, pool)
}
fn denied<T>(r: Result<T>) {
    let e = r.err().expect("new funding was accepted");
    assert!(
        e.to_string().contains("qualification_funding_denied"),
        "{e:#}"
    );
}
#[test]
fn restriction_denies_bootstrap_promotion_and_source_work_with_visible_logs() {
    let (_dir, mut s, pool) = fixture();
    for wallet in s.status().unwrap().pools[0].addresses.iter() {
        s.record_credit(&wallet.id, "5000000", "block", 1).unwrap();
    }
    // A reservation inherited from an older process must not permit new calculation.
    s.reserve("old", Some(&pool), 1, 100, 1000).unwrap();
    let before = serde_json::to_value(s.status().unwrap()).unwrap();
    let logs = Logs::default();
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        s.deny_new_funding();
        s.deny_new_funding(); // Reapplying cannot lift the restriction.
        assert_eq!(s.ensure_pool("fixture", "5").unwrap(), pool);
        denied(s.ensure_pool("new_pool", "5"));
        denied(s.promote(&pool, 0));
        denied(s.reserve("new", Some(&pool), 1, 100, 1000));
        denied(s.reserve("shield", None, 1, 100, 1000));
        denied(s.require_spend_ready(10, 100));
        denied(s.prepare("old", 1, b"next", b"new bytes"));
    });
    assert_eq!(before, serde_json::to_value(s.status().unwrap()).unwrap());
    assert_eq!(
        s.db.query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        s.db.query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for stage in [
        "pool_bootstrap",
        "pool_promotion",
        "source_reservation",
        "treasury_preparation",
        "source_preparation_commit",
    ] {
        assert!(
            logs.contains(stage),
            "missing denial log for {stage}: {logs}"
        );
    }
    assert!(logs.contains("qualification_funding_denied"));
    assert!(logs.contains("limit is zero"));
}
#[test]
fn restriction_keeps_prepared_evidence_and_reconciliation_across_reopen() {
    let (dir, mut s, pool) = fixture();
    let id = s.id().to_owned();
    s.reserve("accepted", Some(&pool), 1, 100, 1000).unwrap();
    let facts = TransactionFacts {
        txid: "existing".into(),
        expiry_height: 100,
        amount_zatoshis: 80,
        fee_zatoshis: 20,
        deadline: 1000,
    };
    s.prepare_with_facts("accepted", 1, b"saved", b"saved bytes", Some(facts.clone()))
        .unwrap();
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    s.deny_new_funding(); // Supervisor must do this on EVERY launch.
    s.reserve("accepted", Some(&pool), 1, 100, 1000).unwrap(); // Idempotent, no increase.
    assert!(s.reserve("accepted", Some(&pool), 1, 101, 1000).is_err());
    assert_eq!(
        s.prepare_with_facts("accepted", 1, b"saved", b"saved bytes", Some(facts))
            .unwrap(),
        2
    );
    assert_eq!(
        s.prepared_bytes("accepted").unwrap().as_slice(),
        b"saved bytes"
    );
    // Existing accepted bytes retain ordinary deadline/broadcast guards.
    s.request_broadcast("accepted", 1, 1, false).unwrap();
    assert!(s.request_broadcast("accepted", 1, 1, false).is_err());
    s.confirm_spend("accepted", 100, 1).unwrap();
    denied(s.reserve("replacement", Some(&pool), 1, 100, 1000));
    assert_eq!(s.snapshot().unwrap().0, 2);
}

#[test]
fn restriction_does_not_poison_queued_preparation_as_interrupted_work() {
    let (_dir, mut s, _pool) = fixture();
    let job = s.funding_jobs().unwrap().remove(0);
    s.save_funding_quote(&job.id, b"saved quote").unwrap();
    s.deny_new_funding();
    denied(s.advance_funding(
        &job.id,
        funding::FundingPhase::Quoted,
        funding::FundingPhase::Preparing,
    ));
    let saved = s
        .funding_jobs()
        .unwrap()
        .into_iter()
        .find(|j| j.id == job.id)
        .unwrap();
    assert_eq!(saved.phase, funding::FundingPhase::Quoted);
    assert_eq!(saved.operation_id, job.operation_id);
    assert!(!s.operation_pending(&job.operation_id).unwrap());
}

struct TestPermits {
    treasury: String,
    db: Mutex<Connection>,
    jobs: std::sync::atomic::AtomicU32,
}
impl TestPermits {
    fn new(treasury: &str) -> Self {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE runs(id TEXT PRIMARY KEY); INSERT INTO runs VALUES('run');")
            .unwrap();
        db.execute_batch(crate::qualification::funding::SCHEMA)
            .unwrap();
        Self {
            treasury: treasury.into(),
            db: Mutex::new(db),
            jobs: 2.into(),
        }
    }
}
impl super::super::restriction::FundingPermits for TestPermits {
    fn validate_treasury(&self, treasury: &str) -> Result<()> {
        ensure!(treasury == self.treasury, "treasury mismatch");
        Ok(())
    }
    fn validate_state(&self, state: &serde_json::Value) -> Result<()> {
        self.validate_treasury(state["treasury_status"]["treasury_id"].as_str().unwrap())
    }
    fn reserve_allocations(
        &self,
        treasury: &str,
        pool: &str,
        sequences: &[i64],
    ) -> Result<Vec<String>> {
        use crate::qualification::funding::{self, Limits, Request};
        use sha2::{Digest, Sha256};
        self.validate_treasury(treasury)?;
        let requests: Vec<_> = sequences
            .iter()
            .map(|seq| Request {
                intent: format!("{pool}_{seq}"),
                pool: pool.into(),
                source_bound: 100,
            })
            .collect();
        let limit = Limits {
            jobs: self.jobs.load(std::sync::atomic::Ordering::SeqCst),
            source_zatoshis: 1000,
        };
        let mut db = self.db.lock().unwrap();
        let mut tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        funding::reserve(&mut tx, "run", &requests, limit, limit, 1)?;
        tx.commit()?;
        let mut jobs = vec![];
        for request in requests {
            let hash = Sha256::digest(request.intent.as_bytes());
            let job = Uuid::from_bytes(hash[..16].try_into().unwrap()).to_string();
            funding::link(&mut db, "run", &request.intent, &job)?;
            jobs.push(job);
        }
        Ok(jobs)
    }
    fn check_preparation(&self, treasury: &str, job: &str, input: u64) -> Result<()> {
        self.validate_treasury(treasury)?;
        crate::qualification::funding::check_preparation(
            &self.db.lock().unwrap(),
            "run",
            job,
            input,
        )
    }
    fn reserve_preparation(
        &self,
        treasury: &str,
        job: &str,
        operation: &str,
        input: u64,
    ) -> Result<()> {
        self.validate_treasury(treasury)?;
        let mut db = self.db.lock().unwrap();
        let mut tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        crate::qualification::funding::reserve_source(&mut tx, "run", job, operation, input)?;
        tx.commit()?;
        Ok(())
    }
    fn retire_unprepared(&self, treasury: &str, job: &str, operation: &str) -> Result<()> {
        self.validate_treasury(treasury)?;
        crate::qualification::funding::retire_unprepared(
            &self.db.lock().unwrap(),
            "run",
            job,
            operation,
        )
    }
}
#[test]
fn permit_hooks_bound_bootstrap_promotion_and_preparation_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let permits = Arc::new(TestPermits::new(s.id()));
    s.install_funding_permits(permits.clone()).unwrap();
    assert!(s.install_funding_permits(permits.clone()).is_err());
    permits.jobs.store(1, std::sync::atomic::Ordering::SeqCst);
    denied(s.ensure_pool("a", "1"));
    assert!(s.status().unwrap().pools.is_empty());
    permits.jobs.store(2, std::sync::atomic::Ordering::SeqCst);
    s.db.execute_batch("CREATE TRIGGER fail_allocation BEFORE INSERT ON wallets BEGIN SELECT RAISE(ABORT,'fixture allocation crash'); END;").unwrap();
    assert!(s.ensure_pool("a", "1").is_err());
    assert!(s.status().unwrap().pools.is_empty());
    // Reservations committed in the other database remain charged. Retrying the
    // same pool/sequence must reuse them despite the rolled-back random pool ID.
    assert_eq!(
        permits
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM funding_permits", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    s.db.execute_batch("DROP TRIGGER fail_allocation").unwrap();
    let pool = s.ensure_pool("a", "1").unwrap();
    let jobs = s.funding_jobs().unwrap();
    assert_eq!(jobs.len(), 2);
    let job = &jobs[0];
    denied(s.reserve(&job.operation_id, Some(&pool), 1, 101, 1000));
    assert!(s.reserve("unbound", Some(&pool), 1, 1, 1000).is_err());
    s.reserve(&job.operation_id, Some(&pool), 1, 100, 1000)
        .unwrap();
    s.save_funding_quote(&job.id, b"fixture quote").unwrap();
    s.advance_funding(
        &job.id,
        funding::FundingPhase::Quoted,
        funding::FundingPhase::Preparing,
    )
    .unwrap();
    s.prepare(&job.operation_id, 1, b"next", b"signed").unwrap();
    for job in &jobs {
        s.record_credit(&job.wallet_id, "1000000", "block", 1)
            .unwrap();
    }
    let before = serde_json::to_value(s.status().unwrap()).unwrap();
    denied(s.promote(&pool, 0));
    assert_eq!(before, serde_json::to_value(s.status().unwrap()).unwrap());
    let view = super::super::base::ChainView {
        admission_valid_until: u64::MAX,
        anchor: super::super::base::Anchor {
            height: 2,
            hash: "block2".into(),
        },
        balances: [
            (jobs[0].wallet_id.clone(), U256::ZERO),
            (jobs[1].wallet_id.clone(), U256::from(1_000_000)),
        ]
        .into(),
        released: vec![],
        resolutions: Default::default(),
    };
    denied(s.admit_for(&pool, U256::from(1), "hash", view, None));
    let status = s.status().unwrap();
    assert_eq!(status.pools[0].generation, 0);
    assert_eq!(status.pools[0].addresses.len(), 2);
    assert_eq!(status.pools[0].addresses[0].confirmed_balance, "0");
    assert_eq!(status.pools[0].addresses[0].role, "ACTIVE");
    assert_eq!(status.pools[0].addresses[1].role, "READY");
    assert_eq!(
        s.db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let id = s.id().to_owned();
    drop(s);
    let mut reopened =
        Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    reopened.install_funding_permits(permits).unwrap();
    assert_eq!(reopened.ensure_pool("a", "1").unwrap(), pool);
    assert_eq!(reopened.funding_jobs().unwrap()[0].id, job.id);
    assert_eq!(
        reopened
            .prepared_bytes(&job.operation_id)
            .unwrap()
            .as_slice(),
        b"signed"
    );
}

#[test]
fn archived_unprepared_recovery_releases_only_the_old_operation_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let permits = Arc::new(TestPermits::new(store.id()));
    store.install_funding_permits(permits.clone()).unwrap();
    let pool = store.ensure_pool("a", "1").unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    store
        .reserve(&job.operation_id, Some(&pool), 1, 100, 1000)
        .unwrap();
    store.recover_unprepared_funding(&job.id).unwrap();
    let treasury = store.id().to_owned();
    drop(store);
    // The treasury reset committed; the registry still charges the old attempt.
    assert_eq!(
        permits
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT SUM(source_bound) FROM funding_source_attempts WHERE released=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        100
    );
    let mut store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &treasury,
    )
    .unwrap();
    store.install_funding_permits(permits.clone()).unwrap();
    let new = store.funding_jobs().unwrap().remove(0);
    assert_ne!(job.operation_id, new.operation_id);
    store
        .reserve(&new.operation_id, Some(&pool), 1, 100, 1000)
        .unwrap();
    assert!(
        store
            .reserve(&job.operation_id, Some(&pool), 1, 100, 1000)
            .is_err()
    );
    assert!(
        permits
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT released FROM funding_source_attempts WHERE operation=?1",
                [&job.operation_id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    assert_eq!(
        permits
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT SUM(source_bound) FROM funding_source_attempts WHERE released=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        100
    );
}

#[cfg(unix)]
#[test]
fn qualification_snapshot_accepts_parent_alias_but_refuses_database_symlink() {
    let (dir, store, _) = fixture();
    let expected = store.qualification_state().unwrap();
    drop(store);
    let aliases = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(dir.path(), aliases.path().join("parent")).unwrap();
    assert_eq!(
        qualification_state(&aliases.path().join("parent/state")).unwrap(),
        expected
    );
    let database = dir.path().join("state/state.sqlite");
    let saved = dir.path().join("state/saved.sqlite");
    std::fs::rename(&database, &saved).unwrap();
    std::os::unix::fs::symlink(&saved, &database).unwrap();
    assert!(qualification_state(&dir.path().join("state")).is_err());
}
