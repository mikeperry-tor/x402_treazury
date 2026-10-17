//! Canonical source reversal uses the original transaction's complete input proof.
use super::*;

pub(super) fn saved_accounting(state: &Path, operation: &str) -> (i64, i64, i64, i64) {
    let db = rusqlite::Connection::open_with_flags(
        state.join("state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.query_row(
        "SELECT reserved,consumed,COALESCE((SELECT reversed_consumed FROM expired_operations WHERE id=?1),-1),(SELECT COUNT(*) FROM active_source_obligations WHERE operation_id=?1) FROM budget_entries WHERE id=?1",
        [operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap()
}

#[tokio::test]
async fn confirmed_reorg_expiry_reverses_only_exact_original_debit_and_survives_reopen() {
    let (dir, net, mut owner, endpoint) = funded_fixture(1).await;
    let pool = owner
        .ensure_capacity_group(
            "confirmed_expiry".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    owner
        .allocate_capacity_kind(pool, AllocationKind::Small, true)
        .await
        .unwrap();
    let store = owner.store_handle();
    let job = store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let deadline = now().unwrap() + 7200;
    save_quotes(&store, vec![job.clone()], deadline).await;
    let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
    let raw = prepared.bytes().to_vec();
    let facts = serde_json::to_string(prepared.facts().unwrap()).unwrap();
    let gross = prepared.facts().unwrap().amount_zatoshis + prepared.facts().unwrap().fee_zatoshis;
    let buried = u64::from(prepared.facts().unwrap().expiry_height) + 1;
    let stop = CancellationToken::new();
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint.clone(),
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    assert_eq!(
        owner
            .submit_prepared(job.operation_id.clone(), &mut sender, false, &stop)
            .await
            .unwrap(),
        SubmissionOutcome::Accepted
    );
    net.chain.write().await.mine_mempool();
    assert_eq!(
        owner
            .reconcile_prepared(job.operation_id.clone(), &mut sender, &stop)
            .await
            .unwrap(),
        TransactionPresence::Confirmed { height: 24 }
    );
    owner.sync_once(&stop).await.unwrap();
    let state = dir.path().join("state");
    assert_eq!(
        saved_accounting(&state, &job.operation_id),
        (0, gross as i64, -1, 0)
    );
    {
        let mut chain = net.chain.write().await;
        assert_eq!(chain.reorg_to(23).len(), 1);
        chain.mine_empty_blocks(1);
    }
    // Scanning independent funds completes, but the original input owner remains.
    owner.sync_once(&stop).await.unwrap();
    assert_eq!(
        saved_accounting(&state, &job.operation_id),
        (0, gross as i64, -1, 1)
    );
    assert!(
        store
            .call(|s| s.require_source_inputs_available())
            .await
            .is_err()
    );
    let early = owner
        .recover_expired(job.operation_id.clone(), &mut sender, &stop)
        .await
        .unwrap_err();
    assert!(matches!(
        early.downcast_ref::<crate::rotation::transaction::RecoveryEvidenceError>(),
        Some(crate::rotation::transaction::RecoveryEvidenceError::ExpiryNotBuried)
    ));
    assert_eq!(
        saved_accounting(&state, &job.operation_id),
        (0, gross as i64, -1, 1)
    );
    net.chain
        .write()
        .await
        .mine_empty_blocks((buried - 24).try_into().unwrap());
    let outcome = owner
        .recover_funding_job(job.id.clone(), 1, false, &mut sender, &stop)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        recovery::RecoveryOutcome::OperatorRequired("funding_disabled")
    );
    assert_eq!(
        saved_accounting(&state, &job.operation_id),
        (0, 0, gross as i64, 0)
    );
    let operation = job.operation_id.clone();
    store
        .call(move |s| {
            assert_eq!(s.prepared_bytes(&operation)?.as_slice(), raw);
            let original = s.operation(&operation)?;
            assert_eq!(serde_json::to_string(&original.facts)?, facts);
            assert_eq!(original.attempts, 1);
            assert_eq!(original.submission, "EXPIRED");
            s.require_source_inputs_available()
        })
        .await
        .unwrap();
    let id = owner.status().await.unwrap().treasury_id;
    drop(store);
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        state.clone(),
        dir.path().join("key"),
        id,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint, 1, 300).unwrap());
    owner
        .recover_expired(job.operation_id.clone(), &mut sender, &stop)
        .await
        .unwrap();
    assert_eq!(
        saved_accounting(&state, &job.operation_id),
        (0, 0, gross as i64, 0)
    );
    let db = rusqlite::Connection::open_with_flags(
        state.join("state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let counts: (i64,i64,i64) = db.query_row("SELECT (SELECT COUNT(*) FROM expired_operations),(SELECT COUNT(*) FROM funding_recovery),(SELECT COUNT(*) FROM treasury_operations)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(counts, (1, 1, 1));
    let original_submission: String = db
        .query_row(
            "SELECT submission FROM treasury_operations WHERE id=?1",
            [&job.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original_submission, "CONFIRMED");
    drop(db);
    owner.close().await.unwrap();
}
