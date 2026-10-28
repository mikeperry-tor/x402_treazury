use super::*;
use crate::rotation::{restriction::FundingRestriction, store::funding::FundingJob};

fn fixture() -> (tempfile::TempDir, Store, FundingJob) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = store
        .ensure_capacity_group(
            "pool",
            &test_support::reserves("2"),
            Some(U256::from(1_000_000)),
            0,
        )
        .unwrap();
    store
        .allocate_capacity_kind(&pool, AllocationKind::Small, true)
        .unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    (dir, store, job)
}
fn complete(store: &Store, job: &FundingJob) {
    store
        .db
        .execute(
            "UPDATE funding_jobs SET state='COMPLETE' WHERE id=?1",
            [&job.id],
        )
        .unwrap();
    store.db.execute("UPDATE funding_progress SET phase='\"COMPLETE\"',last_error='settlement_unverified' WHERE job_id=?1", [&job.id]).unwrap();
}
fn prepare(store: &mut Store, job: &FundingJob) {
    store
        .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
        .unwrap();
    let revision = store.snapshot().unwrap().0;
    store
        .prepare_with_facts(
            &job.operation_id,
            revision,
            b"next",
            b"exact signed bytes",
            Some(TransactionFacts {
                txid: "fixture".into(),
                expiry_height: 100,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: Some(10000),
            }),
        )
        .unwrap();
}
fn binding(store: &Store, job: &FundingJob) -> (Option<String>, i64) {
    store
        .db
        .query_row(
            "SELECT wallet_id,generation FROM reserve_claims WHERE pool_id=?1 AND slot='small'",
            [&job.pool_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

#[test]
fn completed_reserve_replacement_preserves_history_and_late_credit_across_restart() {
    let (dir, mut store, job) = fixture();
    complete(&store, &job);
    let original = serde_json::to_value(store.funding_job(&job.id).unwrap()).unwrap();
    let generation = binding(&store, &job).1;
    let before = store.status().unwrap();
    assert!(before.pools[0].funding_degraded);
    assert_eq!(
        before.pools[0].reserve_claims[0]
            .blocked_funding_job_id
            .as_deref(),
        Some(job.id.as_str())
    );
    assert!(!store.treasury_observation_needed(&["pool".into()]).unwrap());
    store.fill_funding_vacancies("pool").unwrap();
    let (replacement, next) = binding(&store, &job);
    let replacement = replacement.unwrap();
    assert_ne!(replacement, job.wallet_id);
    assert_eq!(next, generation + 1);
    assert_eq!(store.funding_jobs().unwrap().len(), 2);
    let fresh = store
        .funding_jobs()
        .unwrap()
        .into_iter()
        .find(|j| j.wallet_id == replacement)
        .unwrap();
    assert_ne!(fresh.operation_id, job.operation_id);
    assert!(store.prepared_bytes(&fresh.operation_id).is_err());
    assert!(store.treasury_observation_needed(&["pool".into()]).unwrap());
    assert!(!store.status().unwrap().pools[0].funding_degraded);
    store.fill_funding_vacancies("pool").unwrap();
    assert_eq!(store.funding_jobs().unwrap().len(), 2);
    assert_eq!(
        serde_json::to_value(store.funding_job(&job.id).unwrap()).unwrap(),
        original
    );
    let id = store.id.clone();
    drop(store);
    let mut store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    store.begin_balance(&job.pool_id, &job.wallet_id).unwrap();
    store
        .finish_balance(&job.pool_id, &job.wallet_id, Some(U256::from(2_000_000)))
        .unwrap();
    assert_eq!(binding(&store, &job), (Some(replacement), next));
    let status = store.status().unwrap();
    let old = status.pools[0]
        .addresses
        .iter()
        .find(|w| w.id == job.wallet_id)
        .unwrap();
    assert_eq!(old.role, "RETAINED");
    assert!(old.destination_recorded);
    assert!(
        capacity::payment_ownership(&store.db, &job.pool_id, &job.wallet_id)
            .unwrap()
            .is_some()
    );
    assert!(status.treasury_operations.is_empty());
}

#[test]
fn original_source_must_resolve_before_replacement_and_signed_bytes_survive() {
    let (_dir, mut store, job) = fixture();
    prepare(&mut store, &job);
    complete(&store, &job);
    let before = binding(&store, &job);
    store.fill_funding_vacancies("pool").unwrap();
    assert_eq!(binding(&store, &job), before);
    assert_eq!(
        store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
        b"exact signed bytes"
    );
    assert!(store.status().unwrap().pools[0].funding_degraded);
    store.confirm_spend(&job.operation_id).unwrap();
    let original = serde_json::to_value(store.operation(&job.operation_id).unwrap()).unwrap();
    store.fill_funding_vacancies("pool").unwrap();
    assert_ne!(binding(&store, &job).0, before.0);
    assert_eq!(
        serde_json::to_value(store.operation(&job.operation_id).unwrap()).unwrap(),
        original
    );
    assert_eq!(
        store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
        b"exact signed bytes"
    );
}

#[test]
fn accepted_calculation_and_archived_pending_source_keep_original_binding() {
    for archived in [false, true] {
        let (_dir, mut store, job) = fixture();
        store
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
            .unwrap();
        if archived {
            // Preserve the exact old attempt as recovery history while a new
            // current operation has no source work of its own.
            store.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase) VALUES (?1,?2,'\"COMPLETE\"')", params![job.operation_id,job.id]).unwrap();
            store
                .db
                .execute(
                    "UPDATE funding_progress SET operation_id='new-operation' WHERE job_id=?1",
                    [&job.id],
                )
                .unwrap();
        }
        complete(&store, &job);
        let before = binding(&store, &job);
        store.fill_funding_vacancies("pool").unwrap();
        assert_eq!(binding(&store, &job), before);
        assert!(store.preparation_pending().unwrap());
        assert_eq!(store.funding_jobs().unwrap().len(), 1);
    }
}

#[test]
fn disabled_funding_and_noncompleted_or_credited_slots_are_not_replaced() {
    for restriction in [
        FundingRestriction::Paused,
        FundingRestriction::DenyNewFunding,
    ] {
        let (_dir, mut store, job) = fixture();
        complete(&store, &job);
        let before = binding(&store, &job);
        store.funding_restriction = restriction;
        assert!(store.fill_funding_vacancies("pool").is_err());
        assert_eq!(binding(&store, &job), before);
        assert_eq!(store.funding_jobs().unwrap().len(), 1);
    }
    for mutation in [
        "",
        "UPDATE reserve_claims SET enabled=0",
        "UPDATE pools SET enabled=0",
        "UPDATE wallets SET role='RETAINED'",
        "UPDATE reserve_claims SET generation=generation+1",
    ] {
        let (_dir, mut store, job) = fixture();
        if !mutation.is_empty() {
            complete(&store, &job);
            store.db.execute_batch(mutation).unwrap();
        }
        let before = binding(&store, &job);
        store.fill_funding_vacancies("pool").unwrap();
        assert_eq!(binding(&store, &job), before);
    }
    let (_dir, mut store, job) = fixture();
    store
        .finish_balance(&job.pool_id, &job.wallet_id, Some(U256::from(1)))
        .unwrap();
    complete(&store, &job);
    let before = binding(&store, &job);
    store.fill_funding_vacancies("pool").unwrap();
    assert_eq!(binding(&store, &job), before);
    assert!(!store.status().unwrap().pools[0].funding_degraded);
}

#[test]
fn blocked_small_source_does_not_hold_completed_large_reserve() {
    let (_dir, mut store, job) = fixture();
    let mut reserves = test_support::reserves("2");
    reserves.large = Some(reserves.small.clone());
    store
        .ensure_capacity_group("pool", &reserves, Some(U256::from(1_000_000)), 0)
        .unwrap();
    store
        .allocate_capacity_kind(&job.pool_id, AllocationKind::Large, true)
        .unwrap();
    let large = store
        .funding_jobs()
        .unwrap()
        .into_iter()
        .find(|j| j.id != job.id)
        .unwrap();
    prepare(&mut store, &job);
    complete(&store, &job);
    complete(&store, &large);
    let small = binding(&store, &job);
    store.fill_funding_vacancies("pool").unwrap();
    assert_eq!(binding(&store, &job), small);
    assert_eq!(store.funding_jobs().unwrap().len(), 3);
    let report = store.status().unwrap();
    let claims = &report.pools[0].reserve_claims;
    assert_eq!(
        claims[0].blocked_funding_job_id.as_deref(),
        Some(job.id.as_str())
    );
    assert!(claims[1].blocked_funding_job_id.is_none());
    assert_ne!(
        claims[1].wallet_id.as_deref(),
        Some(large.wallet_id.as_str())
    );
    assert_eq!(
        store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
        b"exact signed bytes"
    );
}

#[test]
fn proven_expired_source_can_be_replaced_without_erasing_expiry_or_bytes() {
    let (_dir, mut store, job) = fixture();
    prepare(&mut store, &job);
    complete(&store, &job);
    // Faithful final journal facts after the existing canonical expiry proof.
    let revision = store.snapshot().unwrap().0;
    store
        .db
        .execute(
            "UPDATE outgoing SET state='RESOLVED' WHERE id=?1",
            [&job.operation_id],
        )
        .unwrap();
    store
        .db
        .execute(
            "INSERT INTO expired_operations(id,height,revision) VALUES (?1,101,?2)",
            params![job.operation_id, revision],
        )
        .unwrap();
    let before = binding(&store, &job);
    store.fill_funding_vacancies("pool").unwrap();
    assert_ne!(binding(&store, &job).0, before.0);
    assert_eq!(
        store.operation(&job.operation_id).unwrap().submission,
        "EXPIRED"
    );
    assert_eq!(
        store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
        b"exact signed bytes"
    );
}

#[test]
fn never_credited_destination_closes_only_when_every_original_source_cannot_deliver() {
    for source in [
        "unprepared",
        "accepted",
        "pending",
        "expired",
        "confirmed",
        "unknown",
    ] {
        let (_dir, mut store, job) = fixture();
        complete(&store, &job);
        if source == "accepted" {
            store
                .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                .unwrap();
        } else if source != "unprepared" {
            prepare(&mut store, &job);
            match source {
                "expired" => {
                    let revision = store.snapshot().unwrap().0;
                    store
                        .db
                        .execute(
                            "UPDATE outgoing SET state='RESOLVED' WHERE id=?1",
                            [&job.operation_id],
                        )
                        .unwrap();
                    store
                        .db
                        .execute(
                            "INSERT INTO expired_operations(id,height,revision) VALUES(?1,101,?2)",
                            params![job.operation_id, revision],
                        )
                        .unwrap();
                }
                "confirmed" => store.confirm_spend(&job.operation_id).unwrap(),
                "unknown" => {
                    store
                        .db
                        .execute(
                            "UPDATE outgoing SET state='RESOLVED' WHERE id=?1",
                            [&job.operation_id],
                        )
                        .unwrap();
                }
                _ => {}
            }
        }
        let closed = matches!(source, "unprepared" | "expired");
        assert_eq!(
            !funding::destination_obligation_pending(&store.db, &job.wallet_id).unwrap(),
            closed,
            "{source}"
        );
        assert!(
            !funding::initial_credit_recorded(&store.db, &job.wallet_id).unwrap(),
            "closure is not positive credit"
        );
        // A confirmed archived deposit remains capable of delivery even if the
        // current operation never started. Every attempt must close together.
        store.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase) VALUES('archived',?1,'\"COMPLETE\"')",[&job.id]).unwrap();
        let facts = serde_json::to_string(&TransactionFacts {
            txid: "archive".into(),
            expiry_height: 100,
            amount_zatoshis: 1,
            fee_zatoshis: 1,
            deadline: None,
        })
        .unwrap();
        let revision = store.snapshot().unwrap().0;
        store.db.execute("INSERT INTO outgoing(id,state,raw,revision,preparation) VALUES('archived','RESOLVED',X'00',?1,X'00')",[revision]).unwrap();
        store.db.execute("INSERT INTO treasury_operations(id,facts,submission,attempts) VALUES('archived',?1,'CONFIRMED',1)",[facts]).unwrap();
        assert!(
            funding::destination_obligation_pending(&store.db, &job.wallet_id).unwrap(),
            "{source}: archived deposit still owns delivery uncertainty"
        );
    }
}

#[test]
fn replacement_logs_recovery_once_and_pending_source_as_delay() {
    if crate::test_log_capture::run_in_child(
        "rotation::store::reserve_replacement::tests::replacement_logs_recovery_once_and_pending_source_as_delay",
    ) {
        return;
    }
    let (_dir, mut store, job) = fixture();
    prepare(&mut store, &job);
    complete(&store, &job);
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    store.fill_funding_vacancies("pool").unwrap();
    store.report_blocked_reserves("pool").unwrap();
    store.confirm_spend(&job.operation_id).unwrap();
    store.fill_funding_vacancies("pool").unwrap();
    store.fill_funding_vacancies("pool").unwrap();
    let logged = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        logged.contains("INFO") && logged.contains("replacement is waiting"),
        "{logged}"
    );
    assert_eq!(
        logged
            .matches("a fresh reserve will be allocated automatically")
            .count(),
        1,
        "{logged}"
    );
    assert!(
        !logged.contains("WARN") && !logged.contains("ERROR"),
        "{logged}"
    );
    assert!(
        !logged.contains("exact signed bytes") && !logged.contains("settlement_unverified"),
        "{logged}"
    );
}

