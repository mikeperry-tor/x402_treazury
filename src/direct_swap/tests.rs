use super::*;
use futures_util::FutureExt;

#[tokio::test]
async fn committed_quote_is_observed_without_foreground_acknowledgment() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"wallet",
    )
    .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let operation = store
        .call(|s| {
            let op = s
                .reserve_direct("owner", "receive", "request", json!({"amount":"100"}))?
                .0;
            s.save_direct_address(
                &op.id,
                crate::rotation::near::PUBLIC_REFUND_UA,
                1,
                b"derived",
            )?;
            Ok(op)
        })
        .await
        .unwrap();
    let origin = route::Asset {
        asset_id: "foreign".into(),
        blockchain: "eth".into(),
        decimals: 18,
        symbol: "TEST".into(),
        metadata: Default::default(),
    };
    let destination = route::Asset {
        asset_id: route::ZEC.into(),
        blockchain: "zec".into(),
        decimals: 8,
        symbol: "ZEC".into(),
        metadata: Default::default(),
    };
    let now = crate::rotation::base::now().unwrap();
    let request = route::request(
        &origin,
        &destination,
        crate::rotation::near::PUBLIC_REFUND_UA,
        "payer-refund",
        "100",
        route::SwapType::ExactInput,
        100,
        now + 1800,
        false,
    )
    .unwrap();
    let response = json!({"quoteRequest":request,"quote":{"amountIn":"100","amountOut":"90","minAmountOut":"89","depositAddress":"foreign-deposit","depositMemo":"required","deadline":request["deadline"]}});
    let quote = route::validate_quote(request, response, now).unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let response = json!({"status":"SUCCESS","quoteResponse":quote.response});
    let app = axum::Router::new().route(
        "/v0/status",
        axum::routing::get({
            let entered = entered.clone();
            move || {
                let entered = entered.clone();
                let response = response.clone();
                async move {
                    entered.notify_one();
                    axum::Json(response)
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let near = NearClient::test_at(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let http = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (treasury, commands) = crate::treasury::actor::channel();
    let service = Service::new(
        store.clone(),
        treasury,
        near,
        serde_json::from_value(json!({})).unwrap(),
        0,
    );
    let stop = CancellationToken::new();
    let task = tokio::spawn({
        let service = service.clone();
        let stop = stop.clone();
        async move { service.run(stop).await }
    });
    let id = operation.id.clone();
    // The Store callback commits and signals even though its reply reports a lost acknowledgment.
    store
        .call(move |s| {
            s.save_direct_quote(&id, serde_json::to_value(quote)?)?;
            Err::<(), _>(anyhow::anyhow!("fixture lost acknowledgment"))
        })
        .await
        .unwrap_err();
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .expect("committed quote was not acquired by its observation owner");
    stop.cancel();
    task.await.unwrap().unwrap();
    let status = service.status("owner", &operation.id).await.unwrap();
    assert_eq!(
        status["operation"]["status"]["provider"]["status"],
        "SUCCESS"
    );
    assert!(store.treasury_changes().notified().now_or_never().is_some());
    assert!(
        commands.test_is_empty(),
        "receive observation never submits or prepares ZEC"
    );
    http.abort();
    drop((service, store, commands));
    worker.await.unwrap();
}
