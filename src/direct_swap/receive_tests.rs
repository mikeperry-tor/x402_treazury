//! Receive presentation is derived from immutable saved terms, without provider I/O.
use super::receive::{Receive, instructions, refund_deadline};
use super::*;

fn foreign() -> DirectOperation {
    serde_json::from_value(json!({
        "id":"operation","route":"near","owner":"owner","kind":"receive",
        "request_id":"original","state":"pending",
        "terms":{"asset_id":"token:source","chain":"stellar","amount":"12345",
            "refund_address":"payer-refund","request_id":"original"},
        "address":"treasury-shielded-receiver",
        "quote":{"deposit":"foreign-deposit","deposit_memo":"memo-7","input":"12345",
            "deadline":1500,"request":{"deadline":"1970-01-01T01:20:00+01:00"},
            "response":{"quote":{"timeWhenInactive":"1970-01-01T00:30:00Z"}}},
        "status":null
    }))
    .unwrap()
}

#[test]
fn foreign_instructions_distinguish_three_times_and_exact_source_payment() {
    let operation = foreign();
    let saved = operation.clone();
    let display = instructions(&operation, Some(1000));
    assert_eq!(display["recipient"], "foreign-deposit");
    assert_ne!(display["recipient"], operation.address.unwrap());
    assert_eq!(display["asset_id"], "token:source");
    assert_eq!(display["chain"], "stellar");
    assert_eq!(display["amount_atomic"], "12345");
    assert_eq!(display["deposit_memo"], "memo-7");
    assert_eq!(display["exact_amount_required"], true);
    assert_eq!(display["partial_payments_accepted"], false);
    assert_eq!(display["network_fees_paid_separately"], true);
    assert_eq!(display["address_reusable"], false);
    assert!(
        display["payer_instruction"]
            .as_str()
            .unwrap()
            .contains("Never reuse")
    );
    assert_eq!(display["current_time_utc"], "1970-01-01T00:16:40Z");
    assert_eq!(
        display["requested_refund_start_utc"],
        "1970-01-01T00:20:00Z"
    );
    assert_eq!(display["seconds_until_requested_refund_start"], 200);
    assert_eq!(
        display["deposit_address_expires_at_utc"],
        "1970-01-01T00:25:00Z"
    );
    assert_eq!(display["seconds_until_deposit_address_expiry"], 500);
    assert_eq!(
        display["deposit_address_cold_at_utc"],
        "1970-01-01T00:30:00Z"
    );
    assert_eq!(display["seconds_until_deposit_address_cold"], 800);
    for now in [1800, 1801] {
        let display = instructions(&saved, Some(now));
        for field in [
            "seconds_until_requested_refund_start",
            "seconds_until_deposit_address_expiry",
            "seconds_until_deposit_address_cold",
        ] {
            assert_eq!(display[field], 0);
        }
    }
    assert_eq!(
        saved.state, "pending",
        "countdowns do not settle or expire work"
    );
}

#[test]
fn optional_timing_is_unknown_and_native_partials_have_no_expiry() {
    let mut operation = foreign();
    operation.quote.as_mut().unwrap()["response"]["quote"]["timeWhenInactive"] = json!("invalid");
    let original = operation.clone();
    let display = instructions(&operation, Some(1000));
    assert!(display["deposit_address_cold_at_utc"].is_null());
    assert!(display["seconds_until_deposit_address_cold"].is_null());
    assert_eq!(display["seconds_until_requested_refund_start"], 200);
    assert_eq!(operation, original);
    let mut missing = operation.clone();
    missing.quote.as_mut().unwrap()["deposit"] = Value::Null;
    assert!(instructions(&missing, Some(1000)).is_null());
    missing = operation.clone();
    missing.terms["amount"] = Value::Null;
    assert!(instructions(&missing, Some(1000)).is_null());
    let no_clock = instructions(&operation, None);
    assert!(no_clock["current_time_utc"].is_null());
    assert!(no_clock["seconds_until_requested_refund_start"].is_null());
    assert_eq!(
        no_clock["requested_refund_start_utc"],
        "1970-01-01T00:20:00Z"
    );
    operation.route = DirectRoute::Native;
    operation.terms["asset_id"] = json!(route::ZEC);
    operation.terms["chain"] = json!("zec");
    operation.quote = None;
    let display = instructions(&operation, Some(1000));
    assert_eq!(display["recipient"], "treasury-shielded-receiver");
    assert_eq!(display["amount_zatoshis"], "12345");
    assert_eq!(display["partial_payments_accepted"], true);
    assert_eq!(display["exact_amount_required"], false);
    assert_eq!(display["address_reusable"], true);
    for field in [
        "requested_refund_start_utc",
        "seconds_until_requested_refund_start",
        "deposit_address_expires_at_utc",
        "seconds_until_deposit_address_expiry",
        "deposit_address_cold_at_utc",
        "seconds_until_deposit_address_cold",
    ] {
        assert!(display[field].is_null());
    }
    operation.kind = "send".into();
    assert!(instructions(&operation, Some(1000)).is_null());
}

#[test]
fn relative_refund_preference_is_checked_and_absence_preserves_historical_terms() {
    assert_eq!(refund_deadline(1000, None).unwrap(), 8200);
    assert_eq!(refund_deadline(1000, Some(1)).unwrap(), 1001);
    assert!(refund_deadline(1000, Some(0)).is_err());
    assert!(refund_deadline(1000, Some(u64::MAX)).is_err());
    assert!(refund_deadline(1000, Some(i64::MAX as u64)).is_err());
    let old = json!({"asset_id":"token:source","chain":"stellar","amount":"12345",
        "refund_address":"payer-refund","request_id":"original"});
    let args: Receive = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(args).unwrap(), old);
}

