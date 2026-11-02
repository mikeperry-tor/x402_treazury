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
    provider_status: &'static str,
}
async fn grpc(State(f): State<Endpoints>) -> StatusCode {
    f.occupied.cancel();
    f.release.cancelled().await;
    StatusCode::SERVICE_UNAVAILABLE
}
async fn status(State(f): State<Endpoints>) -> Json<Value> {
    f.occupied.cancelled().await;
    f.status_reads.fetch_add(1, Ordering::SeqCst);
    Json(json!({"status":f.provider_status,"quoteResponse":f.quote.response}))
}
async fn base(State(f): State<Endpoints>, Json(v): Json<Value>) -> Json<Value> {
    f.occupied.cancelled().await;
    let result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = v["params"][0].as_str().unwrap();
            let height = if matches!(tag, "latest" | "finalized") {
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
        sizing_preview: None,
        request,
        response,
        input: 100_000,
        deadline: now + 7200,
        deposit: Some("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into()),
    }
}
fn funding_backend(
    handle: crate::treasury::actor::TreasuryHandle,
    store: crate::rotation::store::StoreHandle,
    origin: &str,
) -> Backend {
    Backend {
        treasury:handle,store,near:NearClient::test_at(origin).unwrap(),base:BaseRpc::new(&format!("{origin}/base"), 12).unwrap(),
        wallets:BTreeMap::from([("research".into(),serde_json::from_value(json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},"max_api_payment_usdc":"1","max_funding_spend_zec":"0.002","max_conversion_overhead_percent":5})).unwrap())]),
        funding:serde_json::from_value(json!({"base_rpc_url_env":"BASE","confidentiality":"public"})).unwrap(),max_network_fee:30_000,
    }
}

