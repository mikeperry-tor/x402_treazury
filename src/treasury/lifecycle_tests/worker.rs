//! Real scheduler + Backend + treasury actor; only the chains and bridge are local fixtures.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    funding::{Backend, FundingWorker},
    near::NearClient,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
struct Bridge {
    store: StoreHandle,
    deposits: Arc<Vec<String>>,
    quotes: Arc<Mutex<BTreeMap<String, Value>>>,
    quote_count: Arc<AtomicUsize>,
    base_calls: Arc<AtomicUsize>,
    quotes_entered: Arc<tokio::sync::Semaphore>,
    release_quotes: CancellationToken,
    signed: Arc<Mutex<Vec<Value>>>,
    destination: Arc<Mutex<DestinationChain>>,
    timestamp: u64,
}
struct DestinationChain {
    height: u64,
    credits: BTreeMap<String, u64>,
}
async fn quote(State(f): State<Bridge>, Json(request): Json<Value>) -> Json<Value> {
    if request["dry"] == true {
        assert_eq!(request["swapType"], "EXACT_OUTPUT");
        assert_eq!(request["slippageTolerance"], 0);
        return Json(
            json!({"quoteRequest":request,"quote":{"amountIn":"50000","amountOut":request["amount"],"amountInUsd":"5"}}),
        );
    }
    assert_eq!(request["swapType"], "EXACT_INPUT");
    assert_eq!(request["amount"], "50000");
    let index = f.quote_count.fetch_add(1, Ordering::SeqCst);
    let deposit = &f.deposits[index];
    let response = json!({"quoteRequest":request,"quote":{"amountIn":"50000","amountOut":"5000000","minAmountOut":"4875000","amountInUsd":"5","depositAddress":deposit,"deadline":request["deadline"]}});
    assert!(
        f.quotes
            .lock()
            .unwrap()
            .insert(deposit.clone(), response.clone())
            .is_none()
    );
    f.quotes_entered.add_permits(1);
    f.release_quotes.cancelled().await;
    Json(response)
}
async fn status(
    State(f): State<Bridge>,
    Query(query): Query<BTreeMap<String, String>>,
) -> Json<Value> {
    let response = f.quotes.lock().unwrap()[&query["depositAddress"]].clone();
    // Destination delivery is tied to the source journal, not just a quote.
    let recipient = response["quoteRequest"]["recipient"]
        .as_str()
        .unwrap()
        .to_owned();
    let delivered = credited(&f.store, recipient).await;
    Json(
        json!({"status":if delivered {"SUCCESS"} else {"PENDING_DEPOSIT"},"quoteResponse":response}),
    )
}
async fn credited(store: &StoreHandle, recipient: String) -> bool {
    store
        .call(move |s| {
            Ok(s.funding_jobs()?.into_iter().any(|j| {
                j.recipient.eq_ignore_ascii_case(&recipient)
                    && s.operation(&j.operation_id)
                        .is_ok_and(|o| o.submission == "CONFIRMED")
            }))
        })
        .await
        .unwrap()
}
async fn base(State(f): State<Bridge>, Json(v): Json<Value>) -> Json<Value> {
    f.base_calls.fetch_add(1, Ordering::SeqCst);
    let confirmed = f
        .store
        .call(|s| {
            Ok(s.funding_jobs()?
                .into_iter()
                .filter(|j| {
                    s.operation(&j.operation_id)
                        .is_ok_and(|o| o.submission == "CONFIRMED")
                })
                .map(|j| j.recipient.to_ascii_lowercase())
                .collect::<Vec<_>>())
        })
        .await
        .unwrap();
    // Mine destination delivery once per source confirmation. Historical block
    // hashes keep their original balances when another wallet is funded later.
    let mut chain = f.destination.lock().unwrap();
    for recipient in confirmed {
        if !chain.credits.contains_key(&recipient) {
            chain.height += 10;
            let height = chain.height - 2;
            chain.credits.insert(recipient, height);
        }
    }
    let result = match v["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = v["params"][0].as_str().unwrap();
            let h = if matches!(tag, "latest" | "finalized") {
                chain.height
            } else {
                u64::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap()
            };
            json!({"number":format!("0x{h:x}"),"hash":format!("0x{h:064x}"),"timestamp":format!("0x{:x}",f.timestamp)})
        }
        "eth_call" => {
            let data = v["params"][0]["data"].as_str().unwrap();
            let height = if let Some(tag) = v["params"][1].as_str() {
                assert_eq!(tag, "latest");
                chain.height
            } else {
                assert_eq!(v["params"][1]["requireCanonical"], true);
                let hash = v["params"][1]["blockHash"].as_str().unwrap();
                u64::from_str_radix(hash.trim_start_matches("0x"), 16).unwrap()
            };
            let recipient = format!("0x{}", &data[data.len() - 40..]).to_ascii_lowercase();
            let amount = if data.starts_with("0x70a08231")
                && chain
                    .credits
                    .get(&recipient)
                    .is_some_and(|at| *at <= height)
            {
                5_000_000u64
            } else {
                0
            };
            json!(format!("0x{amount:064x}"))
        }
        method => panic!("unexpected Base method {method}"),
    };
    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
}
async fn seller(State(f): State<Bridge>, headers: HeaderMap) -> Response {
    if let Some(signature) = headers.get("payment-signature") {
        let value: Value = serde_json::from_slice(&STANDARD.decode(signature).unwrap()).unwrap();
        f.signed.lock().unwrap().push(value);
        return "paid".into_response();
    }
    let challenge = json!({"x402Version":2,"resource":{"url":"https://fixture.invalid/pay","description":"fixture","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
    )
        .into_response()
}
struct CountedSender {
    inner: submission::GrpcSubmission,
    sends: Arc<AtomicUsize>,
    lookups: Arc<AtomicUsize>,
}
impl TransactionSubmission for CountedSender {
    async fn preflight(&mut self, tx: &PreparedTransaction, retry: bool) -> Result<()> {
        self.inner.preflight(tx, retry).await
    }
    async fn submit(&mut self, tx: BroadcastTransaction) -> Result<SubmissionOutcome> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        self.inner.submit(tx).await
    }
    async fn lookup(&mut self, tx: &PreparedTransaction) -> Result<TransactionPresence> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.inner.lookup(tx).await
    }
}
async fn tick(worker: &FundingWorker<Backend>, clock: (u64, tokio::time::Instant)) {
    let instant = clock.0 + clock.1.elapsed().as_secs();
    tokio::time::timeout(Duration::from_secs(90), worker.tick(instant))
        .await
        .unwrap()
        .unwrap();
}
async fn complete(
    worker: &FundingWorker<Backend>,
    net: &MockNet,
    count: usize,
    clock: (u64, tokio::time::Instant),
) {
    for _ in 0..80 {
        // Drain quote/preparation/submission handoffs at the same instant.
        for _ in 0..3 {
            tick(worker, clock).await;
        }
        let done = worker
            .store
            .call(move |s| {
                Ok(s.funding_jobs()?
                    .iter()
                    .filter(|j| j.phase == crate::rotation::store::funding::FundingPhase::Complete)
                    .count())
            })
            .await
            .unwrap();
        if done == count {
            return;
        }
        let mut chain = net.chain.write().await;
        chain.mine_mempool();
        chain.mine_empty_blocks(1);
        drop(chain);
        // Recurring Source/Status/Credit and the shared Base observer all use
        // this same Tokio elapsed time, including the poll=1 fixture's jitter.
        tokio::time::advance(Duration::from_secs(61)).await;
    }
    panic!(
        "funding did not complete: {:?}",
        worker.store.call(|s| s.funding_jobs()).await.unwrap()
    );
}

