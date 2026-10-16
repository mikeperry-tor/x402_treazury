use super::*;
use crate::rotation::{
    config::WalletConfig,
    store::{Store, funding::FundingPhase},
    transaction::TransactionFacts,
};
use crate::treasury::actor::{self, ObservedCommand};
use serde_json::json;

#[tokio::test]
async fn concrete_backend_validates_persisted_bindings_before_command_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    crate::rotation::store::test_support::declare_two_reserves(
        &mut store,
        "pool",
        "5",
        Some(alloy_primitives::U256::from(1_000_000)),
        0,
    )
    .unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    let (store, worker) = StoreHandle::spawn(store);
    let (treasury, mut commands) = actor::channel();
    let policy = || {
        serde_json::from_value::<WalletConfig>(json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},"max_api_payment_usdc":"1","max_funding_spend_zec":"0.002","max_conversion_overhead_percent":5})).unwrap()
    };
    let mut backend = Backend {
        treasury,
        store: store.clone(),
        near: super::super::near::NearClient::new(None).unwrap(),
        base: super::super::base::BaseRpc::new("http://127.0.0.1:1", 12, 120).unwrap(),
        wallets: std::collections::BTreeMap::from([("pool".into(), policy())]),
        funding: serde_json::from_value(
            json!({"base_rpc_url_env":"BASE","confidentiality":"public"}),
        )
        .unwrap(),
        max_network_fee: 30_000,
    };
    let request = super::super::near::request(
        &super::super::near::Assets {
            origin: "nep141:zec.omft.near".into(),
            destination: "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near".into(),
        },
        &job.recipient,
        "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F",
        &job.target,
        "public",
        100,
        now().unwrap() + 1000,
        false,
    )
    .unwrap();
    let response = json!({"quoteRequest":request,"quote":{"amountIn":"100000","amountOut":job.target,"amountInUsd":"5.25","depositAddress":"t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F","deadline":request["deadline"]}});
    let quote = Quote {
        request,
        response,
        input: 100000,
        deadline: now().unwrap() + 1000,
        deposit: Some("t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into()),
    };
    let original_job = job.clone();
    let original_quote = serde_json::to_vec(&quote).unwrap();
    store
        .call(move |s| s.save_funding_quote(&original_job.id, &original_quote))
        .await
        .unwrap();
    let before = serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap();
    for field in [
        "recipient",
        "amount",
        "confidentiality",
        "input",
        "deposit",
        "deadline",
        "request",
        "response",
    ] {
        let mut bad = quote.clone();
        match field {
            "recipient" | "amount" | "confidentiality" => {
                let value = match field {
                    "recipient" => json!(format!("0x{:040x}", 9)),
                    "amount" => json!("4000000"),
                    _ => json!("basic"),
                };
                bad.request[field] = value.clone();
                bad.response["quoteRequest"][field] = value;
                if field == "amount" {
                    bad.response["quote"]["amountOut"] = json!("4000000");
                    bad.response["quote"]["amountInUsd"] = json!("4");
                }
            }
            "input" => {
                bad.input = 90_000;
                bad.response["quote"]["amountIn"] = json!("90000");
            }
            "deposit" => {
                bad.deposit = Some("t1different-deposit".into());
                bad.response["quote"]["depositAddress"] = json!("t1different-deposit");
            }
            "deadline" => {
                bad.deadline += 1;
                bad.response["quote"]["deadline"] = json!(
                    chrono::DateTime::from_timestamp(bad.deadline as i64, 0)
                        .unwrap()
                        .to_rfc3339()
                );
            }
            "request" => {
                bad.request["opaque"] = json!("changed");
                bad.response["quoteRequest"] = bad.request.clone();
            }
            "response" => bad.response["opaque"] = json!("changed"),
            _ => unreachable!(),
        }
        let error = backend.prepare(&job, &bad).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("persisted funding quote mismatch"),
            "{field}: {error}"
        );
        assert!(commands.test_is_empty(), "{field} dispatched a command");
    }
    let (result, observed) = tokio::join!(backend.prepare(&job, &quote), commands.test_observe());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("fixture command observed")
    );
    let ObservedCommand::Prepare(prepared) = observed else {
        panic!("wrong command")
    };
    assert_eq!(prepared.operation_id, job.operation_id);
    assert_eq!(prepared.pool_id, Some(job.pool_id.clone()));
    assert_eq!(prepared.recipient, quote.deposit.clone().unwrap());
    assert_eq!(prepared.amount_zatoshis, 100000);
    assert_eq!(prepared.max_fee_zatoshis, 30000);
    assert_eq!(prepared.max_input_zatoshis, 130000);
    assert_eq!(
        before,
        serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap()
    );
    // Model a quote crossing its margin after the scheduler commits PREPARING.
    // The concrete adapter must prove no treasury command was dispatched, so the
    // store can defer and refresh without charging a transaction recovery attempt.
    let other = store
        .call(|s| Ok(s.funding_jobs()?.remove(1)))
        .await
        .unwrap();
    let mut expired = quote.clone();
    expired.request["recipient"] = json!(other.recipient);
    expired.response["quoteRequest"] = expired.request.clone();
    expired.deadline = now().unwrap() + 299;
    expired.response["quote"]["deadline"] = json!(
        chrono::DateTime::from_timestamp(expired.deadline as i64, 0)
            .unwrap()
            .to_rfc3339()
    );
    let j = other.clone();
    let encoded = serde_json::to_vec(&expired).unwrap();
    store
        .call(move |s| {
            s.save_funding_quote(&j.id, &encoded)?;
            s.advance_funding(&j.id, FundingPhase::Quoted, FundingPhase::Preparing)
        })
        .await
        .unwrap();
    let error = backend.prepare(&other, &expired).await.unwrap_err();
    assert!(error.is::<crate::rotation::transaction::PreparationDeferred>());
    assert!(commands.test_is_empty());
    store
        .call(move |s| {
            s.require_unstarted_preparation(&other.operation_id)?;
            s.defer_unstarted_preparation(&other.id)?;
            s.refresh_unprepared_quote(&other.id)?;
            assert_eq!(s.funding_recovery_count(&other.id)?, 0);
            Ok(())
        })
        .await
        .unwrap();
    let j = job.clone();
    let bytes = serde_json::to_vec(&quote).unwrap();
    store
        .call(move |s| {
            s.save_funding_quote(&j.id, &bytes)?;
            s.advance_funding(&j.id, FundingPhase::Quoted, FundingPhase::Preparing)?;
            s.reserve(&j.operation_id, Some(&j.pool_id), 1, 200000)?;
            let revision = s.snapshot()?.0;
            s.prepare_with_facts(
                &j.operation_id,
                revision,
                b"next",
                b"signed fixture",
                Some(TransactionFacts {
                    txid: "fixture".into(),
                    expiry_height: 100,
                    amount_zatoshis: 100000,
                    fee_zatoshis: 10000,
                    deadline: u64::MAX,
                }),
            )?;
            Ok(())
        })
        .await
        .unwrap();
    backend.funding.confidentiality = "basic".into();
    assert!(
        backend
            .submit(&job)
            .await
            .unwrap_err()
            .to_string()
            .contains("funding mode changed")
    );
    backend.funding.confidentiality = "public".into();
    if let WalletConfig::ZcashRotation {
        max_funding_spend_zec,
        ..
    } = backend.wallets.get_mut("pool").unwrap()
    {
        *max_funding_spend_zec = Some("0.001".into());
    }
    assert!(
        backend
            .submit(&job)
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds current input cap")
    );
    assert!(commands.test_is_empty());
    backend.wallets.insert("pool".into(), policy());
    // Corrupt public facts to exercise the adapter's checked sum independently
    // of Store's preparation validator (which normally excludes this input).
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    let original: String = db
        .query_row(
            "SELECT facts FROM treasury_operations WHERE id=?1",
            [&job.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut corrupt: serde_json::Value = serde_json::from_str(&original).unwrap();
    corrupt["amount_zatoshis"] = json!(u64::MAX);
    db.execute(
        "UPDATE treasury_operations SET facts=?1 WHERE id=?2",
        rusqlite::params![corrupt.to_string(), job.operation_id],
    )
    .unwrap();
    assert!(
        backend
            .submit(&job)
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds current input cap")
    );
    assert!(commands.test_is_empty());
    db.execute(
        "UPDATE treasury_operations SET facts=?1 WHERE id=?2",
        rusqlite::params![original, job.operation_id],
    )
    .unwrap();
    drop(db);
    let (result, observed) = tokio::join!(backend.submit(&job), commands.test_observe());
    assert!(result.is_err());
    let ObservedCommand::Submit(id, retry) = observed else {
        panic!("wrong command")
    };
    assert_eq!(id, job.operation_id);
    assert!(!retry);
    let (recovered, observed) = tokio::join!(backend.recover(&job), commands.test_observe());
    assert!(matches!(
        recovered.unwrap().map(|result| result.outcome),
        Some(crate::treasury::recovery::RecoveryOutcome::Waiting(
            "waiting_for_transaction_expiry"
        ))
    ));
    let ObservedCommand::RecoverFunding(id, limit, allowed) = observed else {
        panic!("wrong recovery command")
    };
    assert_eq!(id, job.id);
    assert_eq!(limit, backend.max_attempts(&job));
    assert_eq!(allowed, backend.funding.auto_fund);
    backend.funding.auto_fund = false;
    assert!(
        backend
            .submit(&job)
            .await
            .unwrap_err()
            .to_string()
            .contains("funding_disabled")
    );
    assert!(commands.test_is_empty());
    let (recovered, observed) = tokio::join!(backend.recover(&job), commands.test_observe());
    assert!(recovered.unwrap().is_some());
    assert!(matches!(
        observed,
        ObservedCommand::RecoverFunding(_, _, false)
    ));
    let (result, observed) = tokio::join!(backend.reconcile(&job), commands.test_observe());
    assert!(result.is_err());
    let ObservedCommand::Reconcile(id) = observed else {
        panic!("wrong command")
    };
    assert_eq!(id, job.operation_id);
    // Count actual commands crossing the production backend/treasury boundary.
    // The fixture rejects ordinary reconciliation; recovery reports an owned
    // observation. A duplicate command either violates the match or times out.
    let (id, operation) = (job.id.clone(), job.operation_id.clone());
    store
        .call(move |s| {
            s.request_broadcast(&operation, now()?, 1, false)?;
            s.reconcile_funding_phase(&id, &operation)
        })
        .await
        .unwrap();
    let coordinator = FundingWorker::new(store.clone(), backend, 60);
    for timed_out in [false, true] {
        let id = job.id.clone();
        let current = store
            .call(move |s| {
                if timed_out {
                    s.mark_funding_timeout(&id)?;
                }
                s.funding_job(&id)?.context("fixture job missing")
            })
            .await
            .unwrap();
        let clock = TickClock {
            epoch: now().unwrap(),
            started: std::time::Instant::now(),
        };
        let (result, observed) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                coordinator.source_step(&current, &clock),
                commands.test_observe()
            )
        })
        .await
        .unwrap();
        match observed {
            ObservedCommand::Reconcile(id) if !timed_out => {
                assert_eq!(id, job.operation_id);
                assert!(result.is_err());
            }
            ObservedCommand::RecoverFunding(id, _, false) if timed_out => {
                assert_eq!(id, job.id);
                assert!(result.unwrap().is_none());
            }
            _ => panic!("wrong source command or duplicate ordinary observation"),
        }
        assert!(commands.test_is_empty());
        let operation = job.operation_id.clone();
        assert_eq!(
            store
                .call(move |s| Ok(s.operation(&operation)?.attempts))
                .await
                .unwrap(),
            1
        );
    }
    let FundingWorker {
        backend: owned_backend,
        store: owned_store,
        ..
    } = coordinator;
    drop(owned_store);
    backend = std::sync::Arc::try_unwrap(owned_backend)
        .ok()
        .unwrap()
        .into_inner();
    let id = job.operation_id.clone();
    store
        .call(move |s| s.confirm_spend(&id, 110000, 1))
        .await
        .unwrap();
    assert!(backend.reconcile(&job).await.unwrap());
    assert!(commands.test_is_empty());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let rpc = |axum::Json(v): axum::Json<serde_json::Value>| async move {
        let result = match v["method"].as_str().unwrap() {
            "eth_chainId" => json!("0x2105"),
            "eth_getBlockByNumber" => {
                let height = if v["params"][0] == "latest" {
                    100
                } else {
                    u64::from_str_radix(
                        v["params"][0].as_str().unwrap().trim_start_matches("0x"),
                        16,
                    )
                    .unwrap()
                };
                json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
            }
            "eth_call" => json!(format!("0x{:064x}", 5000000)),
            _ => panic!("unexpected RPC"),
        };
        axum::Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
    };
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/", axum::routing::post(rpc))
                .route(
                    "/forbidden",
                    axum::routing::post(|| async {
                        (axum::http::StatusCode::FORBIDDEN, "secret upstream body")
                    }),
                )
                .route(
                    "/limited",
                    axum::routing::post(|| async {
                        (
                            axum::http::StatusCode::TOO_MANY_REQUESTS,
                            "secret upstream body",
                        )
                    }),
                ),
        )
        .await
        .unwrap()
    });
    let unchanged = serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap();
    for (path, code) in [("forbidden", "403"), ("limited", "429")] {
        backend.base = super::super::base::BaseRpc::new(&format!("{url}/{path}"), 12, 120).unwrap();
        let error = backend
            .credit(&job)
            .await
            .unwrap_err()
            .context("base_credit_unverified");
        let category = safe_error(&error, &FundingPhase::VerifyingCredit);
        assert!(category.contains(code), "{category}");
        assert!(category.contains("retaining reservations"));
        assert!(!category.contains("secret"));
        assert_eq!(
            unchanged,
            serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap()
        );
    }
    backend.base = super::super::base::BaseRpc::new(&url, 12, 120).unwrap();
    let before = serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap();
    let mut missing = job.clone();
    missing.wallet_id = "not-a-candidate".into();
    assert!(
        backend
            .credit(&missing)
            .await
            .unwrap_err()
            .to_string()
            .contains("funding credit query")
    );
    assert_eq!(
        before,
        serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap()
    );
    backend.credit(&job).await.unwrap();
    // Pause the payment owner after provisional validation. Actual funding-credit
    // query construction and persistence must not create an unsigned lease or bypass its group gate.
    let gate = store.group_gate(&job.pool_id).unwrap();
    let guard = gate.lock().await;
    let pool = job.pool_id.clone();
    let query = store.call(move |s| s.chain_query(&pool)).await.unwrap();
    let view = backend.base.view(query).await.unwrap();
    let pool = job.pool_id.clone();
    let lease = store
        .call(move |s| {
            s.admit_for(
                &pool,
                alloy_primitives::U256::from(1),
                "lease-race",
                view,
                None,
            )
        })
        .await
        .unwrap();
    {
        let base = backend.base.clone();
        let credit = backend.credit(&job);
        tokio::pin!(credit);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut credit)
                .await
                .is_err()
        );
        let pool = job.pool_id.clone();
        let query = store.call(move |s| s.chain_query(&pool)).await.unwrap();
        let view = base.view(query).await.unwrap();
        let pool = job.pool_id.clone();
        store
            .call(move |s| s.reconcile_pool(&pool, view))
            .await
            .unwrap();
        store
            .call(move |s| {
                s.journal_authorization(
                    &lease,
                    crate::rotation::store::Authorization {
                        payer: lease.address.clone(),
                        payee: format!("0x{:040x}", 1),
                        nonce: format!("0x{:064x}", 1),
                        valid_after: 0,
                        valid_before: now()? + 100,
                    },
                    None,
                )
            })
            .await
            .unwrap();
        drop(guard);
        credit.await.unwrap();
    }
    assert!(commands.test_is_empty());
    server.abort();
    drop(backend);
    drop(store);
    worker.await.unwrap();
}