#[test]
fn diagnostic_phase_does_not_prevent_closed_source_inventory_replacement() {
    let (_dir, mut store, job) = fixture();
    complete(&store, &job);
    store
        .db
        .execute(
            "UPDATE funding_progress SET phase='\"RECOVERY_REQUIRED\"' WHERE job_id=?1",
            [&job.id],
        )
        .unwrap();
    let original = serde_json::to_value(store.funding_job(&job.id).unwrap()).unwrap();
    let before = binding(&store, &job);
    store.fill_funding_vacancies("pool").unwrap();
    assert_ne!(binding(&store, &job).0, before.0);
    assert_eq!(
        serde_json::to_value(store.funding_job(&job.id).unwrap()).unwrap(),
        original
    );
}

#[test]
fn resolved_signed_record_without_confirmation_or_expiry_keeps_slot() {
    let (_dir, mut store, job) = fixture();
    prepare(&mut store, &job);
    complete(&store, &job);
    store
        .db
        .execute(
            "UPDATE outgoing SET state='RESOLVED' WHERE id=?1",
            [&job.operation_id],
        )
        .unwrap();
    store
        .db
        .execute(
            "DELETE FROM treasury_operations WHERE id=?1",
            [&job.operation_id],
        )
        .unwrap();
    let before = binding(&store, &job);
    store.fill_funding_vacancies("pool").unwrap();
    assert_eq!(binding(&store, &job), before);
    assert_eq!(
        store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
        b"exact signed bytes"
    );
}

