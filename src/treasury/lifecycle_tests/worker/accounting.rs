//! A slow residual source pass retains the owner without holding wallet or Store locks.
use super::*;
use futures_util::FutureExt;

#[tokio::test]
async fn residual_accounting_keeps_base_and_store_live_and_resumes_accepted_command() {
    let (dir, net, mut owner, endpoint) = funded_fixture(1).await;
    let pool = owner
        .ensure_capacity_group(
            "accounting".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    owner
        .allocate_capacity_kind(pool.clone(), AllocationKind::Small, true)
        .await
        .unwrap();
    let store = owner.store_handle();
    let job = store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let deadline = now().unwrap() + 7200;
    save_quotes(&store, vec![job.clone()], deadline).await;
    let prepared = owner.prepare(request(&job, deadline)).await.unwrap();
    let gross = prepared.facts().unwrap().amount_zatoshis + prepared.facts().unwrap().fee_zatoshis;
    let mut sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    let stop = CancellationToken::new();
    owner
        .submit_prepared(job.operation_id.clone(), &mut sender, false, &stop)
        .await
        .unwrap();
    net.chain.write().await.mine_mempool();
    assert!(matches!(
        owner
            .reconcile_prepared(job.operation_id.clone(), &mut sender, &stop)
            .await
            .unwrap(),
        TransactionPresence::Confirmed { .. }
    ));
    let wallet = owner.client.as_ref().unwrap().wallet().clone();
    let gate = Arc::new(crate::treasury::source_support::AccountingTestGate::default());
    owner.accounting_test_hook = Some(gate.clone());
    let f = Bridge {
        store: store.clone(),
        deposits: Arc::new(vec![]),
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().route("/base", post(base)).with_state(f);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = BaseRpc::new(&format!("{origin}/base"), 2, 120).unwrap();
    let (handle, commands) = actor::channel();
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    let caller = handle.clone();
    let sync_waiter = tokio::spawn(async move { caller.sync().await });
    tokio::time::timeout(Duration::from_secs(30), gate.entered.cancelled())
        .await
        .unwrap();
    assert!(
        wallet.try_write().is_ok(),
        "residual publication must release SDK read ownership"
    );
    let operation = job.operation_id.clone();
    let original = prepared.bytes().to_vec();
    let query = tokio::time::timeout(
        Duration::from_secs(5),
        store.call(move |s| {
            assert_eq!(s.prepared_bytes(&operation)?.as_slice(), original);
            assert_eq!(s.operation(&operation)?.attempts, 1);
            s.chain_query(&pool)
        }),
    )
    .await
    .unwrap()
    .unwrap();
    let view = tokio::time::timeout(Duration::from_secs(5), base.view(query))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view.balances.len(), 1);
    assert!(
        view.balances
            .values()
            .all(|v| *v == alloy_primitives::U256::from(5_000_000u64))
    );
    let command = handle.refund_address(job.id.clone());
    tokio::pin!(command);
    // Poll through the available bounded queue into the reply wait.
    assert!(command.as_mut().now_or_never().is_none());
    sync_waiter.abort();
    assert!(sync_waiter.await.unwrap_err().is_cancelled());
    assert!(
        command.as_mut().now_or_never().is_none(),
        "accepted SDK work must retain the existing owner"
    );
    gate.release.cancel();
    let address = tokio::time::timeout(Duration::from_secs(30), command)
        .await
        .unwrap()
        .unwrap();
    assert!(!address.is_empty());
    let operation = job.operation_id.clone();
    store
        .call(move |s| {
            assert_eq!(
                s.refund_address(&job.id)?.as_deref(),
                Some(address.as_str())
            );
            assert_eq!(s.operation(&operation)?.attempts, 1);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        super::super::confirmed_expiry::saved_accounting(
            &dir.path().join("state"),
            prepared.operation_id()
        ),
        (0, gross as i64, -1, 0),
    );
    stop.cancel();
    server.abort();
    let _ = server.await;
    drop(store);
    tokio::time::timeout(Duration::from_secs(30), task)
        .await
        .expect("owned actor drains after fixture Store handles drop")
        .unwrap()
        .unwrap();
}
