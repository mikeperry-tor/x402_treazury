//! Bootstrap routes capture ordinary pool authority, never foreground funding.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    manager::ManagedPool,
    store::{AllocationKind, Store, StoreHandle, funding::FundingJob},
    test_capacity::small_reserve,
};
use serde_json::json;

struct Fixture {
    _dir: tempfile::TempDir,
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    bootstrap: Arc<ManagedPool>,
    destination: Arc<ManagedPool>,
    job: FundingJob,
}
impl Fixture {
    fn new(bootstrap_cap: &str, destination_cap: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut state = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let mut pools = Vec::new();
        for (name, cap) in [
            ("bootstrap", bootstrap_cap),
            ("destination", destination_cap),
        ] {
            let pool = state
                .ensure_capacity_group(
                    name,
                    &small_reserve("2"),
                    SpendPolicy::dollars(cap).unwrap().max_atomic,
                    0,
                )
                .unwrap();
            state
                .allocate_capacity_kind(&pool, AllocationKind::Small, true)
                .unwrap();
            pools.push(pool);
        }
        let jobs = state.funding_jobs().unwrap();
        let job = jobs
            .iter()
            .find(|job| job.pool_id == pools[1])
            .unwrap()
            .clone();
        let bootstrap_job = jobs.iter().find(|job| job.pool_id == pools[0]).unwrap();
        state
            .record_credit(&bootstrap_job.wallet_id, "2000000", "fixture", 100)
            .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let mut managers =
            pools
                .into_iter()
                .zip([bootstrap_cap, destination_cap])
                .map(|(pool, cap)| {
                    Arc::new(
                        ManagedPool::new(
                            store.clone(),
                            pool,
                            BaseRpc::new("https://rpc.example").unwrap(),
                            SpendPolicy::dollars(cap).unwrap(),
                        )
                        .unwrap(),
                    )
                });
        let bootstrap = managers.next().unwrap();
        let destination = managers.next().unwrap();
        drop(managers);
        Self {
            _dir: dir,
            store,
            worker,
            bootstrap,
            destination,
            job,
        }
    }
    fn client(&self) -> PaidClient {
        PaidClient::managed(self.destination.clone()).with_bootstrap(self.bootstrap.clone())
    }
    async fn credit(&self, amount: u64) {
        let job = self.job.clone();
        self.store
            .call(move |state| {
                state.record_funding_balance(
                    &job.id,
                    &job.operation_id,
                    &job.wallet_id,
                    &job.recipient,
                    U256::from(amount),
                )
            })
            .await
            .unwrap();
    }
    async fn demand(&self) -> bool {
        let pool = self.job.pool_id.clone();
        self.store
            .call(move |state| state.has_bootstrap_demand(&pool))
            .await
            .unwrap()
    }
    async fn close(self) {
        drop(self.bootstrap);
        drop(self.destination);
        drop(self.store);
        self.worker.await.unwrap();
    }
}

#[tokio::test]
async fn concurrent_capture_coalesces_demand_and_partial_credit_never_reopens_bootstrap() {
    let fixture = Fixture::new("0.1", "0.5");
    let client = fixture.client();
    let metadata = client.capture_effective(false).await.unwrap();
    assert!(Arc::ptr_eq(
        metadata.managed.as_ref().unwrap(),
        &fixture.bootstrap
    ));
    assert!(!fixture.demand().await);
    let (one, two) = tokio::join!(
        client.capture_effective(true),
        client.capture_effective(true)
    );
    let (one, two) = (one.unwrap(), two.unwrap());
    assert!(fixture.demand().await);
    assert!(Arc::ptr_eq(
        one.managed.as_ref().unwrap(),
        two.managed.as_ref().unwrap()
    ));
    fixture.credit(1).await;
    let direct = client.capture_effective(true).await.unwrap();
    assert!(Arc::ptr_eq(
        direct.managed.as_ref().unwrap(),
        &fixture.destination
    ));
    assert!(
        Arc::ptr_eq(
            one.capture_effective(false)
                .await
                .unwrap()
                .managed
                .as_ref()
                .unwrap(),
            &fixture.bootstrap
        ),
        "captured work retains its authority"
    );
    fixture.credit(0).await;
    assert!(Arc::ptr_eq(
        client
            .capture_effective(false)
            .await
            .unwrap()
            .managed
            .as_ref()
            .unwrap(),
        &fixture.destination
    ));
    assert_eq!(
        fixture
            .store
            .call(|state| Ok(state.funding_jobs()?.len()))
            .await
            .unwrap(),
        2,
        "captures allocate nothing"
    );
    drop((client, metadata, one, two, direct));
    fixture.close().await;
}

