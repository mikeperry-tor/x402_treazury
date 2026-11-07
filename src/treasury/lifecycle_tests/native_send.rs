//! Native direct sends use real SDK proofs and the same durable source owner.
use super::*;
use crate::rotation::store::DirectSendAuthority;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn native_request(id: String, recipient: String, amount: u64) -> PrepareRequest {
    PrepareRequest {
        managed_funding: None,
        operation_id: id,
        pool_id: None,
        deadline: None,
        recipient,
        amount_zatoshis: amount,
        max_fee_zatoshis: 100_000,
        max_input_zatoshis: u64::MAX,
    }
}
async fn accept(
    owner: &Treasury,
    recipient: String,
    request_id: &'static str,
    cap: Option<u64>,
) -> String {
    owner
        .store
        .call(move |s| {
            let operation = s
                .reserve_native(
                    "owner",
                    "send",
                    request_id,
                    json!({"asset_id":crate::direct_swap::route::ZEC,"chain":"zec",
                "amount":"50000","recipient":recipient,"max_source_zatoshis":null}),
                )?
                .0;
            let operation = s.accept_direct_send(
                &operation.id,
                None,
                DirectSendAuthority {
                    max_fee_zatoshis: 100_000,
                    max_debit_zatoshis: cap,
                },
            )?;
            assert!(operation.quote.is_none());
            Ok(operation.id)
        })
        .await
        .unwrap()
}
async fn fee(owner: &Treasury, recipient: &str) -> u64 {
    let payment = zcash_client_backend::zip321::Payment::new(
        recipient.parse().unwrap(),
        Some(zcash_protocol::value::Zatoshis::from_u64(50_000).unwrap()),
        None,
        None,
        None,
        vec![],
    )
    .unwrap();
    let preview = owner
        .client
        .as_ref()
        .unwrap()
        .preview_send(
            zcash_client_backend::zip321::TransactionRequest::new(vec![payment]).unwrap(),
            zip32::AccountId::ZERO,
        )
        .await
        .unwrap();
    zingolib::data::proposal::total_fee(&preview)
        .unwrap()
        .into_u64()
}

