//! SDK chain reconstruction does not resurrect completed cross-chain work.
use super::*;

struct NoDisclosure;
impl TransactionSubmission for NoDisclosure {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("terminal source repair cannot submit");
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("canonical terminal repair cannot consult optional inclusion hints");
    }
}
async fn prepare(owner: &mut Treasury) -> (FundingJob, PreparedTransaction) {
    let pool = owner
        .ensure_capacity_group(
            "terminal".into(),
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
    let job = owner
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let deadline = now().unwrap() + 7200;
    save_quotes(&owner.store, vec![job.clone()], deadline).await;
    let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
    (job, prepared)
}

#[tokio::test]
async fn completed_expiry_evidence_stays_historical_across_sdk_rebuild_and_restart() {
    for terminal in [
        crate::rotation::near::SwapStatus::Failed,
        crate::rotation::near::SwapStatus::Refunded,
    ] {
        let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
        let stop = CancellationToken::new();
        let (job, prepared) = prepare(&mut owner).await;
        let expiry = prepared.facts().unwrap().expiry_height;
        {
            let mut chain = net.chain.write().await;
            let n = expiry + 2 - chain.tip();
            chain.mine_empty_blocks(n);
        }
        owner
            .recover_expired(job.operation_id.clone(), &mut NoDisclosure, &stop)
            .await
            .unwrap();
        let operation = job.operation_id.clone();
        let inputs = owner
            .store
            .call(move |s| s.prepared_inputs(&operation))
            .await
            .unwrap();
        let identity = owner.status().await.unwrap().treasury_id;
        owner.close().await.unwrap();
        {
            let mut chain = net.chain.write().await;
            chain.reorg_to(23);
            chain.mine_block(vec![prepared.bytes().to_vec()]);
            chain.mine_empty_blocks(expiry + 2 - 24);
        }
        owner = Treasury::open_with_network(
            dir.path().join("state"),
            dir.path().join("key"),
            identity,
            TreasuryNetwork::Synthetic,
        )
        .await
        .unwrap();
        owner.configure_sync(SyncSettings::new(endpoint, 2).unwrap());
        if owner.sync_once(&stop).await.is_err() {
            owner.sync_once(&stop).await.unwrap();
        }
        {
            let wallet = owner.client.as_ref().unwrap().wallet().read().await;
            let txid = submission::decode(prepared.bytes()).unwrap().txid();
            assert!(
                wallet
                    .wallet_transactions
                    .get(&txid)
                    .unwrap()
                    .status()
                    .is_confirmed(),
                "SDK independently owns current transaction observation"
            );
        }
        let operation = job.operation_id.clone();
        let raw = prepared.bytes().to_vec();
        let checked_job = job.id.clone();
        owner
            .store
            .call(move |s| {
                assert_eq!(
                    s.operation(&operation)?.submission,
                    "EXPIRED",
                    "a completed source journal is historical evidence, not a live reorg projection"
                );
                assert!(!s.operation_pending(&operation)?);
                assert!(!s.source_previously_closed(&operation)?);
                assert_eq!(s.operation(&operation)?.attempts, 0);
                assert_eq!(s.prepared_bytes(&operation)?.as_slice(), raw);
                assert_eq!(s.prepared_inputs(&operation)?, inputs);
                assert_eq!(s.funding_recovery_count(&checked_job)?, 1);
                Ok(())
            })
            .await
            .unwrap();
        let error = owner
            .reconcile_prepared(job.operation_id.clone(), &mut NoDisclosure, &stop)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("transaction is not pending"));
        // Exercise the existing canonical-proof/Store boundary with the retained
        // authenticated original. Ordinary reconciliation remains pending-only;
        // scans must not reopen completed historical source journals.
        assert!(
            owner
                .confirm_sdk_transaction(&prepared)
                .await
                .unwrap()
                .is_some()
        );
        let (job_id, operation) = (job.id.clone(), job.operation_id.clone());
        owner
            .store
            .call(move |s| s.record_provider_outcome(&job_id, &operation, &terminal, now()?))
            .await
            .unwrap();
        let identity = owner.status().await.unwrap().treasury_id;
        owner.close().await.unwrap();
        let mut owner = Treasury::open_with_network(
            dir.path().join("state"),
            dir.path().join("key"),
            identity,
            TreasuryNetwork::Synthetic,
        )
        .await
        .unwrap();
        use crate::treasury::recovery::RecoveryOutcome;
        for (allowed, expected) in [
            (false, RecoveryOutcome::Waiting("funding_disabled")),
            (
                true,
                RecoveryOutcome::OperatorRequired("recovery_attempt_limit_reached"),
            ),
            (
                true,
                RecoveryOutcome::OperatorRequired("recovery_attempt_limit_reached"),
            ),
        ] {
            assert_eq!(
                owner
                    .recover_funding_job(job.id.clone(), 0, allowed, &mut NoDisclosure, &stop)
                    .await
                    .unwrap(),
                expected
            );
        }
        assert_eq!(
            owner
                .recover_funding_job(job.id.clone(), 1, true, &mut NoDisclosure, &stop)
                .await
                .unwrap(),
            RecoveryOutcome::Recovered
        );
        let raw = prepared.bytes().to_vec();
        owner
            .store
            .call(move |s| {
                assert_ne!(
                    s.funding_job(&job.id)?.unwrap().operation_id,
                    job.operation_id
                );
                assert_eq!(
                    s.funding_recovery_count(&job.id)?,
                    1,
                    "reconfirmation does not consume the original twice"
                );
                assert_eq!(s.operation(&job.operation_id)?.submission, "CONFIRMED");
                assert_eq!(
                    s.operation(&job.operation_id)?.attempts,
                    0,
                    "recovery must not broadcast"
                );
                assert_eq!(s.prepared_bytes(&job.operation_id)?.as_slice(), raw);
                assert!(!s.operation_pending(&job.operation_id)?);
                Ok(())
            })
            .await
            .unwrap();
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn fulfilled_source_work_survives_sdk_reorg_without_confirmation_or_credit_fabrication() {
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let stop = CancellationToken::new();
    let (job, prepared) = prepare(&mut owner).await;
    let operation = job.operation_id.clone();
    let job_id = job.id.clone();
    let (inputs, native_submission) = owner
        .store
        .call(move |s| {
            s.request_broadcast(&operation, now()?, 23, false)?;
            let native_submission = s.operation(&operation)?.submission;
            s.record_provider_outcome(
                &job_id,
                &operation,
                &crate::rotation::near::SwapStatus::Success,
                now()?,
            )?;
            Ok((s.prepared_inputs(&operation)?, native_submission))
        })
        .await
        .unwrap();
    let address = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let replacement = faucet_funding_transaction(vec![(&address, 2_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.reorg_to(20);
        chain.mine_block(vec![replacement]);
        chain.mine_empty_blocks(2);
    }
    let identity = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint, 2).unwrap());
    if owner.sync_once(&stop).await.is_err() {
        owner.sync_once(&stop).await.unwrap();
    }
    assert_eq!(
        owner
            .recover_funding_job(job.id.clone(), 3, true, &mut NoDisclosure, &stop)
            .await
            .unwrap(),
        recovery::RecoveryOutcome::Waiting("swap_fulfilled_waiting_for_credit")
    );
    assert_eq!(
        owner
            .reconcile_prepared(job.operation_id.clone(), &mut NoDisclosure, &stop)
            .await
            .unwrap(),
        TransactionPresence::Unknown
    );
    let operation = job.operation_id.clone();
    let job_id = job.id.clone();
    let raw = prepared.bytes().to_vec();
    owner
        .store
        .call(move |s| {
            assert!(!s.operation_pending(&operation)?);
            assert_eq!(
                s.operation(&operation)?.submission,
                native_submission,
                "NEAR fulfillment preserves the original native disclosure label"
            );
            assert_eq!(s.operation(&operation)?.attempts, 1);
            assert_eq!(s.prepared_bytes(&operation)?.as_slice(), raw);
            assert_eq!(s.prepared_inputs(&operation)?, inputs);
            assert_eq!(s.funding_recovery_count(&job_id)?, 0);
            let facts = s.funding_facts(&job_id, &operation)?;
            assert!(!facts.initial_credit_recorded);
            assert!(
                facts.credit_needed(),
                "the existing destination credit lane remains required"
            );
            assert_eq!(
                s.funding_job(&job_id)?.unwrap().phase,
                crate::rotation::store::funding::FundingPhase::VerifyingCredit
            );
            s.require_preparation_available()?;
            Ok(())
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn fulfilled_archived_request_does_not_starve_disclosed_replacement_after_credit() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct ObserveOnly {
        lookups: Arc<AtomicUsize>,
    }
    impl TransactionSubmission for ObserveOnly {
        async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
            panic!("observation cannot replay an original");
        }
        async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            Ok(TransactionPresence::Unknown)
        }
    }
    let (_dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let stop = CancellationToken::new();
    let (old, old_raw) = prepare(&mut owner).await;
    {
        let mut chain = net.chain.write().await;
        let remaining = old_raw.facts().unwrap().expiry_height + 2 - chain.tip();
        chain.mine_empty_blocks(remaining);
    }
    owner
        .recover_expired(old.operation_id.clone(), &mut NoDisclosure, &stop)
        .await
        .unwrap();
    let previous = old.clone();
    let current = owner
        .store
        .call(move |s| {
            s.reauthorize_closed_funding_for_operation(&previous.id, &previous.operation_id, 3)?;
            Ok(s.funding_job(&previous.id)?.unwrap())
        })
        .await
        .unwrap();
    let deadline = now().unwrap() + 7200;
    save_quotes(&owner.store, vec![current.clone()], deadline).await;
    let prepared = owner.prepare(request(&current, deadline)).await.unwrap();
    let mut submitter = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    owner
        .submit_prepared(current.operation_id.clone(), &mut submitter, false, &stop)
        .await
        .unwrap();
    let (original, replacement) = (old.clone(), current.clone());
    owner
        .store
        .call(move |s| {
            s.record_provider_outcome(
                &original.id,
                &original.operation_id,
                &crate::rotation::near::SwapStatus::Success,
                1,
            )?;
            s.record_destination_readiness(&replacement.id, "1", "first positive credit", 1)?;
            assert!(s.finish_funding_if_proven(&replacement.id, &replacement.operation_id)?);
            assert!(s.operation_pending(&replacement.operation_id)?);
            assert!(s.unowned_pending_sources(None)?.is_empty());
            let work = s.funding_work_page("", s.funding_work_boundary()?)?;
            assert_eq!(
                work.len(),
                1,
                "the funding scheduler retains source ownership"
            );
            assert_eq!(work[0].0, replacement.id);
            assert!(
                !s.funding_facts(&replacement.id, &replacement.operation_id)?
                    .provider_success
            );
            Ok(())
        })
        .await
        .unwrap();
    let lookups = Arc::new(AtomicUsize::new(0));
    let observer = ObserveOnly {
        lookups: lookups.clone(),
    };
    let store = owner.store_handle();
    // Start with a due source observation, independently of startup discovery.
    owner.startup_discovery = false;
    owner.observation = Default::default();
    let (handle, commands) = actor::channel();
    let task = tokio::spawn(owner.run_commands(commands, observer, stop.clone()));
    let observed = handle
        .recover_funding(current.id.clone(), 3, false)
        .await
        .unwrap();
    assert!(
        observed.source_observed,
        "completed destination objective must not short-circuit native source observation"
    );
    let op = current.operation_id.clone();
    assert!(store.call(move |s| s.operation_pending(&op)).await.unwrap());
    // This independent command follows the actor's post-recovery maintenance.
    // It must complete without a second lookup of the still-pending source.
    handle.refund_address(current.id.clone()).await.unwrap();
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![prepared.bytes().to_vec()]);
        chain.mine_empty_blocks(1);
    }
    handle.sync().await.unwrap();
    let observed = handle
        .recover_funding(current.id.clone(), 3, false)
        .await
        .unwrap();
    assert!(observed.source_observed);
    assert_eq!(observed.outcome, recovery::RecoveryOutcome::Recovered);
    let original_bytes = prepared.bytes().to_vec();
    store
        .call(move |s| {
            assert!(!s.operation_pending(&current.operation_id)?);
            assert_eq!(s.operation(&current.operation_id)?.submission, "CONFIRMED");
            assert_eq!(s.operation(&current.operation_id)?.attempts, 1);
            assert_eq!(
                s.prepared_bytes(&current.operation_id)?.as_slice(),
                original_bytes.as_slice()
            );
            assert_eq!(s.funding_jobs()?.len(), 1);
            assert_eq!(s.funding_recovery_count(&current.id)?, 1);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    stop.cancel();
    drop(handle);
    drop(store);
    task.await.unwrap().unwrap();
}
