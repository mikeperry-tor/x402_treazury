use super::*;
use crate::rotation::base::PendingAuthorization;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Chain {
    head: u64,
    wrong_chain: bool,
    wrong_hash: bool,
    wrong_amount: bool,
    wrong_nonce: bool,
    wrong_time: bool,
    pause: bool,
}
struct Fixture {
    store: StoreHandle,
    base: BaseRpc,
    pool: String,
    wallet: String,
    calls: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn unsupported_undisputed_chain_block_fails_before_rpc() {
    let f = Fixture::new().await;
    let before = f.calls.load(Ordering::SeqCst);
    assert_eq!(
        f.repair()
            .await
            .unwrap_err()
            .downcast_ref::<EvidenceFailure>(),
        Some(&EvidenceFailure::ChainRepairRequired)
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), before);
    f.assert_blocked().await;
}

#[tokio::test]
async fn production_invalidation_repairs_canonically_and_preserves_pending() {
    let f = Fixture::new().await;
    let wallet = f.wallet.clone();
    let original = f
        .store
        .call(move |s| {
            let original: String = s.db.query_row(
                "SELECT support FROM payment_resolutions WHERE attempt_id='resolved'",
                [],
                |r| r.get(0),
            )?;
            let anchor = checkpoint(&s.db, &wallet)?.unwrap();
            s.invalidate_wallet_evidence(&wallet, &anchor, true, false)?;
            assert_eq!(exposure(&s.db, &wallet)?, U256::from(14));
            Ok(original)
        })
        .await
        .unwrap();
    assert!(f.repair().await.unwrap());
    assert!(!f.repair().await.unwrap());
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| {
            assert_eq!(exposure(&s.db, &wallet)?, U256::from(7));
            assert_eq!(
                s.db.query_row(
                    "SELECT support FROM payment_resolutions WHERE attempt_id='resolved'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                original
            );
            assert_eq!(
                s.db.query_row(
                    "SELECT count(*) FROM payment_resolution_repairs WHERE activated=1",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
}

fn remove_resolved_fixture(s: &mut Store) -> Result<()> {
    s.db.execute(
        "DELETE FROM payment_resolutions WHERE attempt_id='resolved'",
        [],
    )?;
    s.db.execute(
        "DELETE FROM payment_admission_evidence WHERE attempt_id='resolved'",
        [],
    )?;
    s.db.execute("DELETE FROM payment_attempts WHERE id='resolved'", [])?;
    Ok(())
}

#[tokio::test]
async fn zero_resolved_invalidation_distinguishes_missing_journal_from_empty_history() {
    for retain_anchor in [false, true] {
        let f = Fixture::new().await;
        let wallet = f.wallet.clone();
        f.store.call(move |s| {
            let anchor=checkpoint(&s.db,&wallet)?.unwrap();
            remove_resolved_fixture(s)?;
            s.db.execute("UPDATE wallets SET evidence_repair=NULL,financial_height=CASE WHEN ?2 THEN financial_height ELSE NULL END,financial_hash=CASE WHEN ?2 THEN financial_hash ELSE NULL END,debit_height=CASE WHEN ?2 THEN debit_height ELSE NULL END,debit_hash=CASE WHEN ?2 THEN debit_hash ELSE NULL END WHERE id=?1",params![wallet,retain_anchor])?;
            s.invalidate_wallet_evidence(&wallet,&anchor,retain_anchor,false)?;
            let reason: Option<String>=s.db.query_row("SELECT evidence_repair FROM wallets WHERE id=?1",[&wallet],|r|r.get(0))?;
            assert_eq!(reason.is_some(),retain_anchor);
            if !retain_anchor { check_capacity_repair(&s.db,&wallet)?; }
            Ok(())
        }).await.unwrap();
        let before = f.calls.load(Ordering::SeqCst);
        if retain_anchor {
            assert_eq!(
                f.repair()
                    .await
                    .unwrap_err()
                    .downcast_ref::<EvidenceFailure>(),
                Some(&EvidenceFailure::MalformedSupport)
            );
            f.assert_blocked().await;
        } else {
            assert!(!f.repair().await.unwrap());
        }
        assert_eq!(before, f.calls.load(Ordering::SeqCst));
        if !retain_anchor {
            let pool = f.pool.clone();
            let wallet = f.wallet.clone();
            let query = f
                .store
                .call(move |s| s.scoped_chain_query(&pool, vec![wallet], vec![]))
                .await
                .unwrap();
            let fresh = f.base.view(query).await.unwrap();
            let pool = f.pool.clone();
            f.store
                .call(move |s| s.publish_scoped_evidence(&pool, fresh))
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn explicit_empty_chain_block_uses_fresh_capacity_without_inventing_history() {
    let f = Fixture::new().await;
    let wallet = f.wallet.clone();
    f.store.call(move |s| {
        remove_resolved_fixture(s)?;
        s.db.execute("UPDATE wallets SET financial_height=NULL,financial_hash=NULL,debit_height=NULL,debit_hash=NULL WHERE id=?1",[wallet])?;
        Ok(())
    }).await.unwrap();
    assert!(f.repair().await.unwrap());
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| {
            assert_eq!(exposure(&s.db, &wallet)?, U256::from(7));
            assert!(checkpoint(&s.db, &wallet)?.is_none());
            assert!(debit_checkpoint(&s.db, &wallet)?.is_none());
            check_capacity_repair(&s.db, &wallet)?;
            assert_eq!(
                s.db.query_row(
                    "SELECT resolution_count FROM payment_resolution_repairs WHERE activated=1",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                0
            );
            assert_eq!(
                s.db.query_row("SELECT role FROM wallets WHERE id=?1", [wallet], |r| r
                    .get::<_, String>(
                    0
                ))?,
                "ACTIVE"
            );
            Ok(())
        })
        .await
        .unwrap();
}
impl Fixture {
    async fn new() -> Self {
        Self::with_outcome(false).await
    }
    async fn with_outcome(expired: bool) -> Self {
        let chain = Arc::new(Mutex::new(Chain {
            head: 100,
            ..Default::default()
        }));
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let resume = release.clone();
        let state = chain.clone();
        let counted = calls.clone();
        let timestamp = crate::rotation::base::now().unwrap();
        let app = Router::new().route("/",post(move |Json(v):Json<Value>| {
            let state=state.clone();
            let signal=signal.clone();
            let resume=resume.clone();
            counted.fetch_add(1,Ordering::SeqCst);
            async move {
                let pause=state.lock().unwrap().pause;
                if pause {signal.notify_one();resume.notified().await;}
                let s=state.lock().unwrap();
                let result=match v["method"].as_str().unwrap() {
                    "eth_chainId"=>json!(if s.wrong_chain {"0x1"}else{"0x2105"}),
                    "eth_getBlockByNumber"=>{
                        let tag=v["params"][0].as_str().unwrap();
                        let h=if tag=="latest" {s.head}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                        let hash=h+if s.wrong_hash && h==88 {1000}else{0};
                        let time=timestamp+u64::from(s.wrong_time);
                        json!({"number":format!("0x{h:x}"),"hash":format!("0x{hash:064x}"),"timestamp":format!("0x{time:x}")})
                    },
                    "eth_call"=>{
                        assert_eq!(v["params"][1]["requireCanonical"],true);
                        let balance=v["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231");
                        let value=if balance {5_000_000+u64::from(s.wrong_amount)}else{u64::from(!s.wrong_nonce)};
                        json!(format!("0x{value:064x}"))
                    },
                    other=>panic!("unexpected mutation: {other}"),
                };
                Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = BaseRpc::new(
            &format!("http://{}/", listener.local_addr().unwrap()),
            12,
            120,
        )
        .unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "repair",
            "1",
            Some(U256::from(100_000_000)),
            0,
        )
        .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        let wallet = job.wallet_id.clone();
        let query = store
            .scoped_chain_query(&pool, vec![wallet.clone()], vec![])
            .unwrap();
        let support = store
            .record_verified_credit(&wallet, base.view(query).await.unwrap())
            .unwrap();
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        store
            .db
            .execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1", [&wallet])
            .unwrap();
        let payer = support.observation.address.clone();
        let nonce = format!("0x{:064x}", 1);
        let valid_before = if expired {
            timestamp - 1
        } else {
            timestamp + 100
        };
        for (id, seq) in [("resolved", 1), ("pending", 2)] {
            store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence,payee,valid_after) VALUES(?1,?2,?3,0,'7','fixture','POSSIBLY_SUBMITTED',?4,?5,?6,?7,?4,0)",
                params![id,pool,wallet,payer,format!("0x{seq:064x}"),i64::try_from(valid_before).unwrap(),seq]).unwrap();
            let o = &support.observation;
            store.db.execute("INSERT INTO payment_admission_evidence(attempt_id,block_height,block_hash,balance,reserved_before,valid_until,support,latest_height,latest_hash,endpoint,payer_role) VALUES(?1,?2,?3,?4,'0',?5,?6,?7,?8,?9,'ACTIVE')",
                params![id,i64::try_from(o.confirmed.anchor.height).unwrap(),o.confirmed.anchor.hash,o.available().to_string(),o.valid_until.to_string(),serde_json::to_string(&support).unwrap(),i64::try_from(o.latest.anchor.height).unwrap(),o.latest.anchor.hash,o.endpoint]).unwrap();
            add_exposure(&store.db, &wallet, U256::from(7)).unwrap();
        }
        chain.lock().unwrap().wrong_nonce = expired;
        let pending = PendingAuthorization {
            id: "resolved".into(),
            wallet: wallet.clone(),
            payer,
            nonce,
            valid_before,
        };
        let query = store
            .scoped_chain_query(&pool, vec![], vec![pending])
            .unwrap();
        store
            .publish_scoped_evidence(&pool, base.view(query).await.unwrap())
            .unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET evidence_repair='chain_repair_required' WHERE id=?1",
                [&wallet],
            )
            .unwrap();
        chain.lock().unwrap().head = 120;
        let (store, _) = StoreHandle::spawn(store);
        Self {
            store,
            base,
            pool,
            wallet,
            calls,
            server,
            _dir: dir,
        }
    }
    async fn repair(&self) -> Result<bool> {
        repair_wallet(
            self.store.clone(),
            self.base.clone(),
            self.pool.clone(),
            self.wallet.clone(),
        )
        .await
    }
    async fn assert_blocked(&self) {
        let wallet = self.wallet.clone();
        self.store
            .call(move |s| {
                let flag: Option<String> = s.db.query_row(
                    "SELECT evidence_repair FROM wallets WHERE id=?1",
                    [wallet],
                    |r| r.get(0),
                )?;
                assert_eq!(flag.as_deref(), Some("chain_repair_required"));
                assert_eq!(
                    s.db.query_row("SELECT COUNT(*) FROM wallet_evidence_repairs", [], |r| r
                        .get::<_, i64>(0))?,
                    0
                );
                Ok(())
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn independent_nonce_resolution_progresses_without_clearing_capacity_repair() {
    for expired in [false, true] {
        for reason in ["chain_repair_required", "evidence_repair_required"] {
            let f = Fixture::with_outcome(expired).await;
            let wallet = f.wallet.clone();
            let pool = f.pool.clone();
            let (mixed_query, history_query, original) = f
                .store
                .call(move |s| {
                    s.db.execute(
                        "UPDATE wallets SET evidence_repair=?2 WHERE id=?1",
                        params![wallet, reason],
                    )?;
                    let original: String = s.db.query_row(
                        "SELECT support FROM payment_resolutions WHERE attempt_id='resolved'",
                        [],
                        |r| r.get(0),
                    )?;
                    let boundary = s.history_boundary(&pool)?;
                    let pending = s
                        .history_page(&pool, -1, boundary, 64)?
                        .into_iter()
                        .map(|(_, auth)| auth.map_err(anyhow::Error::new))
                        .collect::<Result<Vec<_>>>()?;
                    assert_eq!(pending.len(), 1);
                    Ok((
                        s.scoped_chain_query(&pool, vec![wallet], pending.clone())?,
                        s.scoped_chain_query(&pool, vec![], pending)?,
                        original,
                    ))
                })
                .await
                .unwrap();
            // Mixed publication must not partially release a liability when its
            // accompanying current balance cannot pass the capacity gate.
            let mixed = f.base.view(mixed_query).await.unwrap();
            let pool = f.pool.clone();
            let wallet = f.wallet.clone();
            f.store
                .call(move |s| {
                    assert!(s.publish_scoped_evidence(&pool, mixed).is_err());
                    assert_eq!(exposure(&s.db, &wallet)?, U256::from(7));
                    Ok(())
                })
                .await
                .unwrap();
            let history = f.base.view(history_query).await.unwrap();
            let endpoint = history.resolution_certificates["pending"]
                .endpoint()
                .to_owned();
            let stale = history.clone();
            let pool = f.pool.clone();
            let wallet = f.wallet.clone();
            f.store
                .call(move |s| {
                    bump_generation(&s.db, "wallet", &wallet)?;
                    assert_eq!(
                        s.publish_scoped_evidence(&pool, stale)
                            .unwrap_err()
                            .downcast_ref::<EvidenceFailure>(),
                        Some(&EvidenceFailure::Invalidated)
                    );
                    Ok(())
                })
                .await
                .unwrap();
            let pool = f.pool.clone();
            let query = f
                .store
                .call(move |s| {
                    let pending = s
                        .history_page(&pool, -1, s.history_boundary(&pool)?, 64)?
                        .into_iter()
                        .map(|(_, auth)| auth.map_err(anyhow::Error::new))
                        .collect::<Result<Vec<_>>>()?;
                    s.scoped_chain_query(&pool, vec![], pending)
                })
                .await
                .unwrap();
            let history = f.base.view(query).await.unwrap();
            let pool = f.pool.clone();
            let wallet = f.wallet.clone();
            f.store
                .call(move |s| {
                    s.publish_scoped_evidence(&pool, history)?;
                    assert_eq!(exposure(&s.db, &wallet)?, U256::ZERO);
                    let saved: String = s.db.query_row(
                        "SELECT support FROM payment_resolutions WHERE attempt_id='resolved'",
                        [],
                        |r| r.get(0),
                    )?;
                    assert_eq!(saved, original);
                    let flag: String = s.db.query_row(
                        "SELECT evidence_repair FROM wallets WHERE id=?1",
                        [&wallet],
                        |r| r.get(0),
                    )?;
                    assert_eq!(flag, reason);
                    assert!(check_capacity_repair(&s.db, &wallet).is_err());
                    assert_eq!(
                        s.db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                            .get::<_, i64>(0))?,
                        2
                    );
                    assert_eq!(
                        s.db.query_row("SELECT COUNT(*) FROM wallet_evidence_repairs", [], |r| {
                            r.get::<_, i64>(0)
                        })?,
                        0
                    );
                    let stale = s.capture_evidence(&wallet)?;
                    let generation_before = generation(&s.db, "endpoint", &endpoint)?;
                    s.invalidate_endpoint_evidence(&endpoint)?;
                    assert_eq!(
                        check_capture(
                            &s.db,
                            s.evidence_namespace,
                            &stale,
                            &endpoint,
                            generation_before
                        )
                        .unwrap_err()
                        .downcast_ref::<EvidenceFailure>(),
                        Some(&EvidenceFailure::Invalidated)
                    );
                    Ok(())
                })
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn chain_invalidation_restores_exact_disputed_exposure_without_rewriting_proofs() {
    for amount_conflict in [false, true] {
        let f = Fixture::new().await;
        let wallet = f.wallet.clone();
        f.store
            .call(move |s| {
                let original: String = s.db.query_row(
                    "SELECT support FROM payment_resolutions WHERE attempt_id='resolved'",
                    [],
                    |r| r.get(0),
                )?;
                let checkpoint = checkpoint(&s.db, &wallet)?.unwrap();
                let capture = s.capture_evidence(&wallet)?;
                for _ in 0..2 {
                    s.invalidate_wallet_evidence(&wallet, &checkpoint, true, amount_conflict)?;
                    assert_eq!(
                        exposure(&s.db, &wallet)?,
                        U256::from(if amount_conflict { 7 } else { 14 })
                    );
                }
                assert_ne!(capture.generation, s.capture_evidence(&wallet)?.generation);
                assert_eq!(
                    s.db.query_row(
                        "SELECT support FROM payment_resolutions WHERE attempt_id='resolved'",
                        [],
                        |r| r.get::<_, String>(0)
                    )?,
                    original
                );
                assert_eq!(
                    s.db.query_row(
                        "SELECT state FROM payment_attempts WHERE id='resolved'",
                        [],
                        |r| r.get::<_, String>(0)
                    )?,
                    "RESOLVED"
                );
                assert_eq!(
                    s.db.query_row(
                        "SELECT (state='RESOLVED' AND (SELECT payment_resolutions_disputed FROM wallets WHERE id=wallet_id)) FROM payment_attempts WHERE id='pending'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    0
                );
                let sibling: String = s.db.query_row(
                    "SELECT id FROM wallets WHERE id!=?1 LIMIT 1",
                    [&wallet],
                    |r| r.get(0),
                )?;
                assert_eq!(exposure(&s.db, &sibling)?, U256::ZERO);
                Ok(())
            })
            .await
            .unwrap();
    }
}