#[tokio::test(flavor = "current_thread")]
async fn native_self_payment_uses_actual_total_cap_and_retains_bytes_after_cancellation_and_reopen()
{
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let recipient = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let actual_fee = fee(&owner, &recipient).await;
    let total = 50_000 + actual_fee;
    assert!(
        actual_fee < 100_000,
        "fixture must distinguish actual fee from configured maximum"
    );
    let rejected = accept(&owner, recipient.clone(), "below-actual", Some(total - 1)).await;
    let error = owner
        .prepare(native_request(rejected.clone(), recipient.clone(), 50_000))
        .await
        .err()
        .unwrap();
    assert!(error.is::<PreparationDeferred>(), "{error:#}");
    owner
        .store
        .call(move |s| {
            s.require_unstarted_preparation(&rejected)?;
            assert!(!s.operation_pending(&rejected)?);
            assert!(s.operation_facts(&rejected)?.is_none());
            Ok(())
        })
        .await
        .unwrap();

    let id = accept(&owner, recipient.clone(), "accepted", Some(total)).await;
    let treasury_id = owner.status().await.unwrap().treasury_id;
    let expected_identity = crate::network::IsolationId::direct_swap(&treasury_id, &id);
    let (entered, reached) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    owner.calculation_control = Some(tokio::sync::Mutex::new(publication::CalculationControl {
        entered: Some(entered),
        before_acceptance: false,
        release: wait,
        calls: calls.clone(),
        panic_calculation: false,
        panic_facts: false,
    }));
    {
        let preparation = owner.prepare(native_request(id.clone(), recipient, 50_000));
        tokio::pin!(preparation);
        tokio::select! {
            result = &mut preparation => panic!("preparation returned before accepted barrier: {}",result.is_ok()),
            result = reached => assert_eq!(result.unwrap(), id),
        }
    }
    // The waiter is gone; the existing blocking owner still owns its exact inputs.
    release.send(()).unwrap();
    let prepared = owner.finish_calculation().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(prepared.network_identity().unwrap(), expected_identity);
    assert_eq!(prepared.facts().unwrap().fee_zatoshis, actual_fee);
    assert_eq!(prepared.facts().unwrap().deadline, None);
    let original = prepared.bytes().to_vec();
    let transaction = submission::decode(&original).unwrap();
    assert!(transaction.transparent_bundle().is_none());
    assert!(transaction.ironwood_bundle().is_some());
    let saved = id.clone();
    owner
        .store
        .call(move |s| {
            assert!(!s.prepared_inputs(&saved)?.is_empty());
            assert!(s.funding_jobs()?.is_empty());
            assert!(s.direct_operation("owner", &saved)?.quote.is_none());
            Ok(())
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        treasury_id,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    let operation = id.clone();
    let bytes = original.clone();
    owner
        .store
        .call(move |s| {
            let restored = PreparedTransaction::load(s, &operation)?;
            assert_eq!(restored.bytes(), bytes);
            assert_eq!(restored.network_identity()?, expected_identity);
            assert_eq!(restored.facts().unwrap().deadline, None);
            Ok(())
        })
        .await
        .unwrap();
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    assert_eq!(
        owner
            .submit_prepared(id.clone(), &mut sender, false, &CancellationToken::new())
            .await
            .unwrap(),
        SubmissionOutcome::Accepted
    );
    net.chain.write().await.mine_mempool();
    net.chain.write().await.mine_empty_blocks(1);
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    assert!(matches!(
        owner
            .reconcile_for_observation(id.clone(), &mut sender, &CancellationToken::new())
            .await
            .unwrap(),
        TransactionPresence::Confirmed { .. }
    ));
    let saved = id.clone();
    owner
        .store
        .call(move |s| {
            assert_eq!(s.operation(&saved)?.submission, "CONFIRMED");
            assert_eq!(s.operation_facts(&saved)?.unwrap().deadline, None);
            assert!(s.funding_jobs()?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    assert!(net.chain.read().await.taddr_request_log().is_empty());
    owner.close().await.unwrap();
}

#[tokio::test]
async fn native_unsupported_receivers_and_unbound_work_fail_before_input_ownership() {
    let (_dir, net, mut owner, _) = funded_fixture(2).await;
    let native_ua = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    owner
        .client
        .as_mut()
        .unwrap()
        .generate_unified_address(ReceiverSelection::sapling_only(), zip32::AccountId::ZERO)
        .await
        .unwrap();
    let sapling_ua = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await
        .members()
        .find(|address| address["has_sapling"] == true && address["has_orchard"] == false)
        .unwrap()["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let zcash_keys::address::Address::Unified(sapling) =
        zcash_keys::address::Address::decode(&owner.network.chain(), &sapling_ua).unwrap()
    else {
        panic!("expected Sapling-only negative fixture")
    };
    let sapling_address = zcash_keys::address::Address::from(*sapling.sapling().unwrap())
        .encode(&owner.network.chain());
    let zcash_keys::address::Address::Unified(native) =
        zcash_keys::address::Address::decode(&owner.network.chain(), &native_ua).unwrap()
    else {
        panic!("expected Ironwood-compatible fixture")
    };
    let zcash_keys::address::Address::Transparent(transparent) =
        zcash_keys::address::Address::decode(&owner.network.chain(), DEPOSIT).unwrap()
    else {
        panic!("expected transparent negative fixture")
    };
    let mixed = zcash_keys::address::UnifiedAddress::from_receivers(
        native.orchard().copied(),
        None,
        Some(transparent),
        None,
        None,
    )
    .unwrap()
    .encode(&owner.network.chain());
    let tex = zcash_keys::address::Address::Tex([1; 20]).encode(&owner.network.chain());
    for recipient in [sapling_address, sapling_ua, mixed, DEPOSIT.to_owned(), tex] {
        assert!(crate::treasury::native_recipient(&owner.network.chain(), &recipient).is_err());
        let id = uuid::Uuid::new_v4().to_string();
        assert!(
            owner
                .prepare(native_request(id.clone(), recipient, 50_000))
                .await
                .is_err()
        );
        owner
            .store
            .call(move |s| {
                s.require_unstarted_preparation(&id)?;
                assert!(!s.operation_pending(&id)?);
                Ok(())
            })
            .await
            .unwrap();
    }
    let id = uuid::Uuid::new_v4().to_string();
    assert!(
        owner
            .prepare(native_request(id.clone(), native_ua, 50_000))
            .await
            .is_err(),
        "native work needs an accepted durable direct binding"
    );
    owner
        .store
        .call(move |s| {
            s.require_unstarted_preparation(&id)?;
            assert!(s.status()?.treasury_operations.is_empty());
            assert!(s.funding_jobs()?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    assert!(net.chain.read().await.taddr_request_log().is_empty());
    owner.close().await.unwrap();
}