#[test]
fn credit_persistence_diagnostics_do_not_misidentify_local_failures_as_rpc_errors() {
    for (reason, expected) in [
        (
            "insufficient confirmed credit",
            "confirmed balance is below the funding target",
        ),
        (
            "credit verification requires an allocated candidate",
            "candidate role changed during verification",
        ),
        (
            "secret sqlite path and private wallet identifier",
            "local credit state could not be committed",
        ),
    ] {
        let error = anyhow::anyhow!(reason)
            .context(super::super::base::VerificationStage(
                "funding credit persistence",
            ))
            .context("base_credit_unverified");
        let public = safe_error(&error, &FundingPhase::VerifyingCredit);
        assert!(public.contains(expected), "{public}");
        assert!(public.contains("retaining reservations"));
        assert!(!public.contains("RPC"));
        assert!(!public.contains("secret"));
        assert!(!public.contains("private"));
    }
}

#[test]
fn near_diagnostics_preserve_stage_and_safe_categories_only() {
    for (reason, expected) in [
        ("near_http_503", "HTTP 503"),
        ("near_http_429", "HTTP 429"),
        ("near_unavailable: timeout", "request timeout"),
        ("near_unavailable: connect", "connection failure"),
        ("near_unavailable: transport", "request transport failure"),
        (
            "near_response_failed: body transport",
            "response body transport failure",
        ),
        ("invalid NEAR JSON", "invalid JSON response"),
        ("quote overhead limit", "fee/overhead cap exceeded"),
        ("quote input limit", "input cap exceeded"),
        ("quote output mismatch", "output target mismatch"),
        (
            "quote binding mismatch: secret-recipient",
            "quote binding mismatch",
        ),
        ("near_http_503 secret-token", "validation or internal error"),
    ] {
        let error = anyhow::anyhow!(reason).context(NearStage("quote"));
        let public = safe_error(&error, &FundingPhase::Allocated);
        assert!(
            public.contains(expected) && public.contains("NEAR quote"),
            "{public}"
        );
        assert!(!public.contains("secret"), "{public}");
    }
    let error = anyhow::anyhow!("invalid token catalog").context(NearStage("asset catalog"));
    assert!(
        safe_error(&error, &FundingPhase::Allocated)
            .contains("NEAR asset catalog: invalid asset catalog")
    );
}

