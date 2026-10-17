use super::*;
use crate::rotation::transaction::{
    BroadcastTransaction, SubmissionOutcome, TransactionPresence, TransactionSubmission,
};
use std::sync::{Arc, Mutex};

struct NoNetwork;
impl TransactionSubmission for NoNetwork {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("observation cannot submit")
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("injected observations replace network")
    }
}
fn outage() -> anyhow::Error {
    crate::treasury::diagnostics::tip_failure(
        tonic::Status::unavailable("fixture"),
        "fixture_observation",
    )
}
#[tokio::test]
async fn pending_outage_preserves_independent_history_and_prefix_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mut owner=Treasury::create(state.clone(),dir.path().join("key"),3_400_000,Some(Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
    let (pool, wallet, revision) = owner
        .store
        .call(|s| {
            let pool = s.ensure_capacity_group(
                "observation",
                &crate::rotation::store::test_support::reserves("1"),
                None,
                0,
            )?;
            s.allocate_capacity_kind(&pool, crate::rotation::store::AllocationKind::Small, true)?;
            Ok((pool, s.funding_jobs()?.remove(0).wallet_id, s.snapshot()?.0))
        })
        .await
        .unwrap();
    let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
    // Inventory-only rows: the injected observer is the read-only fault boundary,
    // so no synthetic bytes or header may become financial authority in this test.
    for id in ["a", "b", "z"] {
        db.execute("INSERT INTO budget_entries VALUES(?1,NULL,0,0,1,0,0)", [id])
            .unwrap();
        db.execute(
            "INSERT INTO outgoing VALUES(?1,?2,X'00',?3)",
            rusqlite::params![
                id,
                if id == "z" { "PREPARED" } else { "RESOLVED" },
                revision
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO shielding_operations VALUES(?1,?2,?3,X'00',1,0)",
            rusqlite::params![id, pool, wallet],
        )
        .unwrap();
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let mut maintenance = ShieldingMaintenance::default();
    maintenance.observation_override = Some(Box::new(move |id| {
        captured.lock().unwrap().push(id.to_owned());
        if id == "z" { Err(outage()) } else { Ok(false) }
    }));
    owner
        .maintain_shielding(
            &mut maintenance,
            None,
            &mut NoNetwork,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), ["z", "a", "b"]);
    assert_eq!(maintenance.operation, "z");
    // A later fatal error in the first prefix row must not inherit the priority
    // slot's out-of-order key, otherwise that unprocessed prefix is skipped.
    maintenance.operation.clear();
    maintenance.observation_override = Some(Box::new(|id| {
        if id == "a" {
            Err(rusqlite::Error::InvalidQuery.into())
        } else {
            Ok(false)
        }
    }));
    assert!(
        owner
            .maintain_shielding(
                &mut maintenance,
                None,
                &mut NoNetwork,
                &CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert!(maintenance.operation.is_empty());
    // Retired SDK capability and arbitrary ownership failures remain fatal.
    assert!(scoped_observation(Err(outage()), false, &mut None).is_err());
    assert!(scoped_observation(Err(anyhow::anyhow!("owner missing")), true, &mut None).is_err());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    owner.close().await.unwrap();
}
