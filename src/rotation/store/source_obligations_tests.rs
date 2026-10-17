use super::source_support::{Anchor, Disposition};
use super::*;

fn fixture() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"snapshot",
    )
    .unwrap();
    store.reserve("original", None, 0, 11).unwrap();
    store
        .prepare_with_facts(
            "original",
            1,
            b"prepared",
            b"signed",
            Some(TransactionFacts {
                txid: "original".into(),
                expiry_height: 20,
                amount_zatoshis: 10,
                fee_zatoshis: 1,
                deadline: Some(100),
            }),
        )
        .unwrap();
    store.confirm_spend("original", 11, 0).unwrap();
    (dir, store)
}
fn stage(store: &mut Store, disposition: Disposition, live: bool) -> source_support::Pass {
    let pass = store
        .begin_source_support(
            store.snapshot().unwrap().0,
            Anchor {
                height: 100,
                hash: [1; 32],
            },
        )
        .unwrap();
    let operation = store.operation("original").unwrap();
    let raw = store.prepared_bytes("original").unwrap();
    store
        .stage_source_support(&pass, &operation, &raw, disposition, live)
        .unwrap();
    store.seal_source_support(&pass).unwrap();
    pass
}
fn publish(store: &mut Store, pass: &source_support::Pass) -> (i64, u64) {
    let revision = store.snapshot().unwrap().0;
    let start = SyncObservationStart::capture(300).unwrap();
    let now = start.checked_at();
    let observation = SyncObservation {
        phase: SyncPhase::Ready,
        last_error: None,
        snapshot_revision: revision,
        checked_at: Some(now),
        checkpoint_at: now,
        scanned_blocks: 1,
        target_height: Some(100),
        observed_tip_height: Some(100),
        height: Some(100),
        confirmations: 1,
        max_age_seconds: 300,
        confirmed_pool_balances_zatoshis: None,
        confirmed_shielded_zatoshis: 100,
        spendable_shielded_zatoshis: 100,
    };
    (
        store
            .save_observed_source_snapshot(revision, b"current", observation, start, Some(pass))
            .unwrap(),
        now,
    )
}
#[test]
fn retained_source_ownership_survives_burial_reopen_and_inactive_reconfirmation() {
    let (dir, mut store) = fixture();
    let pass = stage(&mut store, Disposition::Unavailable, true);
    assert!(source_obligations::disputed(&store.db, "original").unwrap());
    publish(&mut store, &pass);
    assert!(store.require_source_inputs_available().is_err());
    let pass = stage(&mut store, Disposition::Unavailable, false);
    publish(&mut store, &pass);
    source_support::tests::cleanup(&mut store);
    let identity = store.id.clone();
    drop(store);
    let mut store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &identity,
    )
    .unwrap();
    assert!(source_obligations::disputed(&store.db, "original").unwrap());
    let pass = stage(&mut store, Disposition::Confirmed(5), false);
    assert!(
        store.require_source_inputs_available().is_err(),
        "staged confirmation is not publication"
    );
    publish(&mut store, &pass);
    assert!(!source_obligations::disputed(&store.db, "original").unwrap());
    store.require_source_inputs_available().unwrap();
    source_support::tests::cleanup(&mut store);
    assert_eq!(
        store.prepared_bytes("original").unwrap().as_slice(),
        b"signed"
    );
}
#[test]
fn buried_archival_absence_is_not_a_new_input_dispute() {
    let (_dir, mut store) = fixture();
    let pass = stage(&mut store, Disposition::Unavailable, false);
    publish(&mut store, &pass);
    store.require_source_inputs_available().unwrap();
    let _pass = stage(&mut store, Disposition::Conflict, false);
    assert!(store.require_source_inputs_available().is_err());
}
#[test]
fn exact_expiry_correction_preserves_debit_facts_and_rejects_old_proof_after_contradiction() {
    let (_dir, mut store) = fixture();
    let pass = stage(&mut store, Disposition::Conflict, true);
    let (revision, now) = publish(&mut store, &pass);
    assert!(
        store
            .resolve_expired("original", revision - 1, now)
            .is_err()
    );
    store.resolve_expired("original", revision, now).unwrap();
    let amounts:(i64,i64,i64,String)=store.db.query_row("SELECT b.reserved,b.consumed,e.reversed_consumed,o.submission FROM budget_entries b JOIN expired_operations e ON e.id=b.id JOIN treasury_operations o ON o.id=b.id WHERE b.id='original'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(amounts, (0, 0, 11, "CONFIRMED".into()));
    assert!(funding::source_attempt_closed(&store.db, "original").unwrap());
    assert!(store.confirm_spend("original", 11, 0).is_err());
    let pass = stage(&mut store, Disposition::Conflict, false);
    publish(&mut store, &pass);
    assert!(!funding::source_attempt_closed(&store.db, "original").unwrap());
    assert!(
        store.resolve_expired("original", revision, now).is_err(),
        "old proof cannot release a new dispute"
    );
    let pass = stage(&mut store, Disposition::Confirmed(5), false);
    let (revision, now) = publish(&mut store, &pass);
    assert!(
        source_obligations::disputed(&store.db, "original").unwrap(),
        "contradicted expiry requires exact recovery, not a confirmation clear"
    );
    store.resolve_expired("original", revision, now).unwrap();
    assert!(funding::source_attempt_closed(&store.db, "original").unwrap());
    assert_eq!(
        store
            .operation_facts("original")
            .unwrap()
            .unwrap()
            .amount_zatoshis,
        10
    );
}

#[test]
fn zero_expiry_never_releases_a_confirmed_dispute() {
    let (_dir, mut store) = fixture();
    store.db.execute("UPDATE treasury_operations SET facts=json_set(facts,'$.expiry_height',0) WHERE id='original'",[]).unwrap();
    let pass = stage(&mut store, Disposition::Conflict, true);
    let (revision, now) = publish(&mut store, &pass);
    let error = store
        .resolve_expired("original", revision, now)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<crate::rotation::transaction::RecoveryEvidenceError>(),
        Some(&crate::rotation::transaction::RecoveryEvidenceError::ExpiryNotBuried)
    );
    assert!(source_obligations::disputed(&store.db, "original").unwrap());
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT consumed FROM budget_entries WHERE id='original'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        11
    );
}