async fn prepared_bootstrap_owner() -> (
    tempfile::TempDir,
    crate::treasury::Treasury,
    crate::rotation::store::funding::FundingJob,
    crate::rotation::store::funding::FundingJob,
    Quote,
) {
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
            s.accept_preparation(
                &original.operation_id,
                Some(&original.pool_id),
                110_000,
                None,
            )?;
            let (revision, snapshot) = s.snapshot()?;
            s.prepare_with_facts(
                &original.operation_id,
                revision,
                &snapshot,
                crate::rotation::store::test_support::transaction_raw(3_000_000).as_slice(),
                Some(TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(3_000_000),
                    expiry_height: 3_000_000,
                    amount_zatoshis: 100_000,
                    fee_zatoshis: 10_000,
                    deadline: Some(instant + 7200),
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
    let owner = crate::treasury::Treasury::open(state.clone(), key.clone(), treasury_id)
        .await
        .unwrap();
    (dir, owner, first, second, accepted)
}

#[tokio::test]
async fn occupied_treasury_sync_does_not_block_proven_funding_completion() {
    let (dir, mut owner, first, second, accepted) = prepared_bootstrap_owner().await;
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let store = owner.store_handle();
    let instant = crate::rotation::base::now().unwrap();
    let f = Endpoints {
        occupied: CancellationToken::new(),
        release: CancellationToken::new(),
        recipient: first.recipient.clone(),
        quote: accepted,
        status_reads: Arc::default(),
        credit_reads: Arc::default(),
        timestamp: instant,
        provider_status: "SUCCESS",
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/base", post(base))
        .route("/v0/status", get(status))
        .fallback(grpc)
        .with_state(f.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    owner.configure_sync(crate::treasury::SyncSettings::new(origin.clone(), 2).unwrap());
    let (handle, commands) = crate::treasury::actor::channel();
    let backend = funding_backend(handle, store.clone(), &origin);
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
                FundingPhase::Complete
            );
            let operation = s.operation(&original.operation_id)?;
            assert_eq!(operation.attempts, 1);
            assert_eq!(operation.submission, "UNKNOWN");
            assert!(!s.operation_pending(&original.operation_id)?);
            assert!(PreparedTransaction::load(s, &original.operation_id).is_err());
            assert_eq!(
                s.prepared_bytes(&original.operation_id)?.as_slice(),
                crate::rotation::store::test_support::transaction_raw(3_000_000).as_slice()
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
    let resolved: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1 AND state='RESOLVED' AND preparation IS NOT NULL)",[&first.operation_id],|r|r.get(0)).unwrap();
    assert!(
        resolved,
        "bound fulfillment closes local source work while preserving its evidence"
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
    assert!(!reopened.operation_pending(&first.operation_id).unwrap());
    assert!(PreparedTransaction::load(&reopened, &first.operation_id).is_err());
    assert_eq!(
        reopened.funding_job(&first.id).unwrap().unwrap().phase,
        FundingPhase::Complete
    );
    assert_eq!(
        reopened
            .prepared_bytes(&first.operation_id)
            .unwrap()
            .as_slice(),
        crate::rotation::store::test_support::transaction_raw(3_000_000).as_slice()
    );
    assert_eq!(reopened.operation(&first.operation_id).unwrap().attempts, 1);
    assert_eq!(
        reopened.operation(&first.operation_id).unwrap().submission,
        "UNKNOWN"
    );
    assert_eq!(reopened.funding_jobs().unwrap().len(), 2);
}

#[tokio::test]
async fn pending_bootstrap_completes_captured_credit_while_source_stays_owned() {
    let (dir, mut owner, first, second, accepted) = prepared_bootstrap_owner().await;
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let store = owner.store_handle();
    let credited = second.clone();
    let pending = first.clone();
    store
        .call(move |s| {
            s.record_credit(&credited.wallet_id, &credited.target, "fixture", 100)?;
            assert!(
                s.funding_facts(&credited.id, &credited.operation_id)?
                    .initial_credit_recorded
            );
            assert!(
                !s.funding_facts(&pending.id, &pending.operation_id)?
                    .initial_credit_recorded
            );
            assert!(s.operation_pending(&pending.operation_id)?);
            Ok(())
        })
        .await
        .unwrap();
    let names = BTreeSet::from(["research".to_owned()]);
    let attempts = BTreeMap::from([("research".to_owned(), 3)]);
    let captured = capture_startup(&store, &names).await.unwrap();
    assert_eq!(captured["research"].len(), 2);
    let captured_claims: Vec<_> = captured["research"]
        .iter()
        .map(|claim| (claim.slot.clone(), claim.generation))
        .collect();
    let original = first.clone();
    let (original_quote, original_identity) = store
        .call(move |s| {
            Ok((
                s.funding_quote_for_operation(&original.id, &original.operation_id)?,
                PreparedTransaction::load(s, &original.operation_id)?.network_identity()?,
            ))
        })
        .await
        .unwrap();
    assert!(
        !check_startup(&store, &names, &attempts, &captured, false)
            .await
            .unwrap()
    );

    let f = Endpoints {
        occupied: CancellationToken::new(),
        release: CancellationToken::new(),
        recipient: first.recipient.clone(),
        quote: accepted,
        status_reads: Arc::default(),
        credit_reads: Arc::default(),
        timestamp: crate::rotation::base::now().unwrap(),
        provider_status: "PROCESSING",
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/base", post(base))
        .route("/v0/status", get(status))
        .fallback(grpc)
        .with_state(f.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    owner.configure_sync(crate::treasury::SyncSettings::new(origin.clone(), 2).unwrap());
    let (handle, commands) = crate::treasury::actor::channel();
    let backend = funding_backend(handle, store.clone(), &origin);
    let sender = crate::treasury::submission::GrpcSubmission::new(origin.clone(), origin).unwrap();
    let initialized = InitializedWallets {
        wallets: BTreeMap::new(),
        managed_pools: Vec::new(),
        runtime: ManagedRuntime::Treasury(Box::new(TreasuryRuntime {
            owner,
            funding: (
                commands,
                sender,
                FundingWorker::new(store.clone(), backend, 1),
            ),
        })),
    };
    let stop = CancellationToken::new();
    let task = tokio::spawn(supervise(initialized, names, attempts, stop.clone()));
    tokio::time::timeout(Duration::from_secs(10), f.occupied.cancelled())
        .await
        .unwrap();
    // The real worker starts only after the supervisor captures incomplete claims.
    // Its credit lane can finish while an accepted source command owns the actor.
    let pending = first.clone();
    let credited = second.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (pending, credited) = (pending.clone(), credited.clone());
            let ready = store
                .call(move |s| {
                    let first = s.funding_facts(&pending.id, &pending.operation_id)?;
                    let second = s.funding_facts(&credited.id, &credited.operation_id)?;
                    if !first.initial_credit_recorded || !second.initial_credit_recorded {
                        return Ok(false);
                    }
                    assert!(!first.provider_success);
                    assert!(s.operation_pending(&pending.operation_id)?);
                    assert_eq!(s.operation(&pending.operation_id)?.attempts, 1);
                    assert_eq!(s.operation(&pending.operation_id)?.submission, "UNKNOWN");
                    assert_eq!(
                        s.prepared_bytes(&pending.operation_id)?.as_slice(),
                        crate::rotation::store::test_support::transaction_raw(3_000_000).as_slice()
                    );
                    Ok(true)
                })
                .await
                .unwrap();
            if ready && f.status_reads.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(f.credit_reads.load(Ordering::SeqCst) > 0);
    drop(store);
    // Cross only the monitor's existing polling cadence after durable completion.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(10), stop.cancelled())
        .await
        .unwrap();
    f.release.cancel();
    let summary = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(summary.captured_completed_groups, ["research"]);
    assert!(summary.incomplete_captured_jobs.is_empty());
    assert_eq!(summary.inventory_ready_groups, ["research"]);
    server.abort();
    let reopened = crate::rotation::store::Store::open(
        &state,
        &key,
        &crate::rotation::store::status(&state).unwrap().treasury_id,
    )
    .unwrap();
    assert!(reopened.operation_pending(&first.operation_id).unwrap());
    assert_eq!(
        reopened
            .funding_quote_for_operation(&first.id, &first.operation_id)
            .unwrap(),
        original_quote
    );
    assert_eq!(
        PreparedTransaction::load(&reopened, &first.operation_id)
            .unwrap()
            .network_identity()
            .unwrap(),
        original_identity
    );
    assert_eq!(
        reopened
            .startup_groups(&["research".into()])
            .unwrap()
            .remove(0)
            .claims
            .into_iter()
            .map(|claim| (claim.slot, claim.generation))
            .collect::<Vec<_>>(),
        captured_claims
    );
    assert_eq!(
        PreparedTransaction::load(&reopened, &first.operation_id)
            .unwrap()
            .bytes(),
        crate::rotation::store::test_support::transaction_raw(3_000_000).as_slice()
    );
    assert_eq!(reopened.operation(&first.operation_id).unwrap().attempts, 1);
    assert_eq!(
        reopened.operation(&first.operation_id).unwrap().submission,
        "UNKNOWN"
    );
    assert_eq!(reopened.funding_jobs().unwrap().len(), 2);
    assert!(
        reopened
            .funding_facts(&first.id, &first.operation_id)
            .unwrap()
            .initial_credit_recorded
    );
    assert!(
        reopened
            .funding_facts(&second.id, &second.operation_id)
            .unwrap()
            .initial_credit_recorded
    );
}