#[test]
fn permit_denial_remains_actionable_without_leaking_registry_errors() {
    let error = anyhow::anyhow!("private registry path and database detail")
        .context(crate::rotation::error::AdmissionError::FundingPermitDenied);
    let message = super::safe_error(&error, &super::FundingPhase::Quoted);
    assert!(message.contains("qualification_funding_denied"));
    assert!(message.contains("inspect qualification registry"));
    assert!(!message.contains("private registry path"));
}

#[test]
fn only_typed_confirmation_waits_are_classified_as_progress() {
    let credit = |reason| {
        anyhow::anyhow!("{}", reason)
            .context(super::super::base::VerificationStage(
                "funding credit persistence",
            ))
            .context("base_credit_unverified")
    };
    assert_eq!(
        waiting_reason(
            &credit("insufficient confirmed credit"),
            &FundingPhase::VerifyingCredit
        ),
        None
    );
    for reason in [
        "disk failure",
        "candidate role changed",
        "confirmation disagrees with wallet sync",
        "treasury_sync_stale",
    ] {
        assert_eq!(
            waiting_reason(&credit(reason), &FundingPhase::VerifyingCredit),
            None
        );
        assert_eq!(
            waiting_reason(
                &anyhow::anyhow!("{}", reason),
                &FundingPhase::DepositPending
            ),
            None
        );
    }
    assert_eq!(
        waiting_reason(
            &anyhow::anyhow!("insufficient confirmed credit"),
            &FundingPhase::VerifyingCredit
        ),
        None
    );
}

