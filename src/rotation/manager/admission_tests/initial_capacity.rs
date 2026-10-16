//! Initial partial credit may pay without completing the original funding claim.
use super::*;
use crate::{catalog::RoutedRequest, payment::PaidClient, rotation::store::Store};

async fn initial_seller(state: State<Fixture>, headers: HeaderMap) -> axum::response::Response {
    if let Some(signature) = headers.get("payment-signature") {
        let payload: Value =
            serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
        let nonce = payload["payload"]["authorization"]["nonce"]
            .as_str()
            .unwrap();
        let db = rusqlite::Connection::open(&state.0.db).unwrap();
        let saved: (String, String, String, String) = db.query_row("SELECT a.amount,e.balance,e.reserved_before,e.payer_role FROM payment_attempts a JOIN payment_admission_evidence e ON e.attempt_id=a.id WHERE a.nonce=?1", [nonce], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(
            saved,
            (
                "100000".into(),
                "2000000".into(),
                "0".into(),
                "ALLOCATED".into()
            )
        );
    }
    seller(state, headers).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Eligible,
    DisabledClaim,
    ChangedClaim,
    Insufficient,
}

#[tokio::test]
async fn initial_partial_credit_pays_after_restart_without_completing_funding() {
    for case in [
        Case::Eligible,
        Case::DisabledClaim,
        Case::ChangedClaim,
        Case::Insufficient,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state");
        let key_path = dir.path().join("key");
        let mut state = Store::create(&state_path, &key_path, 1, b"fixture").unwrap();
        let identity = state.id().to_owned();
        let pool = state
            .ensure_capacity_group(
                "initial",
                &crate::rotation::test_capacity::two_reserves("10"),
                Some(U256::from(20_000_000)),
                0,
            )
            .unwrap();
        let job = state
            .allocate_capacity_kind(&pool, crate::rotation::store::AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let funding = state.funding_job(&job).unwrap().unwrap();
        let wallet = funding.wallet_id;
        let operation = funding.operation_id;
        // No readiness record or synthetic record_credit: only the later RPC
        // observation establishes the initial two-dollar usable balance.
        drop(state);
        let state = Store::open(&state_path, &key_path, &identity).unwrap();
        let db_path = state_path.join("state.sqlite");
        let db = rusqlite::Connection::open(&db_path).unwrap();
        match case {
            Case::DisabledClaim => {
                db.execute(
                    "UPDATE reserve_claims SET enabled=0 WHERE wallet_id=?1",
                    [&wallet],
                )
                .unwrap();
            }
            Case::ChangedClaim => {
                db.execute(
                    "UPDATE reserve_claims SET generation=generation+1 WHERE wallet_id=?1",
                    [&wallet],
                )
                .unwrap();
            }
            _ => {}
        }
        let claim_before: (String, i64, String) = db
            .query_row(
                "SELECT wallet_id,generation,target FROM reserve_claims WHERE wallet_id=?1",
                [&wallet],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let fixture = Fixture {
            holding: if case == Case::Insufficient {
                99_999
            } else {
                2_000_000
            },
            debit: 0,
            price: Arc::new(AtomicU64::new(100_000)),
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
            .route("/rpc", post(rpc))
            .route("/pay", get(initial_seller))
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
        let result = client
            .execute(RoutedRequest {
                method: "GET".into(),
                url: format!("{url}/pay"),
                query: Default::default(),
                body: None,
            })
            .await;
        if case == Case::Eligible {
            assert_eq!(result.unwrap(), "paid");
        } else {
            assert!(result.is_err(), "{case:?}");
        }
        let expected = usize::from(case == Case::Eligible);
        assert_eq!(signatures.load(Ordering::SeqCst), expected, "{case:?}");
        assert_eq!(fixture.sent.lock().unwrap().len(), expected, "{case:?}");
        let claim_after: (String, i64, String) = db
            .query_row(
                "SELECT wallet_id,generation,target FROM reserve_claims WHERE wallet_id=?1",
                [&wallet],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            claim_after, claim_before,
            "payment must retain the original claim"
        );
        let original: (String, String, Option<String>, String, String) = db.query_row("SELECT f.wallet_id,f.target,f.readiness_requirement,p.operation_id,w.role FROM funding_jobs f JOIN funding_progress p ON p.job_id=f.id JOIN wallets w ON w.id=f.wallet_id WHERE f.id=?1", [&job], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(
            original,
            (
                wallet,
                "10000000".into(),
                None,
                operation,
                if case == Case::Eligible {
                    "ACTIVE"
                } else {
                    "ALLOCATED"
                }
                .into()
            )
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1,
            "no replacement allocation"
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            expected as i64
        );
        drop(client);
        drop(store);
        owner.await.unwrap();
        server.abort();
    }
}
