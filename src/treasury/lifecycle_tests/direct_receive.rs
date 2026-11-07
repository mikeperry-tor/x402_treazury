//! Direct invoices receive into their durable shielded SDK address without funding work.
use super::*;

#[tokio::test]
async fn direct_invoice_receipt_survives_reopen_and_finishes_observation_after_scan() {
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let operation = owner
        .store
        .call(|s| {
            Ok(s.reserve_direct(
                "owner",
                "receive",
                "invoice",
                serde_json::json!({"amount":"40000"}),
            )?
            .0
            .id)
        })
        .await
        .unwrap();
    let address = owner.direct_address(operation.clone()).await.unwrap();
    let zcash_keys::address::Address::Unified(receivers) =
        zcash_keys::address::Address::decode(&owner.network.chain(), &address).unwrap()
    else {
        panic!("direct receive requires shielded receivers");
    };
    assert!(receivers.orchard().is_some() && receivers.sapling().is_none());
    assert!(receivers.transparent().is_none());
    let saved = operation.clone();
    owner.store.call(move |s| {
        s.save_direct_quote(&saved, serde_json::json!({"depositAddress":"foreign-chain-deposit"}))?;
        s.save_direct_status(&saved, serde_json::json!({"provider":{"status":"SUCCESS"},"observed_at":crate::rotation::base::now()?}))?;
        assert!(s.treasury_observation_needed(&[])?);
        Ok(())
    }).await.unwrap();
    let receipt = net
        .funding_transaction(vec![(&address, 40_000, None)])
        .await;
    let receipt_id = submission::decode(&receipt).unwrap().txid();
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![receipt]);
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
    let identity = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let mut restored = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    assert_eq!(
        restored.direct_address(operation.clone()).await.unwrap(),
        address
    );
    restored.configure_sync(SyncSettings::new(endpoint, 2).unwrap());
    restored.sync_once(&CancellationToken::new()).await.unwrap();
    {
        let chain = net.chain.read().await;
        assert!(chain.taddr_request_log().is_empty());
        for rpc in [Rpc::TaddressTxids, Rpc::TaddressBalance, Rpc::AddressUtxos] {
            assert_eq!(chain.faults.pending(rpc), 1);
        }
    }
    let status = restored.status().await.unwrap();
    let sync = status.sync.unwrap();
    assert_eq!(sync.confirmed_shielded_zatoshis, 1_040_000);
    assert_eq!(sync.spendable_shielded_zatoshis, 1_040_000);
    assert!(status.funding_jobs.is_empty() && status.treasury_operations.is_empty());
    let wallet = restored.client.as_ref().unwrap().wallet().read().await;
    assert!(
        wallet
            .wallet_transactions
            .get(&receipt_id)
            .unwrap()
            .status()
            .get_confirmed_height()
            .is_some()
    );
    drop(wallet);
    restored
        .store
        .call(move |s| {
            assert!(!s.treasury_observation_needed(&[])?);
            assert_eq!(
                s.direct_operation("owner", &operation)?.status.unwrap()["provider"]["status"],
                "SUCCESS"
            );
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    restored.close().await.unwrap();
}

struct NoDirectSpend;
impl TransactionSubmission for NoDirectSpend {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("receive must never submit ZEC")
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("receive must never inspect an outgoing transaction")
    }
}

