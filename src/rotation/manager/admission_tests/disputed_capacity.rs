use super::*;
use crate::{catalog::RoutedRequest, payment::PaidClient, rotation::store::Store};

#[derive(Clone)]
struct RepairFixture {
    payment: Fixture,
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
}
async fn current_rpc(State(state): State<RepairFixture>, request: Json<Value>) -> Json<Value> {
    if request.0["method"] == "eth_call" {
        let data = request.0["params"][0]["data"].as_str().unwrap();
        if !data.starts_with("0x70a08231") && data.ends_with(&format!("{:064x}", 1)) {
            state.started.notify_one();
            state.release.acquire().await.unwrap().forget();
        }
    }
    if request.0["method"] == "eth_getBlockByNumber" {
        let tag = request.0["params"][0].as_str().unwrap();
        assert!(
            matches!(tag, "latest" | "0x58" | "0x64"),
            "payment must not read the disputed historical checkpoint"
        );
    }
    rpc(State(state.payment), request).await
}

async fn journal_checked_seller(
    State(state): State<RepairFixture>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(signature) = headers.get("payment-signature") {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
        let nonce = payload["payload"]["authorization"]["nonce"]
            .as_str()
            .unwrap();
        let db = rusqlite::Connection::open(&state.payment.db).unwrap();
        let admission: (String, String, String) = db.query_row("SELECT a.amount,e.balance,e.reserved_before FROM payment_attempts a JOIN payment_admission_evidence e ON e.attempt_id=a.id WHERE a.nonce=?1 AND a.state='POSSIBLY_SUBMITTED'", [nonce], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(
            admission,
            (
                state.payment.price.load(Ordering::SeqCst).to_string(),
                "20000000".into(),
                (20_000_000 - state.payment.price.load(Ordering::SeqCst)).to_string()
            )
        );
    }
    seller(State(state.payment), headers).await
}

#[tokio::test]
async fn restarted_disputed_wallet_pays_during_repair_then_reopens_without_replay() {
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state");
    let key_path = dir.path().join("key");
    let mut state = Store::create(&state_path, &key_path, 1, b"fixture").unwrap();
    let identity = state.id().to_owned();
    let pool = state
        .ensure_capacity_group(
            "disputed",
            &crate::rotation::test_capacity::two_reserves("20"),
            Some(U256::from(20_000_000)),
            0,
        )
        .unwrap();
    let job = state
        .allocate_capacity_kind(&pool, crate::rotation::store::AllocationKind::Small, true)
        .unwrap()
        .unwrap();
    let wallet = state.funding_job(&job).unwrap().unwrap().wallet_id;
    let db_path = state_path.join("state.sqlite");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    db.execute("UPDATE wallets SET role='ACTIVE',evidence_repair='chain_repair_required',financial_height=80,financial_hash='disputed',debit_height=80,debit_hash='disputed' WHERE id=?1", [&wallet]).unwrap();
    let payer: String = db
        .query_row("SELECT address FROM wallets WHERE id=?1", [&wallet], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence) VALUES('old',?1,?2,0,'1000000','fixture','RESOLVED',?3,?3,?4,0,1,1)", rusqlite::params![pool,wallet,payer,format!("0x{:064x}",1)]).unwrap();
    db.execute(
        "UPDATE wallets SET payment_resolutions_disputed=1 WHERE id=?1",
        [&wallet],
    )
    .unwrap();
    db.execute("INSERT INTO payment_resolutions(attempt_id,outcome,height,hash,block_time,support) VALUES('old','USED',80,'disputed',1,'original-proof')", []).unwrap();
    drop(state);
    let state = Store::open(&state_path, &key_path, &identity).unwrap();
    let fixture = Fixture {
        holding: 20_000_000,
        debit: 0,
        price: Arc::new(AtomicU64::new(20_000_000)),
        head: Arc::new(AtomicU64::new(100)),
        calls: Arc::default(),
        unsigned: Arc::default(),
        sent: Arc::default(),
        timestamps: Arc::new(StdMutex::new(
            [(88, now().unwrap()), (100, now().unwrap())].into(),
        )),
        used: Arc::default(),
        db: db_path,
    };
    let repair_fixture = RepairFixture {
        payment: fixture.clone(),
        started: Arc::default(),
        release: Arc::new(tokio::sync::Semaphore::new(0)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/rpc", post(current_rpc))
        .route("/pay", get(journal_checked_seller))
        .with_state(repair_fixture.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (store, owner) = StoreHandle::spawn(state);
    let mut manager = ManagedPool::new(
        store.clone(),
        pool.clone(),
        BaseRpc::new(&format!("{url}/rpc"), 12, 120).unwrap(),
        SpendPolicy::dollars("20").unwrap(),
    )
    .unwrap();
    let signatures = Arc::new(AtomicUsize::new(0));
    manager.test_admission_hook = Some(Arc::new({
        let signatures = signatures.clone();
        move |stage| {
            if matches!(stage, AdmissionTestStage::Signed { .. }) {
                signatures.fetch_add(1, Ordering::SeqCst);
            }
            async {}.boxed()
        }
    }));
    let repair = tokio::spawn({
        let store = store.clone();
        let pool = pool.clone();
        let wallet = wallet.clone();
        let base = BaseRpc::new(&format!("{url}/rpc"), 12, 120).unwrap();
        async move { crate::rotation::store::repair_wallet(store, base, pool, wallet).await }
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        repair_fixture.started.notified(),
    )
    .await
    .unwrap();
    let client = PaidClient::managed(Arc::new(manager));
    let route = || RoutedRequest {
        method: "GET".into(),
        url: format!("{url}/pay"),
        query: Default::default(),
        body: None,
    };
    assert!(client.execute(route()).await.is_err());
    assert_eq!(signatures.load(Ordering::SeqCst), 0);
    assert!(fixture.sent.lock().unwrap().is_empty());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM payment_admission_evidence", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    fixture.price.store(19_000_000, Ordering::SeqCst);
    assert_eq!(client.execute(route()).await.unwrap(), "paid");
    assert_eq!(signatures.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.sent.lock().unwrap().len(), 1);
    assert_eq!(fixture.unsigned.load(Ordering::SeqCst), 2);
    let retained: (i64, String, String) = db.query_row("SELECT (SELECT payment_resolutions_disputed FROM wallets WHERE id=a.wallet_id),r.hash,r.support FROM payment_attempts a JOIN payment_resolutions r ON r.attempt_id=a.id WHERE a.id='old'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(retained, (1, "disputed".into(), "original-proof".into()));
    assert!(
        !repair.is_finished(),
        "payment must complete while historical repair stays owned"
    );
    repair_fixture.release.add_permits(1);
    assert!(repair.await.unwrap().unwrap());
    let audit: (String,String,String) = db.query_row("SELECT r.outcome,r.support,e.outcome FROM payment_resolutions r JOIN effective_payment_resolutions e USING(attempt_id) WHERE r.attempt_id='old'", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        audit,
        (
            "USED".into(),
            "original-proof".into(),
            "EXPIRED_UNUSED".into()
        )
    );
    assert_eq!(
        db.query_row(
            "SELECT payment_resolutions_disputed FROM wallets WHERE id=?1",
            [&wallet],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(client);
    drop(store);
    owner.await.unwrap();
    let reopened = Store::open(&state_path, &key_path, &identity).unwrap();
    let (store, owner) = StoreHandle::spawn(reopened);
    let manager = ManagedPool::new(
        store.clone(),
        pool,
        BaseRpc::new(&format!("{url}/rpc"), 12, 120).unwrap(),
        SpendPolicy::dollars("20").unwrap(),
    )
    .unwrap();
    let client = PaidClient::managed(Arc::new(manager));
    fixture.price.store(1_000_000, Ordering::SeqCst);
    assert_eq!(client.execute(route()).await.unwrap(), "paid");
    let sent = fixture.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert_ne!(
        sent[0], sent[1],
        "restart must create a distinct new authorization, never replay"
    );
    assert_eq!(db.query_row("SELECT COUNT(*),COUNT(DISTINCT nonce),SUM(CAST(amount AS INTEGER)) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap(),(2,2,20_000_000));
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT support FROM payment_resolutions WHERE attempt_id='old'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "original-proof"
    );
    drop(client);
    drop(store);
    owner.await.unwrap();
    server.abort();
}
