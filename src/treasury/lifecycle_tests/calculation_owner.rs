//! Actual deposit/shield calculation remains owned after its async waiter leaves.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Release(Option<std::sync::mpsc::Sender<()>>);
impl Release {
    fn release(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.release();
    }
}
#[derive(Clone, Copy)]
enum Kind {
    Deposit,
    Shield,
}

fn hold(
    owner: &mut Treasury,
    panic_calculation: bool,
    panic_facts: bool,
    before_acceptance: bool,
) -> (
    tokio::sync::oneshot::Receiver<String>,
    Release,
    Arc<AtomicUsize>,
) {
    let (entered, reached) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    owner.calculation_control = Some(tokio::sync::Mutex::new(publication::CalculationControl {
        entered: Some(entered),
        before_acceptance,
        release: wait,
        calls: calls.clone(),
        panic_calculation,
        panic_facts,
    }));
    (reached, Release(Some(release)), calls)
}

async fn fixture(kind: Kind) -> (tempfile::TempDir, MockNet, Treasury, String) {
    let (dir, net, mut owner, _) = funded_fixture(2).await;
    let destination = match kind {
        Kind::Deposit => DEPOSIT.to_owned(),
        Kind::Shield => {
            let (_, receiver) = owner
                .client
                .as_mut()
                .unwrap()
                .generate_transparent_address(zip32::AccountId::ZERO, false)
                .await
                .unwrap();
            let address =
                zcash_keys::address::Address::Transparent(receiver).encode(&owner.network.chain());
            let credit = faucet_funding_transaction(vec![(&address, 1_000_000, None)]).await;
            {
                let mut chain = net.chain.write().await;
                chain.mine_block(vec![credit]);
                chain.mine_empty_blocks(2);
            }
            owner.sync_once(&CancellationToken::new()).await.unwrap();
            address
        }
    };
    net.chain
        .write()
        .await
        .faults
        .inject(Rpc::SendTransaction, Fault::EndStream { after: 0 });
    (dir, net, owner, destination)
}
async fn no_send(net: &MockNet) {
    assert_eq!(
        net.chain.read().await.faults.pending(Rpc::SendTransaction),
        1,
        "accepted calculation must never disclose or replay a transaction"
    );
}

async fn prepare(
    owner: &mut Treasury,
    kind: Kind,
    destination: String,
) -> Result<PreparedTransaction> {
    match kind {
        Kind::Deposit => {
            owner
                .prepare(PrepareRequest {
                    managed_funding: None,
                    operation_id: uuid::Uuid::new_v4().to_string(),
                    pool_id: None,
                    recipient: destination,
                    amount_zatoshis: 50_000,
                    max_fee_zatoshis: 50_000,
                    max_input_zatoshis: 100_000,
                    deadline: now()? + 600,
                })
                .await
        }
        Kind::Shield => {
            owner
                .shield_address(destination, None, 30_000, &CancellationToken::new())
                .await
        }
    }
}