#[tokio::test]
async fn directory_and_relay_capture_do_not_activate_and_readiness_uses_effective_cap() {
    let fixture = Fixture::new("1", "0.5");
    let client = fixture.client();
    for metadata in [
        client
            .clone()
            .with_relay_purpose(crate::discovery_relay::Purpose::SourceDiscovery),
        client.clone().for_relay(),
    ] {
        metadata.capture_effective(true).await.unwrap();
    }
    assert!(!fixture.demand().await);
    assert!(
        PaidClient::startup_funding_ready(std::slice::from_ref(&client))
            .await
            .unwrap()
    );
    fixture.credit(1).await;
    assert!(
        !PaidClient::startup_funding_ready(std::slice::from_ref(&client))
            .await
            .unwrap()
    );
    let changes = client.discovery_changes().await.unwrap();
    assert_eq!(changes.len(), 2, "both route candidates can wake readiness");
    let without = client.clone().without_bootstrap();
    assert!(Arc::ptr_eq(
        without
            .capture_effective(false)
            .await
            .unwrap()
            .managed
            .as_ref()
            .unwrap(),
        &fixture.destination
    ));
    drop((client, without));
    fixture.close().await;
}

#[tokio::test]
async fn trusted_bound_route_activates_free_calls_but_invalid_calls_and_help_do_not() {
    use axum::{Router, routing::get};
    let fixture = Fixture::new("0.1", "0.5");
    let (address, server) =
        crate::test_tls::serve(Router::new().route("/read/{q}", get(|| async { "free" }))).await;
    let config: crate::catalog::Config = toml::from_str("").unwrap();
    let document = json!({"paths":{"/read/{q}":{"get":{"parameters":[{"in":"path","name":"q","required":true,"schema":{"type":"string"}}],"responses":{}}}}});
    let tool = crate::catalog::build_tools(&config, &document, "api")
        .unwrap()
        .remove(0);
    let mut bound =
        crate::catalog_state::bind(vec![(tool, fixture.client(), format!("https://{address}"))])
            .remove(0);
    assert!(bound.invoke_output(&Default::default()).await.is_err());
    for invalid_query in [json!({"nested": 1}), json!([["nested"]])] {
        let args = json!({"q":"value", "filter":invalid_query});
        let error = bound
            .invoke_output(args.as_object().unwrap())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("query"), "{error:#}");
        assert!(!fixture.demand().await);
    }
    let args = json!({"q":"value"});
    assert_eq!(
        fixture
            .client()
            .execute(
                bound
                    .tool
                    .route(&bound.base, args.as_object().unwrap())
                    .unwrap()
            )
            .await
            .unwrap(),
        "free",
    );
    assert!(!fixture.demand().await);
    let mut help = bound.clone();
    help.tool.help_url = Some("https://unused.example/help".into());
    help.help.set("cached help".into()).unwrap();
    assert_eq!(
        help.invoke_output(&Default::default()).await.unwrap().text,
        "cached help"
    );
    assert!(!fixture.demand().await);
    // A tool whose authored name contains help remains an ordinary API call.
    bound.tool.name = "api_help".into();
    assert_eq!(
        bound
            .invoke_output(json!({"q":"value"}).as_object().unwrap())
            .await
            .unwrap()
            .text,
        "free"
    );
    assert!(fixture.demand().await);
    assert_eq!(
        fixture
            .store
            .call(|state| Ok(state.funding_jobs()?.len()))
            .await
            .unwrap(),
        2
    );
    drop((bound, help));
    server.abort();
    fixture.close().await;
}

