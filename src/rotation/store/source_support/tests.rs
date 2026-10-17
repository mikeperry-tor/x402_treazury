use super::*;

fn fixture(count: usize) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"snapshot",
    )
    .unwrap();
    for index in 0..count {
        let id = format!("operation-{index:04}");
        store.reserve(&id, None, 0, 11).unwrap();
        let revision = store.snapshot().unwrap().0;
        store
            .prepare_with_facts(
                &id,
                revision,
                b"prepared snapshot",
                b"exact original bytes",
                Some(TransactionFacts {
                    txid: format!("{index:064x}"),
                    expiry_height: 10,
                    amount_zatoshis: 10,
                    fee_zatoshis: 1,
                    deadline: Some(100),
                }),
            )
            .unwrap();
        store.confirm_spend(&id, 11, 0).unwrap();
    }
    (dir, store)
}
fn observation(start: &SyncObservationStart, revision: i64) -> SyncObservation {
    SyncObservation {
        phase: SyncPhase::Ready,
        last_error: None,
        snapshot_revision: revision,
        checked_at: Some(start.checked_at()),
        checkpoint_at: 0,
        scanned_blocks: 1,
        target_height: Some(100),
        height: Some(100),
        observed_tip_height: Some(100),
        confirmations: 1,
        max_age_seconds: 300,
        confirmed_pool_balances_zatoshis: None,
        confirmed_shielded_zatoshis: 0,
        spendable_shielded_zatoshis: 0,
    }
}
fn stage(store: &mut Store, disposition: Disposition) -> Pass {
    stage_return(store, disposition, None)
}
fn stage_return(store: &mut Store, disposition: Disposition, returned: Option<U256>) -> Pass {
    let pass = store
        .begin_source_support(
            store.snapshot().unwrap().0,
            Anchor {
                height: 100,
                hash: [1; 32],
            },
        )
        .unwrap();
    let mut after = String::new();
    loop {
        let page = store.source_recheck_page(&after, pass.boundary).unwrap();
        assert!(page.len() <= 64);
        let Some(last) = page.last() else { break };
        after = last.operation_id.clone();
        for operation in page {
            let raw = store.prepared_bytes(&operation.operation_id).unwrap();
            store
                .stage_source_support(&pass, &operation, &raw, disposition, false)
                .unwrap();
            if let Some(amount) = returned {
                store
                    .stage_return_support(&pass, &operation.operation_id, amount, false)
                    .unwrap();
            }
        }
    }
    store.seal_source_support(&pass).unwrap();
    pass
}
fn activate_pass(store: &mut Store, pass: &Pass) {
    let start = SyncObservationStart::capture(300).unwrap();
    store
        .save_observed_source_snapshot(
            pass.base_revision,
            b"scanned snapshot",
            observation(&start, pass.base_revision),
            start,
            Some(pass),
        )
        .unwrap();
}
#[test]
fn complete_sync_replaces_support_and_preserves_exact_debit_bytes_across_reopen() {
    let (dir, mut store) = fixture(1);
    let id = "operation-0000";
    let pass = stage(&mut store, Disposition::Confirmed(5));
    assert!(
        !confirmed(&store.db, id).unwrap(),
        "inactive prefix is not evidence"
    );
    activate_pass(&mut store, &pass);
    assert!(confirmed(&store.db, id).unwrap());
    let pass = stage(&mut store, Disposition::Unavailable);
    assert!(
        !confirmed(&store.db, id).unwrap(),
        "observed loss revokes before final publication"
    );
    activate_pass(&mut store, &pass);
    assert!(!confirmed(&store.db, id).unwrap());
    let pass = stage(&mut store, Disposition::Confirmed(7));
    activate_pass(&mut store, &pass);
    assert!(confirmed(&store.db, id).unwrap());
    assert_eq!(
        store.prepared_bytes(id).unwrap().as_slice(),
        b"exact original bytes"
    );
    let amounts: (i64, i64) = store
        .db
        .query_row(
            "SELECT consumed,reserved FROM budget_entries WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(amounts, (11, 0));
    let identity = store.id.clone();
    drop(store);
    let store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &identity,
    )
    .unwrap();
    assert!(
        confirmed(&store.db, id).unwrap(),
        "historical canonical support remains readable after reopen"
    );
}
#[test]
fn changed_inventory_stale_snapshot_and_expired_observation_cannot_activate() {
    let (_dir, mut store) = fixture(1);
    let pass = stage(&mut store, Disposition::Confirmed(5));
    store
        .db
        .execute(
            "UPDATE treasury_operations SET facts=json_set(facts,'$.fee_zatoshis',2)",
            [],
        )
        .unwrap();
    let start = SyncObservationStart::capture(300).unwrap();
    assert!(
        store
            .save_observed_source_snapshot(
                pass.base_revision,
                b"stale",
                observation(&start, pass.base_revision),
                start,
                Some(&pass)
            )
            .is_err()
    );
    assert_eq!(store.snapshot().unwrap().0, pass.base_revision);
    let pass = stage(&mut store, Disposition::Confirmed(5));
    store
        .save_snapshot(pass.base_revision, b"other accepted work")
        .unwrap();
    let start = SyncObservationStart::capture(300).unwrap();
    assert!(
        store
            .save_observed_source_snapshot(
                pass.base_revision,
                b"stale",
                observation(&start, pass.base_revision),
                start,
                Some(&pass)
            )
            .is_err()
    );
    let pass = stage(&mut store, Disposition::Confirmed(5));
    let start = SyncObservationStart::capture(300).unwrap();
    start.expire_for_test();
    store
        .save_observed_source_snapshot(
            pass.base_revision,
            b"useful stale scan",
            observation(&start, pass.base_revision),
            start,
            Some(&pass),
        )
        .unwrap();
    assert!(!confirmed(&store.db, "operation-0000").unwrap());
}
#[test]
fn pages_and_cleanup_are_bounded_and_incomplete_pass_cannot_publish() {
    let (_dir, mut store) = fixture(130);
    let pass = store
        .begin_source_support(
            store.snapshot().unwrap().0,
            Anchor {
                height: 100,
                hash: [1; 32],
            },
        )
        .unwrap();
    let page = store.source_recheck_page("", pass.boundary).unwrap();
    assert_eq!(page.len(), 64);
    let operation = &page[0];
    let raw = store.prepared_bytes(&operation.operation_id).unwrap();
    store
        .stage_source_support(&pass, operation, &raw, Disposition::Confirmed(5), false)
        .unwrap();
    assert!(store.seal_source_support(&pass).is_err());
    let start = SyncObservationStart::capture(300).unwrap();
    assert!(
        store
            .save_observed_source_snapshot(
                pass.base_revision,
                b"partial",
                observation(&start, pass.base_revision),
                start,
                Some(&pass)
            )
            .is_err()
    );
    cleanup(&mut store);
    let complete = stage(&mut store, Disposition::Confirmed(5));
    let mut calls = 0;
    let mut cursor = CleanupCursor::default();
    loop {
        let before: i64 = store
            .db
            .query_row("SELECT count(*) FROM source_support_rows", [], |r| r.get(0))
            .unwrap();
        let next = store.cleanup_source_support_page(cursor).unwrap();
        let after: i64 = store
            .db
            .query_row("SELECT count(*) FROM source_support_rows", [], |r| r.get(0))
            .unwrap();
        assert!(before - after <= 64);
        calls += 1;
        let Some(next) = next else { break };
        cursor = next;
    }
    assert!(calls >= 4);
    let start = SyncObservationStart::capture(300).unwrap();
    assert!(
        store
            .save_observed_source_snapshot(
                complete.base_revision,
                b"cancelled",
                observation(&start, complete.base_revision),
                start,
                Some(&complete)
            )
            .is_err()
    );
}
#[test]
fn current_contradiction_and_new_scan_never_fall_back_to_old_support() {
    let (_dir, mut store) = fixture(1);
    let pass = stage(&mut store, Disposition::Confirmed(5));
    activate_pass(&mut store, &pass);
    let _interrupted = stage(&mut store, Disposition::Conflict);
    assert!(!confirmed(&store.db, "operation-0000").unwrap());
    let pass = stage(&mut store, Disposition::Confirmed(6));
    activate_pass(&mut store, &pass);
    let revision = store.snapshot().unwrap().0;
    let start = SyncObservationStart::capture(300).unwrap();
    store
        .save_observed_sync_snapshot(
            revision,
            b"new scan without source verification",
            observation(&start, revision),
            start,
        )
        .unwrap();
    assert!(!confirmed(&store.db, "operation-0000").unwrap());
}