#[tokio::test]
async fn quote_http_is_independent_only_after_the_owned_refund_prerequisite() {
    use axum::{
        Json, Router,
        routing::{get, post},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let dir = tempfile::tempdir().unwrap();
    let mut owner = crate::treasury::Treasury::create(
        dir.path().join("state"), dir.path().join("key"), 2_000_000,
        Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into())),
    ).await.unwrap();
    owner.configure_sync(
        crate::treasury::SyncSettings::new("http://127.0.0.1:1".into(), 3, 300).unwrap(),
    );
    let pool = owner
        .ensure_capacity_group(
            "pool".into(),
            crate::rotation::store::test_support::reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .await
        .unwrap();
    owner
        .allocate_capacity_kind(
            pool.clone(),
            crate::rotation::store::AllocationKind::Small,
            true,
        )
        .await
        .unwrap();
    let store = owner.store_handle();
    let job = store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route("/v0/tokens", get({
        let calls = calls.clone();
        move || { let calls = calls.clone(); async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Json(serde_json::from_str::<serde_json::Value>(include_str!("../fixtures/near/tokens.json")).unwrap())
        }}
    })).route("/v0/quote", post({
        let calls = calls.clone();
        move |Json(request): Json<serde_json::Value>| { let calls = calls.clone(); async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Json(json!({"quoteRequest":request,"quote":{"amountIn":"100000","amountOut":request["amount"],"amountInUsd":"5.25","depositAddress":"t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F","deadline":request["deadline"]}}))
        }}
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (treasury, commands) = actor::channel();
    let backend = Backend {
        treasury, store: store.clone(), near: super::super::near::NearClient::test_at(&origin).unwrap(),
        base: super::super::base::BaseRpc::new("http://127.0.0.1:1",12,120).unwrap(),
        wallets: [("pool".into(), serde_json::from_value(json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},"max_api_payment_usdc":"1","max_funding_spend_zec":"0.002","max_conversion_overhead_percent":5})).unwrap())].into(),
        funding: serde_json::from_value(json!({"base_rpc_url_env":"BASE","confidentiality":"public"})).unwrap(),
        max_network_fee: 10000,
    };
    let expected = job.clone();
    let prerequisite = tokio::spawn(async move {
        let mut backend = backend;
        let request = backend.quote_prerequisite(&job).await.unwrap();
        (backend, request)
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while commands.test_is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!prerequisite.is_finished());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "quote cannot bypass its treasury prerequisite"
    );
    let stop = CancellationToken::new();
    let actor = tokio::spawn(
        owner.run_commands(
            commands,
            crate::treasury::submission::GrpcSubmission::new(
                "http://127.0.0.1:1".into(),
                "http://127.0.0.1:1".into(),
            )
            .unwrap(),
            stop.clone(),
        ),
    );
    let (backend, request) = prerequisite.await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "prerequisite has no unsigned HTTP work"
    );
    let observations = backend.observations();
    let backend = tokio::sync::Mutex::new(backend);
    let source_owner = backend.lock().await;
    let quote = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        observations.quote(request.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(quote.request["recipient"], expected.recipient);
    assert_eq!(quote.request["amount"], expected.target);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one asset read and one quote while source backend remains owned"
    );
    let (id, operation) = (expected.id.clone(), expected.operation_id.clone());
    let encoded = serde_json::to_vec(&quote).unwrap();
    let target = expected.target.clone();
    store
        .call(move |s| s.save_funding_quote_for_operation(&id, &operation, &encoded, &target))
        .await
        .unwrap();
    assert!(
        observations.quote(request).await.is_err(),
        "completed prerequisite cannot replay quote acquisition after acceptance"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let status = store.call(|s| s.status()).await.unwrap();
    assert!(status.treasury_operations.is_empty());
    assert!(!status.outgoing_pending);
    let db = rusqlite::Connection::open_with_flags(
        dir.path().join("state/state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(source_owner);
    drop(backend);
    drop(observations);
    drop(store);
    stop.cancel();
    actor.await.unwrap().unwrap();
    server.abort();
}

#[test]
fn startup_condition_classification_uses_producer_types_not_message_copies() {
    use crate::rotation::error::{AdmissionError, FundingCondition};
    for condition in [
        FundingCondition::Disabled,
        FundingCondition::CostLimit,
        FundingCondition::AmountLimit,
        FundingCondition::InputLimit,
        FundingCondition::InsufficientFunds,
    ] {
        let text = condition.to_string();
        assert_eq!(super::funding_condition(&anyhow::anyhow!(text)), None);
        assert_eq!(
            super::funding_condition(
                &anyhow::Error::new(condition).context("arbitrary annotation")
            ),
            Some(condition)
        );
    }
    assert_eq!(
        super::funding_condition(&anyhow::Error::new(crate::rotation::near::HttpStatus(401))),
        Some(FundingCondition::AuthenticationRequired)
    );
    assert_eq!(
        super::funding_condition(&anyhow::anyhow!("near_http_401")),
        None
    );
    assert_eq!(
        super::funding_condition(&anyhow::Error::new(AdmissionError::FundingPermitDenied)),
        Some(FundingCondition::QualificationDenied)
    );
}