#[tokio::test]
async fn participant_help_reads_healthy_origin_while_wallet_store_is_busy() {
    use axum::{Router, routing::get};
    let fixture = Fixture::new("0.1", "0.5");
    let (address, server) =
        crate::test_tls::serve(Router::new().route("/help", get(|| async { "provider help" })))
            .await;
    let config: crate::catalog::Config = toml::from_str("").unwrap();
    let document = json!({"paths":{"/read":{"get":{"responses":{}}}}});
    let mut tool = crate::catalog::build_tools(&config, &document, "api")
        .unwrap()
        .remove(0);
    tool.help_url = Some(format!("https://{address}/help"));
    let help =
        crate::catalog_state::bind(vec![(tool, fixture.client(), format!("https://{address}"))])
            .remove(0);
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let worker = tokio::spawn({
        let store = fixture.store.clone();
        async move {
            store
                .call(move |_| {
                    let _ = entered.send(());
                    let _ = resume.recv();
                    Ok(())
                })
                .await
                .unwrap();
        }
    });
    waiting.await.unwrap();
    let args = Default::default();
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), help.invoke_output(&args)).await;
    release.send(()).unwrap();
    worker.await.unwrap();
    assert_eq!(
        result
            .expect("unsigned help should bypass wallet storage")
            .unwrap()
            .text,
        "provider help"
    );
    assert!(!fixture.demand().await);
    server.abort();
    drop(help);
    fixture.close().await;
}

#[tokio::test]
async fn effective_caps_and_unsigned_handoff_keep_the_original_payer() {
    use axum::{Router, response::IntoResponse, routing::get};
    use std::sync::Mutex;
    for (bootstrap_cap, amount, credit_during_unsigned) in
        [("0.1", "200000", false), ("1", "750000", true)]
    {
        let fixture = Fixture::new(bootstrap_cap, "0.5");
        let initial_address = fixture
            .bootstrap
            .candidate()
            .await
            .unwrap()
            .address
            .to_ascii_lowercase();
        let destination_address = fixture.job.recipient.to_ascii_lowercase();
        let signed = Arc::new(Mutex::new(Vec::new()));
        let signatures = signed.clone();
        let credit = Arc::new(Mutex::new(
            credit_during_unsigned.then(|| (fixture.store.clone(), fixture.job.clone())),
        ));
        let handler_credit = credit.clone();
        let app = Router::new().route("/paid", get(move |headers: reqwest::header::HeaderMap| {
            let signatures = signatures.clone();
            let credit = handler_credit.clone();
            async move {
                if let Some(header) = headers.get("payment-signature") {
                    let payload: Value = serde_json::from_slice(&STANDARD.decode(header.as_bytes()).unwrap()).unwrap();
                    signatures.lock().unwrap().push(payload["payload"]["authorization"]["from"].as_str().unwrap().to_owned());
                    return "paid".into_response();
                }
                let job = credit.lock().unwrap().take();
                if let Some((store, job)) = job {
                    store.call(move |state| state.record_funding_balance(&job.id, &job.operation_id, &job.wallet_id, &job.recipient, U256::ONE)).await.unwrap();
                }
                let challenge = json!({"x402Version":2,"resource":{"url":"https://localhost/paid"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":amount,"payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                (reqwest::StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))], "unpaid").into_response()
            }
        }));
        let (address, server) = crate::test_tls::serve(app).await;
        let config: crate::catalog::Config = toml::from_str("").unwrap();
        let document = json!({"paths":{"/paid":{"get":{"responses":{}}}}});
        let tool = crate::catalog::build_tools(&config, &document, "paid")
            .unwrap()
            .remove(0);
        let bound = crate::catalog_state::bind(vec![(
            tool,
            fixture.client(),
            format!("https://{address}"),
        )])
        .remove(0);
        let first = bound.invoke_output(&Default::default()).await;
        assert!(
            fixture.demand().await,
            "cap refusal cannot undo accepted demand"
        );
        if credit_during_unsigned {
            assert_eq!(first.unwrap().text, "paid");
            assert_eq!(
                signed.lock().unwrap().as_slice(),
                &[initial_address],
                "credit during challenge does not move accepted call"
            );
            let second = bound.invoke_output(&Default::default()).await.unwrap_err();
            assert!(second.is::<crate::rotation::error::AdmissionError>());
            assert_eq!(
                signed.lock().unwrap().len(),
                1,
                "destination's lower cap applies after handoff"
            );
        } else {
            assert!(
                first
                    .unwrap_err()
                    .is::<crate::rotation::error::AdmissionError>()
            );
            assert!(signed.lock().unwrap().is_empty());
            fixture.credit(1).await;
            assert_eq!(
                bound.invoke_output(&Default::default()).await.unwrap().text,
                "paid"
            );
            assert_eq!(
                signed.lock().unwrap().as_slice(),
                &[destination_address],
                "destination's larger cap applies after first credit"
            );
        }
        assert_eq!(
            fixture
                .store
                .call(|state| Ok(state.funding_jobs()?.len()))
                .await
                .unwrap(),
            2
        );
        drop((bound, credit));
        server.abort();
        // Allow the aborted handler's retained StoreHandle to be dropped.
        let _ = server.await;
        fixture.close().await;
    }
}