#[tokio::test]
async fn production_worker_bootstraps_concurrent_groups_and_funds_rotation() {
    let (dir, mut net, owner, endpoint) = funded_fixture(2).await;
    let mut external = net
        .client(
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
            None,
        )
        .await;
    let mut deposits = Vec::new();
    for _ in 0..3 {
        let (_, address) = external
            .generate_transparent_address(zip32::AccountId::ZERO, false)
            .await
            .unwrap();
        deposits.push(
            zcash_keys::address::Address::Transparent(address)
                .encode(&TreasuryNetwork::Synthetic.chain()),
        );
    }
    for name in ["alpha", "beta"] {
        owner
            .ensure_capacity_group(
                name.into(),
                crate::rotation::test_capacity::small_reserve("5"),
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0,
            )
            .await
            .unwrap();
    }
    // Reopen after historical shortages with funded, spendable treasury state.
    // Both groups must reach the real worker without repair or an attempt reset.
    let identity = owner.status().await.unwrap().treasury_id;
    owner
        .store_handle()
        .call(|s| {
            for name in ["alpha", "beta"] {
                s.fill_funding_vacancies(name)?;
            }
            let jobs = s.funding_jobs()?;
            assert_eq!(jobs.len(), 2);
            for job in jobs {
                s.defer_funding(
                    &job.id,
                    0,
                    Some("treasury_insufficient_spendable_funds; previous owner"),
                    false,
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
    let mut owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        identity,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 2).unwrap());
    let store = owner.store_handle();
    store
        .call(|s| {
            for name in ["alpha", "beta"] {
                let group = s.startup_groups(&[name.into()])?.remove(0);
                for claims in [None, Some(group.claims.as_slice())] {
                    assert!(s.startup_obstruction(name, 3, claims)?.is_none());
                }
            }
            assert!(s.status()?.sync.unwrap().spendable_shielded_zatoshis > 100_000);
            Ok(())
        })
        .await
        .unwrap();
    let f = Bridge {
        store: store.clone(),
        deposits: Arc::new(deposits),
        quotes: Default::default(),
        quote_count: Default::default(),
        base_calls: Default::default(),
        quotes_entered: Arc::new(tokio::sync::Semaphore::new(0)),
        release_quotes: CancellationToken::new(),
        signed: Default::default(),
        destination: Arc::new(Mutex::new(DestinationChain {
            height: 100,
            credits: BTreeMap::new(),
        })),
        timestamp: now().unwrap(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/v0/tokens",
            get(|| async {
                Json(
                    serde_json::from_str::<Value>(include_str!(
                        "../../../tests/fixtures/near/tokens.json"
                    ))
                    .unwrap(),
                )
            }),
        )
        .route("/v0/quote", post(quote))
        .route("/v0/status", get(status))
        .route("/base", post(base))
        .route("/pay", get(seller))
        .with_state(f.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = BaseRpc::new(&format!("{origin}/base"), 2).unwrap();
    let (handle, commands) = actor::channel();
    let backend = Backend { treasury: handle.clone(), store: store.clone(), near: NearClient::synthetic_at(&origin).unwrap(), base: base.clone(),
        wallets: ["alpha", "beta"].into_iter().map(|name| (name.into(), serde_json::from_value(json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},"max_api_payment_usdc":"1","max_funding_spend_zec":"0.002","max_conversion_overhead_percent":5})).unwrap())).collect(),
        funding: serde_json::from_value(json!({"base_rpc_url_env":"BASE","confidentiality":"public"})).unwrap(), max_network_fee: 30_000 };
    let worker = FundingWorker::new(store.clone(), backend, 1);
    let sends = Arc::new(AtomicUsize::new(0));
    let lookups = Arc::new(AtomicUsize::new(0));
    let sender = CountedSender {
        inner: submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        sends: sends.clone(),
        lookups: lookups.clone(),
    };
    let stop = CancellationToken::new();
    let clock = (now().unwrap(), tokio::time::Instant::now());
    let actor = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    let first = worker.clone();
    let first = tokio::spawn(async move { tick(&first, clock).await });
    tokio::time::timeout(Duration::from_secs(30), f.quotes_entered.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    let jobs = store.call(|s| s.funding_jobs()).await.unwrap();
    assert_eq!(jobs.len(), 2);
    let refunds = store
        .call(|s| {
            s.funding_jobs()?
                .iter()
                .map(|j| s.refund_address(&j.id).map(|a| a.unwrap()))
                .collect::<Result<Vec<_>>>()
        })
        .await
        .unwrap();
    assert_ne!(refunds[0], refunds[1]);
    let quotes = f.quotes.lock().unwrap().clone();
    for (job, refund) in jobs.iter().zip(&refunds) {
        let request = &quotes
            .values()
            .find(|q| q["quoteRequest"]["recipient"] == job.recipient)
            .unwrap()["quoteRequest"];
        assert_eq!(request["refundTo"], *refund);
        let operation = job.operation_id.clone();
        store
            .call(move |s| s.require_unstarted_preparation(&operation))
            .await
            .unwrap();
    }
    // Prerequisite dispatch keeps its ordinary reactor wake in real time.
    // From quote release onward, keep the clock stable across SDK/Store work
    // so local handoffs and recurring network observations can be distinguished.
    let active = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    let handoff_started = tokio::time::Instant::now();
    f.release_quotes.cancel();
    first.await.unwrap();
    for _ in 0..6 {
        if sends.load(Ordering::SeqCst) == 1 {
            break;
        }
        tick(&worker, clock).await;
    }
    assert_eq!(
        sends.load(Ordering::SeqCst),
        1,
        "jobs: {:?}",
        store.call(|s| s.funding_jobs()).await.unwrap()
    );
    // Same-epoch handoffs reached submission without advancing a poll clock.
    assert_eq!(handoff_started.elapsed(), Duration::ZERO);
    // No new block or refund derivation: subminute passes reuse observations.
    let before = store.call(|s| s.treasury_sync_state()).await.unwrap();
    let observed = lookups.load(Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(59)).await;
    tick(&worker, clock).await;
    assert_eq!(lookups.load(Ordering::SeqCst), observed);
    let after = store.call(|s| s.treasury_sync_state()).await.unwrap();
    assert_eq!(before.snapshot_revision, after.snapshot_revision);
    assert_eq!(
        before.sync.unwrap().checked_at,
        after.sync.unwrap().checked_at
    );
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(2)).await;
    tick(&worker, clock).await;
    assert_eq!(lookups.load(Ordering::SeqCst), observed + 1);
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    complete(&worker, &net, 2, clock).await;
    assert_eq!(sends.load(Ordering::SeqCst), 2);
    let pool = store
        .call(|s| {
            Ok(s.status()?
                .pools
                .into_iter()
                .find(|p| p.name == "alpha")
                .unwrap()
                .id)
        })
        .await
        .unwrap();
    let manager = Arc::new(
        crate::rotation::manager::ManagedPool::new(
            store.clone(),
            pool,
            base,
            crate::payment::SpendPolicy::dollars("1").unwrap(),
        )
        .unwrap(),
    );
    let payer = crate::payment::PaidClient::managed(manager);
    payer
        .execute(crate::catalog::RoutedRequest {
            headers: Default::default(),
            method: "GET".into(),
            url: format!("{origin}/pay"),
            query: BTreeMap::new(),
            body: None,
        })
        .await
        .unwrap();
    assert_eq!(f.signed.lock().unwrap().len(), 1);
    // Actual signed admission consumes the reserve, then the same worker funds
    // exactly one replacement without resolving the old payment liability.
    complete(&worker, &net, 3, clock).await;
    assert_eq!(f.quote_count.load(Ordering::SeqCst), 3);
    assert_eq!(sends.load(Ordering::SeqCst), 3);
    store
        .call(|s| {
            let jobs = s.funding_jobs()?;
            assert_eq!(jobs.len(), 3);
            for job in jobs {
                assert_eq!(s.operation(&job.operation_id)?.attempts, 1);
            }
            Ok(())
        })
        .await
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unresolved, 1);
    drop(db);
    drop(payer);
    drop(worker);
    drop(f);
    server.abort();
    let _ = server.await;
    drop(store);
    stop.cancel();
    actor.await.unwrap().unwrap();
    tokio::time::resume();
    active.abort();
}

