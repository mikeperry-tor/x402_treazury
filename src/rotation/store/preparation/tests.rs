use super::*;

fn fixture(dir: &Path) -> Store {
    Store::create(
        &dir.join("state"),
        &dir.join("key"),
        1,
        b"committed initial wallet",
    )
    .unwrap()
}
fn begin(store: &mut Store) -> Preparation {
    let revision = store.snapshot().unwrap().0;
    store
        .begin_owned_preparation(&Uuid::new_v4().to_string(), revision, None)
        .unwrap()
}
fn facts() -> TransactionFacts {
    TransactionFacts {
        txid: "fixture-txid".into(),
        expiry_height: 200,
        amount_zatoshis: 70,
        fee_zatoshis: 10,
        deadline: 1000,
    }
}
fn reserve(store: &mut Store, owner: &Preparation) {
    store.require_owned_reservation(owner).unwrap();
    store.reserve(owner.operation(), None, 1, 80).unwrap();
}
fn commit(store: &mut Store, owner: &Preparation) -> Result<i64> {
    store.prepare_owned(
        owner,
        b"committed calculated wallet",
        b"exact source bytes",
        facts(),
    )
}

#[test]
fn provenance_fences_generic_commit_wrong_owner_and_closed_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = fixture(dir.path());
    let owner = begin(&mut s);
    assert!(s.require_unstarted_preparation(owner.operation()).is_err());
    reserve(&mut s, &owner);
    assert!(s.abandon_unprepared(owner.operation()).is_err());
    assert!(
        s.prepare_with_facts(
            owner.operation(),
            owner.revision(),
            b"wallet",
            b"bytes",
            Some(facts())
        )
        .is_err()
    );
    let mut wrong = owner.clone();
    wrong.token = Uuid::new_v4().to_string();
    assert!(commit(&mut s, &wrong).is_err());
    assert!(s.close_retired_preparation(&wrong).is_err());
    assert!(s.close_retired_preparation(&owner).unwrap());
    assert!(s.require_preparation_owner(&owner).is_err());
    assert!(commit(&mut s, &owner).is_err());
    assert!(
        s.begin_owned_preparation(owner.operation(), owner.revision(), None)
            .is_err()
    );
    assert_eq!(
        s.db.query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(s.snapshot().unwrap().0, owner.revision());
}

#[test]
fn cleanup_retains_committed_capability_and_rejects_unexplained_effects() {
    for cause in ["committed", "consumed", "revision", "unknown_contract"] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = fixture(dir.path());
        let owner = begin(&mut s);
        reserve(&mut s, &owner);
        match cause {
            "committed" => {
                commit(&mut s, &owner).unwrap();
            }
            "consumed" => {
                s.db.execute(
                    "UPDATE budget_entries SET consumed=1 WHERE id=?1",
                    [owner.operation()],
                )
                .unwrap();
            }
            "revision" => {
                s.db.execute("INSERT INTO snapshots SELECT revision+1,bytes FROM snapshots ORDER BY revision DESC LIMIT 1",[]).unwrap();
            }
            _ => {
                s.db.execute_batch("DROP TRIGGER immutable_preparation; UPDATE treasury_preparations SET contract_version=999").unwrap();
            }
        }
        if cause == "committed" {
            assert!(!s.close_retired_preparation(&owner).unwrap());
            assert_eq!(
                &*s.prepared_bytes(owner.operation()).unwrap(),
                b"exact source bytes"
            );
        } else {
            assert!(s.close_retired_preparation(&owner).is_err());
        }
        assert_eq!(
            s.db.query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(
            !s.db
                .query_row("SELECT closed FROM treasury_preparations", [], |r| r
                    .get::<_, bool>(0))
                .unwrap()
        );
    }
}

