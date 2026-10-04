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
    store.ensure_pool("pool", "5").unwrap();
    let job = store.funding_jobs().unwrap().remove(0);
    let (store, worker) = StoreHandle::spawn(store);
    let (treasury, mut commands) = actor::channel();
    let policy = || {
        serde_json::from_value::<WalletConfig>(json!({"mode":"zcash_rotation","deposit_size":"5","max_input_zec":"0.002","max_fee_bps":500})).unwrap()
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
        daily_limit: 1_000_000,
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
    let before = serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap();
    for field in ["recipient", "amount", "confidentiality"] {
        let mut bad = quote.clone();
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
        let error = backend.prepare(&job, &bad).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("persisted funding binding mismatch"),
            "{field}: {error}"
        );
        assert!(commands.test_is_empty());
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
    assert_eq!(prepared.max_fee_zatoshis, 100000);
    assert_eq!(prepared.max_input_zatoshis, 200000);
    assert_eq!(prepared.daily_limit_zatoshis, 1_000_000);
    assert_eq!(
        before,
        serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap()
    );
    let j = job.clone();
    let bytes = serde_json::to_vec(&quote).unwrap();
    store
        .call(move |s| {
            s.save_funding_quote(&j.id, &bytes)?;
            s.advance_funding(&j.id, FundingPhase::Quoted, FundingPhase::Preparing)?;
            s.reserve(&j.operation_id, Some(&j.pool_id), 1, 200000, 1_000_000)?;
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
    if let WalletConfig::ZcashRotation { max_input_zec, .. } =
        backend.wallets.get_mut("pool").unwrap()
    {
        *max_input_zec = "0.001".into();
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
    let (result, observed) = tokio::join!(backend.reconcile(&job), commands.test_observe());
    assert!(result.is_err());
    let ObservedCommand::Reconcile(id) = observed else {
        panic!("wrong command")
    };
    assert_eq!(id, job.operation_id);
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
                json!({"number":if v["params"][0]=="latest" {"0x64"}else{v["params"][0].as_str().unwrap()},"hash":format!("0x{:064x}",1),"timestamp":format!("0x{:x}",now().unwrap())})
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
            .contains("missing candidate balance")
    );
    assert_eq!(
        before,
        serde_json::to_value(store.call(|s| s.status()).await.unwrap()).unwrap()
    );
    backend.credit(&job).await.unwrap();
    assert!(commands.test_is_empty());
    server.abort();
    drop(backend);
    drop(store);
    worker.await.unwrap();
}
