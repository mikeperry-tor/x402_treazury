//! Foreign acceptance and dry preview retain a source owner without NEAR availability.
use super::*;
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

struct ObservedSubmission {
    inner: submission::GrpcSubmission,
    submitted: tokio::sync::mpsc::UnboundedSender<()>,
}
impl TransactionSubmission for ObservedSubmission {
    async fn preflight(&mut self, tx: &PreparedTransaction, retry: bool) -> Result<()> {
        self.inner.preflight(tx, retry).await
    }
    async fn submit(&mut self, tx: BroadcastTransaction) -> Result<SubmissionOutcome> {
        let result = self.inner.submit(tx).await?;
        self.submitted.send(()).unwrap();
        Ok(result)
    }
    async fn lookup(&mut self, tx: &PreparedTransaction) -> Result<TransactionPresence> {
        self.inner.lookup(tx).await
    }
}

struct NoForeignSpend;
impl TransactionSubmission for NoForeignSpend {
    async fn submit(&mut self, _: BroadcastTransaction) -> Result<SubmissionOutcome> {
        panic!("admission and preview must not submit")
    }
    async fn lookup(&mut self, _: &PreparedTransaction) -> Result<TransactionPresence> {
        panic!("admission and preview must not inspect source transactions")
    }
}

#[tokio::test(flavor = "current_thread")]
async fn foreign_preview_acceptance_and_native_only_source_recovery_preserve_bound_work() {
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    // Keep this fixture's already-scanned SDK snapshot stable while checking dry
    // preview. Submission still uses the real synthetic-chain gRPC boundary.
    owner.sync_settings = None;
    let store = owner.store.clone();
    let treasury_id = owner.status().await.unwrap().treasury_id;
    let quote_calls = Arc::new(AtomicUsize::new(0));
    let status_calls = Arc::new(AtomicUsize::new(0));
    let calls = quote_calls.clone();
    let statuses = status_calls.clone();
    let app=Router::new()
        .route("/v0/tokens",get(||async{Json(json!([
            {"assetId":"nep141:zec.omft.near","blockchain":"zec","decimals":8,"symbol":"ZEC"},
            {"assetId":"foreign:usd","blockchain":"foreign","decimals":6,"symbol":"USD"}
        ]))}))
        .route("/v0/quote",post(move |Json(request):Json<Value>|{
            let calls=calls.clone();
            async move {
                calls.fetch_add(1,Ordering::SeqCst);
                assert_eq!(request["swapType"],"EXACT_OUTPUT");
                assert_eq!(request["amount"],"5000000");
                assert_eq!(request["recipient"],"customer-destination");
                let mut response=json!({"quoteRequest":request,"quote":{"amountIn":"50000","minAmountIn":"49500","amountOut":"5000000","minAmountOut":"5000000"}});
                if response["quoteRequest"]["dry"]==false {
                    response["quote"]["depositAddress"]=json!(DEPOSIT);
                    response["quote"]["deadline"]=response["quoteRequest"]["deadline"].clone();
                }
                Json(response)
            }
        }))
        .route("/v0/status",get(move ||{
            let statuses=statuses.clone();
            async move { statuses.fetch_add(1,Ordering::SeqCst); Json(json!({"status":"PENDING_DEPOSIT"})) }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let http = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (treasury, commands) = super::super::actor::channel();
    let actor_stop = CancellationToken::new();
    let actor = tokio::spawn(owner.run_commands(commands, NoForeignSpend, actor_stop.clone()));
    let service = Service::new(
        store.clone(),
        treasury.clone(),
        crate::rotation::near::NearClient::synthetic_at(&origin).unwrap(),
        serde_json::from_value(json!({"auto_fund":false,"poll_seconds":1})).unwrap(),
        30_000,
    );
    let config = Config {
        send: true,
        max_send_zec: Some("0.001".into()),
        ..Default::default()
    };
    let destination = json!({"asset_id":"foreign:usd","chain":"foreign","amount":"5000000","recipient":"customer-destination"});
    let before = store.call(|s| Ok(s.snapshot()?.0)).await.unwrap();
    let preview = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_swap_preview",
            destination.clone(),
        )
        .await
        .unwrap();
    assert_eq!(preview["executable"], false);
    assert!(preview["quote"]["deposit"].is_null());
    assert_eq!(
        store.call(|s| Ok(s.snapshot()?.0)).await.unwrap(),
        before,
        "dry preview must not derive an address or save an SDK snapshot"
    );
    store
        .call(|s| {
            assert!(s.direct_request("owner", "send", "accepted").is_err());
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    let mut request = destination.clone();
    request["request_id"] = json!("accepted");
    request["max_source_zatoshis"] = json!("50000");
    let accepted = service
        .invoke("owner", &config, "x402_treazury_swap_send", request.clone())
        .await
        .unwrap();
    let operation = accepted["operation"]["id"].as_str().unwrap().to_owned();
    assert_eq!(accepted["operation"]["quote"]["request"]["dry"], false);
    assert_eq!(accepted["operation"]["quote"]["deposit"], DEPOSIT);
    assert_eq!(accepted["operation"]["quote"]["input"], "50000");
    let duplicate = service
        .invoke("owner", &config, "x402_treazury_swap_send", request.clone())
        .await
        .unwrap();
    assert_eq!(duplicate["operation"]["id"], operation);
    assert_eq!(
        quote_calls.load(Ordering::SeqCst),
        2,
        "identical acceptance may not mint another executable quote"
    );
    let mut conflicting = request.clone();
    conflicting["recipient"] = json!("different-recipient");
    assert!(
        service
            .invoke("owner", &config, "x402_treazury_swap_send", conflicting)
            .await
            .is_err()
    );
    let mut denied = request;
    denied["request_id"] = json!("too-small");
    denied["max_source_zatoshis"] = json!("49999");
    assert!(
        service
            .invoke("owner", &config, "x402_treazury_swap_send", denied)
            .await
            .is_err()
    );
    assert_eq!(quote_calls.load(Ordering::SeqCst), 3);
    store
        .call(|s| {
            assert!(
                s.direct_request("owner", "send", "too-small")?
                    .authority
                    .is_none()
            );
            assert!(!s.status()?.outgoing_pending);
            assert!(s.funding_jobs()?.is_empty());
            assert!(s.status()?.pools.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    // Reopen accepted work without NEAR. The first actor had no scan transport
    // while verifying dry preview; source preparation needs its real SDK sync.
    actor_stop.cancel();
    drop((service, treasury, store));
    actor.await.unwrap().unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        treasury_id.clone(),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    let store = owner.store.clone();
    let (treasury, commands) = super::super::actor::channel();
    let (submitted, mut broadcast) = tokio::sync::mpsc::unbounded_channel();
    let sender = ObservedSubmission {
        inner: submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        submitted,
    };
    let actor_stop = CancellationToken::new();
    let actor = tokio::spawn(owner.run_commands(commands, sender, actor_stop.clone()));
    let source_only = Service::native(store.clone(), treasury.clone(), 30_000, 1);
    let service_stop = CancellationToken::new();
    let progress = tokio::spawn({
        let service = source_only.clone();
        let stop = service_stop.clone();
        async move { service.run(stop).await }
    });
    // Fixture hang guard; production source work retains its durable owner.
    assert!(
        tokio::time::timeout(Duration::from_secs(180), broadcast.recv())
            .await
            .expect("persisted foreign source failed to submit without a NEAR backend")
            .is_some()
    );
    treasury.preview_address().await.unwrap(); // Command barrier after journal publication.
    service_stop.cancel();
    progress.await.unwrap().unwrap();
    assert!(
        broadcast.try_recv().is_err(),
        "source owner must not duplicate submission"
    );
    assert_eq!(quote_calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        status_calls.load(Ordering::SeqCst),
        0,
        "native-only recovery cannot perform provider I/O"
    );
    let saved = operation.clone();
    store
        .call(move |s| {
            let prepared = PreparedTransaction::load(s, &saved)?;
            assert_eq!(
                prepared.network_identity()?,
                crate::network::IsolationId::direct_swap(&treasury_id, &saved)
            );
            assert_eq!(prepared.facts().unwrap().amount_zatoshis, 50_000);
            assert_eq!(s.operation_recipient(&saved)?.as_deref(), Some(DEPOSIT));
            assert_eq!(s.operation(&saved)?.submission, "BROADCAST");
            assert!(s.funding_jobs()?.is_empty());
            assert!(s.status()?.pools.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    assert!(net.chain.read().await.taddr_request_log().is_empty());
    actor_stop.cancel();
    drop((source_only, treasury, store));
    actor.await.unwrap().unwrap();
    http.abort();
}