#[tokio::test]
async fn committed_before_lost_ack_cannot_be_closed_or_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = fixture(dir.path());
    let owner = begin(&mut s);
    reserve(&mut s, &owner);
    let (store, worker) = super::super::StoreHandle::spawn(s);
    let accepted = owner.clone();
    let (committed, observed) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let waiter = tokio::spawn({
        let store = store.clone();
        async move {
            store
                .call(move |s| {
                    commit(s, &accepted)?;
                    committed.send(()).unwrap();
                    released.recv().unwrap();
                    Ok(())
                })
                .await
        }
    });
    observed.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    store
        .call(move |s| {
            assert!(!s.close_retired_preparation(&owner)?);
            assert_eq!(
                &*s.prepared_bytes(owner.operation())?,
                b"exact source bytes"
            );
            assert!(s.abandon_unprepared(owner.operation()).is_err());
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    worker.await.unwrap();
}

#[test]
#[ignore = "subprocess boundary helper"]
fn preparation_process_child() {
    let dir = std::path::PathBuf::from(std::env::var_os("X402_PREPARATION_FIXTURE_DIR").unwrap());
    let id = std::env::var("X402_PREPARATION_FIXTURE_ID").unwrap();
    let phase = std::env::var("X402_PREPARATION_FIXTURE_PHASE").unwrap();
    let mut s = Store::open(&dir.join("state"), &dir.join("key"), &id).unwrap();
    let owner = begin(&mut s);
    if phase != "accepted" {
        reserve(&mut s, &owner);
    }
    if phase == "committed" {
        commit(&mut s, &owner).unwrap();
    }
    // No store, SQLite or SDK destructor can supply a cleanup claim.
    std::process::exit(0);
}

#[test]
fn process_death_keeps_precommit_provenance_and_exact_committed_capability() {
    for phase in ["accepted", "reserved", "committed"] {
        let dir = tempfile::tempdir().unwrap();
        let s = fixture(dir.path());
        let id = s.id().to_owned();
        drop(s);
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "rotation::store::preparation::tests::preparation_process_child",
                "--ignored",
            ])
            .env("X402_PREPARATION_FIXTURE_DIR", dir.path())
            .env("X402_PREPARATION_FIXTURE_ID", &id)
            .env("X402_PREPARATION_FIXTURE_PHASE", phase)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        let mut s = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        let operation: String = s.db.query_row("SELECT operation_id FROM treasury_preparations WHERE contract_version=1 AND closed=0",[],|r|r.get(0)).unwrap();
        assert!(s.begin_owned_preparation(&operation, 1, None).is_err());
        let committed = phase == "committed";
        assert_eq!(s.snapshot().unwrap().0, if committed { 2 } else { 1 });
        assert_eq!(s.operation_pending(&operation).unwrap(), committed);
        if committed {
            assert_eq!(
                &*s.prepared_bytes(&operation).unwrap(),
                b"exact source bytes"
            );
        }
        assert_eq!(
            s.db.query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            i64::from(phase != "accepted")
        );
    }
}

#[test]
fn cleanup_persistence_failure_keeps_reservation_and_provenance_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = fixture(dir.path());
    let owner = begin(&mut s);
    reserve(&mut s, &owner);
    s.db.execute_batch("CREATE TRIGGER reject_closure BEFORE UPDATE OF closed ON treasury_preparations BEGIN SELECT RAISE(ABORT,'fixture closure failure'); END").unwrap();
    assert!(s.close_retired_preparation(&owner).is_err());
    assert_eq!(
        s.db.query_row("SELECT reserved FROM budget_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        80
    );
    s.require_preparation_owner(&owner).unwrap();
    assert!(!s.operation_pending(owner.operation()).unwrap());
}

