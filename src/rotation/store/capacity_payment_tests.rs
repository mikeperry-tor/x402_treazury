//! Payment admission with real canonical balance certificates and retained disputes.
use super::*;
use crate::rotation::base::{BaseRpc, now};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};

async fn balance_rpc() -> (BaseRpc, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/", post(|Json(request): Json<Value>| async move {
        let result = match request["method"].as_str().unwrap() {
            "eth_chainId" => json!("0x2105"),
            "eth_getBlockByNumber" => {
                let tag = request["params"][0].as_str().unwrap();
                let height = if tag == "latest" { 100 } else { u64::from_str_radix(&tag[2..], 16).unwrap() };
                assert!(height > 80, "capacity capture must not depend on disputed historical blocks");
                json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
            }
            "eth_call" => {
                assert!(request["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231"));
                assert_eq!(request["params"][1]["requireCanonical"], true);
                json!(format!("0x{:064x}", 20))
            }
            _ => panic!("unexpected RPC operation"),
        };
        Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc = BaseRpc::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        12,
        120,
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (rpc, server)
}

fn disputed_store(dir: &std::path::Path) -> (Store, String, String) {
    let mut store = Store::create(&dir.join("state"), &dir.join("key"), 1, b"fixture").unwrap();
    let pool =
        test_support::declare_two_reserves(&mut store, "capacity", "1", Some(U256::from(100)), 0)
            .unwrap();
    let wallet = store
        .status()
        .unwrap()
        .pools
        .remove(0)
        .addresses
        .remove(0)
        .id;
    store.db.execute("UPDATE wallets SET role='ACTIVE',evidence_repair='chain_repair_required',financial_height=80,financial_hash='disputed',debit_height=80,debit_hash='disputed' WHERE id=?1", [&wallet]).unwrap();
    store
        .db
        .execute(
            "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1 WHERE wallet_id=?1",
            [&wallet],
        )
        .unwrap();
    store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,resolution_disputed) VALUES('old',?1,?2,0,'1','fixture','RESOLVED',1)", params![pool,wallet]).unwrap();
    store.db.execute("INSERT INTO payment_resolutions(attempt_id,outcome,height,hash,block_time,support) VALUES('old','USED',80,'disputed',1,'immutable')", []).unwrap();
    (store, pool, wallet)
}

fn auth(prepared: &PreparedPayment) -> Authorization {
    Authorization {
        payer: prepared.address.clone(),
        payee: format!("0x{:040x}", 2),
        nonce: format!("0x{:064x}", 2),
        valid_after: 0,
        valid_before: now().unwrap() + 100,
    }
}

fn signed_count(store: &Store) -> i64 {
    store
        .db
        .query_row("SELECT COUNT(*) FROM payment_admission_evidence", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[tokio::test]
async fn disputed_capacity_admits_only_remaining_balance_after_restart() {
    let (rpc, server) = balance_rpc().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut store, pool, wallet) = disputed_store(dir.path());
    let identity = store.id().to_owned();
    let view = rpc
        .view(
            store
                .payment_balance_query(&pool, vec![wallet.clone()])
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(store.publish_scoped_evidence(&pool, view.clone()).is_err());
    assert!(
        rpc.recheck_balance(
            view.balance_certificates[&wallet].clone(),
            store.capture_evidence(&wallet).unwrap()
        )
        .await
        .is_err(),
        "header recheck must not convert capacity-only evidence into strict evidence"
    );
    let untouched: (i64, i64, String) = store.db.query_row("SELECT (SELECT COUNT(*) FROM wallet_retirements),(SELECT COUNT(*) FROM funding_jobs WHERE readiness_requirement IS NOT NULL),role FROM wallets WHERE id=?1", [&wallet], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(untouched, (0, 0, "ACTIVE".into()));
    assert!(
        store
            .publish_candidate_evidence(&pool, &wallet, view.clone())
            .unwrap()
            .1
            .is_some()
    );
    let candidate = crate::rotation::manager::PaymentCandidateHandle {
        wallet: wallet.clone(),
        address: store
            .db
            .query_row("SELECT address FROM wallets WHERE id=?1", [&wallet], |r| {
                r.get(0)
            })
            .unwrap(),
    };
    assert!(
        store
            .prepare_payment(
                &pool,
                U256::from(20),
                "too-large",
                view.clone(),
                Some(&candidate)
            )
            .is_err()
    );
    assert_eq!(signed_count(&store), 0);
    drop(store);
    let mut store = Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &identity,
    )
    .unwrap();
    assert_eq!(
        evidence::exposure(&store.db, &wallet).unwrap(),
        U256::from(1)
    );
    assert!(
        store
            .publish_candidate_evidence(&pool, &wallet, view)
            .is_err(),
        "restart must invalidate the old capture"
    );
    let view = rpc
        .view(
            store
                .payment_balance_query(&pool, vec![wallet.clone()])
                .unwrap(),
        )
        .await
        .unwrap();
    let prepared = store
        .prepare_payment(&pool, U256::from(19), "remaining", view, Some(&candidate))
        .unwrap();
    assert_eq!(prepared.reserved, U256::from(1));
    store
        .journal_authorization(&prepared, auth(&prepared), None)
        .unwrap();
    assert_eq!(signed_count(&store), 1);
    assert_eq!(
        evidence::exposure(&store.db, &wallet).unwrap(),
        U256::from(20)
    );
    let original: (String, String, i64) = store.db.query_row("SELECT r.hash,r.support,a.resolution_disputed FROM payment_resolutions r JOIN payment_attempts a ON a.id=r.attempt_id WHERE a.id='old'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(original, ("disputed".into(), "immutable".into(), 1));
    let after_payment: (i64, i64, String) = store.db.query_row("SELECT (SELECT COUNT(*) FROM wallet_retirements),(SELECT COUNT(*) FROM funding_jobs WHERE readiness_requirement IS NOT NULL),role FROM wallets WHERE id=?1", [&wallet], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        after_payment, untouched,
        "payment cannot grant funding or retirement authority"
    );
    server.abort();
}

#[tokio::test]
async fn concurrent_resolution_or_checkpoint_change_prevents_signing() {
    let (rpc, server) = balance_rpc().await;
    for mutation in [
        "UPDATE payment_attempts SET resolution_disputed=0 WHERE id='old'",
        "UPDATE wallets SET financial_hash='changed' WHERE role='ACTIVE'",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, pool, wallet) = disputed_store(dir.path());
        let view = rpc
            .view(
                store
                    .payment_balance_query(&pool, vec![wallet.clone()])
                    .unwrap(),
            )
            .await
            .unwrap();
        let candidate = crate::rotation::manager::PaymentCandidateHandle {
            wallet: wallet.clone(),
            address: store
                .db
                .query_row("SELECT address FROM wallets WHERE id=?1", [&wallet], |r| {
                    r.get(0)
                })
                .unwrap(),
        };
        let prepared = store
            .prepare_payment(&pool, U256::from(19), "race", view, Some(&candidate))
            .unwrap();
        store.db.execute(mutation, []).unwrap();
        assert!(
            store
                .journal_authorization(&prepared, auth(&prepared), None)
                .is_err()
        );
        assert_eq!(signed_count(&store), 0);
        let attempts: i64 = store
            .db
            .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(attempts, 1);
    }
    server.abort();
}
