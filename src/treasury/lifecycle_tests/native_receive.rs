//! Native invoices use actual shielded SDK receivers without a bridge or spend.
use super::*;
use pepper_sync::wallet::{OutputInterface, WalletTransaction};
use serde_json::json;
use zingo_status::confirmation_status::ConfirmationStatus;

#[tokio::test]
async fn native_receive_resumes_reservation_and_tracks_partial_excess_receipts_after_reopen() {
    let (dir, net, owner, endpoint) = funded_fixture(2).await;
    let operation = owner
        .store
        .call(|s| {
            Ok(
                s.reserve_native("owner", "receive", "native", json!({"amount":"40000"}))?
                    .0
                    .id,
            )
        })
        .await
        .unwrap();
    let identity = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity.clone(),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let retry = owner
        .store
        .call(|s| {
            Ok(
                s.reserve_native("owner", "receive", "native", json!({"amount":"40000"}))?
                    .0
                    .id,
            )
        })
        .await
        .unwrap();
    assert_eq!(operation, retry);
    let address = owner.direct_address(operation.clone()).await.unwrap();
    assert_eq!(
        owner.direct_address(operation.clone()).await.unwrap(),
        address
    );
    let other = owner
        .store
        .call(|s| {
            Ok(
                s.reserve_native("owner", "receive", "other", json!({"amount":"12000"}))?
                    .0
                    .id,
            )
        })
        .await
        .unwrap();
    let other_address = owner.direct_address(other).await.unwrap();
    assert_ne!(address, other_address);
    let zcash_keys::address::Address::Unified(receivers) =
        zcash_keys::address::Address::decode(&owner.network.chain(), &address).unwrap()
    else {
        panic!("expected shielded UA");
    };
    assert!(receivers.transparent().is_none());
    assert!(receivers.orchard().is_some() && receivers.sapling().is_none());
    let partial = net
        .funding_transaction(vec![
            (&address, 20_000, None),
            (&other_address, 12_000, None),
        ])
        .await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![partial]);
        chain.mine_empty_blocks(2);
        for rpc in [Rpc::TaddressTxids, Rpc::TaddressBalance, Rpc::AddressUtxos] {
            chain.faults.inject(
                rpc,
                Fault::Fail(
                    tonic::Code::PermissionDenied,
                    "forbidden transparent address RPC".into(),
                ),
            );
        }
    }
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    let partial = owner.native_receipt(operation.clone()).await.unwrap();
    assert_eq!(partial["received_zatoshis"], 20_000);
    assert_eq!(partial["confirmed_zatoshis"], 20_000);
    assert_eq!(partial["spendable_zatoshis"], 20_000);
    let pending = operation.clone();
    owner
        .store
        .call(move |s| {
            s.save_native_status(&pending, partial)?;
            assert_eq!(s.direct_operation("owner", &pending)?.state, "pending");
            assert!(s.treasury_observation_needed(&[])?);
            Ok(())
        })
        .await
        .unwrap();
    let excess = net
        .funding_transaction(vec![(&address, 30_000, None)])
        .await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![excess]);
        chain.mine_empty_blocks(2);
    }
    owner.sync_once(&CancellationToken::new()).await.unwrap();
    let received = owner.native_receipt(operation.clone()).await.unwrap();
    assert_eq!(received["received_zatoshis"], 50_000);
    assert_eq!(received["confirmed_zatoshis"], 50_000);
    let completed = operation.clone();
    owner
        .store
        .call(move |s| {
            s.save_native_status(&completed, received)?;
            let operation = s.direct_operation("owner", &completed)?;
            assert_eq!(operation.state, "confirmed");
            assert!(operation.quote.is_none());
            assert!(s.funding_jobs()?.is_empty());
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    // Model a creation reply lost after reservation but before deriving its address.
    let service_terms = json!({"asset_id":"nep141:zec.omft.near","chain":"zec","amount":"70000","refund_address":null,"request_id":"service-restart"});
    let pending_terms = service_terms.clone();
    let reserved_service = owner
        .store
        .call(move |s| {
            Ok(
                s.reserve_native("owner", "receive", "service-restart", pending_terms)?
                    .0
                    .id,
            )
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
    let mut reopened = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity.clone(),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let historical = reopened.native_receipt(operation.clone()).await.unwrap();
    assert_eq!(historical["confirmed_zatoshis"], 50_000);
    assert_eq!(historical["spendable_zatoshis"], 50_000);
    {
        let chain = net.chain.read().await;
        assert!(chain.taddr_request_log().is_empty());
        for rpc in [Rpc::TaddressTxids, Rpc::TaddressBalance, Rpc::AddressUtxos] {
            assert_eq!(chain.faults.pending(rpc), 1);
        }
    }
    assert!(
        reopened
            .status()
            .await
            .unwrap()
            .treasury_operations
            .is_empty()
    );
    let cached_service = crate::direct_swap::Service::status_only(reopened.store.clone());
    let cached_paid = cached_service
        .invoke(
            "owner",
            &crate::direct_swap::Config::default(),
            "x402_treazury_swap_status",
            json!({"operation_id":operation}),
        )
        .await
        .unwrap();
    assert_eq!(cached_paid["zec_receipt"]["confirmed_zatoshis"], 50_000);
    assert_eq!(cached_paid["zec_receipt"]["spendable_zatoshis"], 50_000);
    drop(cached_service);
    let received_transactions = historical["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| zcash_protocol::TxId::from_hex(note["txid"].as_str().unwrap()).unwrap())
        .collect::<Vec<_>>();
    // Model facts the SDK has already learned: spending changes availability,
    // while the original receiver's historical receipts remain intact.
    {
        let mut wallet = reopened.client.as_ref().unwrap().wallet().write().await;
        let spent = zcash_protocol::TxId::from_bytes([95; 32]);
        for txid in &received_transactions {
            let transaction = wallet.wallet_transactions.get_mut(txid).unwrap();
            for note in transaction.ironwood_notes_mut() {
                note.set_spending_transaction(Some(spent));
            }
        }
        wallet.wallet_transactions.insert(
            spent,
            WalletTransaction::new_for_test(spent, ConfirmationStatus::Confirmed(25.into())),
        );
    }
    persist_sdk_facts(&mut reopened).await;
    let store = reopened.store.clone();
    let (handle, commands) = super::super::actor::channel();
    let stop = CancellationToken::new();
    let actor = tokio::spawn(reopened.run_commands(commands, NoNativeSpend, stop.clone()));
    // Any accidental bridge request reaches an unavailable endpoint and fails this flow.
    let service = crate::direct_swap::Service::new(
        store.clone(),
        handle.clone(),
        crate::rotation::near::NearClient::synthetic_at("http://127.0.0.1:1").unwrap(),
        serde_json::from_value(json!({})).unwrap(),
    );
    let config = crate::direct_swap::Config {
        receive: true,
        ..Default::default()
    };
    let spent_status = service
        .invoke(
            "owner",
            &crate::direct_swap::Config::default(),
            "x402_treazury_swap_status",
            json!({"operation_id":operation}),
        )
        .await
        .unwrap();
    assert_eq!(spent_status["operation"]["state"], "confirmed");
    assert_eq!(spent_status["zec_receipt"]["confirmed_zatoshis"], 50_000);
    assert_eq!(spent_status["zec_receipt"]["spendable_zatoshis"], 0);
    let accepted = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_payment_request_create",
            service_terms.clone(),
        )
        .await
        .unwrap();
    assert_eq!(accepted["operation"]["id"], reserved_service);
    assert_eq!(accepted["operation"]["route"], "native");
    assert!(accepted["operation"]["quote"].is_null());
    let recovered_address = accepted["payment_instructions"]["recipient"]
        .as_str()
        .unwrap();
    assert!(!recovered_address.is_empty());
    let same = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_payment_request_create",
            service_terms.clone(),
        )
        .await
        .unwrap();
    assert_eq!(same["operation"]["id"], reserved_service);
    assert_eq!(
        same["payment_instructions"],
        accepted["payment_instructions"]
    );
    let mut fresh_terms = service_terms;
    fresh_terms["request_id"] = json!("service-fresh");
    let fresh = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_payment_request_create",
            fresh_terms,
        )
        .await
        .unwrap();
    assert_ne!(fresh["operation"]["id"], accepted["operation"]["id"]);
    assert_ne!(
        fresh["payment_instructions"]["recipient"],
        accepted["payment_instructions"]["recipient"]
    );
    let empty_receipt = handle
        .native_receipt(reserved_service.clone())
        .await
        .unwrap();
    assert_eq!(empty_receipt["received_zatoshis"], 0);
    let cached = service
        .invoke(
            "owner",
            &crate::direct_swap::Config::default(),
            "x402_treazury_swap_status",
            json!({"kind":"receive","request_id":"service-restart"}),
        )
        .await
        .unwrap();
    assert_eq!(cached["operation"]["id"], reserved_service);
    store
        .call(|s| {
            assert!(s.funding_jobs()?.is_empty());
            assert!(s.status()?.treasury_operations.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    stop.cancel();
    drop(service);
    drop(handle);
    drop(store);
    actor.await.unwrap().unwrap();

    // Model a canonical reconstruction that has removed these incoming SDK
    // transactions. This is a saved-state regression, not consensus qualification.
    let mut reconstructed = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    {
        let mut wallet = reconstructed
            .client
            .as_ref()
            .unwrap()
            .wallet()
            .write()
            .await;
        for txid in received_transactions {
            wallet.wallet_transactions.remove(&txid);
        }
    }
    persist_sdk_facts(&mut reconstructed).await;
    let store = reconstructed.store.clone();
    let direct_changed = store.direct_changes();
    let (handle, commands) = super::super::actor::channel();
    let stop = CancellationToken::new();
    let actor = tokio::spawn(reconstructed.run_commands(commands, NoNativeSpend, stop.clone()));
    let service = crate::direct_swap::Service::new(
        store.clone(),
        handle.clone(),
        crate::rotation::near::NearClient::synthetic_at("http://127.0.0.1:1").unwrap(),
        serde_json::from_value(json!({})).unwrap(),
    );
    let reorg = service
        .invoke(
            "owner",
            &crate::direct_swap::Config::default(),
            "x402_treazury_swap_status",
            json!({"operation_id":operation}),
        )
        .await
        .unwrap();
    assert_eq!(reorg["operation"]["state"], "pending");
    assert_eq!(reorg["zec_receipt"]["confirmed_zatoshis"], 0);
    assert_eq!(reorg["zec_receipt"]["received_zatoshis"], 0);
    assert_eq!(reorg["zec_receipt"]["spendable_zatoshis"], 0);
    use futures_util::FutureExt;
    assert!(direct_changed.notified().now_or_never().is_some());
    store
        .call(move |s| {
            assert!(s.treasury_observation_needed(&[])?);
            assert!(
                s.direct_pending()?
                    .iter()
                    .any(|pending| pending.id == operation)
            );
            assert!(s.funding_jobs()?.is_empty());
            assert!(s.status()?.treasury_operations.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    stop.cancel();
    drop(service);
    drop(handle);
    drop(store);
    actor.await.unwrap().unwrap();
}

async fn persist_sdk_facts(owner: &mut Treasury) {
    let bytes = snapshot(owner.client.as_ref().unwrap()).await.unwrap();
    let expected = owner.revision;
    owner.revision = owner
        .store
        .call(move |s| s.save_snapshot(expected, &bytes))
        .await
        .unwrap();
}

struct NoNativeSpend;
impl TransactionSubmission for NoNativeSpend {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("native receive must not submit ZEC")
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("native receive must not inspect outgoing transactions")
    }
}