#[test]
fn managed_cleanup_archives_exact_attempt_and_preserves_destination_observation() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = fixture(dir.path());
    super::super::test_support::declare_two_reserves(
        &mut s,
        "managed",
        "0.000010",
        Some(U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = s.funding_jobs().unwrap().remove(0);
    let quote = serde_json::to_vec(
        &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
    )
    .unwrap();
    s.save_funding_quote(&job.id, &quote).unwrap();
    let encrypted_quote: Vec<u8> =
        s.db.query_row(
            "SELECT quote FROM funding_progress WHERE job_id=?1",
            [&job.id],
            |r| r.get(0),
        )
        .unwrap();
    let authority = crate::rotation::transaction::ManagedFundingAuthority {
        job_id: job.id.clone(),
        auto_fund: true,
        max_attempts: 0,
    };
    let owner = s
        .begin_owned_preparation(&job.operation_id, 1, Some(&authority))
        .unwrap();
    s.reserve(&job.operation_id, Some(&job.pool_id), 1, 80)
        .unwrap();
    // New independent destination credit cannot rewrite the accepted source owner.
    s.record_destination_readiness(&job.id, &job.target, "credit", 1)
        .unwrap();
    assert!(
        s.close_unprepared_funding(&job.id).is_err(),
        "legacy PREPARING is insufficient cleanup proof"
    );
    assert!(s.close_retired_preparation(&owner).unwrap());
    let saved: (String, Vec<u8>) =
        s.db.query_row(
            "SELECT operation_id,quote FROM funding_recovery WHERE job_id=?1",
            [&job.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved.0, job.operation_id);
    assert_eq!(saved.1, encrypted_quote);
    assert_eq!(
        s.funding_job(&job.id).unwrap().unwrap().phase,
        super::super::funding::FundingPhase::RecoveryRequired
    );
    assert!(
        s.finish_funding_if_proven(&job.id, &job.operation_id)
            .unwrap()
    );
    assert!(commit(&mut s, &owner).is_err());
}

#[test]
fn accepted_provenance_can_commit_after_destination_readiness_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = fixture(dir.path());
    super::super::test_support::declare_two_reserves(
        &mut s,
        "managed",
        "0.000010",
        Some(U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = s.funding_jobs().unwrap().remove(0);
    let quote = serde_json::to_vec(
        &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
    )
    .unwrap();
    s.save_funding_quote(&job.id, &quote).unwrap();
    let authority = crate::rotation::transaction::ManagedFundingAuthority {
        job_id: job.id.clone(),
        auto_fund: true,
        max_attempts: 0,
    };
    let owner = s
        .begin_owned_preparation(&job.operation_id, 1, Some(&authority))
        .unwrap();
    s.reserve(&job.operation_id, Some(&job.pool_id), 1, 80)
        .unwrap();
    s.record_destination_readiness(&job.id, &job.target, "credit", 1)
        .unwrap();
    s.db.execute(
        "UPDATE wallets SET role='RETIRED' WHERE id=?1",
        [&job.wallet_id],
    )
    .unwrap();
    commit(&mut s, &owner).unwrap();
    assert!(!s.close_retired_preparation(&owner).unwrap());
    assert_eq!(
        s.funding_job(&job.id).unwrap().unwrap().phase,
        super::super::funding::FundingPhase::Prepared
    );
    assert_eq!(
        s.db.query_row("SELECT COUNT(*) FROM funding_recovery", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        &*s.prepared_bytes(owner.operation()).unwrap(),
        b"exact source bytes"
    );
}

#[test]
fn accepted_owner_overrides_stale_phase_and_contradictory_archive() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = fixture(dir.path());
    super::super::test_support::declare_two_reserves(
        &mut s,
        "managed",
        "0.000010",
        Some(U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = s.funding_jobs().unwrap().remove(0);
    let quote = serde_json::to_vec(
        &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
    )
    .unwrap();
    s.save_funding_quote(&job.id, &quote).unwrap();
    let authority = crate::rotation::transaction::ManagedFundingAuthority {
        job_id: job.id.clone(),
        auto_fund: true,
        max_attempts: 0,
    };
    let _owner = s
        .begin_owned_preparation(&job.operation_id, 1, Some(&authority))
        .unwrap();
    s.db.execute(
        "UPDATE funding_progress SET phase='\"QUOTED\"' WHERE job_id=?1",
        [&job.id],
    )
    .unwrap();
    let stale = s.funding_job(&job.id).unwrap().unwrap();
    let facts = s.funding_facts(&job.id, &job.operation_id).unwrap();
    assert!(facts.preparation_started);
    assert!(!super::super::funding::safely_unprepared(&stale, &facts));
    assert!(!super::super::funding::can_recover_facts(
        &stale, None, &facts
    ));
    assert!(s.close_unprepared_funding(&job.id).is_err());
    assert!(
        s.refresh_unprepared_quote_for_operation(&job.id, &job.operation_id)
            .is_err()
    );
    // A conflicting legacy archive cannot hide an accepted SDK owner.
    s.db.execute(
        "UPDATE funding_progress SET phase='\"PREPARING\"' WHERE job_id=?1",
        [&job.id],
    )
    .unwrap();
    s.record_destination_readiness(&job.id, &job.target, "credit", 1)
        .unwrap();
    super::super::funding::archive_unprepared(&s.db, &job.id, &job.operation_id).unwrap();
    let facts = s.funding_facts(&job.id, &job.operation_id).unwrap();
    assert!(facts.archived && !facts.source_closed);
    assert!(
        !s.finish_funding_if_proven(&job.id, &job.operation_id)
            .unwrap()
    );
    // An unknown contract's claimed closure is likewise not our proof.
    s.db.execute_batch("DROP TRIGGER immutable_preparation; UPDATE treasury_preparations SET contract_version=999,closed=1").unwrap();
    let facts = s.funding_facts(&job.id, &job.operation_id).unwrap();
    assert!(!facts.source_closed);
    s.db.execute("DELETE FROM treasury_preparations", [])
        .unwrap();
    let facts = s.funding_facts(&job.id, &job.operation_id).unwrap();
    assert!(
        !facts.preparation_started && !facts.source_closed,
        "legacy archived PREPARING is not proof of clean source ownership"
    );
}

#[test]
fn malformed_provenance_is_operation_scoped_but_sql_failures_propagate() {
    for shared_sql in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = fixture(dir.path());
        let owner = begin(&mut s);
        if shared_sql {
            s.db.execute_batch(
                "ALTER TABLE treasury_preparations RENAME TO unavailable_preparations",
            )
            .unwrap();
        } else {
            s.db.execute_batch(
                "DROP TRIGGER immutable_preparation; UPDATE treasury_preparations SET token=X'ff'",
            )
            .unwrap();
        }
        let error = source_state(&s.db, owner.operation()).unwrap_err();
        assert_eq!(
            error.is::<super::super::scheduling::FundingDecodeFailure>(),
            !shared_sql
        );
        if !shared_sql {
            assert_eq!(
                source_state(&s.db, "independent-operation").unwrap(),
                (false, false)
            );
        }
    }
}