#[tokio::test]
async fn signed_retry_stays_in_bootstrap_inventory_and_later_payment_opens_destination_refill() {
    use axum::{Router, response::IntoResponse, routing::get};
    use std::sync::Mutex;

    let fixture = Fixture::new("1", "1");
    let initial = fixture
        .bootstrap
        .candidate()
        .await
        .unwrap()
        .address
        .to_ascii_lowercase();
    let destination = fixture.job.recipient.to_ascii_lowercase();
    let signed = Arc::new(Mutex::new(Vec::new()));
    let credit = Arc::new(Mutex::new(Some((
        fixture.store.clone(),
        fixture.job.clone(),
    ))));
    let challenge = json!({"x402Version":2,"resource":{"url":"https://localhost/paid"},
        "accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"10000",
        "payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,
        "extra":{"name":"USD Coin","version":"2"}}]});
    let challenge = STANDARD.encode(challenge.to_string());
    let app = Router::new().route(
        "/paid",
        get({
            let signed = signed.clone();
            let credit = credit.clone();
            move |headers: reqwest::header::HeaderMap| {
                let (signed, credit, challenge) =
                    (signed.clone(), credit.clone(), challenge.clone());
                async move {
                    let Some(signature) = headers.get("payment-signature") else {
                        return (
                            reqwest::StatusCode::PAYMENT_REQUIRED,
                            [("payment-required", challenge)],
                        )
                            .into_response();
                    };
                    let payload: Value =
                        serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap())
                            .unwrap();
                    let ordinal = {
                        let mut signed = signed.lock().unwrap();
                        signed.push(
                            payload["payload"]["authorization"]["from"]
                                .as_str()
                                .unwrap()
                                .to_ascii_lowercase(),
                        );
                        signed.len()
                    };
                    if ordinal == 2 {
                        let (store, job) = credit.lock().unwrap().take().unwrap();
                        store
                            .call(move |s| {
                                s.record_credit(&job.wallet_id, "1000000", "fixture", 101)
                            })
                            .await
                            .unwrap();
                        let failure: Value = serde_json::from_str(include_str!(
                            "../../tests/fixtures/x402/insufficient-funds.json"
                        ))
                        .unwrap();
                        return (
                            reqwest::StatusCode::PAYMENT_REQUIRED,
                            [(
                                "payment-response",
                                STANDARD.encode(failure["settlement"].to_string()),
                            )],
                        )
                            .into_response();
                    }
                    "paid".into_response()
                }
            }
        }),
    );
    let (address, server) = crate::test_tls::serve(app).await;
    let config: crate::catalog::Config = toml::from_str("").unwrap();
    let tool = crate::catalog::build_tools(
        &config,
        &json!({"paths":{"/paid":{"get":{"responses":{}}}}}),
        "paid",
    )
    .unwrap()
    .remove(0);
    let base = format!("https://{address}");
    let args = Default::default();
    // Consume bootstrap's READY claim, then make one sibling available.
    let bootstrap = PaidClient::managed(fixture.bootstrap.clone());
    assert_eq!(
        bootstrap
            .execute(tool.route(&base, &args).unwrap())
            .await
            .unwrap(),
        "paid"
    );
    let bootstrap_pool = fixture.bootstrap.group_name().to_owned();
    let sibling = fixture
        .store
        .call(move |s| {
            let id = s
                .allocate_capacity_kind(&bootstrap_pool, AllocationKind::Small, true)?
                .unwrap();
            let job = s.funding_job(&id)?.unwrap();
            s.record_credit(&job.wallet_id, "1000000", "fixture", 100)?;
            Ok(job.recipient.to_ascii_lowercase())
        })
        .await
        .unwrap();
    let bound = crate::catalog_state::bind(vec![(tool, fixture.client(), base)]).remove(0);
    assert_eq!(bound.invoke(&args).await.unwrap(), "paid");
    assert_eq!(*signed.lock().unwrap(), [initial.clone(), initial, sibling]);
    assert_eq!(bound.invoke(&args).await.unwrap(), "paid");
    assert_eq!(signed.lock().unwrap().last(), Some(&destination));
    assert_eq!(signed.lock().unwrap().len(), 4);

    let original = fixture.job.clone();
    fixture
        .store
        .call(move |s| {
            assert_eq!(s.funding_jobs()?.len(), 3, "API admission does not refill");
            assert_eq!(
                s.bootstrap_allocation_groups(&["destination".into()])?,
                ["destination".into()].into()
            );
            s.fill_funding_vacancies("destination")?;
            let jobs = s.funding_jobs()?;
            assert_eq!(jobs.len(), 4);
            assert_eq!(
                jobs.iter()
                    .filter(|job| job.pool_id == original.pool_id)
                    .count(),
                2
            );
            assert!(
                jobs.iter()
                    .any(|job| job.id == original.id && job.operation_id == original.operation_id)
            );
            Ok(())
        })
        .await
        .unwrap();
    server.abort();
    let _ = server.await;
    drop((bound, bootstrap, credit));
    fixture.close().await;
}

