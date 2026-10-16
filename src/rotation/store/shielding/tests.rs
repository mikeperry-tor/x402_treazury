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
            "INSERT INTO expired_operations VALUES('first',100,?1)",
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
