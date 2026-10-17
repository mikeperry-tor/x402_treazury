//! Durable provider hints survive status outages; exact chain proofs close old flags.
use super::*;

#[tokio::test]
async fn production_worker_closes_conflicting_status_only_after_canonical_proofs() {
    let (dir, mut net, owner, endpoint) = funded_fixture(1).await;
    let mut external = net
        .client("legal winner thank year wave sausage worth useful legal winner thank yellow")
        .await;
    let (_, address) = external
        .generate_transparent_address(zip32::AccountId::ZERO, false)
        .await
        .unwrap();
    let deposit = zcash_keys::address::Address::Transparent(address)
        .encode(&TreasuryNetwork::Synthetic.chain());
    owner
        .ensure_capacity_group(
            "settlement".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    let store = owner.store_handle();
    let f = Bridge {
        store: store.clone(),
        deposits: Arc::new(vec![deposit]),
        quotes: Default::default(),
        quote_count: Default::default(),
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
    let stage = Arc::new(AtomicUsize::new(0));
    let outages = Arc::new(AtomicUsize::new(0));
    let receipts = Arc::new(AtomicUsize::new(0));
    let status_stage = stage.clone();
    let status_outages = outages.clone();
    let receipt_stage = stage.clone();
    let receipt_calls = receipts.clone();
    let app = Router::new()
        .route(
            "/v0/tokens",
            get(|| async {
                Json(
                    serde_json::from_str::<Value>(include_str!(
                        "../../../../tests/fixtures/near/tokens.json"
                    ))
                    .unwrap(),
                )
            }),
        )
        .route("/v0/quote", post(quote))
        .route(
            "/v0/status",
            get(
                move |state: State<Bridge>, query: Query<BTreeMap<String, String>>| {
                    let stage = status_stage.clone();
                    let outages = status_outages.clone();
                    async move {
                        let n = stage.load(Ordering::SeqCst);
                        if n >= 3 {
                            outages.fetch_add(1, Ordering::SeqCst);
                            return StatusCode::SERVICE_UNAVAILABLE.into_response();
                        }
                        let Json(mut value) = status(state, query).await;
                        if value["status"] == "SUCCESS" {
                            value["status"] =
                                json!(["FAILED", "REFUNDED", "INCOMPLETE_DEPOSIT"][n]);
                            if n < 2 {
                                value.as_object_mut().unwrap().remove("swapDetails");
                            }
                        }
                        Json(value).into_response()
                    }
                },
            ),
        )
        .route(
            "/base",
            post(move |state: State<Bridge>, request: Json<Value>| {
                let stage = receipt_stage.clone();
                let calls = receipt_calls.clone();
                async move {
                    if request.0["method"] == "eth_getTransactionReceipt" {
                        calls.fetch_add(1, Ordering::SeqCst);
                        if stage.load(Ordering::SeqCst) < 3 {
                            return Json(
                                json!({"jsonrpc":"2.0","id":request.0["id"],"result":null}),
                            );
                        }
                    }
                    base(state, request).await
                }
            }),
        )
        .with_state(f.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (handle, commands) = actor::channel();
    let backend=Backend { treasury:handle.clone(),store:store.clone(),near:NearClient::synthetic_at(&origin).unwrap(),base:BaseRpc::new(&format!("{origin}/base"),2,120).unwrap(),wallets:[("settlement".into(),serde_json::from_value(json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},"max_api_payment_usdc":"1","max_funding_spend_zec":"0.002","max_conversion_overhead_percent":5})).unwrap())].into(),funding:serde_json::from_value(json!({"base_rpc_url_env":"BASE","confidentiality":"public"})).unwrap(),max_network_fee:30_000 };
    let worker = FundingWorker::new(store.clone(), backend, 1);
    let sends = Arc::new(AtomicUsize::new(0));
    let sender = CountedSender {
        inner: submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        sends: sends.clone(),
        lookups: Default::default(),
    };
    let stop = CancellationToken::new();
    let actor = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    let epoch = now().unwrap();
    let mut pass = 0;
    let mut original = None;
    for wanted in 0..4 {
        stage.store(wanted, Ordering::SeqCst);
        let mut reached = false;
        for _ in 0..20 {
            pass += 1;
            tokio::time::timeout(Duration::from_secs(90), worker.tick(epoch + pass * 1000))
                .await
                .unwrap()
                .unwrap();
            let observed = store
                .call(|s| {
                    let job = s.funding_jobs()?.remove(0);
                    let facts = s.funding_facts(&job.id, &job.operation_id)?;
                    Ok((job, facts))
                })
                .await
                .unwrap();
            let (job, facts) = observed;
            reached = match wanted {
                0 => facts.provider_failed,
                1 => facts.provider_refunded,
                2 => facts.provider_incomplete && receipts.load(Ordering::SeqCst) > 0,
                _ => job.phase == crate::rotation::store::funding::FundingPhase::Complete,
            };
            if wanted < 3 {
                assert_ne!(
                    job.phase,
                    crate::rotation::store::funding::FundingPhase::Complete
                );
            }
            if reached {
                let operation = job.operation_id.clone();
                let current = store
                    .call(move |s| {
                        let saved = s.operation(&operation)?;
                        Ok((
                            operation.clone(),
                            s.prepared_bytes(&operation)?,
                            saved.facts,
                        ))
                    })
                    .await
                    .unwrap();
                if wanted == 0 {
                    original = Some(current);
                } else {
                    assert_eq!(Some(&current), original.as_ref());
                }
                if wanted == 3 {
                    assert!(
                        facts.provider_failed
                            && facts.provider_refunded
                            && facts.provider_incomplete
                    );
                    assert!(!facts.provider_success);
                    assert!(
                        facts.delivery_verified
                            && !facts.return_pending
                            && !facts.source_accounting_pending
                    );
                    assert_eq!(facts.source_attempts, 1);
                }
                break;
            }
            let mut chain = net.chain.write().await;
            chain.mine_mempool();
            chain.mine_empty_blocks(1);
        }
        assert!(
            reached,
            "stage {wanted}: {:?}",
            store.call(|s| s.funding_jobs()).await.unwrap()
        );
    }
    let (operation, _, facts) = original.unwrap();
    let db = rusqlite::Connection::open_with_flags(
        dir.path().join("state/state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let accounting: (i64, i64) = db
        .query_row(
            "SELECT reserved,consumed FROM budget_entries WHERE id=?1",
            [&operation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        accounting,
        (0, (facts.amount_zatoshis + facts.fee_zatoshis) as i64)
    );
    drop(db);
    assert!(outages.load(Ordering::SeqCst) > 0);
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    assert_eq!(f.quote_count.load(Ordering::SeqCst), 1);
    server.abort();
    let _ = server.await;
    drop(worker);
    drop(f);
    drop(store);
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(30), actor)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