/// Store-only fixtures explicitly publish the canonical evidence their test owns.
pub(crate) fn publish_confirmed(store: &mut Store) {
    let pass = stage(store, Disposition::Confirmed(5));
    activate_pass(store, &pass);
}

pub(crate) fn publish_confirmed_returns(store: &mut Store, returned: U256) {
    let pass = stage_return(store, Disposition::Confirmed(5), Some(returned));
    activate_pass(store, &pass);
}

pub(crate) fn publish_unavailable(store: &mut Store) {
    let pass = stage(store, Disposition::Unavailable);
    activate_pass(store, &pass);
}

#[test]
fn sealed_history_continues_after_stale_checkpoint_without_restaging() {
    let (_dir, mut store) = fixture(130);
    let pass = stage(&mut store, Disposition::Confirmed(5));
    let start = SyncObservationStart::capture(300).unwrap();
    start.expire_for_test();
    store
        .save_observed_source_snapshot(
            pass.base_revision,
            b"useful stale scan",
            observation(&start, pass.base_revision),
            start,
            Some(&pass),
        )
        .unwrap();
    assert!(!confirmed(&store.db, "operation-0000").unwrap());
    let revision = store.snapshot().unwrap().0;
    let continued = store
        .continue_source_support(&pass, revision)
        .unwrap()
        .unwrap();
    let start = SyncObservationStart::capture(300).unwrap();
    store
        .save_observed_source_snapshot(
            revision,
            b"fresh incremental scan",
            observation(&start, revision),
            start,
            Some(&continued),
        )
        .unwrap();
    assert!(confirmed(&store.db, "operation-0000").unwrap());
    let rows: i64 = store
        .db
        .query_row("SELECT COUNT(*) FROM source_support_rows", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        rows, 130,
        "historical pages are reused without copying or restaging"
    );
    let original: i64 = store
        .db
        .query_row(
            "SELECT base_revision FROM source_support_passes WHERE id=?1",
            [&pass.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, pass.base_revision);
    let mut altered_anchor = pass.clone();
    altered_anchor.anchor.hash = [2; 32];
    assert!(
        store
            .continue_source_support(&altered_anchor, store.snapshot().unwrap().0)
            .unwrap()
            .is_none()
    );
    store
        .db
        .execute(
            "UPDATE treasury_operations SET attempts=attempts+1 WHERE id='operation-0000'",
            [],
        )
        .unwrap();
    assert!(
        store
            .continue_source_support(&pass, store.snapshot().unwrap().0)
            .unwrap()
            .is_none()
    );
}

pub(crate) fn cleanup(store: &mut Store) {
    let mut cursor = CleanupCursor::default();
    while let Some(next) = store.cleanup_source_support_page(cursor).unwrap() {
        cursor = next;
    }
}

#[test]
fn return_inventory_change_reobserves_once_without_repeating_unresolved_pass() {
    let (_dir, mut store) = fixture(1);
    let id = "operation-0000";
    let pass = stage_return(&mut store, Disposition::Confirmed(5), Some(U256::ZERO));
    activate_pass(&mut store, &pass);
    assert!(store.source_observation_current(id).unwrap());
    store
        .db
        .execute(
            "INSERT INTO refund_outputs VALUES('return',0,?1,1,5,1)",
            [id],
        )
        .unwrap();
    assert!(!store.source_observation_current(id).unwrap());
    assert!(
        store.source_confirmation_supported(id).unwrap(),
        "return inventory does not erase canonical gross source accounting"
    );
    assert!(returned(&store.db, id).unwrap().is_none());
    let pass = stage(&mut store, Disposition::Confirmed(5));
    activate_pass(&mut store, &pass);
    for _ in 0..3 {
        assert!(
            store.source_observation_current(id).unwrap(),
            "observed unresolved return must reuse the same pass"
        );
        assert!(returned(&store.db, id).unwrap().is_none());
        store
            .db
            .execute(
                "UPDATE refund_outputs SET amount=amount WHERE operation_id=?1",
                [id],
            )
            .unwrap();
    }
    store
        .db
        .execute(
            "UPDATE refund_outputs SET amount=2 WHERE operation_id=?1",
            [id],
        )
        .unwrap();
    assert!(!store.source_observation_current(id).unwrap());
}
