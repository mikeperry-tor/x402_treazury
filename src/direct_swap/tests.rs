use super::*;
use futures_util::FutureExt;

#[test]
fn swap_definitions_share_destination_inputs_and_scope_refunds_to_foreign_receives() {
    let config = Config {
        receive: true,
        send: true,
        ..Default::default()
    };
    for foreign in [false, true] {
        let tools = definitions_for(&config, foreign);
        let schema = |name| {
            &tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap()
                .input_schema
        };
        let preview = schema(PREVIEW);
        let send = schema(SEND);
        let receive = schema(RECEIVE);
        let mut send_fields = send["properties"].as_object().unwrap().clone();
        assert!(send_fields.remove("request_id").is_some());
        assert_eq!(&send_fields, preview["properties"].as_object().unwrap());
        assert_eq!(
            preview["required"],
            json!(["asset_id", "chain", "amount", "recipient"])
        );
        assert_eq!(
            send["required"],
            json!(["asset_id", "chain", "amount", "recipient", "request_id"])
        );
        assert_eq!(
            receive["required"],
            json!(["asset_id", "chain", "amount", "request_id"])
        );
        let refund = receive["properties"].get("refund_address");
        assert_eq!(refund.is_some(), foreign);
        assert_eq!(
            receive["properties"].get("refund_after_seconds").is_some(),
            foreign
        );
        assert!(preview["properties"].get("refund_address").is_none());
    }
}

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
async fn known_send_caps_leave_new_request_ids_available_and_preserve_saved_work() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"wallet",
    )
    .unwrap();
    let recipient = crate::rotation::near::test_shielded_refund(&state.network().chain());
    let (store, worker) = StoreHandle::spawn(state);
    let (treasury, commands) = crate::treasury::actor::channel();
    let service = Service::new(
        store.clone(),
        treasury,
        NearClient::test_at("http://127.0.0.1:1").unwrap(),
        serde_json::from_value(json!({})).unwrap(),
    );
    let mut native = json!({"asset_id":route::ZEC,"chain":"zec","amount":"200",
        "recipient":recipient,"request_id":"new-native"});
    let mut foreign = json!({"asset_id":"token:base-usdc","chain":"base","amount":"100",
        "recipient":"fixture-recipient","request_id":"new-foreign"});
    let mut config = Config {
        send: true,
        max_send_zec: Some("0".into()),
        ..Default::default()
    };
    assert!(
        service
            .invoke("owner", &config, SEND, foreign.clone())
            .await
            .is_err()
    );
    config.max_send_zec = Some("0.0001".into());
    assert!(
        service
            .invoke("owner", &config, SEND, foreign.clone())
            .await
            .is_err()
    );
    config.max_send_zec = Some("0.000101".into());
    assert!(
        service
            .invoke("owner", &config, SEND, native.clone())
            .await
            .is_err()
    );
    store
        .call(|s| {
            for id in ["new-native", "new-foreign"] {
                assert!(s.direct_request("owner", "send", id).is_err(), "{id}");
            }
            Ok(())
        })
        .await
        .unwrap();
    assert!(commands.test_is_empty());

    let mut previously_reserved = native.clone();
    previously_reserved["request_id"] = json!("previous-native");
    let terms = previously_reserved.clone();
    store
        .call(move |s| {
            s.reserve_native("owner", "send", "previous-native", terms)?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        service
            .invoke("owner", &config, SEND, previously_reserved.clone())
            .await
            .is_err()
    );
    assert!(
        store
            .call(|s| Ok(s
                .direct_request("owner", "send", "previous-native")?
                .authority
                .is_none()))
            .await
            .unwrap()
    );

    native["amount"] = json!("100");
    let accepted = service
        .invoke("owner", &config, SEND, native.clone())
        .await
        .unwrap();
    assert_eq!(
        accepted["operation"]["authority"]["max_debit_zatoshis"],
        10_100
    );
    config.max_send_zec = Some("0.000102".into());
    let recovered = service
        .invoke("owner", &config, SEND, previously_reserved)
        .await
        .unwrap();
    assert_eq!(recovered["operation"]["request_id"], "previous-native");
    assert_eq!(
        recovered["operation"]["authority"]["max_debit_zatoshis"],
        10_200
    );
    foreign["request_id"] = json!("previous-foreign");
    let terms = foreign.clone();
    let previous = store
        .call(move |s| {
            Ok(
                s.reserve_direct("owner", "send", "previous-foreign", terms)?
                    .0,
            )
        })
        .await
        .unwrap();
    for maximum in ["0", "malformed"] {
        config.max_send_zec = Some(maximum.into());
        for (request, saved) in [
            (&native, accepted["operation"].clone()),
            (&foreign, serde_json::to_value(&previous).unwrap()),
        ] {
            let retry = service
                .invoke("owner", &config, SEND, request.clone())
                .await
                .unwrap();
            assert_eq!(retry["operation"], saved);
        }
    }
    assert!(commands.test_is_empty());
    drop((service, store, commands));
    worker.await.unwrap();
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

#[tokio::test]
async fn exact_foreign_send_retry_succeeds_without_near_while_new_work_and_mismatches_fail() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"wallet",
    )
    .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let foreign = json!({
        "asset_id": "token:base-usdc",
        "chain": "base",
        "amount": "100",
        "recipient": "fixture-recipient",
        "request_id": "saved-foreign-send"
    });
    let mut accepted_terms = foreign.clone();
    accepted_terms["request_id"] = json!("accepted-foreign-send");
    let (saved, accepted) = store
        .call({
            let terms = foreign.clone();
            let accepted_terms = accepted_terms.clone();
            move |s| {
                let saved = s
                    .reserve_direct("owner", "send", "saved-foreign-send", terms)?
                    .0;
                let (accepted, _, _) = s.reserve_direct_route(
                    "owner",
                    "send",
                    "accepted-foreign-send",
                    accepted_terms,
                    crate::rotation::store::DirectRoute::Near,
                    |_| {
                        Ok(Some(crate::rotation::store::DirectSendAuthority {
                            max_debit_zatoshis: Some(1_000_000),
                        }))
                    },
                )?;
                Ok((saved, accepted))
            }
        })
        .await
        .unwrap();
    let (treasury, commands) = crate::treasury::actor::channel();
    let service = Service::native(store.clone(), treasury, 5);
    let config = Config {
        send: true,
        max_send_zec: Some("0.00000001".into()),
        ..Default::default()
    };

    let retry = service
        .invoke("owner", &config, SEND, foreign.clone())
        .await
        .unwrap();
    assert_eq!(retry["operation"], serde_json::to_value(&saved).unwrap());

    let accepted_retry = service
        .invoke("owner", &config, SEND, accepted_terms)
        .await
        .unwrap();
    assert_eq!(
        accepted_retry["operation"],
        serde_json::to_value(&accepted).unwrap()
    );

    let mut mismatched = foreign.clone();
    mismatched["amount"] = json!("200");
    let mismatch_error = service
        .invoke("owner", &config, SEND, mismatched)
        .await
        .unwrap_err();
    assert!(
        mismatch_error
            .to_string()
            .contains("request ID already used with different terms"),
        "{mismatch_error:#}"
    );

    let mut fresh = foreign;
    fresh["request_id"] = json!("new-foreign-send");
    let fresh_error = service
        .invoke("owner", &config, SEND, fresh)
        .await
        .unwrap_err();
    assert!(
        fresh_error
            .to_string()
            .contains("Foreign swaps unavailable"),
        "{fresh_error:#}"
    );

    let preview_error = service
        .invoke(
            "owner",
            &config,
            PREVIEW,
            json!({
                "asset_id": "token:base-usdc",
                "chain": "base",
                "amount": "100",
                "recipient": "fixture-recipient"
            }),
        )
        .await
        .unwrap_err();
    assert!(
        preview_error
            .to_string()
            .contains("Foreign swaps unavailable"),
        "{preview_error:#}"
    );

    assert!(commands.test_is_empty());
    store
        .call(move |s| {
            assert!(
                s.direct_request("owner", "send", "new-foreign-send")
                    .is_err()
            );
            assert_eq!(
                s.direct_request("owner", "send", "saved-foreign-send")?,
                saved
            );
            Ok(())
        })
        .await
        .unwrap();
    drop((service, store, commands));
    worker.await.unwrap();
}
