//! Actual SDK receiver evidence drives managed refund recovery, including reopen.
use super::*;

struct NoReplay;
impl TransactionSubmission for NoReplay {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("refund recovery must never replay the original deposit");
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("confirmed refund recovery needs no additional inclusion lookup");
    }
}

#[tokio::test]
async fn managed_refund_receipt_rebuilds_after_restart_and_replaces_without_replay() {
    let (dir, net, mut owner, endpoint) = funded_fixture(3).await;
    let pool = owner
        .ensure_capacity_group(
            "refund".into(),
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
    let refund = owner.refund_address(job.id.clone()).await.unwrap();
    let deadline = now().unwrap() + 7200;
    let saved = job.clone();
    let bound = refund.clone();
    owner.store.call(move |s| {
        let quote = Quote { sizing_preview: None, request: serde_json::json!({"amount":saved.target,"recipient":saved.recipient,"refundTo":bound}), response: serde_json::json!({}), input:50_000, deadline, deposit:Some(DEPOSIT.into()) };
        s.save_funding_quote(&saved.id, &serde_json::to_vec(&quote)?)
    }).await.unwrap();
    let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
    let original_bytes = prepared.bytes().to_vec();
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint.clone(),
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    let stop = CancellationToken::new();
    owner
        .submit_prepared(job.operation_id.clone(), &mut sender, false, &stop)
        .await
        .unwrap();
    {
        let mut chain = net.chain.write().await;
        chain.mine_mempool();
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&stop).await.unwrap();
    owner
        .reconcile_prepared(job.operation_id.clone(), &mut sender, &stop)
        .await
        .unwrap();
    let saved = job.clone();
    owner
        .store
        .call(move |s| {
            s.record_provider_outcome(
                &saved.id,
                &saved.operation_id,
                &crate::rotation::near::SwapStatus::Refunded,
                now()?,
            )
        })
        .await
        .unwrap();
    owner.observe_funding_refunds().await.unwrap();
    let saved = job.clone();
    owner
        .store
        .call(move |s| {
            assert!(s.treasury_observation_needed(&[])?);
            assert!(
                s.funding_facts(&saved.id, &saved.operation_id)?
                    .status_needed()
            );
            Ok(())
        })
        .await
        .unwrap();
    // Unrelated incoming funds are not a refund-receiver observation.
    let unrelated = owner.preview_address().await.unwrap();
    let receipt = net
        .funding_transaction(vec![(&unrelated, 20_000, None)])
        .await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![receipt]);
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&stop).await.unwrap();
    owner.observe_funding_refunds().await.unwrap();
    let outcome = owner
        .recover_funding_job(job.id.clone(), true, &mut NoReplay, &stop)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        recovery::RecoveryOutcome::Waiting("waiting_for_confirmed_refund")
    );
    // A positive partial return suffices; no equality to the 50,000-zatoshi input.
    let receipt = net.funding_transaction(vec![(&refund, 20_000, None)]).await;
    net.chain.write().await.mine_block(vec![receipt]);
    owner.sync_once(&stop).await.unwrap();
    owner.observe_funding_refunds().await.unwrap();
    let saved = job.clone();
    owner
        .store
        .call(move |s| {
            assert!(
                !s.funding_facts(&saved.id, &saved.operation_id)?
                    .refund_observed
            );
            Ok(())
        })
        .await
        .unwrap();
    net.chain.write().await.mine_empty_blocks(2);
    owner.sync_once(&stop).await.unwrap();
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
    owner.configure_sync(SyncSettings::new(endpoint, 3).unwrap());
    // Rebuild from saved notes without another scan or another balance increase.
    owner.observe_funding_refunds().await.unwrap();
    let saved = job.clone();
    owner
        .store
        .call(move |s| {
            let facts = s.funding_facts(&saved.id, &saved.operation_id)?;
            assert!(facts.refund_observed);
            assert!(!facts.status_needed());
            assert!(!s.treasury_observation_needed(&[])?);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        owner
            .recover_funding_job(job.id.clone(), false, &mut NoReplay, &stop)
            .await
            .unwrap(),
        recovery::RecoveryOutcome::Waiting("funding_disabled")
    );
    assert_eq!(
        owner
            .recover_funding_job(job.id.clone(), true, &mut NoReplay, &stop)
            .await
            .unwrap(),
        recovery::RecoveryOutcome::Recovered
    );
    let saved = job.clone();
    owner
        .store
        .call(move |s| {
            let current = s.funding_job(&saved.id)?.unwrap();
            assert_ne!(current.operation_id, saved.operation_id);
            assert_eq!(
                current.phase,
                crate::rotation::store::funding::FundingPhase::Allocated
            );
            assert_eq!(
                s.prepared_bytes(&saved.operation_id)?.as_slice(),
                original_bytes.as_slice()
            );
            assert_eq!(s.operation(&saved.operation_id)?.attempts, 1);
            assert!(
                s.funding_status_quote(&saved.id, &saved.operation_id)
                    .is_ok()
            );
            Ok(())
        })
        .await
        .unwrap();
    let fresh = owner.refund_address(job.id).await.unwrap();
    assert_ne!(fresh, refund, "replacement derives a fresh receiver");
    assert!(net.chain.read().await.taddr_request_log().is_empty());
    owner.close().await.unwrap();
}
