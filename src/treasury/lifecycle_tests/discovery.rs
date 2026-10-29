//! Exact destination discovery through the production treasury and Base owners.
use super::*;
use crate::rotation::{
    base::BaseRpc,
    funding::{Backend, FundingBackend},
    near::NearClient,
};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

// Count transport construction even when a connection carries no RPC.
struct CountingListener {
    listener: tokio::net::TcpListener,
    connections: Arc<AtomicUsize>,
}
impl axum::serve::Listener for CountingListener {
    type Io = tokio::net::TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let connection = axum::serve::Listener::accept(&mut self.listener).await;
        self.connections.fetch_add(1, Ordering::SeqCst);
        connection
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

async fn base_fixture(balance: u64) -> (BaseRpc, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = Router::new().route(
        "/",
        post(move |Json(v): Json<Value>| {
            let calls = counter.clone();
            async move {
                assert_eq!(v["method"], "eth_call");
                assert_eq!(v["params"][1], "latest");
                calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({"jsonrpc":"2.0", "id":v["id"], "result":format!("0x{balance:064x}")}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 2).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, calls, server)
}

#[tokio::test]
async fn preparation_validates_network_once_and_rechecks_current_authority() {
    for actor in [false, true] {
        for matching_chain in [false, true] {
            check_preparation_probes(actor, matching_chain).await;
        }
    }
}

async fn check_preparation_probes(actor: bool, matching_chain: bool) {
    let (dir, _net, mut owner, endpoint) = funded_fixture(2).await;
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
    let deadline = now().unwrap() + 7200;
    save_quotes(&store, vec![job.clone()], deadline).await;
    let observed = owner.reusable_observation().await.unwrap().unwrap();
    let height = u32::try_from(observed.height.unwrap()).unwrap();
    let hash = owner
        .client
        .as_ref()
        .unwrap()
        .wallet()
        .read()
        .await
        .wallet_blocks
        .get(&height.into())
        .unwrap()
        .block_hash()
        .0;
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let treasury_id = store.call(|s| Ok(s.id().to_owned())).await.unwrap();
    let identity = crate::network::IsolationId::treasury(&treasury_id);
    let database = dir.path().join("state/state.sqlite");
    let pool = job.pool_id.clone();
    let app = Router::new().fallback(move |uri: axum::http::Uri| {
        let calls = recorded.clone();
        let database = database.clone();
        let pool = pool.clone();
        let identity = identity.clone();
        async move {
            let method = uri.path().rsplit('/').next().unwrap().to_owned();
            calls.lock().unwrap().push(method.clone());
            let mut message = Vec::new();
            match method.as_str() {
                "GetLightdInfo" => {
                    let chain = if matching_chain {
                        TreasuryNetwork::Synthetic.rpc_name()
                    } else {
                        "main"
                    };
                    message.extend([0x22, chain.len() as u8]);
                    message.extend(chain.as_bytes());
                    message.push(0x38);
                    protobuf_varint(u64::from(height), &mut message);
                    if !actor && matching_chain {
                        // Revoke current authority after the initial Store check.
                        // No Store handle is retained by this endpoint fixture.
                        rusqlite::Connection::open(database)
                            .unwrap()
                            .execute("UPDATE pools SET enabled=0 WHERE id=?1", [pool])
                            .unwrap();
                    }
                }
                "GetLatestBlock" => {
                    assert!(actor && matching_chain);
                    message.push(0x08);
                    protobuf_varint(u64::from(height), &mut message);
                    message.extend([0x12, hash.len() as u8]);
                    message.extend(hash);
                    // Force a subsequent factory call to create a new transport.
                    // The current canonical-validation request retains its clone.
                    crate::network::global().release_grpc(&identity).await;
                }
                _ => panic!("unexpected preparation RPC: {method}"),
            }
            let body = [
                vec![0],
                (message.len() as u32).to_be_bytes().to_vec(),
                message,
            ]
            .concat();
            (
                [("content-type", "application/grpc"), ("grpc-status", "0")],
                body,
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    // Keep the actual same-owner scan revision and anchor; only redirect this
    // fixture's validation endpoint, without configure_sync invalidating reuse.
    owner.sync_settings.as_mut().unwrap().endpoint =
        format!("http://{}", listener.local_addr().unwrap());
    owner.startup_discovery = false;
    let connections = Arc::new(AtomicUsize::new(0));
    let listener = CountingListener {
        listener,
        connections: connections.clone(),
    };
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (base, base_calls, base_server) = base_fixture(1).await;
    let mut remaining_owner = Some(owner);
    let error = if actor {
        let (handle, commands) = actor::channel();
        let stop = CancellationToken::new();
        let sender = submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap();
        let task = tokio::spawn(remaining_owner.take().unwrap().run_commands(
            commands,
            sender,
            stop.clone(),
        ));
        let error = handle
            .prepare_funding(request(&job, deadline), base)
            .await
            .err()
            .unwrap();
        // A second local rejection remains available through the same healthy
        // owner, without another canonical probe or accepted calculation.
        let mut invalid = request(&job, deadline);
        invalid.recipient = "invalid".into();
        let rejected = handle.prepare(invalid).await.err().unwrap();
        assert!(rejected.is::<PreparationDeferred>(), "{rejected:#}");
        stop.cancel();
        drop(handle);
        (error, Some(task))
    } else {
        let owner = remaining_owner.as_mut().unwrap();
        let error = owner.prepare(request(&job, deadline)).await.err().unwrap();
        assert!(owner.healthy);
        // Close only after checking the persistent ownership below.
        (error, None)
    };
    assert!(error.0.is::<PreparationDeferred>(), "{:#}", error.0);
    assert_eq!(
        connections.load(Ordering::SeqCst),
        1,
        "preparation must not open another connection after canonical validation"
    );
    assert_eq!(
        base_calls.load(Ordering::SeqCst),
        usize::from(actor && matching_chain)
    );
    assert_eq!(
        *calls.lock().unwrap(),
        if actor && matching_chain {
            vec!["GetLightdInfo", "GetLatestBlock"]
        } else {
            vec!["GetLightdInfo"]
        }
    );
    store
        .call(move |s| {
            s.require_unstarted_preparation(&job.operation_id)?;
            s.require_preparation_binding(
                &job.operation_id,
                Some(&job.pool_id),
                DEPOSIT,
                50_000,
                deadline,
            )?;
            let facts = s.funding_facts(&job.id, &job.operation_id)?;
            assert!(!facts.preparation_accepted && !facts.source_pending);
            assert_eq!(facts.initial_credit_recorded, actor && matching_chain);
            assert!(!s.status()?.outgoing_pending && s.status()?.treasury_operations.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    if let Some(task) = error.1 {
        task.await.unwrap().unwrap();
    } else {
        remaining_owner.unwrap().close().await.unwrap();
    }
    server.abort();
    base_server.abort();
}

fn protobuf_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

#[tokio::test]
async fn destination_discovery_follows_sdk_spendability_and_prevents_another_deposit() {
    check_destination(false, false).await;
    check_destination(true, false).await;
}

#[tokio::test]
async fn failed_positive_credit_publication_blocks_preparation_through_cached_cooldown() {
    check_destination(true, true).await;
}

#[tokio::test]
async fn optional_destination_failure_logs_debug_and_prepares_once() {
    if crate::test_log_capture::run_in_child(
        "treasury::lifecycle_tests::discovery::optional_destination_failure_logs_debug_and_prepares_once",
    ) {
        return;
    }
    use tracing::instrument::WithSubscriber;
    let (_dir, _net, mut owner, _endpoint) = funded_fixture(2).await;
    let pool = owner
        .ensure_capacity_group(
            "optional_destination".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    owner
        .allocate_capacity_kind(pool, AllocationKind::Small, true)
        .await
        .unwrap();
    let job = owner
        .store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let deadline = now().unwrap() + 7200;
    save_quotes(&owner.store, vec![job.clone()], deadline).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 2).unwrap();
    let app = Router::new().fallback(|| async {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "PRIVATE_UPSTREAM_BODY",
        )
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let logs = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(logs.reopen().unwrap())
        .finish();
    let prepared = owner
        .prepare_with_observation(
            request(&job, deadline),
            Some(base),
            Some(&CancellationToken::new()),
        )
        .with_subscriber(subscriber)
        .await
        .unwrap();
    let text = std::fs::read_to_string(logs.path()).unwrap();
    let event = text
        .lines()
        .find(|line| line.contains("funding_destination_observation_unavailable"))
        .unwrap();
    assert!(event.contains("DEBUG"), "{text}");
    assert!(
        !text.contains("WARN") && !text.contains("PRIVATE_"),
        "{text}"
    );
    owner
        .store
        .call(move |s| {
            let original = PreparedTransaction::load(s, &job.operation_id)?;
            assert_eq!(original.bytes(), prepared.bytes());
            assert_eq!(s.operation(&job.operation_id)?.attempts, 0);
            assert_eq!(s.status()?.treasury_operations.len(), 1);
            Ok(())
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
    server.abort();
}

async fn check_destination(funded: bool, publication_failure: bool) {
    let (dir, _net, owner, endpoint) =
        funded_fixture_amount(2, if funded { 1_000_000 } else { 10_000 }).await;
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
    let deadline = now().unwrap() + 7200;
    save_quotes(&store, vec![job.clone()], deadline).await;
    let (base, calls, server) = base_fixture(1).await;
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    let sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    if publication_failure {
        db.execute_batch("CREATE TRIGGER reject_credit BEFORE UPDATE OF readiness_requirement ON funding_jobs BEGIN SELECT RAISE(ABORT,'fixture credit publication failure'); END;").unwrap();
    }
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    let error = handle
        .prepare_funding(request(&job, deadline), base.clone())
        .await
        .err()
        .unwrap();
    assert!(error.is::<PreparationDeferred>(), "{error:#}");
    if !funded {
        assert_eq!(
            error.downcast_ref::<crate::rotation::error::FundingCondition>(),
            Some(&crate::rotation::error::FundingCondition::InsufficientFunds)
        );
    }
    if publication_failure {
        assert!(
            error.is::<crate::rotation::store::observation::BalanceStoreUnavailable>(),
            "{error:#}"
        );
        db.execute_batch("DROP TRIGGER reject_credit;").unwrap();
        let cached = handle
            .prepare_funding(request(&job, deadline), base)
            .await
            .err()
            .unwrap();
        assert!(
            cached.is::<crate::rotation::store::observation::BalanceStoreUnavailable>(),
            "{cached:#}"
        );
        assert!(cached.is::<PreparationDeferred>());
    }
    assert_eq!(calls.load(Ordering::SeqCst), usize::from(funded));
    store
        .call(move |s| {
            let facts = s.funding_facts(&job.id, &job.operation_id)?;
            assert_eq!(
                facts.initial_credit_recorded,
                funded && !publication_failure
            );
            assert!(!facts.preparation_accepted && !facts.source_pending);
            s.require_unstarted_preparation(&job.operation_id)?;
            assert!(!s.status()?.outgoing_pending && s.status()?.treasury_operations.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    stop.cancel();
    drop(handle);
    drop(store);
    task.await.unwrap().unwrap();
    server.abort();
}

#[tokio::test]
async fn credit_reuses_destination_observation_without_restoring_a_journal_debit() {
    let (_dir, _net, owner, _endpoint) = funded_fixture_amount(2, 10_000).await;
    owner
        .ensure_capacity_group(
            "alpha".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
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
    let (base, calls, server) = base_fixture(5_000_000).await;
    store
        .observe_balance(job.pool_id.clone(), job.wallet_id.clone(), base.clone())
        .await
        .unwrap();
    let observed = job.clone();
    store
        .call(move |s| {
            let payer = s.payment_candidate(&observed.pool_id)?;
            let payment = s.prepare_advisory_payment(
                &observed.pool_id,
                alloy_primitives::U256::from(1_000_000u64),
                "fixture",
                &payer,
            )?;
            let clock = now()?;
            s.journal_authorization(
                &payment,
                crate::rotation::store::Authorization {
                    payer: payer.address,
                    payee: "0x0000000000000000000000000000000000000001".into(),
                    nonce: format!("0x{:064x}", 1),
                    valid_after: clock.saturating_sub(1),
                    valid_before: clock + 60,
                },
                None,
            )
        })
        .await
        .unwrap();
    let (handle, _commands) = actor::channel();
    let mut backend = Backend {
        treasury: handle,
        store: store.clone(),
        near: NearClient::synthetic_at("http://127.0.0.1:1").unwrap(),
        base,
        wallets: BTreeMap::new(),
        funding: serde_json::from_value(json!({"base_rpc_url_env":"BASE"})).unwrap(),
        max_network_fee: 30_000,
    };
    backend.credit(&job).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "credit handoff shares the recipient cooldown"
    );
    store
        .call(move |s| {
            let value = s
                .status()?
                .pools
                .into_iter()
                .flat_map(|p| p.addresses)
                .find(|w| w.id == job.wallet_id)
                .unwrap();
            assert_eq!(value.confirmed_balance, "4000000");
            assert!(
                s.funding_facts(&job.id, &job.operation_id)?
                    .initial_credit_recorded
            );
            Ok(())
        })
        .await
        .unwrap();
    drop(backend);
    drop(store);
    owner.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn new_claim_resumes_shared_capital_discovery_and_detects_a_later_deposit() {
    let (_dir, net, mut owner, endpoint) = funded_fixture_amount(2, 10_000).await;
    owner
        .ensure_capacity_group(
            "alpha".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    owner.configure_funding_discovery(vec!["alpha".into()]);
    let address = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let store = owner.store_handle();
    let initial_revision = owner.revision;
    let (base, calls, server) = base_fixture(0).await;
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    let sender = submission::GrpcSubmission::with_network(
        endpoint.clone(),
        endpoint,
        TreasuryNetwork::Synthetic,
    )
    .unwrap();
    // Keep the test runtime active while SDK scans run on their private runtime;
    // paused-time auto-advance must not race blocking Store/scan publication.
    let active = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    let observation_due = owner.observation.due();
    let task = tokio::spawn(owner.run_commands(commands, sender, stop.clone()));
    // Complete the finite startup request by observing its already-current scan.
    tokio::task::yield_now().await;
    let job = store
        .call(|s| {
            s.fill_funding_vacancies("alpha")?;
            Ok(s.funding_jobs()?.remove(0))
        })
        .await
        .unwrap();
    let deadline = now().unwrap() + 7200;
    save_quotes(&store, vec![job.clone()], deadline).await;
    let error = handle
        .prepare_funding(request(&job, deadline), base.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<crate::rotation::error::FundingCondition>(),
        Some(&crate::rotation::error::FundingCondition::InsufficientFunds)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let deposit = faucet_funding_transaction(vec![(&address, 1_000_000, None)]).await;
    {
        let mut chain = net.chain.write().await;
        chain.mine_block(vec![deposit]);
        chain.mine_empty_blocks(2);
    }
    let before_due = observation_due
        .checked_duration_since(tokio::time::Instant::now() + Duration::from_secs(1))
        .expect("fixture setup must finish before the shared observation is due");
    tokio::time::advance(before_due).await;
    assert_eq!(
        store
            .call(|s| Ok(s.status()?.snapshot_revision))
            .await
            .unwrap(),
        initial_revision
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    loop {
        let ready = store
            .call(|s| {
                Ok(s.status()?.sync.is_some_and(|sync| {
                    sync.phase == SyncPhase::Ready && sync.spendable_shielded_zatoshis == 1_010_000
                }))
            })
            .await
            .unwrap();
        if ready {
            break;
        }
        tokio::task::yield_now().await;
    }
    // The new claim's wakeup/capital wait discovered the deposit without an
    // explicit sync command; immediate preparation uses actual SDK spendability.
    handle
        .prepare_funding(request(&job, deadline), base)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    stop.cancel();
    drop(handle);
    drop(store);
    task.await.unwrap().unwrap();
    tokio::time::resume();
    active.abort();
    server.abort();
}