#[test]
fn bootstrap_follows_only_abandoned_generations_and_stops_after_credited_successor_is_consumed() {
    let (_dir, mut store, job) = fixture();
    let captured = store
        .startup_groups(&["pool".into()])
        .unwrap()
        .remove(0)
        .claims;
    assert_eq!(
        store.startup_claim_paths(&job.pool_id, &captured).unwrap()[0]
            .job
            .id,
        job.id
    );
    complete(&store, &job);
    store
        .replace_completed_reserve(&job.pool_id, AllocationKind::Small)
        .unwrap();
    assert!(
        store
            .startup_claim_paths(&job.pool_id, &captured)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .startup_claim_balances(&job.pool_id, &captured)
            .is_err()
    );
    store.fill_funding_vacancies("pool").unwrap();
    let first = store
        .startup_claim_paths(&job.pool_id, &captured)
        .unwrap()
        .remove(0)
        .job;
    assert_ne!(first.id, job.id);
    complete(&store, &first);
    store.fill_funding_vacancies("pool").unwrap();
    let credited = store
        .startup_claim_paths(&job.pool_id, &captured)
        .unwrap()
        .remove(0)
        .job;
    assert_ne!(credited.id, first.id);
    store
        .finish_balance(
            &job.pool_id,
            &credited.wallet_id,
            Some(U256::from(2_000_000)),
        )
        .unwrap();
    let candidate = crate::rotation::manager::PaymentCandidateHandle {
        wallet: credited.wallet_id.clone(),
        address: credited.recipient.clone(),
    };
    let admission = store
        .prepare_advisory_payment(&job.pool_id, U256::from(1), "fixture", &candidate)
        .unwrap();
    let now = crate::rotation::base::now().unwrap();
    store
        .journal_authorization(
            &admission,
            Authorization {
                payer: credited.recipient.clone(),
                payee: "0x0000000000000000000000000000000000000001".into(),
                nonce: format!("{:064x}", 1),
                valid_after: now,
                valid_before: now + 3600,
            },
            None,
        )
        .unwrap();
    store.fill_funding_vacancies("pool").unwrap();
    assert_ne!(
        binding(&store, &job).0.as_deref(),
        Some(credited.wallet_id.as_str())
    );
    let paths = store.startup_claim_paths(&job.pool_id, &captured).unwrap();
    assert_eq!(paths[0].job.id, credited.id);
    assert!(paths[0].destination_ready);
    assert_eq!(
        store
            .startup_claim_balances(&job.pool_id, &captured)
            .unwrap(),
        vec![("small".into(), "1.999999".into())]
    );
    assert!(store.status().unwrap().treasury_operations.is_empty());
}

#[test]
fn original_late_credit_can_complete_bootstrap_without_reclaiming_replacement() {
    let (_dir, mut store, job) = fixture();
    let captured = store
        .startup_groups(&["pool".into()])
        .unwrap()
        .remove(0)
        .claims;
    complete(&store, &job);
    store.fill_funding_vacancies("pool").unwrap();
    let replacement = binding(&store, &job);
    store
        .finish_balance(&job.pool_id, &job.wallet_id, Some(U256::from(1_000_000)))
        .unwrap();
    let paths = store.startup_claim_paths(&job.pool_id, &captured).unwrap();
    assert_eq!(paths[0].job.id, job.id);
    assert!(paths[0].destination_ready);
    assert_eq!(
        store
            .startup_claim_balances(&job.pool_id, &captured)
            .unwrap(),
        vec![("small".into(), "1.000000".into())]
    );
    assert_eq!(binding(&store, &job), replacement);
}
