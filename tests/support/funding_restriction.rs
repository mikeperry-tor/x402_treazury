//! The restriction is enforced at store boundaries, independently of worker settings.
use super::*;
use crate::test_funding_quote as quote_fixture;
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
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        s,
        "fixture",
        crate::rotation::test_capacity::two_reserves("5"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
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
fn restriction_allows_declaration_but_denies_allocation_and_source_work() {
    if crate::test_log_capture::run_in_child(
        "rotation::store::funding_restriction_tests::restriction_allows_declaration_but_denies_allocation_and_source_work",
    ) {
        return;
    }
    let (_dir, mut s, pool) = fixture();
    for wallet in s.status().unwrap().pools[0].addresses.iter() {
        s.record_credit(&wallet.id, "5000000", "block", 1).unwrap();
    }
    // A reservation inherited from an older process must not permit new calculation.
    s.accept_preparation("old", Some(&pool), 100, None).unwrap();
    let before_jobs = s.funding_jobs().unwrap().len();
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
        assert_eq!(
            s.ensure_capacity_group(
                "fixture",
                &crate::rotation::test_capacity::two_reserves("5"),
                Some(U256::from(1_000_000)),
                0
            )
            .unwrap(),
            pool
        );
        let declared = s
            .ensure_capacity_group(
                "new_pool",
                &crate::rotation::test_capacity::two_reserves("5"),
                Some(U256::from(1_000_000)),
                0,
            )
            .unwrap();
        denied(s.allocate_capacity_kind(&declared, AllocationKind::Small, true));
        denied(s.accept_preparation("new", Some(&pool), 100, None));
        denied(s.accept_preparation("shield", None, 100, None));
        denied(s.require_preparation_available());
        s.prepare("old", 1, b"next", b"new bytes").unwrap();
    });
    assert_eq!(before_jobs, s.funding_jobs().unwrap().len());
    assert_eq!(
        s.db.query_row(
            "SELECT COUNT(*) FROM operation_network WHERE preparation IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        s.db.query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for stage in [
        "capacity_allocation",
        "source_preparation_start",
        "treasury_preparation",
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
    s.accept_preparation("accepted", Some(&pool), 100, None)
        .unwrap();
    let facts = TransactionFacts {
        txid: "existing".into(),
        expiry_height: 100,
        amount_zatoshis: 80,
        fee_zatoshis: 20,
        deadline: Some(1000),
    };
    s.prepare_with_facts("accepted", 1, b"saved", b"saved bytes", Some(facts.clone()))
        .unwrap();
    drop(s);
    let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    s.deny_new_funding(); // Supervisor must do this on EVERY launch.
    s.accept_preparation("accepted", Some(&pool), 100, None)
        .unwrap(); // Idempotent, no increase.
    assert!(
        s.accept_preparation("accepted", Some(&pool), 101, None)
            .is_err()
    );
    assert_eq!(
        s.prepare_with_facts("accepted", 1, b"saved", b"saved bytes", Some(facts))
            .unwrap(),
        2
    );
    assert_eq!(
        s.prepared_bytes("accepted").unwrap().as_slice(),
        b"saved bytes"
    );
    // Prepared bytes remain observable, but no first submission is authorized.
    denied(s.request_broadcast("accepted", 1, 1, false));
    assert!(s.request_broadcast("accepted", 1, 1, false).is_err());
    s.confirm_spend("accepted").unwrap();
    denied(s.accept_preparation("replacement", Some(&pool), 100, None));
    assert_eq!(s.snapshot().unwrap().0, 2);
}

#[test]
fn restriction_does_not_poison_queued_preparation_as_interrupted_work() {
    let (_dir, mut s, _pool) = fixture();
    let job = s.funding_jobs().unwrap().remove(0);
    s.save_funding_quote(
        &job.id,
        &quote_fixture::bound(&job.target, &job.recipient, "saved quote"),
    )
    .unwrap();
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
fn permit_hooks_bound_each_allocation_and_preparation_before_mutation() {
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
    denied(crate::rotation::test_capacity::declare_and_allocate!(
        s,
        "a",
        crate::rotation::test_capacity::two_reserves("1"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    ));
    assert_eq!(s.status().unwrap().pools.len(), 1);
    assert_eq!(s.funding_jobs().unwrap().len(), 1);
    permits.jobs.store(2, std::sync::atomic::Ordering::SeqCst);
    s.db.execute_batch("CREATE TRIGGER fail_allocation BEFORE INSERT ON wallets BEGIN SELECT RAISE(ABORT,'fixture allocation crash'); END;").unwrap();
    assert!(
        crate::rotation::test_capacity::declare_and_allocate!(
            s,
            "a",
            crate::rotation::test_capacity::two_reserves("1"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .is_err()
    );
    assert_eq!(s.status().unwrap().pools.len(), 1);
    assert_eq!(s.funding_jobs().unwrap().len(), 1);
    // The first allocation remains committed. The failed second allocation
    // retains its external permit and reuses it at the same stable sequence.
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
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        s,
        "a",
        crate::rotation::test_capacity::two_reserves("1"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let jobs = s.funding_jobs().unwrap();
    assert_eq!(jobs.len(), 2);
    let job = &jobs[0];
    denied(s.accept_preparation(&job.operation_id, Some(&pool), 101, None));
    assert!(
        s.accept_preparation("unbound", Some(&pool), 1, None)
            .is_err()
    );
    s.save_funding_quote(
        &job.id,
        &quote_fixture::bound(&job.target, &job.recipient, "fixture quote"),
    )
    .unwrap();
    s.accept_preparation(&job.operation_id, Some(&pool), 100, None)
        .unwrap();
    assert_eq!(
        s.funding_job(&job.id).unwrap().unwrap().phase,
        funding::FundingPhase::Preparing
    );
    s.prepare(&job.operation_id, 1, b"next", b"signed").unwrap();
    for job in &jobs {
        s.record_credit(&job.wallet_id, "1000000", "block", 1)
            .unwrap();
    }
    // Consuming existing inventory is not allocation authority. A full slot is
    // idempotent under the permit ceiling and does not request a third permit.
    assert!(
        s.allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap()
            .is_none()
    );
    assert_eq!(s.funding_jobs().unwrap().len(), 2);
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
    assert_eq!(
        crate::rotation::test_capacity::declare_and_allocate!(
            reopened,
            "a",
            crate::rotation::test_capacity::two_reserves("1"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap(),
        pool
    );
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
fn uncertain_preparation_keeps_the_same_operation_and_permit_after_restart() {
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
    let pool = crate::rotation::test_capacity::declare_and_allocate!(
        store,
        "a",
        crate::rotation::test_capacity::two_reserves("1"),
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0
    )
    .unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    store
        .accept_preparation(&job.operation_id, Some(&pool), 100, None)
        .unwrap();
    assert!(store.recover_unprepared_funding(&job.id).is_err());
    let treasury = store.id().to_owned();
    drop(store);
    // No result proves whether proposal work started; the permit stays charged.
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
    let same = store.funding_jobs().unwrap().remove(0);
    assert_eq!(job.operation_id, same.operation_id);
    assert!(store.recover_unprepared_funding(&job.id).is_err());
    assert!(
        !permits
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

#[test]
fn jobless_shielding_retains_bytes_but_job_permits_cannot_authorize_its_first_submission() {
    use crate::rotation::store::preparation::{CalculationInputs, InputPool, OwnedInput};
    use crate::rotation::transaction::PreparedTransaction;
    let (_restricted_dir, mut restricted, _) = fixture();
    restricted
        .install_funding_permits(Arc::new(TestPermits::new(restricted.id())))
        .unwrap();
    assert!(restricted.require_preparation_available().is_ok());
    assert!(!restricted.automatic_shielding_available().unwrap());
    assert!(
        restricted
            .accept_preparation("unauthorized shield", None, 100, None)
            .is_err()
    );
    assert!(
        restricted
            .require_unstarted_preparation("unauthorized shield")
            .is_ok()
    );
    let (_dir, mut store, _) = fixture();
    let address = "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F";
    let facts = TransactionFacts {
        txid: "shield fixture".into(),
        expiry_height: 100,
        amount_zatoshis: 0,
        fee_zatoshis: 100,
        deadline: None,
    };
    let manifest = CalculationInputs {
        destination: address.into(),
        inputs: vec![OwnedInput {
            pool: InputPool::Transparent,
            txid: "original input".into(),
            output_index: 0,
        }],
    };
    assert!(store.automatic_shielding_available().unwrap());
    store
        .accept_preparation("shield", None, 100, Some(manifest))
        .unwrap();
    assert!(store.automatic_shielding_available().unwrap());
    store
        .prepare_with_facts("shield", 1, b"next", b"exact shield", Some(facts))
        .unwrap();
    assert!(store.automatic_shielding_submission("shield", 100).unwrap());
    assert!(!store.automatic_shielding_submission("shield", 99).unwrap());
    let identity = PreparedTransaction::load(&store, "shield")
        .unwrap()
        .network_identity()
        .unwrap();
    assert_eq!(
        identity,
        crate::network::IsolationId::transparent_treasury(store.network(), store.id(), address)
            .unwrap()
    );
    assert_ne!(
        identity,
        crate::network::IsolationId::transparent_treasury(
            store.network(),
            "another treasury",
            address
        )
        .unwrap()
    );
    store
        .install_funding_permits(Arc::new(TestPermits::new(store.id())))
        .unwrap();
    assert!(!store.automatic_shielding_submission("shield", 100).unwrap());
    denied(store.request_broadcast("shield", 1, 1, false));
    assert_eq!(store.operation("shield").unwrap().attempts, 0);
    let retained = PreparedTransaction::load(&store, "shield").unwrap();
    assert_eq!(retained.bytes(), b"exact shield");
    assert_eq!(retained.network_identity().unwrap(), identity);
    assert!(
        store
            .accept_preparation("duplicate", None, 1, None)
            .is_err()
    );
}
