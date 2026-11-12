//! Catalog replacement does not own managed payer transitions or funding.
use super::*;
use crate::{
    payment::SpendPolicy,
    rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{AllocationKind, Store, StoreHandle},
        test_capacity::{small_reserve, two_reserves},
    },
};
use alloy_primitives::U256;
use axum::{Json, Router, response::IntoResponse, routing::get};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[tokio::test]
async fn shared_refresh_preserves_bootstrap_work_and_follows_destination_rotation() {
    let directory = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &directory.path().join("state"),
        &directory.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let policy = SpendPolicy::dollars("1").unwrap();
    let bootstrap = state
        .ensure_capacity_group("bootstrap", &small_reserve("2"), policy.max_atomic, 0)
        .unwrap();
    let destination = state
        .ensure_capacity_group("destination", &two_reserves("2"), policy.max_atomic, 0)
        .unwrap();
    for (pool, kind) in [
        (&bootstrap, AllocationKind::Small),
        (&destination, AllocationKind::Small),
        (&destination, AllocationKind::Large),
    ] {
        state
            .allocate_capacity_kind(pool, kind, true)
            .unwrap()
            .unwrap();
    }
    let jobs = state.funding_jobs().unwrap();
    let bootstrap_job = jobs.iter().find(|job| job.pool_id == bootstrap).unwrap();
    state
        .record_credit(&bootstrap_job.wallet_id, "2000000", "fixture", 100)
        .unwrap();
    let destination_jobs: Vec<_> = jobs
        .iter()
        .filter(|job| job.pool_id == destination)
        .cloned()
        .collect();
    let mut job_ids: Vec<_> = jobs.iter().map(|job| job.id.clone()).collect();
    job_ids.sort();
    let (store, worker) = StoreHandle::spawn(state);
    let managed = |pool| {
        Arc::new(
            ManagedPool::new(
                store.clone(),
                pool,
                BaseRpc::new("https://rpc.example").unwrap(),
                SpendPolicy::dollars("1").unwrap(),
            )
            .unwrap(),
        )
    };
    let bootstrap_manager = managed(bootstrap.clone());
    let destination_manager = managed(destination.clone());
    let bootstrap_address = bootstrap_manager.candidate().await.unwrap().address;
    let client = PaidClient::managed(destination_manager.clone())
        .with_bootstrap(bootstrap_manager.clone())
        .with_metadata_payer(Arc::new(PaidClient::managed(bootstrap_manager.clone())));

    let documents = Arc::new(AtomicUsize::new(0));
    let unsigned = Arc::new(AtomicUsize::new(0));
    let removed = Arc::new(AtomicUsize::new(0));
    let signatures = Arc::new(Mutex::new(Vec::<Value>::new()));
    let signed_entered = Arc::new(Notify::new());
    let signed_release = Arc::new(Notify::new());
    let refresh_entered = Arc::new(Notify::new());
    let refresh_release = Arc::new(Notify::new());
    let (address, http) = crate::test_tls::serve(
        Router::new()
            .route(
                "/spec",
                get({
                    let documents = documents.clone();
                    let entered = refresh_entered.clone();
                    let release = refresh_release.clone();
                    move || {
                        let documents = documents.clone();
                        let entered = entered.clone();
                        let release = release.clone();
                        async move {
                            if documents.fetch_add(1, Ordering::SeqCst) == 0 {
                                Json(json!({"paths":{
                                    "/paid":{"get":{"operationId":"original"}},
                                    "/gone":{"get":{"operationId":"gone"}}
                                }}))
                            } else {
                                entered.notify_one();
                                release.notified().await;
                                Json(json!({"paths":{
                                    "/paid":{"get":{"operationId":"replacement"}},
                                    "/fresh":{"get":{"operationId":"fresh"}}
                                }}))
                            }
                        }
                    }
                }),
            )
            .route(
                "/gone",
                get({
                    let removed = removed.clone();
                    move || {
                        removed.fetch_add(1, Ordering::SeqCst);
                        async { axum::http::StatusCode::GONE }
                    }
                }),
            )
            .route("/fresh", get(|| async { "fresh definitions" }))
            .route(
                "/paid",
                get({
                    let unsigned = unsigned.clone();
                    let signatures = signatures.clone();
                    let entered = signed_entered.clone();
                    let release = signed_release.clone();
                    move |headers: reqwest::header::HeaderMap| {
                        let unsigned = unsigned.clone();
                        let signatures = signatures.clone();
                        let entered = entered.clone();
                        let release = release.clone();
                        async move {
                            if let Some(header) = headers.get("payment-signature") {
                                let payload: Value = serde_json::from_slice(
                                    &STANDARD.decode(header.as_bytes()).unwrap(),
                                )
                                .unwrap();
                                let first = {
                                    let mut signatures = signatures.lock().unwrap();
                                    signatures.push(payload);
                                    signatures.len() == 1
                                };
                                if first {
                                    entered.notify_one();
                                    release.notified().await;
                                }
                                return "original paid result".into_response();
                            }
                            unsigned.fetch_add(1, Ordering::SeqCst);
                            let challenge = json!({
                                "x402Version":2,
                                "resource":{"url":"https://localhost/paid"},
                                "accepts":[{
                                    "scheme":"exact","network":"eip155:8453",
                                    "asset":crate::payment::USDC,"amount":"100",
                                    "payTo":"0x0000000000000000000000000000000000000003",
                                    "maxTimeoutSeconds":60,
                                    "extra":{"name":"USD Coin","version":"2"}
                                }]
                            });
                            (
                                axum::http::StatusCode::PAYMENT_REQUIRED,
                                [(
                                    "payment-required",
                                    STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
                                )],
                                "challenge",
                            )
                                .into_response()
                        }
                    }
                }),
            ),
    )
    .await;
    let path = directory.path().join("deployment.toml");
    std::fs::write(&path, format!(
        "version=1\n[wallets.payer]\nmode='static'\nprivate_key_env='UNUSED'\n\
         [sources.api]\nspec='https://{address}/spec'\nbase_url='https://{address}'\n\
         [servers.one]\nlisten='127.0.0.1:0'\nbearer_token_env='UNUSED'\nwallet='payer'\nsources=['api']\n\
         [servers.two]\nlisten='127.0.0.1:0'\nbearer_token_env='UNUSED'\nwallet='payer'\nsources=['api']\n"
    )).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let catalog = Arc::new(crate::catalog_state::CatalogState::new(
        crate::catalog_state::CatalogSnapshot {
            views: BTreeMap::from([("one".into(), Vec::new()), ("two".into(), Vec::new())]),
            ..Default::default()
        },
    ));
    let runtime = Runtime::new(
        deployment,
        BTreeMap::from([("payer".into(), client.clone())]),
        catalog.clone(),
        None,
        false,
    );
    runtime.publish().await.unwrap();
    let before = catalog.read();
    let paid = before.views["one"]
        .iter()
        .find(|tool| tool.tool.path == "/paid")
        .unwrap()
        .clone();
    let gone_one = before.views["one"]
        .iter()
        .find(|tool| tool.tool.path == "/gone")
        .unwrap()
        .clone();
    let gone_two = before.views["two"]
        .iter()
        .find(|tool| tool.tool.path == "/gone")
        .unwrap()
        .clone();
    assert!(Arc::ptr_eq(
        gone_one.refresh.as_ref().unwrap(),
        gone_two.refresh.as_ref().unwrap()
    ));
    let stop = CancellationToken::new();
    let publisher = tokio::spawn(runtime.run(stop.clone()));
    let accepted = tokio::spawn(async move { paid.invoke_output(&Default::default()).await });
    signed_entered.notified().await;
    let original_signature = signatures.lock().unwrap()[0].clone();
    assert_eq!(
        original_signature["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase(),
        bootstrap_address.to_ascii_lowercase()
    );

    let arguments = Default::default();
    let (first, second) = tokio::join!(
        gone_one.invoke_output(&arguments),
        gone_two.invoke_output(&arguments),
    );
    assert!(first.is_err() && second.is_err());
    refresh_entered.notified().await;
    assert_eq!(documents.load(Ordering::SeqCst), 2);
    assert!(!accepted.is_finished());
    store
        .call(move |state| {
            for job in destination_jobs {
                state.record_funding_balance(
                    &job.id,
                    &job.operation_id,
                    &job.wallet_id,
                    &job.recipient,
                    U256::from(2_000_000),
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    refresh_release.notify_one();
    // Test watchdog only: a lost publication must fail the regression visibly.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if catalog.read().views["one"]
                .iter()
                .any(|tool| tool.tool.path == "/fresh")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("replacement catalog was not published");
    let current = catalog.read();
    assert!(gone_one.refresh.as_ref().unwrap().check().is_err());
    for endpoint in ["one", "two"] {
        assert!(
            !current.views[endpoint]
                .iter()
                .any(|tool| tool.tool.path == "/gone")
        );
        let fresh = current.views[endpoint]
            .iter()
            .find(|tool| tool.tool.path == "/fresh")
            .unwrap();
        assert_eq!(
            fresh.invoke_output(&Default::default()).await.unwrap().text,
            "fresh definitions"
        );
    }
    let destination_call = current.views["two"]
        .iter()
        .find(|tool| tool.tool.path == "/paid")
        .unwrap();
    destination_call
        .invoke_output(&Default::default())
        .await
        .unwrap();
    let active = destination_manager.candidate().await.unwrap();
    assert_eq!(
        signatures.lock().unwrap()[1]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase(),
        active.address.to_ascii_lowercase()
    );
    assert_ne!(
        active.address.to_ascii_lowercase(),
        bootstrap_address.to_ascii_lowercase()
    );
    let pool = destination.clone();
    let low = active.wallet.clone();
    store
        .call(move |state| state.low_balance(&pool, &low))
        .await
        .unwrap();
    let rotated_call = current.views["one"]
        .iter()
        .find(|tool| tool.tool.path == "/paid")
        .unwrap();
    rotated_call
        .invoke_output(&Default::default())
        .await
        .unwrap();
    let rotated = destination_manager.candidate().await.unwrap();
    assert_ne!(rotated.wallet, active.wallet);
    assert_eq!(
        signatures.lock().unwrap()[2]["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase(),
        rotated.address.to_ascii_lowercase()
    );
    assert!(!accepted.is_finished());
    signed_release.notify_one();
    let original = accepted.await.unwrap().unwrap();
    assert_eq!(original.text, "original paid result");
    assert!(original.payment_may_have_occurred);
    assert_eq!(signatures.lock().unwrap()[0], original_signature);
    assert_eq!(signatures.lock().unwrap().len(), 3);
    assert_eq!(unsigned.load(Ordering::SeqCst), 3);
    assert_eq!(removed.load(Ordering::SeqCst), 2);
    assert_eq!(documents.load(Ordering::SeqCst), 2);

    let signed = signatures.lock().unwrap().clone();
    store
        .call(move |state| {
            let mut actual_jobs: Vec<_> = state
                .funding_jobs()?
                .into_iter()
                .map(|job| job.id)
                .collect();
            actual_jobs.sort();
            assert_eq!(
                actual_jobs, job_ids,
                "refresh and payer handoff must not allocate or top up"
            );
            let status = state.status()?;
            assert_eq!(
                status
                    .pools
                    .iter()
                    .map(|pool| pool.addresses.len())
                    .sum::<usize>(),
                3
            );
            let mut pending = state.payment_chain_query(&bootstrap)?.pending;
            pending.extend(state.payment_chain_query(&destination)?.pending);
            assert_eq!(
                pending.len(),
                3,
                "every disclosed signature retains its own liability"
            );
            for payload in signed {
                let authorization = &payload["payload"]["authorization"];
                assert!(pending.iter().any(|attempt| {
                    attempt
                        .payer
                        .eq_ignore_ascii_case(authorization["from"].as_str().unwrap())
                        && attempt
                            .nonce
                            .eq_ignore_ascii_case(authorization["nonce"].as_str().unwrap())
                }));
            }
            Ok(())
        })
        .await
        .unwrap();
    stop.cancel();
    drop(publisher.await.unwrap());
    client.drain().await;
    drop((
        current,
        before,
        catalog,
        gone_one,
        gone_two,
        client,
        bootstrap_manager,
        destination_manager,
        store,
    ));
    http.abort();
    let _ = http.await;
    worker.await.unwrap();
}
