use super::*;
use futures_util::FutureExt;

#[tokio::test]
async fn asset_search_ranks_before_paging_and_exposes_native_currency_name() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"wallet",
    )
    .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let app = axum::Router::new().route("/v0/tokens", axum::routing::get(|| async {
        axum::Json(json!([
            {"assetId":"token:eth-usdc","blockchain":"eth","symbol":"USDC","decimals":6},
            {"assetId":"token:base-usdc","blockchain":"base","symbol":"USDC","decimals":6},
            {"assetId":"token:btc","blockchain":"btc","symbol":"BTC","decimals":8,"coingeckoId":"bitcoin"}
        ]))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let near = NearClient::test_at(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (treasury, commands) = crate::treasury::actor::channel();
    let service = Service::new(
        store.clone(),
        treasury,
        near,
        serde_json::from_value(json!({})).unwrap(),
    );
    let config = Config {
        receive: true,
        ..Default::default()
    };
    let mut pages = Vec::new();
    for offset in [0, 1] {
        let page = service
            .invoke(
                "owner",
                &config,
                "x402_treazury_swap_assets",
                json!({"query":"USDC base","offset":offset,"limit":1}),
            )
            .await
            .unwrap();
        assert_eq!(page["total"], 2);
        assert_eq!(page["complete"], true);
        assert_eq!(page["foreign_assets_available"], true);
        assert_eq!(
            page["next_offset"],
            if offset == 0 { json!(1) } else { Value::Null }
        );
        pages.push(page["assets"][0]["assetId"].clone());
    }
    assert_eq!(pages, [json!("token:base-usdc"), json!("token:eth-usdc")]);
    let page = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_swap_assets",
            json!({"query":"Zcash"}),
        )
        .await
        .unwrap();
    assert_eq!(page["total"], 1);
    assert_eq!(page["assets"][0]["assetId"], route::ZEC);
    let blank = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_swap_assets",
            json!({"query":"  "}),
        )
        .await
        .unwrap();
    assert_eq!(blank["assets"][0]["native"], true);
    assert_eq!(blank["assets"][1]["assetId"], "token:eth-usdc");
    assert!(
        commands.test_is_empty(),
        "search cannot occupy the SDK owner"
    );
    http.abort();
    drop((service, store, commands));
    worker.await.unwrap();
}

#[tokio::test]
async fn request_id_recovers_owned_status_without_admission_or_backend() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"wallet",
    )
    .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let (receive, send) = store
        .call(|s| {
            Ok((
                s.reserve_direct("owner", "receive", "same", json!({"original":1}))?
                    .0,
                s.reserve_direct("owner", "send", "same", json!({"original":2}))?
                    .0,
            ))
        })
        .await
        .unwrap();
    let service = Service::status_only(store.clone());
    for (kind, id) in [("receive", receive.id), ("send", send.id)] {
        let reference = json!({"kind":kind,"request_id":"same"});
        let result = service
            .invoke(
                "owner",
                &Config::default(),
                "x402_treazury_swap_status",
                reference.clone(),
            )
            .await
            .unwrap();
        assert_eq!(result["operation"]["id"], id);
        assert!(
            service
                .invoke(
                    "other",
                    &Config::default(),
                    "x402_treazury_swap_status",
                    reference
                )
                .await
                .is_err()
        );
    }
    for reference in [
        json!({}),
        json!({"kind":"receive"}),
        json!({"request_id":"same"}),
        json!({"kind":"unknown","request_id":"same"}),
        json!({"operation_id":"id","kind":"receive","request_id":"same"}),
    ] {
        assert!(
            service
                .invoke(
                    "owner",
                    &Config::default(),
                    "x402_treazury_swap_status",
                    reference
                )
                .await
                .is_err()
        );
    }
    store
        .call(|s| {
            assert!(s.funding_jobs()?.is_empty());
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    drop((service, store));
    worker.await.unwrap();
}

#[test]
fn direct_send_total_debit_configuration_is_optional_and_exact() {
    for value in [
        json!({}),
        json!({"max_send_zec":null}),
        json!({"max_send_zec":"none"}),
    ] {
        assert_eq!(
            serde_json::from_value::<Config>(value)
                .unwrap()
                .max_debit_zatoshis()
                .unwrap(),
            None
        );
    }
    let config: Config = serde_json::from_value(json!({"max_send_zec":"0.01000001"})).unwrap();
    assert_eq!(config.max_debit_zatoshis().unwrap(), Some(1_000_001));
    for value in [
        "-1",
        "1e-8",
        "0.000000001",
        "21000000.00000001",
        "unlimited",
    ] {
        let config: Config = serde_json::from_value(json!({"max_send_zec":value})).unwrap();
        assert!(config.validate().is_err());
    }
}

#[tokio::test]
async fn native_receipt_reuses_snapshot_evidence_without_occupying_sdk_owner() {
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
                .reserve_native("owner", "receive", "request", json!({"amount":"100"}))?
                .0;
            let revision = s.save_direct_address(
                &op.id,
                crate::rotation::near::PUBLIC_REFUND_UA,
                1,
                b"derived",
            )?;
            s.save_native_status(
                &op.id,
                json!({"confirmed_zatoshis":0,"snapshot_revision":revision}),
            )?;
            s.direct_operation("owner", &op.id)
        })
        .await
        .unwrap();
    let (treasury, mut commands) = crate::treasury::actor::channel();
    let service = Service::native(store.clone(), treasury, crate::rotation::config::poll());
    assert!(!service.observe_once(&operation).await.unwrap());
    let reference = json!({"operation_id":operation.id});
    let current = service
        .invoke(
            "owner",
            &Config::default(),
            "x402_treazury_swap_status",
            reference.clone(),
        )
        .await
        .unwrap();
    assert_eq!(current["receipt_matches_saved_snapshot"], true);
    assert_eq!(current["receipt_refresh_unavailable"], false);
    assert!(commands.test_is_empty());
    store
        .call(|s| {
            s.save_snapshot(2, b"new observation")?;
            Ok(())
        })
        .await
        .unwrap();
    let disabled = Config::default();
    let (result, command) = tokio::join!(
        service.invoke("owner", &disabled, "x402_treazury_swap_status", reference),
        commands.test_observe()
    );
    let result = result.unwrap();
    assert_eq!(result["receipt_matches_saved_snapshot"], false);
    assert_eq!(result["receipt_refresh_unavailable"], true);
    assert_eq!(result["zec_receipt"]["snapshot_revision"], 2);
    assert!(
        matches!(command,crate::treasury::actor::ObservedCommand::NativeReceipt(id) if id==operation.id)
    );
    drop((service, store, commands));
    worker.await.unwrap();
}

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