#[tokio::test]
async fn receive_service_http_actor_and_store_preserve_one_invoice_across_duplicates_and_restart() {
    use crate::direct_swap::{Config, Service};
    use axum::{
        Json, Router,
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let (dir, owner) = unfunded_fixture().await;
    let treasury_id = owner.status().await.unwrap().treasury_id;
    let store = owner.store.clone();
    let quote_calls = Arc::new(AtomicUsize::new(0));
    let quote_entered = Arc::new(tokio::sync::Notify::new());
    let release_quote = Arc::new(tokio::sync::Notify::new());
    let calls = quote_calls.clone();
    let entered = quote_entered.clone();
    let release = release_quote.clone();
    let app = Router::new()
        .route("/v0/tokens", get(|| async {
            Json(json!([
                {"assetId":"nep141:zec.omft.near","blockchain":"zec","decimals":8,"symbol":"ZEC"},
                {"assetId":"foreign:usd","blockchain":"foreign","decimals":6,"symbol":"USD"}
            ]))
        }))
        .route("/v0/quote", post(move |Json(request): Json<Value>| {
            let calls = calls.clone();
            let entered = entered.clone();
            let release = release.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                entered.notify_one();
                release.notified().await;
                Json(json!({"quoteRequest":request,"quote":{
                    "amountIn":"5000000","amountOut":"40000","minAmountOut":"40000",
                    "depositAddress":"foreign-deposit","depositMemo":"mandatory-memo",
                    "deadline":request["deadline"]
                }}))
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let http = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (treasury, commands) = super::super::actor::channel();
    let actor_stop = CancellationToken::new();
    let actor = tokio::spawn(owner.run_commands(commands, NoDirectSpend, actor_stop.clone()));
    let service = Service::new(
        store.clone(),
        treasury.clone(),
        crate::rotation::near::NearClient::synthetic_at(&origin).unwrap(),
        serde_json::from_value(json!({})).unwrap(),
    );
    let config = Config {
        receive: true,
        send: false,
        ..Default::default()
    };
    let arguments = json!({"asset_id":"foreign:usd","chain":"foreign","amount":"5000000","refund_address":"payer-refund","request_id":"one-invoice"});
    let first_service = service.clone();
    let first_config = config.clone();
    let first_arguments = arguments.clone();
    let mut first = tokio::spawn(async move {
        first_service
            .invoke(
                "owner",
                &first_config,
                "x402_treazury_payment_request_create",
                first_arguments,
            )
            .await
    });
    tokio::select! {
        _ = quote_entered.notified() => {},
        result = &mut first => panic!("invoice creation finished before quote fixture: {result:?}"),
    }
    let during_quote = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_payment_request_create",
            arguments.clone(),
        )
        .await
        .unwrap();
    assert_eq!(during_quote["operation"]["state"], "unknown_create");
    assert!(during_quote["operation"]["quote"].is_null());
    release_quote.notify_one();
    let accepted = first.await.unwrap().unwrap();
    let duplicate = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_payment_request_create",
            arguments.clone(),
        )
        .await
        .unwrap();
    let id = accepted["operation"]["id"].as_str().unwrap().to_owned();
    assert_eq!(during_quote["operation"]["id"], id);
    assert_eq!(duplicate["operation"], accepted["operation"]);
    assert_eq!(accepted["operation"]["quote"]["deposit"], "foreign-deposit");
    assert_eq!(
        accepted["operation"]["quote"]["deposit_memo"],
        "mandatory-memo"
    );
    assert_eq!(quote_calls.load(Ordering::SeqCst), 1);
    let mut conflicting = arguments;
    conflicting["amount"] = json!("6000000");
    assert!(
        service
            .invoke(
                "owner",
                &config,
                "x402_treazury_payment_request_create",
                conflicting
            )
            .await
            .is_err()
    );
    assert!(
        service
            .invoke(
                "other-owner",
                &config,
                "x402_treazury_swap_status",
                json!({"operation_id":id})
            )
            .await
            .is_err()
    );
    assert_eq!(quote_calls.load(Ordering::SeqCst), 1);
    actor_stop.cancel();
    drop(service);
    drop(treasury);
    drop(store);
    actor.await.unwrap().unwrap();
    http.abort();
    let restored = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        treasury_id,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    let saved_id = id.clone();
    let persisted = restored
        .store
        .call(move |s| s.direct_operation("owner", &saved_id))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(persisted).unwrap(),
        accepted["operation"]
    );
    let (unused_handle, unused_commands) = super::super::actor::channel();
    let cached_service = Service::new(
        restored.store.clone(),
        unused_handle,
        crate::rotation::near::NearClient::synthetic_at(&origin).unwrap(),
        serde_json::from_value(json!({})).unwrap(),
    );
    let after_restart = cached_service
        .invoke(
            "owner",
            &Config::default(),
            "x402_treazury_swap_status",
            json!({"operation_id":id}),
        )
        .await
        .unwrap();
    assert_eq!(after_restart["operation"], accepted["operation"]);
    drop(cached_service);
    drop(unused_commands);
    assert!(restored.status().await.unwrap().funding_jobs.is_empty());
    assert!(
        restored
            .status()
            .await
            .unwrap()
            .treasury_operations
            .is_empty()
    );
    restored.close().await.unwrap();
}
