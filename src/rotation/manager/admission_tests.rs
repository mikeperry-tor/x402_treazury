//! Real SDK signing and exact RPC evidence across both independent recovery allowances.
use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use futures_util::FutureExt;
use std::sync::{
    Mutex as StdMutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Fixture {
    head: Arc<AtomicU64>,
    calls: Arc<AtomicUsize>,
    unsigned: Arc<AtomicUsize>,
    sent: Arc<StdMutex<Vec<String>>>,
    timestamps: Arc<StdMutex<std::collections::BTreeMap<u64, u64>>>,
    used: Arc<AtomicBool>,
    db: std::path::PathBuf,
}
async fn rpc(State(f): State<Fixture>, Json(request): Json<Value>) -> Json<Value> {
    f.calls.fetch_add(1, Ordering::SeqCst);
    let result = match request["method"].as_str().unwrap() {
        "eth_chainId" => serde_json::json!("0x2105"),
        "eth_getBlockByNumber" => {
            let tag = request["params"][0].as_str().unwrap();
            let height = if tag == "latest" {
                f.head.load(Ordering::SeqCst)
            } else {
                u64::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap()
            };
            serde_json::json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",f.timestamps.lock().unwrap()[&height])})
        }
        "eth_call" => {
            assert_eq!(request["params"][1]["requireCanonical"], true);
            let height = u64::from_str_radix(
                request["params"][1]["blockHash"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x"),
                16,
            )
            .unwrap();
            let used = f.used.load(Ordering::SeqCst) && height >= 188;
            let amount = if request["params"][0]["data"]
                .as_str()
                .unwrap()
                .starts_with("0x70a08231")
            {
                5_000_000u64 - u64::from(used)
            } else {
                u64::from(used)
            };
            serde_json::json!(format!("0x{amount:064x}"))
        }
        other => panic!("unexpected fixture RPC {other}"),
    };
    Json(serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
}
async fn seller(State(f): State<Fixture>, headers: HeaderMap) -> axum::response::Response {
    if let Some(signature) = headers.get("payment-signature") {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
        let nonce = payload["payload"]["authorization"]["nonce"]
            .as_str()
            .unwrap()
            .to_owned();
        let db = rusqlite::Connection::open(&f.db).unwrap();
        let count:i64=db.query_row("SELECT COUNT(*) FROM payment_attempts WHERE nonce=?1 AND state='POSSIBLY_SUBMITTED'",[&nonce],|row|row.get(0)).unwrap();
        assert_eq!(
            count, 1,
            "no signature may escape before the journal acknowledgement"
        );
        f.sent.lock().unwrap().push(nonce);
        return "paid".into_response();
    }
    f.unsigned.fetch_add(1, Ordering::SeqCst);
    let challenge = serde_json::json!({"x402Version":2,"resource":{"url":"http://localhost/pay"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":2,"extra":{"name":"USD Coin","version":"2"}}]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
    )
        .into_response()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Race {
    CheckpointThenSignature,
    Used,
    Expired,
}
#[tokio::test]
async fn checkpoint_recheck_then_expired_private_signature_replaces_once_without_new_rpc() {
    race(Race::CheckpointThenSignature).await;
}
#[tokio::test]
async fn used_proof_during_acquisition_refreshes_balance_before_signing() {
    race(Race::Used).await;
}
#[tokio::test]
async fn evidence_expiring_before_gate_refreshes_once_before_signing() {
    race(Race::Expired).await;
}
async fn race(race: Race) {
    use crate::rotation::store::Store;
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = state.ensure_pool("combined", "5").unwrap();
    for job in state.funding_jobs().unwrap() {
        state
            .record_credit(&job.wallet_id, "5000000", &format!("0x{:064x}", 88), 88)
            .unwrap();
    }
    let wallet = state.status().unwrap().pools.remove(0).addresses.remove(0);
    let dbpath = dir.path().join("state/state.sqlite");
    let db = rusqlite::Connection::open(&dbpath).unwrap();
    db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence) VALUES ('old',?1,?2,0,'1','fixture','POSSIBLY_SUBMITTED',?3,?3,?4,0,0,1)",rusqlite::params![pool,wallet.id,wallet.address,format!("0x{:064x}",1)]).unwrap();
    db.execute(
        "UPDATE wallets SET unresolved_exposure='1' WHERE id=?1",
        [&wallet.id],
    )
    .unwrap();
    let fixture = Fixture {
        head: Arc::new(AtomicU64::new(100)),
        calls: Arc::default(),
        unsigned: Arc::default(),
        sent: Arc::default(),
        timestamps: Arc::new(StdMutex::new(
            (1..=100).map(|height| (height, now().unwrap())).collect(),
        )),
        used: Arc::default(),
        db: dbpath,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/rpc", post(rpc))
        .route("/pay", get(seller))
        .with_state(fixture.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (store, worker) = StoreHandle::spawn(state);
    let base_rpc = BaseRpc::new(
        &format!("{base}/rpc"),
        12,
        if race == Race::Expired { 1 } else { 120 },
    )
    .unwrap();
    let mut manager = ManagedPool::new(
        store.clone(),
        pool.clone(),
        base_rpc.clone(),
        "5",
        SpendPolicy::dollars("5").unwrap(),
    )
    .unwrap();
    let acquired = Arc::new(tokio::sync::Notify::new());
    let release_acquired = Arc::new(tokio::sync::Notify::new());
    let signed = Arc::new(tokio::sync::Notify::new());
    let release_signed = Arc::new(tokio::sync::Notify::new());
    let signatures = Arc::new(StdMutex::new(Vec::<(String, u64)>::new()));
    manager.test_admission_hook = Some(Arc::new({
        let acquired = acquired.clone();
        let release_acquired = release_acquired.clone();
        let signed = signed.clone();
        let release_signed = release_signed.clone();
        let signatures = signatures.clone();
        move |stage| {
            let acquired = acquired.clone();
            let release_acquired = release_acquired.clone();
            let signed = signed.clone();
            let release_signed = release_signed.clone();
            let signatures = signatures.clone();
            async move {
                match stage {
                    AdmissionTestStage::Journaled => {}
                    AdmissionTestStage::Acquired => {
                        acquired.notify_one();
                        release_acquired.notified().await;
                    }
                    AdmissionTestStage::Signed {
                        nonce,
                        valid_before,
                    } => {
                        let first = {
                            let mut values = signatures.lock().unwrap();
                            values.push((nonce, valid_before));
                            values.len() == 1
                        };
                        if first {
                            signed.notify_one();
                            if race == Race::CheckpointThenSignature {
                                release_signed.notified().await;
                            }
                        }
                    }
                }
            }
            .boxed()
        }
    }));
    let client = crate::payment::PaidClient::managed(Arc::new(manager));
    let call = tokio::spawn(async move {
        client
            .execute(crate::catalog::RoutedRequest {
                method: "GET".into(),
                url: format!("{base}/pay"),
                query: Default::default(),
                body: None,
            })
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), acquired.notified())
        .await
        .unwrap();
    if race == Race::Expired {
        let expires = fixture.timestamps.lock().unwrap()[&100] + 1;
        while now().unwrap() <= expires {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
    fixture
        .timestamps
        .lock()
        .unwrap()
        .extend((101..=200).map(|height| (height, now().unwrap())));
    fixture.head.store(200, Ordering::SeqCst);
    fixture.used.store(race == Race::Used, Ordering::SeqCst);
    if race != Race::Expired {
        let group = pool.clone();
        let query = store
            .call(move |s| {
                let pending = s.chain_query(&group)?.pending;
                s.scoped_chain_query(&group, vec![], pending)
            })
            .await
            .unwrap();
        let historical = base_rpc.view(query).await.unwrap();
        let group = pool.clone();
        store
            .call(move |s| s.publish_scoped_evidence(&group, historical))
            .await
            .unwrap();
        let resolved: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM payment_attempts WHERE id='old' AND state='RESOLVED'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(resolved, 1);
    }
    release_acquired.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(10), signed.notified())
        .await
        .unwrap();
    let rpc_after_recheck = fixture.calls.load(Ordering::SeqCst);
    if race == Race::CheckpointThenSignature {
        let expiry = signatures.lock().unwrap()[0].1;
        let attempts: i64 = db
            .query_row("SELECT COUNT(*) FROM payment_attempts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            attempts, 1,
            "private signature creates no provisional reservation"
        );
        assert!(fixture.sent.lock().unwrap().is_empty());
        while now().unwrap() <= expiry {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        release_signed.notify_one();
    }
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(10), call)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "paid"
    );
    let values = signatures.lock().unwrap().clone();
    assert_eq!(
        values.len(),
        if race == Race::CheckpointThenSignature {
            2
        } else {
            1
        }
    );
    if values.len() == 2 {
        assert_ne!(
            values[0].0, values[1].0,
            "replacement uses a fresh SDK nonce"
        );
    }
    assert_eq!(
        *fixture.sent.lock().unwrap(),
        vec![values.last().unwrap().0.clone()]
    );
    assert_eq!(fixture.unsigned.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.calls.load(Ordering::SeqCst),
        rpc_after_recheck,
        "earlier evidence recheck is not repeated for signature-only expiry"
    );
    drop(values);
    drop(db);
    drop(store);
    worker.await.unwrap();
    server.abort();
}
