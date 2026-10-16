use super::*;
use crate::{catalog::RoutedRequest, payment::PaidClient, rotation::store::Store};

async fn current_rpc(state: State<Fixture>, request: Json<Value>) -> Json<Value> {
    if request.0["method"] == "eth_getBlockByNumber" {
        let tag = request.0["params"][0].as_str().unwrap();
        assert!(
            matches!(tag, "latest" | "0x58" | "0x64"),
            "payment must not read the disputed historical checkpoint"
        );
    }
    rpc(state, request).await
}

async fn journal_checked_seller(
    state: State<Fixture>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(signature) = headers.get("payment-signature") {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
        let nonce = payload["payload"]["authorization"]["nonce"]
            .as_str()
            .unwrap();
        let db = rusqlite::Connection::open(&state.0.db).unwrap();
        let admission: (String, String, String) = db.query_row("SELECT a.amount,e.balance,e.reserved_before FROM payment_attempts a JOIN payment_admission_evidence e ON e.attempt_id=a.id WHERE a.nonce=?1 AND a.state='POSSIBLY_SUBMITTED'", [nonce], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(
            admission,
            ("19000000".into(), "20000000".into(), "1000000".into())
        );
    }
    seller(state, headers).await
}

#[tokio::test]
async fn restarted_disputed_wallet_signs_only_its_remaining_capacity() {
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
    state
        .record_credit(&wallet, "20000000", &format!("0x{:064x}", 88), 88)
        .unwrap();
    let db_path = state_path.join("state.sqlite");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    db.execute("UPDATE wallets SET evidence_repair='chain_repair_required',financial_height=80,financial_hash='disputed',debit_height=80,debit_hash='disputed' WHERE id=?1", [&wallet]).unwrap();
    db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES('old',?1,?2,0,'1000000','fixture','RESOLVED')", rusqlite::params![pool,wallet]).unwrap();
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/rpc", post(current_rpc))
        .route("/pay", get(journal_checked_seller))
        .with_state(fixture.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (store, owner) = StoreHandle::spawn(state);
    let mut manager = ManagedPool::new(
        store.clone(),
        pool,
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
    drop(client);
    drop(store);
    owner.await.unwrap();
    server.abort();
}