#[test]
fn cleanup_advances_over_physical_ineligible_marker_pages() {
    let (_dir, mut store) = fixture();
    let pass = stage(&mut store, Disposition::Confirmed(5), false);
    publish(&mut store, &pass);
    // Housekeeping-only rows: no admission, decoding or broadcast uses these bytes.
    for index in 0..130 {
        let id = format!("marker-{index:03}");
        store.db.execute("INSERT INTO budget_entries SELECT ?1,pool_id,day,original_day,requested,reserved,consumed FROM budget_entries WHERE id='original'",[&id]).unwrap();
        store.db.execute("INSERT INTO outgoing SELECT ?1,state,raw,revision FROM outgoing WHERE id='original'",[&id]).unwrap();
        store.db.execute("INSERT INTO treasury_operations SELECT ?1,facts,submission,attempts FROM treasury_operations WHERE id='original'",[&id]).unwrap();
        store.db.execute("INSERT INTO source_obligations(operation_id,clear_pass_id) VALUES(?1,CASE WHEN ?2 THEN (SELECT pass_id FROM source_support_current WHERE singleton=1) ELSE NULL END)",params![id,index==129]).unwrap();
    }
    let mut cursor = source_support::CleanupCursor::default();
    for expected in ["marker-063", "marker-127", "marker-129"] {
        cursor = store.cleanup_source_support_page(cursor).unwrap().unwrap();
        match &cursor {
            source_support::CleanupCursor::Markers(after) => assert_eq!(after, expected),
            _ => panic!("marker page skipped physical prefix"),
        }
    }
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM source_obligations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        129
    );
}
