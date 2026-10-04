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
