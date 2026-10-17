use super::*;

fn fixture() -> (tempfile::TempDir, Store, String, Policy) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"wallet",
    )
    .unwrap();
    let pool = store
        .ensure_capacity_group("shield", &test_support::reserves("1"), None, 0)
        .unwrap();
    store
        .allocate_capacity_kind(&pool, AllocationKind::Small, true)
        .unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    let revision = store.snapshot().unwrap().0;
    store
        .save_refund_address(&job.id, "test-address", revision, b"wallet with address")
        .unwrap();
    let policy = Policy {
        pool,
        wallet: job.wallet_id,
        address: "test-address".into(),
        max_fee: 10,
        automatic: false,
    };
    (dir, store, job.id, policy)
}
fn input() -> Input {
    Input {
        txid: "ab".repeat(32),
        output_index: 0,
        amount: 100,
        script: vec![1, 2],
    }
}
fn facts(fee: u64) -> TransactionFacts {
    TransactionFacts {
        txid: "cd".repeat(32),
        expiry_height: 100,
        amount_zatoshis: 0,
        fee_zatoshis: fee,
        deadline: Some(1000),
    }
}
fn outgoing(store: &mut Store, operation: &str) {
    store.reserve(operation, None, 0, 10).unwrap();
    let revision = store.snapshot().unwrap().0;
    store
        .prepare_with_facts(
            operation,
            revision,
            b"wallet after calculate",
            b"test raw",
            Some(facts(10)),
        )
        .unwrap();
}
#[test]
fn canonical_outpoints_exact_fee_and_confirmed_claims_are_enforced() {
    let (_dir, mut store, _job, policy) = fixture();
    let mut alias = input();
    alias.txid = format!("0x{}", alias.txid.to_uppercase());
    assert!(Binding::new(policy.clone(), vec![input(), alias.clone()], 10).is_err());
    let binding = Binding::new(policy.clone(), vec![input()], 10).unwrap();
    assert!(binding.check(&store.db, "first", &facts(9)).is_err());
    binding.check(&store.db, "first", &facts(10)).unwrap();
    outgoing(&mut store, "first");
    binding
        .insert(
            &store.db,
            &store.key,
            &store.id,
            store.network.name(),
            "first",
        )
        .unwrap();
    binding
        .check_saved(
            &store.db,
            &store.key,
            &store.id,
            store.network.name(),
            "first",
        )
        .unwrap();
    let alias = Binding::new(policy, vec![alias], 10).unwrap();
    assert!(alias.check_claims(&store.db, "next").is_err());
    store
        .db
        .execute("UPDATE outgoing SET state='RESOLVED' WHERE id='first'", [])
        .unwrap();
    assert!(
        alias.check_claims(&store.db, "next").is_err(),
        "confirmation must retain reorg claim"
    );
    let revision = store.snapshot().unwrap().0;
    store
        .db
        .execute(
            "INSERT INTO expired_operations(id,height,revision) VALUES('first',100,?1)",
            [revision],
        )
        .unwrap();
    alias.check_claims(&store.db, "next").unwrap();
}
#[test]
fn immutable_provenance_is_transactional_and_survives_reopen() {
    let (dir, mut store, _job, policy) = fixture();
    let binding = Binding::new(policy, vec![input()], 10).unwrap();
    outgoing(&mut store, "first");
    {
        let tx = store.db.transaction().unwrap();
        binding
            .insert(&tx, &store.key, &store.id, store.network.name(), "first")
            .unwrap();
        // Model any later failure of the shared outgoing/snapshot commit.
    }
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM shielding_inputs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    binding
        .insert(
            &store.db,
            &store.key,
            &store.id,
            store.network.name(),
            "first",
        )
        .unwrap();
    assert!(
        store
            .db
            .execute("UPDATE shielding_inputs SET amount=101", [])
            .is_err()
    );
    assert!(
        store
            .db
            .execute("UPDATE shielding_operations SET automatic=1", [])
            .is_err()
    );
    assert!(
        store
            .db
            .execute("DELETE FROM shielding_inputs", [])
            .is_err()
    );
    assert!(
        store
            .db
            .execute("DELETE FROM shielding_operations", [])
            .is_err()
    );
    let id = store.id.clone();
    drop(store);
    let reopened = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    binding
        .check_saved(
            &reopened.db,
            &reopened.key,
            &reopened.id,
            reopened.network.name(),
            "first",
        )
        .unwrap();
}
#[test]
fn invalid_raw_bytes_cannot_publish_or_release_owned_preparation() {
    let (_dir, mut store, job, _) = fixture();
    let revision = store.snapshot().unwrap().0;
    let (owner, policy) = store
        .begin_manual_shielding("shield", revision, &job, "test-address", 10)
        .unwrap();
    let binding = Binding::new(policy, vec![input()], 10).unwrap();
    store.reserve("shield", None, 0, 10).unwrap();
    assert!(
        store
            .prepare_owned_shielding(
                &owner,
                b"mutated wallet",
                b"invalid bytes",
                facts(10),
                &binding
            )
            .is_err()
    );
    assert_eq!(store.snapshot().unwrap().0, revision);
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    store.require_preparation_owner(&owner).unwrap();
    assert!(store.close_retired_preparation(&owner).unwrap());
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

struct PositivePermits;
impl crate::rotation::restriction::FundingPermits for PositivePermits {
    fn validate_treasury(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn validate_state(&self, _: &serde_json::Value) -> Result<()> {
        Ok(())
    }
    fn reserve_allocations(&self, _: &str, _: &str, _: &[i64]) -> Result<Vec<String>> {
        panic!("shielding must not allocate")
    }
    fn check_preparation(&self, _: &str, _: &str, _: u64) -> Result<()> {
        Ok(())
    }
    fn reserve_preparation(&self, _: &str, _: &str, _: &str, _: u64) -> Result<()> {
        panic!("shielding must not reserve qualification authority")
    }
    fn retire_unprepared(&self, _: &str, _: &str, _: &str) -> Result<()> {
        panic!("shielding must not retire qualification authority")
    }
}
#[test]
fn positive_qualification_permit_does_not_authorize_shielding_mutation() {
    let (_dir, mut store, job, _) = fixture();
    store
        .install_funding_permits(std::sync::Arc::new(PositivePermits))
        .unwrap();
    let revision = store.snapshot().unwrap().0;
    let error = store
        .begin_manual_shielding("shield", revision, &job, "test-address", 10)
        .err()
        .unwrap();
    assert!(matches!(
        error.downcast_ref::<AdmissionError>(),
        Some(AdmissionError::FundingPermitDenied)
    ));
    assert_eq!(store.snapshot().unwrap().0, revision);
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM treasury_preparations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn absent_delivery_deadline_never_grants_source_submission_authority() {
    let (_dir, mut store, _job, _) = fixture();
    let mut operation_facts = facts(10);
    operation_facts.deadline = None;
    store.reserve("source", None, 0, 10).unwrap();
    let revision = store.snapshot().unwrap().0;
    assert!(
        store
            .prepare_with_facts(
                "source",
                revision,
                b"next",
                b"raw",
                Some(operation_facts.clone())
            )
            .is_err()
    );
    assert!(!store.operation_pending("source").unwrap());
    operation_facts.deadline = Some(400);
    store
        .prepare_with_facts(
            "source",
            revision,
            b"next",
            b"raw",
            Some(operation_facts.clone()),
        )
        .unwrap();
    assert!(store.request_broadcast("source", 101, 99, false).is_err());
    assert!(store.request_broadcast("source", 100, 100, false).is_err());
    assert_eq!(store.operation("source").unwrap().attempts, 0);
    operation_facts.deadline = None;
    store
        .db
        .execute(
            "UPDATE treasury_operations SET facts=?1 WHERE id='source'",
            [serde_json::to_string(&operation_facts).unwrap()],
        )
        .unwrap();
    assert!(crate::rotation::transaction::PreparedTransaction::load(&store, "source").is_err());
    assert!(store.request_broadcast("source", 100, 99, false).is_err());
    operation_facts.deadline = Some(400);
    store
        .db
        .execute(
            "UPDATE treasury_operations SET facts=?1 WHERE id='source'",
            [serde_json::to_string(&operation_facts).unwrap()],
        )
        .unwrap();
    assert_eq!(
        store.request_broadcast("source", 100, 99, false).unwrap(),
        1
    );
}

#[test]
fn historical_numeric_deadlines_decode_without_reinterpreting_source_authority() {
    let historical = r#"{"txid":"old","expiry_height":100,"amount_zatoshis":0,"fee_zatoshis":10,"deadline":200}"#;
    let decoded: TransactionFacts = serde_json::from_str(historical).unwrap();
    assert_eq!(decoded.deadline, Some(200));
    let (_dir, store, _, _) = fixture();
    let window =
        crate::rotation::transaction::SubmissionWindow::load(&store, "not-internal", &decoded)
            .unwrap();
    assert!(window.check(200, false).is_err());
    assert!(window.check(199, false).is_ok());
    assert!(window.check(1, true).is_err());
}

fn serving(store: &Store) -> ServingAuthority {
    store
        .capture_serving_shielding(&[("shield".into(), 10)].into(), true)
        .unwrap()
        .unwrap()
}
fn untouched(store: &Store, revision: i64) {
    assert_eq!(store.snapshot().unwrap().0, revision);
    for table in [
        "treasury_preparations",
        "budget_entries",
        "outgoing",
        "shielding_operations",
        "shielding_inputs",
    ] {
        assert_eq!(
            store
                .db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
}
#[test]
fn serving_capability_is_current_policy_scoped_before_preparation() {
    for cause in [
        "paused", "disabled", "removed", "positive", "zero", "restored", "unlisted", "address",
    ] {
        let (_dir, mut store, job, _) = fixture();
        let mut authority = serving(&store);
        let revision = store.snapshot().unwrap().0;
        match cause {
            "paused" => store.pause_new_funding(),
            "disabled" => store.disable_pool("shield").unwrap(),
            "removed" => store
                .configure_profiles(
                    &std::collections::BTreeSet::new(),
                    &std::collections::BTreeSet::new(),
                )
                .unwrap(),
            "positive" => store
                .install_funding_permits(std::sync::Arc::new(PositivePermits))
                .unwrap(),
            "zero" => store.deny_new_funding(),
            "restored" => store.declare_restored_backup().unwrap(),
            "unlisted" => authority.pools.clear(),
            "address" => (),
            _ => unreachable!(),
        }
        let address = if cause == "address" {
            "another-address"
        } else {
            "test-address"
        };
        assert!(
            store
                .begin_automatic_shielding("auto", revision, &job, address, &authority)
                .is_err(),
            "{cause}"
        );
        untouched(&store, revision);
    }
}
#[test]
fn serving_capability_is_not_restored_from_reopened_state() {
    let (dir, mut store, job, _) = fixture();
    let authority = serving(&store);
    let revision = store.snapshot().unwrap().0;
    // A new process/Store owner cannot inherit a prior owner's live authority.
    let id = store.id.clone();
    drop(store);
    store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    assert!(
        store
            .begin_automatic_shielding("auto", revision, &job, "test-address", &authority)
            .is_err()
    );
    untouched(&store, revision);
    let authority = serving(&store);
    let (owner, policy) = store
        .begin_automatic_shielding("auto", revision, &job, "test-address", &authority)
        .unwrap();
    assert!(policy.automatic);
    assert_eq!(policy.max_fee, 10);
    let binding = Binding::new(policy, vec![input()], 10).unwrap();
    // The only header flag comes from this accepted PolicySource, with no bool
    // accepted by the automatic preparation or publication APIs.
    store.reserve(owner.operation(), None, 0, 10).unwrap();
    store
        .prepare_owned(&owner, b"calculated", b"fixture bytes", facts(10))
        .unwrap();
    binding
        .insert(
            &store.db,
            &store.key,
            &store.id,
            store.network.name(),
            owner.operation(),
        )
        .unwrap();
    assert!(
        store
            .db
            .query_row(
                "SELECT automatic FROM shielding_operations WHERE operation_id='auto'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    binding
        .check_saved(
            &store.db,
            &store.key,
            &store.id,
            store.network.name(),
            owner.operation(),
        )
        .unwrap();
}
#[test]
fn disabled_or_qualification_serving_cannot_capture_authority() {
    for cause in ["false", "paused", "positive", "zero", "restored", "removed"] {
        let (_dir, mut store, _, _) = fixture();
        let revision = store.snapshot().unwrap().0;
        match cause {
            "false" => (),
            "paused" => store.pause_new_funding(),
            "positive" => store
                .install_funding_permits(std::sync::Arc::new(PositivePermits))
                .unwrap(),
            "zero" => store.deny_new_funding(),
            "restored" => store.declare_restored_backup().unwrap(),
            "removed" => store
                .configure_profiles(
                    &std::collections::BTreeSet::new(),
                    &std::collections::BTreeSet::new(),
                )
                .unwrap(),
            _ => unreachable!(),
        }
        assert!(
            store
                .capture_serving_shielding(&[("shield".into(), 10)].into(), cause != "false")
                .unwrap()
                .is_none(),
            "{cause}"
        );
        untouched(&store, revision);
    }
}

#[test]
fn accepted_owner_rechecks_policy_before_reservation_and_publication() {
    let (_dir, mut store, job, _) = fixture();
    let authority = serving(&store);
    let revision = store.snapshot().unwrap().0;
    let (owner, policy) = store
        .begin_automatic_shielding("auto", revision, &job, "test-address", &authority)
        .unwrap();
    let binding = Binding::new(policy, vec![input()], 10).unwrap();
    store.disable_pool("shield").unwrap();
    assert!(store.check_shielding_inputs(&owner, &binding).is_err());
    assert!(
        store
            .prepare_owned_shielding(&owner, b"mutated", b"raw", facts(10), &binding)
            .is_err()
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(store.snapshot().unwrap().0, revision);
    assert!(store.close_retired_preparation(&owner).unwrap());
}

#[test]
fn maintenance_inventory_keeps_historical_addresses_without_provider_closure() {
    let (_dir, mut store, job, _) = fixture();
    let authority = store
        .capture_serving_shielding(
            &std::collections::BTreeMap::from([("shield".into(), 10)]),
            true,
        )
        .unwrap()
        .unwrap();
    store.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,refund) SELECT operation_id,job_id,phase,address FROM funding_progress JOIN funding_refunds USING(job_id) WHERE job_id=?1", [&job]).unwrap();
    store
        .db
        .execute("DELETE FROM funding_refunds WHERE job_id=?1", [&job])
        .unwrap();
    store
        .db
        .execute(
            "UPDATE funding_jobs SET state='COMPLETE' WHERE id=?1",
            [&job],
        )
        .unwrap();
    let before = store.snapshot().unwrap().0;
    let page = store
        .shielding_address_page(&authority, &maintenance::AddressCursor::default())
        .unwrap();
    assert_eq!(page.candidates.len(), 1);
    let candidate = &page.candidates[0];
    assert_eq!(candidate.address, "test-address");
    assert_eq!(store.snapshot().unwrap().0, before);
    assert!(
        store
            .shielding_address_page(&authority, page.next.as_ref().unwrap())
            .unwrap()
            .next
            .is_none()
    );
    let (owner, policy) = store
        .begin_shielding_candidate("historical-shield", before, candidate, &authority)
        .unwrap();
    assert!(policy.automatic);
    assert_eq!(policy.address, "test-address");
    store.close_retired_preparation(&owner).unwrap();
    store.db.execute("UPDATE pools SET enabled=0", []).unwrap();
    assert!(
        store
            .shielding_address_page(&authority, &maintenance::AddressCursor::default())
            .unwrap()
            .candidates
            .is_empty()
    );
    assert!(
        store
            .begin_shielding_candidate("disabled-shield", before, candidate, &authority)
            .is_err()
    );
}

#[test]
fn maintenance_operation_inventory_does_not_depend_on_serving_policy() {
    let (_dir, mut store, _job, policy) = fixture();
    outgoing(&mut store, "manual-old");
    let binding = Binding::new(policy, vec![input()], 10).unwrap();
    binding
        .insert(
            &store.db,
            &store.key,
            &store.id,
            store.network.name(),
            "manual-old",
        )
        .unwrap();
    let authority = store
        .capture_serving_shielding(
            &std::collections::BTreeMap::from([("shield".into(), 10)]),
            true,
        )
        .unwrap()
        .unwrap();
    assert!(
        !store
            .automatic_shielding_submission_candidate("manual-old", &authority)
            .unwrap()
    );
    store.db.execute("UPDATE pools SET enabled=0", []).unwrap();
    store.confirm_spend("manual-old", 10, 1).unwrap();
    assert_eq!(
        store.shielding_operation_page("").unwrap(),
        vec!["manual-old"]
    );
    assert!(
        store
            .shielding_operation_page("manual-old")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn maintenance_address_pages_are_bounded_and_stale_candidates_cannot_mutate() {
    let (_dir, mut store, job, _) = fixture();
    for index in 0..70 {
        store.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,refund) SELECT ?1,job_id,phase,address FROM funding_progress JOIN funding_refunds USING(job_id) WHERE job_id=?2", params![format!("old-{index:03}"),job]).unwrap();
    }
    let authority = store
        .capture_serving_shielding(
            &std::collections::BTreeMap::from([("shield".into(), 10)]),
            true,
        )
        .unwrap()
        .unwrap();
    let first = store
        .shielding_address_page(&authority, &maintenance::AddressCursor::default())
        .unwrap();
    assert_eq!(first.candidates.len(), 1);
    let second = store
        .shielding_address_page(&authority, first.next.as_ref().unwrap())
        .unwrap();
    assert_eq!(second.candidates.len(), 64);
    let third = store
        .shielding_address_page(&authority, second.next.as_ref().unwrap())
        .unwrap();
    assert_eq!(third.candidates.len(), 6);
    assert!(
        store
            .shielding_address_page(&authority, third.next.as_ref().unwrap())
            .unwrap()
            .next
            .is_none()
    );
    let stale = &first.candidates[0];
    store
        .db
        .execute("DELETE FROM funding_refunds WHERE job_id=?1", [&job])
        .unwrap();
    let revision = store.snapshot().unwrap().0;
    assert!(
        store
            .begin_shielding_candidate("stale-candidate", revision, stale, &authority)
            .is_err()
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM treasury_preparations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn maintenance_queries_seek_keys_and_page_sparse_physical_history() {
    let (_dir, store, job, _) = fixture();
    for sql in [
        maintenance::CURRENT_ADDRESSES,
        maintenance::ARCHIVED_ADDRESSES,
    ] {
        let details = store
            .db
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map(params!["", 64], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(details.iter().any(|d| d.contains("SEARCH")), "{details:?}");
        assert!(
            details
                .iter()
                .all(|d| !d.contains("SCAN") && !d.contains("TEMP B-TREE")),
            "{details:?}"
        );
    }
    let details = store
        .db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            maintenance::PENDING_OPERATION
        ))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        details.iter().any(|d| d.contains("one_outgoing")),
        "{details:?}"
    );
    assert!(
        details
            .iter()
            .all(|d| !d.contains("SCAN shielding_operations") && !d.contains("TEMP B-TREE")),
        "{details:?}"
    );
    for index in 0..130 {
        store.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,refund) SELECT ?1,job_id,phase,CASE WHEN ?2=129 THEN address ELSE NULL END FROM funding_progress JOIN funding_refunds USING(job_id) WHERE job_id=?3",params![format!("sparse-{index:03}"),index,job]).unwrap();
    }
    store
        .db
        .execute("DELETE FROM funding_refunds WHERE job_id=?1", [&job])
        .unwrap();
    let authority = store
        .capture_serving_shielding(
            &std::collections::BTreeMap::from([("shield".into(), 10)]),
            true,
        )
        .unwrap()
        .unwrap();
    let first = store
        .shielding_address_page(&authority, &maintenance::AddressCursor::default())
        .unwrap();
    assert!(first.candidates.is_empty());
    let second = store
        .shielding_address_page(&authority, first.next.as_ref().unwrap())
        .unwrap();
    assert!(second.candidates.is_empty());
    let third = store
        .shielding_address_page(&authority, second.next.as_ref().unwrap())
        .unwrap();
    assert_eq!(third.candidates.len(), 1);
    assert!(
        store
            .shielding_address_page(&authority, third.next.as_ref().unwrap())
            .unwrap()
            .next
            .is_none()
    );
}