#[tokio::test]
async fn status_without_backend_retains_quote_and_owner_at_zero_countdown() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"wallet",
    )
    .unwrap();
    let fixture = foreign();
    let (operation, _) = state
        .reserve_direct("owner", "receive", "original", fixture.terms.clone())
        .unwrap();
    state
        .save_direct_address(&operation.id, "treasury-shielded-receiver", 1, b"address")
        .unwrap();
    state
        .save_direct_quote(&operation.id, fixture.quote.clone().unwrap())
        .unwrap();
    let before = state.direct_operation("owner", &operation.id).unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let service = Service::status_only(store.clone());
    let reply = service
        .invoke(
            "owner",
            &Config::default(),
            "x402_treazury_swap_status",
            json!({"kind":"receive","request_id":"original"}),
        )
        .await
        .unwrap();
    assert_eq!(
        reply["payment_instructions"]["recipient"],
        "foreign-deposit"
    );
    assert_eq!(
        reply["payment_instructions"]["seconds_until_requested_refund_start"],
        0
    );
    assert_eq!(reply["operation"]["quote"], fixture.quote.unwrap());
    assert!(
        service
            .invoke(
                "other",
                &Config::default(),
                "x402_treazury_swap_status",
                json!({"operation_id":operation.id})
            )
            .await
            .is_err()
    );
    store
        .call(move |s| {
            assert_eq!(s.direct_operation("owner", &operation.id)?, before);
            assert!(s.funding_jobs()?.is_empty());
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    drop((service, store));
    worker.await.unwrap();
}

#[tokio::test]
async fn invalid_windows_fail_before_reservation_or_sdk_provider_effects() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"wallet",
    )
    .unwrap();
    let (store, worker) = StoreHandle::spawn(state);
    let (treasury, commands) = crate::treasury::actor::channel();
    let service = Service::new(
        store.clone(),
        treasury,
        NearClient::test_at("http://127.0.0.1:1").unwrap(),
        serde_json::from_value(json!({})).unwrap(),
    );
    let config = Config {
        receive: true,
        ..Default::default()
    };
    for args in [
        json!({"asset_id":"token:source","chain":"stellar","amount":"1",
            "refund_address":"payer-refund","request_id":"zero","refund_after_seconds":0}),
        json!({"asset_id":"token:source","chain":"stellar","amount":"1",
            "refund_address":"payer-refund","request_id":"overflow","refund_after_seconds":u64::MAX}),
        json!({"asset_id":route::ZEC,"chain":"zec","amount":"1",
            "request_id":"native","refund_after_seconds":1}),
    ] {
        assert!(
            service
                .invoke(
                    "owner",
                    &config,
                    "x402_treazury_payment_request_create",
                    args
                )
                .await
                .is_err()
        );
    }
    assert!(commands.test_is_empty());
    store
        .call(|s| {
            assert!(s.direct_pending()?.is_empty());
            for request in ["zero", "overflow", "native"] {
                assert!(s.direct_request("owner", "receive", request).is_err());
            }
            assert!(s.funding_jobs()?.is_empty());
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    drop((service, store, commands));
    worker.await.unwrap();
}

#[tokio::test]
async fn identical_create_retry_returns_original_without_renewal_or_sdk_provider_calls() {
    let longest =
        u64::try_from(chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp()).unwrap() - 1000;
    assert!(refund_deadline(1000, Some(longest)).is_ok());
    assert!(refund_deadline(crate::rotation::base::now().unwrap(), Some(longest)).is_err());
    for after in [None, Some(longest)] {
        let dir = tempfile::tempdir().unwrap();
        let mut state = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"wallet",
        )
        .unwrap();
        let mut fixture = foreign();
        if let Some(after) = after {
            fixture.terms["refund_after_seconds"] = json!(after);
        }
        let terms = fixture.terms.clone();
        let (operation, _) = state
            .reserve_direct("owner", "receive", "original", terms.clone())
            .unwrap();
        state
            .save_direct_address(&operation.id, "treasury-shielded-receiver", 1, b"address")
            .unwrap();
        state
            .save_direct_quote(&operation.id, fixture.quote.clone().unwrap())
            .unwrap();
        let before = state.direct_operation("owner", &operation.id).unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let (treasury, commands) = crate::treasury::actor::channel();
        let service = Service::new(
            store.clone(),
            treasury,
            NearClient::test_at("http://127.0.0.1:1").unwrap(),
            serde_json::from_value(json!({"quote_deadline_seconds":1})).unwrap(),
        );
        let config = Config {
            receive: true,
            ..Default::default()
        };
        for _ in 0..2 {
            let reply = service
                .invoke(
                    "owner",
                    &config,
                    "x402_treazury_payment_request_create",
                    terms.clone(),
                )
                .await
                .unwrap();
            assert_eq!(reply["operation"]["id"], operation.id);
            assert_eq!(reply["operation"]["quote"], fixture.quote.clone().unwrap());
            assert_eq!(
                reply["payment_instructions"]["seconds_until_requested_refund_start"],
                0
            );
        }
        let mut changed = terms;
        changed["refund_after_seconds"] = json!(60);
        assert!(
            service
                .invoke(
                    "owner",
                    &config,
                    "x402_treazury_payment_request_create",
                    changed
                )
                .await
                .is_err()
        );
        assert!(commands.test_is_empty());
        store
            .call(move |s| {
                assert_eq!(s.direct_operation("owner", &operation.id)?, before);
                assert_eq!(s.direct_pending()?.len(), 1);
                assert!(s.funding_jobs()?.is_empty());
                assert!(!s.status()?.outgoing_pending);
                Ok(())
            })
            .await
            .unwrap();
        drop((service, store, commands));
        worker.await.unwrap();
    }
}
