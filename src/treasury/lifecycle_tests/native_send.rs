//! Native direct sends use real SDK proofs and the same durable source owner.
use super::*;
use crate::rotation::store::DirectSendAuthority;
use serde_json::json;

fn native_request(id: String, recipient: String, amount: u64) -> PrepareRequest {
    PrepareRequest {
        managed_funding: None,
        operation_id: id,
        pool_id: None,
        deadline: None,
        recipient,
        amount_zatoshis: amount,
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
                "amount":"50000","recipient":recipient}),
                )?
                .0;
            let operation = s.accept_direct_send(
                &operation.id,
                None,
                DirectSendAuthority {
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
async fn native_self_payment_enforces_actual_total_cap_and_confirms() {
    let (_dir, net, mut owner, endpoint) = funded_fixture(2).await;
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
    let prepared = owner
        .prepare(native_request(id.clone(), recipient, 50_000))
        .await
        .unwrap();
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
async fn native_invalid_recipient_and_unbound_work_fail_before_input_ownership() {
    let (_dir, mut owner) = unfunded_fixture().await;
    let native_ua = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    // Binding validation must run before any indexer request. A missing sync
    // configuration would reject for the wrong reason.
    owner.configure_sync(SyncSettings::new("http://127.0.0.1:1".into(), 2).unwrap());
    for (recipient, expected) in [
        (DEPOSIT.to_owned(), "native Zcash recipient requires"),
        (
            native_ua,
            "native send requires an accepted direct operation",
        ),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        let error = owner
            .prepare(native_request(id.clone(), recipient, 50_000))
            .await
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
        owner
            .store
            .call(move |s| {
                s.require_unstarted_preparation(&id)?;
                assert!(!s.operation_pending(&id)?);
                assert!(s.status()?.treasury_operations.is_empty());
                assert!(s.funding_jobs()?.is_empty());
                Ok(())
            })
            .await
            .unwrap();
    }
    owner.close().await.unwrap();
}