#[tokio::test]
async fn unavailable_transport_still_accepts_demand_without_allocating() {
    let fixture = Fixture::new("1", "0.5");
    let client = PaidClient::unavailable(
        crate::rotation::error::AdmissionError::FundingUnavailable("fixture"),
    )
    .with_bootstrap_demand(fixture.store.clone(), fixture.job.pool_id.clone());
    client.capture_effective(false).await.unwrap();
    assert!(!fixture.demand().await);
    let captured = client.capture_effective(true).await.unwrap();
    assert!(fixture.demand().await);
    assert!(captured.discovery_ready().await.is_err());
    assert_eq!(
        fixture
            .store
            .call(|state| Ok(state.funding_jobs()?.len()))
            .await
            .unwrap(),
        2
    );
    drop((client, captured));
    fixture.close().await;
}

#[tokio::test]
async fn readiness_follows_current_finite_cap_when_other_group_is_unlimited() {
    for (bootstrap_cap, destination_cap, before_credit, after_credit) in
        [("1", "none", true, false), ("none", "0.5", false, true)]
    {
        let fixture = Fixture::new(bootstrap_cap, destination_cap);
        let client = fixture.client();
        assert!(client.startup_funding_pool().is_some());
        assert_eq!(
            PaidClient::startup_funding_ready(std::slice::from_ref(&client))
                .await
                .unwrap(),
            before_credit
        );
        fixture.credit(1_000_000).await;
        assert_eq!(
            PaidClient::startup_funding_ready(std::slice::from_ref(&client))
                .await
                .unwrap(),
            after_credit
        );
        drop(client);
        fixture.close().await;
    }
}