#[tokio::test]
async fn production_preparation_scans_new_deposit_without_a_readiness_balance_gate() {
    use crate::rotation::funding::FundingBackend;
    let (_dir, net, mut owner, endpoint) = funded_fixture_amount(2, 10_000).await;
    let address = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    owner
        .ensure_capacity_group(
            "alpha".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    let store = owner.store_handle();
    let job = store
        .call(|s| {
            s.fill_funding_vacancies("alpha")?;
            Ok(s.funding_jobs()?.remove(0))
        })
        .await
        .unwrap();
    let saved = owner.status().await.unwrap().sync.unwrap();
    assert_eq!(saved.spendable_shielded_zatoshis, 10_000);
    let deposit = faucet_funding_transaction(vec![(&address, 1_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![deposit]);
        chain.mine_empty_blocks(2);
    }
    // The fixture's earlier scan precedes this new deposit. Make the next
    // shared observation due; scheduling floors are covered independently.
    owner.observation = Default::default();
    let (handle, commands) = actor::channel();
    let mut backend = Backend {
        treasury: handle.clone(), store: store.clone(),
        near: NearClient::synthetic_at("http://127.0.0.1:1").unwrap(),
        base: BaseRpc::new("http://127.0.0.1:1", 2).unwrap(),
        wallets: BTreeMap::from([("alpha".into(), serde_json::from_value(json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},"max_api_payment_usdc":"1","max_funding_spend_zec":"0.002","max_conversion_overhead_percent":5})).unwrap())]),
        funding: serde_json::from_value(json!({"base_rpc_url_env":"BASE","confidentiality":"public"})).unwrap(),
        max_network_fee: 30_000,
    };
    let quote = Quote {
        sizing_preview: None,
        request: json!({}),
        response: json!({}),
        input: 50_000,
        deadline: now().unwrap() + 7200,
        deposit: None,
    };
    let stop = CancellationToken::new();
    let sends = Arc::new(AtomicUsize::new(0));
    let sender = CountedSender {
        inner: submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        sends: sends.clone(),
        lookups: Arc::new(AtomicUsize::new(0)),
    };
    // Readiness has no balance/scan gate and creates no financial work.
    backend.ready(&job, &quote).await.unwrap();
    assert!(commands.test_is_empty());
    assert_eq!(
        store
            .call(|s| Ok(s.status()?.sync.unwrap().spendable_shielded_zatoshis))
            .await
            .unwrap(),
        10_000
    );
    assert!(
        store
            .call(|s| Ok(
                s.status()?.treasury_operations.is_empty() && !s.status()?.outgoing_pending
            ))
            .await
            .unwrap()
    );
    let deadline = quote.deadline;
    save_quotes(&store, vec![job.clone()], deadline).await;
    // Actual preparation owns SDK synchronization. Queue it before the actor's
    // maintenance tick so an incidental background scan cannot satisfy the test.
    let prepare_handle = handle.clone();
    let base = backend.base.clone();
    // An unavailable advisory destination endpoint cannot prohibit a valid
    // SDK-funded preparation. Actual Store publication failures are different.
    let prepare = tokio::spawn(async move {
        prepare_handle
            .prepare_funding(request(&job, deadline), base)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while commands.test_is_empty() {
            assert!(!prepare.is_finished());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    let prepared = prepare.await.unwrap().unwrap();
    assert_eq!(prepared.facts().unwrap().amount_zatoshis, 50_000);
    store
        .call(|s| {
            let status = s.status()?;
            assert_eq!(status.sync.unwrap().spendable_shielded_zatoshis, 1_010_000);
            assert_eq!(status.treasury_operations.len(), 1);
            assert_eq!(status.treasury_operations[0].attempts, 0);
            assert!(status.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    stop.cancel();
    drop(backend);
    drop(store);
    drop(handle);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn unstarted_sdk_failures_keep_the_quote_and_recovery_allowance_until_funded() {
    use crate::rotation::{
        error::FundingCondition, funding::FundingBackend, store::funding::FundingPhase,
    };
    let (_dir, net, owner, endpoint) = funded_fixture_amount(2, 10_000).await;
    let address = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    owner
        .ensure_capacity_group(
            "alpha".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    let store = owner.store_handle();
    let f = Bridge {
        store: store.clone(),
        deposits: Arc::new(vec![DEPOSIT.into()]),
        quotes: Default::default(),
        quote_count: Default::default(),
        base_calls: Default::default(),
        quotes_entered: Arc::new(tokio::sync::Semaphore::new(0)),
        release_quotes: CancellationToken::new(),
        signed: Default::default(),
        destination: Arc::new(Mutex::new(DestinationChain {
            height: 100,
            credits: BTreeMap::new(),
        })),
        timestamp: now().unwrap(),
    };
    f.release_quotes.cancel();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/v0/tokens",
            get(|| async {
                Json(
                    serde_json::from_str::<Value>(include_str!(
                        "../../../tests/fixtures/near/tokens.json"
                    ))
                    .unwrap(),
                )
            }),
        )
        .route("/v0/quote", post(quote))
        .route("/v0/status", get(status))
        .route("/base", post(base))
        .with_state(f.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (handle, commands) = actor::channel();
    let backend = Backend {
        treasury: handle.clone(), store: store.clone(),
        near: NearClient::synthetic_at(&origin).unwrap(),
        base: BaseRpc::new(&format!("{origin}/base"), 2).unwrap(),
        wallets: BTreeMap::from([("alpha".into(), serde_json::from_value(json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},"max_api_payment_usdc":"1","max_funding_spend_zec":"0.002","max_conversion_overhead_percent":5,"max_attempts":1})).unwrap())]),
        funding: serde_json::from_value(json!({"base_rpc_url_env":"BASE","confidentiality":"public"})).unwrap(),
        max_network_fee: 30_000,
    };
    let worker = FundingWorker::new(store.clone(), backend, 1);
    let sends = Arc::new(AtomicUsize::new(0));
    let stop = CancellationToken::new();
    let sender = CountedSender {
        inner: submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        sends: sends.clone(),
        lookups: Arc::new(AtomicUsize::new(0)),
    };
    let actor = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    let epoch = now().unwrap();
    worker.tick(epoch).await.unwrap();
    let original = store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    assert_eq!(original.phase, FundingPhase::Quoted);
    let id = original.id.clone();
    let operation = original.operation_id.clone();
    let saved_quote = store
        .call(move |s| Ok(s.funding_quote_for_operation(&id, &operation)?.to_vec()))
        .await
        .unwrap();
    let accepted_quote: Quote = serde_json::from_slice(&saved_quote).unwrap();
    // Exceed max_attempts with actual SDK rejections, then run the coordinator's
    // next source pass as well: metadata must not turn these into recovery work.
    // Follow the published source schedule, including jitter and elapsed SDK work.
    let mut next_poll = original.next_poll;
    for _ in 0..4 {
        let error = worker
            .backend
            .lock()
            .await
            .prepare(&original, &accepted_quote)
            .await
            .unwrap_err();
        assert!(error.is::<PreparationDeferred>(), "{error:#}");
        assert_eq!(
            error.downcast_ref::<FundingCondition>(),
            Some(&FundingCondition::InsufficientFunds)
        );
        worker.tick(next_poll).await.unwrap();
        let job = original.clone();
        let quote = saved_quote.clone();
        next_poll = store
            .call(move |s| {
                let current = s.funding_job(&job.id)?.unwrap();
                assert_eq!(current.phase, FundingPhase::Quoted);
                assert_eq!(current.operation_id, job.operation_id);
                assert_eq!(s.funding_recovery_count(&job.id)?, 0);
                assert_eq!(&*s.funding_quote(&job.id)?, quote.as_slice());
                s.require_unstarted_preparation(&job.operation_id)?;
                assert!(
                    !s.status()?.outgoing_pending && s.status()?.treasury_operations.is_empty()
                );
                Ok(current.next_poll)
            })
            .await
            .unwrap();
        assert_eq!(sends.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.base_calls.load(Ordering::SeqCst),
            0,
            "repeated SDK shortages must not probe the destination"
        );
    }
    let deposit = faucet_funding_transaction(vec![(&address, 1_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![deposit]);
        chain.mine_empty_blocks(2);
    }
    // Explicit one-pass discovery makes the new deposit available. Shared
    // automatic capital waiting is covered by the discovery lifecycle test.
    handle.sync().await.unwrap();
    // A known fee rejection has the same unstarted semantics after new funds arrive.
    worker.backend.lock().await.max_network_fee = 1;
    let error = worker
        .backend
        .lock()
        .await
        .prepare(&original, &accepted_quote)
        .await
        .unwrap_err();
    assert!(error.is::<PreparationDeferred>(), "{error:#}");
    assert_eq!(
        error.downcast_ref::<FundingCondition>(),
        Some(&FundingCondition::CostLimit)
    );
    worker.tick(next_poll).await.unwrap();
    let job = original.clone();
    next_poll = store
        .call(move |s| {
            let current = s.funding_job(&job.id)?.unwrap();
            assert_eq!(current.phase, FundingPhase::Quoted);
            assert_eq!(current.operation_id, job.operation_id);
            assert_eq!(s.funding_recovery_count(&job.id)?, 0);
            s.require_unstarted_preparation(&job.operation_id)?;
            Ok(current.next_poll)
        })
        .await
        .unwrap();
    assert_eq!(
        f.base_calls.load(Ordering::SeqCst),
        0,
        "invalid fee proposals must not probe the destination"
    );
    worker.backend.lock().await.max_network_fee = 30_000;
    worker.tick(next_poll).await.unwrap();
    assert_eq!(f.base_calls.load(Ordering::SeqCst), 1);
    let job = original.clone();
    store
        .call(move |s| {
            let current = s.funding_job(&job.id)?.unwrap();
            assert_eq!(current.phase, FundingPhase::Prepared);
            assert_eq!(current.operation_id, job.operation_id);
            assert_eq!(s.funding_recovery_count(&job.id)?, 0);
            assert!(s.require_unstarted_preparation(&job.operation_id).is_err());
            assert_eq!(s.operation(&job.operation_id)?.attempts, 0);
            assert!(s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(f.quote_count.load(Ordering::SeqCst), 1);
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    stop.cancel();
    drop(worker);
    drop(handle);
    drop(store);
    drop(f);
    server.abort();
    let _ = server.await;
    actor.await.unwrap().unwrap();
}
