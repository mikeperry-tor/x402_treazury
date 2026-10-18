//! Real funding adapter/read scheduler and treasury command owner on local endpoints.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    funding::{Backend, FundingWorker},
    near::{NearClient, Quote},
    store::funding::FundingPhase,
    transaction::{PreparedTransaction, TransactionFacts},
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Endpoints {
    occupied: CancellationToken,
    release: CancellationToken,
    recipient: String,
    quote: Quote,
    status_reads: Arc<AtomicUsize>,
    credit_reads: Arc<AtomicUsize>,
    timestamp: u64,
}
async fn grpc(State(f): State<Endpoints>) -> StatusCode {
    f.occupied.cancel();
    f.release.cancelled().await;
    StatusCode::SERVICE_UNAVAILABLE
}
async fn status(State(f): State<Endpoints>) -> Json<Value> {
    f.occupied.cancelled().await;
    f.status_reads.fetch_add(1, Ordering::SeqCst);
    Json(json!({"status":"SUCCESS","quoteResponse":f.quote.response}))
}
async fn base(State(f): State<Endpoints>, Json(v): Json<Value>) -> Json<Value> {
    f.occupied.cancelled().await;
    let result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = v["params"][0].as_str().unwrap();
            let height = if tag == "latest" {
                100
            } else {
                u64::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap()
            };
            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",f.timestamp)})
        }
        "eth_call" => {
            assert_eq!(v["params"][1], "latest");
            let data = v["params"][0]["data"].as_str().unwrap();
            let amount = if data.starts_with("0x70a08231")
                && data
                    .to_lowercase()
                    .ends_with(f.recipient.trim_start_matches("0x").to_lowercase().as_str())
            {
                f.credit_reads.fetch_add(1, Ordering::SeqCst);
                5_000_000u64
            } else {
                0
            };
            json!(format!("0x{amount:064x}"))
        }
        other => panic!("unexpected Base request {other}"),
    };
    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
}
fn quote(job: &crate::rotation::store::funding::FundingJob, now: u64) -> Quote {
    let request = crate::rotation::near::request(
        &crate::rotation::near::Assets {
            origin: "nep141:zec.omft.near".into(),
            destination: crate::rotation::near::USDC.into(),
        },
        &job.recipient,
        "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F",
        &job.target,
        "public",
        100,
        now + 7200,
        false,
    )
    .unwrap();
    let response = json!({"quoteRequest":request,"quote":{"amountIn":"100000","amountOut":job.target,"amountInUsd":"5","depositAddress":"t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F","deadline":request["deadline"]}});
    Quote {
        request,
        response,
        input: 100_000,
        deadline: now + 7200,
        deposit: Some("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into()),
    }
}
#[tokio::test]
async fn occupied_treasury_sync_preserves_source_owner_while_status_and_credit_commit() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let owner=crate::treasury::Treasury::create(state.clone(),key.clone(),2_000_000,Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
    crate::rotation::test_capacity::treasury_pair(
        &owner,
        "research",
        "5",
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0,
    )
    .await
    .unwrap();
    let store = owner.store_handle();
    let jobs = store.call(|s| s.funding_jobs()).await.unwrap();
    let first = jobs[0].clone();
    let second = jobs[1].clone();
    let instant = crate::rotation::base::now().unwrap();
    let accepted = quote(&first, instant);
    let waiting = quote(&second, instant);
    let original = first.clone();
    let original_quote = serde_json::to_vec(&accepted).unwrap();
    let waiting_id = second.id.clone();
    let waiting_quote = serde_json::to_vec(&waiting).unwrap();
    store
        .call(move |s| {
            s.save_funding_quote(&original.id, &original_quote)?;
            s.advance_funding(&original.id, FundingPhase::Quoted, FundingPhase::Preparing)?;
            s.reserve(
                &original.operation_id,
                Some(&original.pool_id),
                u32::try_from(instant / 86400)?,
                110_000,
            )?;
            let (revision, snapshot) = s.snapshot()?;
            s.prepare_with_facts(
                &original.operation_id,
                revision,
                &snapshot,
                b"unfunded-original-prepared-bytes",
                Some(TransactionFacts {
                    txid: "fixture-source".into(),
                    expiry_height: 3_000_000,
                    amount_zatoshis: 100_000,
                    fee_zatoshis: 10_000,
                    deadline: instant + 7200,
                }),
            )?;
            let attempt = s.request_broadcast(&original.operation_id, instant, 2_000_000, false)?;
            s.broadcast_result(&original.operation_id, attempt, false)?;
            s.advance_funding(
                &original.id,
                FundingPhase::Prepared,
                FundingPhase::DepositPending,
            )?;
            s.save_funding_quote(&waiting_id, &waiting_quote)?;
            Ok(())
        })
        .await
        .unwrap();
    // Fixture journaling advanced the durable snapshot. Reopen the real owner
    // at that exact revision, as production does after an accepted preparation.
    let treasury_id = store.call(|s| Ok(s.status()?.treasury_id)).await.unwrap();
    drop(store);
    owner.close().await.unwrap();
    let mut owner = crate::treasury::Treasury::open(state.clone(), key.clone(), treasury_id)
        .await
        .unwrap();
    let store = owner.store_handle();
    let f = Endpoints {
        occupied: CancellationToken::new(),
        release: CancellationToken::new(),
        recipient: first.recipient.clone(),
        quote: accepted,
        status_reads: Arc::default(),
        credit_reads: Arc::default(),
        timestamp: instant,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/base", post(base))
        .route("/v0/status", get(status))
        .fallback(grpc)
        .with_state(f.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    owner.configure_sync(crate::treasury::SyncSettings::new(origin.clone(), 2, 300).unwrap());
    let (handle, commands) = crate::treasury::actor::channel();
    let backend=Backend{
        treasury:handle,store:store.clone(),near:NearClient::test_at(&origin).unwrap(),base:BaseRpc::new(&format!("{origin}/base"),12,120).unwrap(),
        wallets:BTreeMap::from([("research".into(),serde_json::from_value(json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},"max_api_payment_usdc":"1","max_funding_spend_zec":"0.002","max_conversion_overhead_percent":5})).unwrap())]),
        funding:serde_json::from_value(json!({"base_rpc_url_env":"BASE","confidentiality":"public"})).unwrap(),max_network_fee:30_000,
    };
    let stop = CancellationToken::new();
    let worker = tokio::spawn(FundingWorker::new(store.clone(), backend, 1).run(stop.clone()));
    // Queue the worker's original-source reconcile before the actor's first
    // periodic sync tick. Its command branch is biased ahead of that tick.
    tokio::time::timeout(Duration::from_secs(10), async {
        while commands.test_is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let sender = crate::treasury::submission::GrpcSubmission::new(origin.clone(), origin).unwrap();
    let actor = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    tokio::time::timeout(Duration::from_secs(10), f.occupied.cancelled())
        .await
        .unwrap();
    let job = first.id.clone();
    let operation = first.operation_id.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (job, operation) = (job.clone(), operation.clone());
            let facts = store
                .call(move |s| s.funding_facts(&job, &operation))
                .await
                .unwrap();
            if facts.initial_credit_recorded && facts.provider_success {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        !actor.is_finished(),
        "wallet sync must still own the treasury"
    );
    assert!(f.status_reads.load(Ordering::SeqCst) > 0 && f.credit_reads.load(Ordering::SeqCst) > 0);
    let original = first.clone();
    let pending = second.clone();
    store
        .call(move |s| {
            assert_eq!(
                s.funding_job(&original.id)?.unwrap().phase,
                FundingPhase::VerifyingCredit
            );
            let operation = s.operation(&original.operation_id)?;
            assert_eq!(operation.attempts, 1);
            assert_ne!(operation.submission, "CONFIRMED");
            assert_eq!(
                PreparedTransaction::load(s, &original.operation_id)?.bytes(),
                b"unfunded-original-prepared-bytes"
            );
            assert_eq!(
                s.funding_job(&pending.id)?.unwrap().phase,
                FundingPhase::Quoted
            );
            s.require_unstarted_preparation(&pending.operation_id)?;
            Ok(())
        })
        .await
        .unwrap();
    let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
    let accounting: (i64, i64) = db
        .query_row(
            "SELECT reserved,consumed FROM budget_entries WHERE id=?1",
            [&first.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        accounting,
        (110_000, 0),
        "destination/provider progress cannot release source reservation"
    );
    drop(db);
    stop.cancel();
    f.release.cancel();
    tokio::time::timeout(Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(store);
    tokio::time::timeout(Duration::from_secs(10), actor)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    server.abort();
    let reopened = crate::rotation::store::Store::open(
        &state,
        &key,
        &crate::rotation::store::status(&state).unwrap().treasury_id,
    )
    .unwrap();
    assert!(reopened.operation_pending(&first.operation_id).unwrap());
    assert_eq!(
        PreparedTransaction::load(&reopened, &first.operation_id)
            .unwrap()
            .bytes(),
        b"unfunded-original-prepared-bytes"
    );
    assert_eq!(reopened.operation(&first.operation_id).unwrap().attempts, 1);
    assert_eq!(reopened.funding_jobs().unwrap().len(), 2);
}