fn accepted_id(dir: &std::path::Path) -> String {
    rusqlite::Connection::open(dir.join("state/state.sqlite"))
        .unwrap()
        .query_row(
            "SELECT operation_id FROM operation_network WHERE preparation IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn open(dir: &std::path::Path, key: &std::path::Path, identity: &str) -> Result<Store> {
    Store::open_with_network(dir, key, identity, TreasuryNetwork::Synthetic)
}

async fn evidence(
    owner: &Treasury,
    id: String,
    inputs: Vec<crate::rotation::store::preparation::OwnedInput>,
    prepared: &PreparedTransaction,
) {
    let raw = prepared.bytes().to_vec();
    owner
        .store
        .call(move |s| {
            assert_eq!(s.prepared_inputs(&id)?, inputs);
            assert_eq!(s.prepared_bytes(&id)?.as_slice(), raw);
            assert!(s.require_unstarted_preparation(&id).is_err());
            assert!(s.abandon_unprepared(&id).is_err());
            let operation = s.operation(&id)?;
            assert_eq!(operation.attempts, 0);
            assert_eq!(operation.submission, "PREPARED");
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn dropped_deposit_and_shield_waiters_keep_one_owned_calculation_and_unrelated_progress() {
    for kind in [Kind::Deposit, Kind::Shield] {
        let (dir, net, mut owner, destination) = fixture(kind).await;
        let (reached, mut release, calls) = hold(&mut owner, false, false, false);
        {
            let calculation = prepare(&mut owner, kind, destination);
            tokio::pin!(calculation);
            tokio::select! {
                result = &mut calculation => panic!("calculation returned before accepted barrier: {}", result.is_ok()),
                result = reached => result.unwrap(),
            };
        }
        assert!(owner.client.is_none() && owner.running.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // The current-thread executor and Store remain usable while CPU ownership
        // is held elsewhere; the SDK view reports its own unavailability.
        assert!(!owner.status().await.unwrap().outgoing_pending);
        assert!(owner.addresses().await.is_err());
        assert!(owner.snapshot_balances().await.is_err());
        let (send, recv) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            send.send(42).unwrap();
        });
        assert_eq!(recv.await.unwrap(), 42);
        let id = accepted_id(dir.path());
        let read_id = id.clone();
        let inputs = owner
            .store
            .call(move |s| s.prepared_inputs(&read_id))
            .await
            .unwrap();
        assert!(!inputs.is_empty());
        {
            let mut queued = Box::pin(owner.derive_address());
            assert!(futures_util::poll!(queued.as_mut()).is_pending());
        }
        assert!(owner.running.is_some());
        release.release();
        let prepared = owner.finish_calculation().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(owner.healthy && owner.running.is_none());
        assert_eq!(prepared.operation_id(), id);
        let facts = prepared.facts().unwrap();
        assert_eq!(
            facts.amount_zatoshis,
            match kind {
                Kind::Deposit => 50_000,
                Kind::Shield => 0,
            }
        );
        assert!(facts.fee_zatoshis > 0 && facts.fee_zatoshis <= 50_000);
        evidence(&owner, id.clone(), inputs.clone(), &prepared).await;
        let raw = prepared.bytes().to_vec();
        let identity = owner.status().await.unwrap().treasury_id;
        let snapshot = owner
            .store
            .call({
                let id = id.clone();
                move |s| s.prepared_snapshot(&id)
            })
            .await
            .unwrap();
        no_send(&net).await;
        owner.close().await.unwrap();
        let restored = Treasury::open_with_network(
            dir.path().join("state"),
            dir.path().join("key"),
            identity,
            TreasuryNetwork::Synthetic,
        )
        .await
        .unwrap();
        restored
            .store
            .call(move |s| {
                assert_eq!(s.prepared_bytes(&id)?.as_slice(), raw);
                assert_eq!(s.prepared_snapshot(&id)?.as_slice(), snapshot.as_slice());
                assert_eq!(s.prepared_inputs(&id)?, inputs);
                assert_eq!(s.operation(&id)?.attempts, 0);
                Ok(())
            })
            .await
            .unwrap();
        restored.close().await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn calculation_panic_terminates_before_reconstruction_and_facts_panic_retains_exact_bytes() {
    for facts_panic in [false, true] {
        let (dir, net, mut owner, destination) = fixture(Kind::Deposit).await;
        let (reached, mut release, calls) = hold(&mut owner, !facts_panic, facts_panic, false);
        {
            let calculation = prepare(&mut owner, Kind::Deposit, destination);
            tokio::pin!(calculation);
            tokio::select! {
                result = &mut calculation => panic!("unexpected early result: {}", result.is_ok()),
                result = reached => result.unwrap(),
            };
        }
        let id = accepted_id(dir.path());
        release.release();
        let error = owner.finish_calculation().await.err().unwrap();
        assert!(!error.is::<PreparationDeferred>());
        assert!(!owner.healthy && owner.running.is_none() && owner.client.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(facts_panic));
        let check = id.clone();
        owner
            .store
            .call(move |s| {
                assert!(s.require_unstarted_preparation(&check).is_err());
                assert!(!s.prepared_inputs(&check)?.is_empty());
                if facts_panic {
                    let prepared = PreparedTransaction::load(s, &check)?;
                    assert!(!prepared.bytes().is_empty());
                    assert!(prepared.facts().is_none());
                    assert_eq!(s.operation(&check)?.attempts, 0);
                    assert!(s.request_broadcast(&check, 1, 1, false).is_err());
                } else {
                    assert!(s.prepared_bytes(&check).is_err());
                }
                Ok(())
            })
            .await
            .unwrap();
        owner
            .reconstruct_sdk(&CancellationToken::new())
            .await
            .unwrap();
        assert!(owner.healthy);
        owner.close().await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn whole_treasury_drop_keeps_deposit_and_shield_bytes_owned_until_local_commit() {
    for kind in [Kind::Deposit, Kind::Shield] {
        let (dir, net, mut owner, destination) = fixture(kind).await;
        let identity = owner.status().await.unwrap().treasury_id;
        let (reached, mut release, calls) = hold(&mut owner, false, false, false);
        {
            let calculation = prepare(&mut owner, kind, destination);
            tokio::pin!(calculation);
            tokio::select! {
                result = &mut calculation => panic!("unexpected early result: {}", result.is_ok()),
                result = reached => result.unwrap(),
            };
        }
        let id = accepted_id(dir.path());
        let inputs = owner
            .store
            .call({
                let id = id.clone();
                move |s| s.prepared_inputs(&id)
            })
            .await
            .unwrap();
        no_send(&net).await;
        drop(owner);
        assert!(
            open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &identity
            )
            .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        release.release();
        // Exclusive reopen, not a publication event or dropped reply, proves the
        // detached blocking calculation and its last Store sender have drained.
        let store = loop {
            if let Ok(store) = open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &identity,
            ) {
                break store;
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let prepared = PreparedTransaction::load(&store, &id).unwrap();
        assert!(!prepared.bytes().is_empty() && prepared.facts().is_some());
        assert_eq!(store.prepared_inputs(&id).unwrap(), inputs);
        assert_eq!(store.operation(&id).unwrap().attempts, 0);
        assert!(!store.prepared_snapshot(&id).unwrap().is_empty());
        no_send(&net).await;
    }
}

#[test]
fn normal_runtime_drop_drains_started_deposit_and_shield_without_an_external_store_sender() {
    struct Aborted(std::sync::mpsc::Sender<()>);
    impl Drop for Aborted {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    for kind in [Kind::Deposit, Kind::Shield] {
        let (handoff, receive) = std::sync::mpsc::channel();
        let (aborted, runtime_aborted) = std::sync::mpsc::channel();
        let (finished, runtime_finished) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let artifacts = runtime.block_on(async move {
                let (dir, net, mut owner, destination) = fixture(kind).await;
                let identity = owner.status().await.unwrap().treasury_id;
                let (reached, release, calls) = hold(&mut owner, false, false, false);
                {
                    let calculation = prepare(&mut owner, kind, destination);
                    tokio::pin!(calculation);
                    tokio::select! {
                        result = &mut calculation => panic!("unexpected early result: {}", result.is_ok()),
                        result = reached => result.unwrap(),
                    };
                }
                let id = accepted_id(dir.path());
                let inputs = owner.store.call({ let id = id.clone(); move |s| s.prepared_inputs(&id) }).await.unwrap();
                let (started, ready) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    let _guard = Aborted(aborted);
                    let _ = started.send(());
                    std::future::pending::<()>().await;
                });
                ready.await.unwrap();
                drop(owner); // No Store clone is returned to keep its worker alive.
                (dir, identity, id, inputs, release, calls, net.chain.clone())
            });
            handoff.send(artifacts).unwrap();
            drop(runtime);
            finished.send(()).unwrap();
        });
        let (dir, identity, id, inputs, mut release, calls, chain) = receive.recv().unwrap();
        // A real task's Drop proves normal Runtime::drop reached its async-abort
        // phase while the accepted blocking calculation is still held.
        runtime_aborted.recv().unwrap();
        assert!(matches!(
            runtime_finished.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert!(
            open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &identity
            )
            .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        release.release();
        runtime_finished.recv().unwrap();
        thread.join().unwrap();
        assert_eq!(
            chain
                .try_read()
                .unwrap()
                .faults
                .pending(Rpc::SendTransaction),
            1
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let store = open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &identity,
        )
        .unwrap();
        let prepared = PreparedTransaction::load(&store, &id).unwrap();
        assert!(!prepared.bytes().is_empty() && prepared.facts().is_some());
        assert_eq!(store.prepared_inputs(&id).unwrap(), inputs);
        assert_eq!(store.operation(&id).unwrap().attempts, 0);
        assert!(!store.prepared_snapshot(&id).unwrap().is_empty());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_unsigned_preflight_leaves_unstarted_intent_and_next_selection_clears_the_proposal()
 {
    let (_dir, net, mut owner, destination) = fixture(Kind::Deposit).await;
    let id = uuid::Uuid::new_v4().to_string();
    let request = || PrepareRequest {
        managed_funding: None,
        operation_id: id.clone(),
        pool_id: None,
        recipient: destination.clone(),
        amount_zatoshis: 50_000,
        max_fee_zatoshis: 50_000,
        max_input_zatoshis: 100_000,
        deadline: now().unwrap() + 600,
    };
    let revision = owner.revision;
    net.chain.write().await.faults.inject(
        Rpc::LightdInfo,
        Fault::Delay(Duration::from_secs(publication::MAINTENANCE_SECONDS)),
    );
    tokio::time::pause();
    {
        let preparation = owner.prepare(request());
        tokio::pin!(preparation);
        let received = async {
            loop {
                if net.chain.read().await.faults.pending(Rpc::LightdInfo) == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        };
        tokio::select! {
            result = &mut preparation => panic!("preflight returned before held response: {}", result.is_ok()),
            _ = received => {},
        }
    }
    tokio::time::resume();
    assert!(owner.healthy && owner.running.is_none() && owner.client.is_some());
    assert_eq!(owner.revision, revision);
    owner
        .store
        .call({
            let id = id.clone();
            move |s| {
                s.require_unstarted_preparation(&id)?;
                assert!(s.prepared_inputs(&id).is_err());
                assert!(s.prepared_bytes(&id).is_err());
                Ok(())
            }
        })
        .await
        .unwrap();
    let prepared = owner.prepare(request()).await.unwrap();
    assert_eq!(prepared.operation_id(), id);
    assert!(owner.healthy && owner.running.is_none());
    owner
        .store
        .call(move |s| {
            assert_eq!(s.operation(&id)?.attempts, 0);
            assert!(!s.prepared_inputs(&id)?.is_empty());
            assert!(!s.prepared_bytes(&id)?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    no_send(&net).await;
    owner.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_shield_waiter_with_preaccept_stop_closes_cleanly_without_accepting_inputs() {
    let (dir, net, mut owner, destination) = fixture(Kind::Shield).await;
    let identity = owner.status().await.unwrap().treasury_id;
    let (reached, mut release, calls) = hold(&mut owner, false, false, true);
    let stop = CancellationToken::new();
    let operation = {
        let calculation = owner.shield_address(destination, None, 30_000, &stop);
        tokio::pin!(calculation);
        tokio::select! {
            result = &mut calculation => panic!("shielding returned before handoff barrier: {}", result.is_ok()),
            result = reached => result.unwrap(),
        }
    };
    assert!(owner.running.is_some() && owner.client.is_none());
    stop.cancel();
    release.release();
    // This joins the real owner: its existing shielding stop check rejects SQL
    // acceptance and its serialized unstarted proof yields typed Deferred.
    // That expected abandoned operation is not a physical shutdown failure.
    owner.close().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    no_send(&net).await;
    let store = open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &identity,
    )
    .unwrap();
    store.require_unstarted_preparation(&operation).unwrap();
    assert!(store.prepared_inputs(&operation).is_err());
    assert!(store.prepared_bytes(&operation).is_err());
    assert!(store.operation(&operation).is_err());
    assert!(!store.status().unwrap().outgoing_pending);
    assert!(store.status().unwrap().treasury_operations.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn actual_mcp_help_completes_on_same_runtime_while_deposit_and_shield_calculation_are_held() {
    use crate::mcp_wire::McpResponse;
    const HELP: &str =
        "Receive addresses and current wallet ownership remain available in local documentation.";
    for kind in [Kind::Deposit, Kind::Shield] {
        let (dir, net, mut owner, destination) = fixture(kind).await;
        let identity = owner.status().await.unwrap().treasury_id;
        let tool = crate::catalog::build_tools(
            &crate::catalog::Config {
                help_url: Some("https://help.invalid/treasury-owner".into()),
                ..Default::default()
            },
            &serde_json::json!({"paths":{"/unused":{"get":{}}}}),
            "owner",
        )
        .unwrap()
        .into_iter()
        .find(|tool| tool.help_url.is_some())
        .unwrap();
        let name = tool.name.clone();
        let server = crate::server::Server::new(
            vec![tool],
            crate::payment::PaidClient::unsigned(),
            "https://api.invalid".into(),
            None,
        );
        server.catalog.read().views["default"][0]
            .help
            .set(HELP.into())
            .unwrap();
        let mcp_work = server.work.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        // This serving task and preparation below use THIS current-thread runtime.
        let serving = tokio::spawn(async move {
            axum::serve(listener, crate::server::http_app(server, "fixture".into())).await
        });
        let (reached, mut release, calls) = hold(&mut owner, false, false, false);
        let count = calls.clone();
        let store = owner.store.clone();
        let (reported, mut received) = tokio::sync::oneshot::channel();
        let controller = std::thread::spawn(move || {
            let outcome = (|| -> Result<_> {
                let operation = reached
                    .blocking_recv()
                    .context("SDK hold was not reached")?;
                let inputs =
                    store.call_blocking(crate::rotation::store::FailureDisposition::Terminal, {
                        let operation = operation.clone();
                        move |s| s.prepared_inputs(&operation)
                    })?;
                let before = count.load(Ordering::SeqCst);
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let response: serde_json::Value =
                    runtime.block_on(async {
                        tokio::time::timeout(Duration::from_secs(2), async {
                    // The existing fixture watchdog runs outside the serving
                    // runtime, so an inline-CPU regression can still be released.
                    crate::test_tls::client(&endpoint).post(&endpoint).bearer_auth("fixture")
                        .header("accept", "application/json, text/event-stream")
                        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                            "params":{"name":name,"arguments":{}}}))
                        .send().await?.error_for_status()?.mcp_json().await
                    }).await?
                    })?;
                let after = count.load(Ordering::SeqCst);
                Ok((operation, inputs, before, after, response))
            })();
            // No assertions precede release. Unwinding also drops this RAII guard.
            release.release();
            drop(store);
            let _ = reported.send(outcome);
        });
        let (prepared, response) = {
            let mut preparation = Box::pin(prepare(&mut owner, kind, destination));
            tokio::select! {
                result = &mut preparation => {
                    drop(preparation);
                    // A pre-handoff failure leaves the test sender in Treasury;
                    // close it so the external controller cannot wait forever.
                    drop(owner.calculation_control.take());
                    (result, received.await)
                }
                result = &mut received => (preparation.await, result),
            }
        };
        // Always join the controller and accepted SDK owner before assertions,
        // including failed HTTP, a stopped server or a pre-handoff SDK failure.
        let controller = controller.join();
        let closed = owner.close().await;
        serving.abort();
        let serving = serving.await;
        mcp_work.drain().await;
        controller.expect("MCP controller panicked after releasing held work");
        closed.unwrap();
        assert!(serving.unwrap_err().is_cancelled());
        let (operation, inputs, before, after, response) = response.unwrap().unwrap();
        assert_eq!(
            (before, after),
            (0, 0),
            "MCP must finish before SDK calculation is released"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(response["id"], 1);
        assert_ne!(response["result"]["isError"], true, "{response}");
        assert_eq!(response["result"]["content"][0]["text"], HELP);
        let prepared = prepared.unwrap();
        assert_eq!(prepared.operation_id(), operation);
        assert!(prepared.facts().is_some() && !inputs.is_empty());
        let store = open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &identity,
        )
        .unwrap();
        assert_eq!(store.prepared_inputs(&operation).unwrap(), inputs);
        assert_eq!(
            store.prepared_bytes(&operation).unwrap().as_slice(),
            prepared.bytes()
        );
        assert_eq!(
            store.prepared_snapshot(&operation).unwrap().as_slice(),
            store.snapshot().unwrap().1.as_slice()
        );
        assert_eq!(store.operation(&operation).unwrap().attempts, 0);
        assert_eq!(store.operation(&operation).unwrap().submission, "PREPARED");
        let status = store.status().unwrap();
        assert!(status.pools.is_empty() && status.funding_jobs.is_empty());
        assert_eq!(status.treasury_operations.len(), 1);
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        no_send(&net).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn accepted_managed_fixed_and_ranged_quotes_keep_original_authority_after_policy_change() {
    for (reserves, floor, preferred, ceiling, changed) in [
        (
            crate::rotation::test_capacity::small_reserve("5"),
            "5000000",
            "5000000",
            "5000000",
            crate::rotation::test_capacity::small_reserve("9"),
        ),
        (
            crate::rotation::config::ReservesConfig {
                small: crate::rotation::config::ReserveConfig {
                    funding_amount_usdc: None,
                    min_funding_amount_usdc: Some("2".into()),
                    max_funding_amount_usdc: Some("5".into()),
                },
                large: None,
            },
            "2000000",
            "3000000",
            "5000000",
            crate::rotation::config::ReservesConfig {
                small: crate::rotation::config::ReserveConfig {
                    funding_amount_usdc: None,
                    min_funding_amount_usdc: Some("7".into()),
                    max_funding_amount_usdc: Some("8".into()),
                },
                large: None,
            },
        ),
    ] {
        let (dir, net, mut owner, _) = fixture(Kind::Deposit).await;
        let pool = owner
            .ensure_capacity_group("accepted_quote".into(), reserves, None, 0)
            .await
            .unwrap();
        owner
            .allocate_capacity_kind(pool.clone(), AllocationKind::Small, true)
            .await
            .unwrap();
        let deadline = now().unwrap() + 600;
        let (job, original_quote, original_terms) = owner
            .store
            .call(move |s| {
                let jobs = s.funding_jobs()?;
                assert_eq!(jobs.len(), 1);
                let job = &jobs[0];
                let terms = s.select_quote_terms(&job.id, &job.operation_id, floor, preferred)?;
                assert_eq!(terms.target.to_string(), preferred);
                assert_eq!(terms.ceiling.to_string(), ceiling);
                // As in save_quotes, bridge validation is separately covered;
                // this actual saved quote binds the real SDK source request.
                let quote = Quote {
                    sizing_preview: None,
                    request: serde_json::json!({"amount":preferred,"recipient":job.recipient}),
                    response: serde_json::json!({}),
                    input: 50_000,
                    deadline,
                    deposit: Some(DEPOSIT.into()),
                };
                let bytes = serde_json::to_vec(&quote)?;
                s.save_funding_quote(&job.id, &bytes)?;
                let terms = s.quote_terms_snapshot(&job.id)?.unwrap();
                Ok((s.funding_job(&job.id)?.unwrap(), bytes, terms))
            })
            .await
            .unwrap();
        assert_eq!(job.target, floor);
        assert_eq!(job.selected_output.as_deref(), Some(preferred));
        assert_eq!(job.accepted_output.as_deref(), Some(preferred));
        let original = request(&job, deadline);
        let principal = original.amount_zatoshis;
        let fee_cap = original.max_fee_zatoshis;
        let input_cap = original.max_input_zatoshis;
        let store = owner.store.clone();
        let (reached, mut release, calls) = hold(&mut owner, false, false, false);
        let (inputs, preparation_sha) = {
            let calculation = owner.prepare(original);
            tokio::pin!(calculation);
            let accepted = tokio::select! {
                result = &mut calculation => panic!("managed calculation returned before acceptance: {}", result.is_ok()),
                result = reached => result.unwrap(),
            };
            assert_eq!(accepted, job.operation_id);
            let operation = accepted.clone();
            let before = store
                .call(move |s| {
                    let state = s.qualification_state()?;
                    assert_eq!(state["source_preparations"].as_array().unwrap().len(), 1);
                    assert_eq!(state["source_preparations"][0]["operation_id"], operation);
                    Ok((
                        s.prepared_inputs(&operation)?,
                        state["source_preparations"][0]["preparation_sha256"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    ))
                })
                .await
                .unwrap();
            let job_id = job.id.clone();
            let operation = job.operation_id.clone();
            store
                .call(move |s| {
                    s.configure_capacity_group(&pool, None, 0, &changed)?;
                    assert!(!s.accepted_quote_suits_current_claim(&job_id, &operation)?);
                    s.deny_new_funding();
                    Ok(())
                })
                .await
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            // Cancel only after the real policy change. The accepted blocking
            // owner keeps its saved quote/proposal rather than drawing again.
            before
        };
        assert!(!inputs.is_empty());
        assert!(owner.client.is_none() && owner.running.is_some());
        release.release();
        let prepared = owner.finish_calculation().await.unwrap();
        drop(store);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(prepared.operation_id(), job.operation_id);
        let facts = prepared.facts().unwrap();
        assert_eq!(facts.amount_zatoshis, principal);
        assert_eq!(facts.deadline, Some(deadline));
        assert!(facts.fee_zatoshis > 0 && facts.fee_zatoshis <= fee_cap);
        assert!(facts.amount_zatoshis + facts.fee_zatoshis <= input_cap);
        evidence(&owner, job.operation_id.clone(), inputs.clone(), &prepared).await;
        let raw = prepared.bytes().to_vec();
        let snapshot = owner
            .store
            .call({
                let operation = job.operation_id.clone();
                move |s| s.prepared_snapshot(&operation)
            })
            .await
            .unwrap();
        let identity = owner.status().await.unwrap().treasury_id;
        no_send(&net).await;
        owner.close().await.unwrap();
        let restored = Treasury::open_with_network(
            dir.path().join("state"),
            dir.path().join("key"),
            identity,
            TreasuryNetwork::Synthetic,
        )
        .await
        .unwrap();
        restored
            .store
            .call(move |s| {
                let status = s.status()?;
                assert_eq!(status.funding_jobs.len(), 1);
                assert_eq!(status.pools.len(), 1);
                assert_eq!(status.pools[0].addresses.len(), 1);
                let current = &status.funding_jobs[0];
                assert_eq!(
                    (
                        &current.id,
                        &current.operation_id,
                        &current.pool_id,
                        &current.wallet_id,
                        &current.recipient
                    ),
                    (
                        &job.id,
                        &job.operation_id,
                        &job.pool_id,
                        &job.wallet_id,
                        &job.recipient
                    )
                );
                assert_eq!(current.target, floor);
                assert_eq!(current.attempts, 0);
                assert!(!s.accepted_quote_suits_current_claim(&job.id, &job.operation_id)?);
                assert_eq!(current.selected_output.as_deref(), Some(preferred));
                assert_eq!(current.accepted_output.as_deref(), Some(preferred));
                assert_eq!(s.quote_terms_snapshot(&job.id)?, Some(original_terms));
                assert_eq!(s.funding_quote(&job.id)?.as_slice(), original_quote);
                assert_eq!(s.prepared_inputs(&job.operation_id)?, inputs);
                assert_eq!(s.prepared_bytes(&job.operation_id)?.as_slice(), raw);
                assert_eq!(
                    s.prepared_snapshot(&job.operation_id)?.as_slice(),
                    snapshot.as_slice()
                );
                assert_eq!(s.operation(&job.operation_id)?.attempts, 0);
                assert_eq!(s.operation(&job.operation_id)?.submission, "PREPARED");
                let state = s.qualification_state()?;
                assert_eq!(state["source_preparations"].as_array().unwrap().len(), 1);
                assert_eq!(
                    state["source_preparations"][0]["preparation_sha256"],
                    preparation_sha
                );
                assert!(state["payment_attempts"].as_array().unwrap().is_empty());
                assert!(s.abandon_unprepared(&job.operation_id).is_err());
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        no_send(&net).await;
        restored.close().await.unwrap();
    }
}
