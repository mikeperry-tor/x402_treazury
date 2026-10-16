use super::*;
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
    chain: Arc<Mutex<Chain>>,
    calls: Arc<AtomicUsize>,
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
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
            store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) VALUES(?1,?2,?3,0,'7','fixture','POSSIBLY_SUBMITTED',?4,?5,?6,?7)",
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
            chain,
            calls,
            started,
            release,
            server,
            _dir: dir,
        }
    }
    async fn snapshot(&self) -> Snapshot {
        let pool = self.pool.clone();
        let wallet = self.wallet.clone();
        self.store
            .call(move |s| Ok(s.repair_snapshot(&pool, &wallet)?.unwrap()))
            .await
            .unwrap()
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
async fn exact_revalidation_preserves_original_proofs_and_pending_liabilities() {
    let f = Fixture::new().await;
    let before = f.snapshot().await;
    let wallet = f.wallet.clone();
    let original = f
        .store
        .call(move |s| rows(&s.db, &wallet, 0))
        .await
        .unwrap();
    assert!(f.repair().await.unwrap());
    assert!(!f.repair().await.unwrap());
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| {
            assert_eq!(exposure(&s.db, &wallet)?, U256::from(7));
            assert_eq!(checkpoint(&s.db, &wallet)?, before.capture.checkpoint);
            assert_eq!(rows(&s.db, &wallet, 0)?, original);
            let audit: (i64, i64) = s.db.query_row(
                "SELECT resolution_count,balance_count FROM wallet_evidence_repairs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!(audit, (1, 3));
            assert_eq!(
                generation(&s.db, "wallet", &wallet)?,
                before.capture.generation + 1
            );
            assert!(!s.evidence_cache.contains_key(&wallet));
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn contradictory_original_chain_evidence_never_clears_block() {
    for kind in 0..5 {
        let f = Fixture::new().await;
        {
            let mut s = f.chain.lock().unwrap();
            match kind {
                0 => s.wrong_chain = true,
                1 => s.wrong_hash = true,
                2 => s.wrong_amount = true,
                3 => s.wrong_nonce = true,
                _ => s.wrong_time = true,
            }
        }
        assert!(f.repair().await.is_err(), "case {kind}");
        f.assert_blocked().await;
    }
}

#[tokio::test]
async fn incomplete_or_disputed_journals_cannot_be_upgraded_by_current_balances() {
    for sql in [
        "UPDATE payment_resolutions SET support=NULL",
        "UPDATE payment_resolutions SET support='broken'",
        "UPDATE payment_admission_evidence SET support=NULL",
        "UPDATE wallets SET unresolved_exposure='0' WHERE role='ACTIVE'",
        "UPDATE funding_jobs SET readiness_disputed='chain_repair_required'",
        "UPDATE funding_jobs SET state='QUEUED'",
        "UPDATE funding_progress SET provider_refunded=1",
        "UPDATE funding_progress SET phase='\"RECOVERY_REQUIRED\"'",
        "UPDATE funding_jobs SET readiness_requirement='9000000'",
    ] {
        let f = Fixture::new().await;
        f.store
            .call(move |s| {
                s.db.execute(sql, [])?;
                Ok(())
            })
            .await
            .unwrap();
        let count = f.calls.load(Ordering::SeqCst);
        assert!(!matches!(f.repair().await, Ok(true)), "{sql}");
        assert_eq!(count, f.calls.load(Ordering::SeqCst));
        f.assert_blocked().await;
    }
}

#[tokio::test]
async fn commit_rejects_changed_journal_and_rolls_back_audit_failures() {
    for sql in [
        "UPDATE payment_attempts SET requirements_hash='changed' WHERE id='pending'",
        "INSERT INTO evidence_generations(kind,scope,generation) SELECT 'wallet',id,1 FROM wallets",
        "CREATE TRIGGER fail_repair BEFORE INSERT ON wallet_evidence_repairs BEGIN SELECT RAISE(ABORT,'fixture'); END",
    ] {
        let f = Fixture::new().await;
        let verified = verify_snapshot(&f.store, &f.base, f.snapshot().await)
            .await
            .unwrap();
        f.store
            .call(move |s| {
                s.db.execute(sql, [])?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            f.store
                .call(move |s| s.commit_repair(verified))
                .await
                .is_err()
        );
        f.assert_blocked().await;
    }
}

#[tokio::test]
async fn retained_expired_unused_history_can_repair_without_inventing_a_debit() {
    let f = Fixture::with_outcome(true).await;
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| {
            s.db.execute("UPDATE wallets SET role='RETAINED' WHERE id=?1", [wallet])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(f.repair().await.unwrap());
    f.store
        .call(|s| {
            let debit: Option<i64> = s.db.query_row(
                "SELECT debit_height FROM wallets WHERE role='RETAINED'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(debit, None);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn cancellation_during_read_only_revalidation_preserves_block_and_retryability() {
    let f = Fixture::new().await;
    f.chain.lock().unwrap().pause = true;
    let task = tokio::spawn(repair_wallet(
        f.store.clone(),
        f.base.clone(),
        f.pool.clone(),
        f.wallet.clone(),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(5), f.started.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    f.assert_blocked().await;
    f.chain.lock().unwrap().pause = false;
    f.release.notify_one();
    assert!(f.repair().await.unwrap());
}

#[tokio::test]
async fn paged_history_keeps_all_liabilities_and_requires_fresh_final_evidence() {
    let f = Fixture::new().await;
    let wallet = f.wallet.clone();
    f.store.call(move |s|{
        for n in 3..=70i64 {
            let id=format!("pending-{n}");
            s.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) SELECT ?1,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,?2,valid_before,?3 FROM payment_attempts WHERE id='pending'",params![id,format!("0x{n:064x}"),n])?;
            s.db.execute("INSERT INTO payment_admission_evidence(attempt_id,block_height,block_hash,balance,reserved_before,valid_until,support,latest_height,latest_hash,endpoint,payer_role) SELECT ?1,block_height,block_hash,balance,reserved_before,valid_until,support,latest_height,latest_hash,endpoint,payer_role FROM payment_admission_evidence WHERE attempt_id='pending'",[id])?;
            add_exposure(&s.db,&wallet,U256::from(7))?;
        }
        Ok(())
    }).await.unwrap();
    let snapshot = f.snapshot().await;
    assert_eq!(snapshot.attempts, 70);
    // The current confirmed balance must cover the immutable debit floor;
    // valid historical work alone cannot authorize clearing the block.
    f.chain.lock().unwrap().head = 90;
    assert!(f.repair().await.is_err());
    f.assert_blocked().await;
    f.chain.lock().unwrap().head = 120;
    assert!(f.repair().await.unwrap());
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| {
            assert_eq!(exposure(&s.db, &wallet)?, U256::from(69 * 7));
            assert_eq!(
                s.db.query_row(
                    "SELECT balance_count FROM wallet_evidence_repairs",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                71
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn restart_rejects_pre_restart_verification_and_rechecks_original_support() {
    let f = Fixture::new().await;
    let verified = verify_snapshot(&f.store, &f.base, f.snapshot().await)
        .await
        .unwrap();
    let state = f._dir.path().join("state");
    let key = f._dir.path().join("key");
    f.store
        .call(move |s| {
            let id = s.id().to_owned();
            let tmp = tempfile::tempdir()?;
            let dummy = Store::create(
                &tmp.path().join("state"),
                &tmp.path().join("key"),
                1,
                b"unfunded-reopen",
            )?;
            drop(std::mem::replace(s, dummy));
            *s = Store::open(&state, &key, &id)?;
            assert!(s.commit_repair(verified).is_err());
            Ok(())
        })
        .await
        .unwrap();
    f.assert_blocked().await;
    assert!(f.repair().await.unwrap());
}

#[tokio::test]
async fn serving_maintenance_automatically_revalidates_chain_block() {
    use crate::payment::SpendPolicy;
    use crate::rotation::manager::ManagedPool;
    let f = Fixture::new().await;
    // Remove the pending row for this test so concurrent ordinary history
    // reconciliation cannot legitimately change the repair snapshot.
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| {
            s.db.execute(
                "DELETE FROM payment_admission_evidence WHERE attempt_id='pending'",
                [],
            )?;
            s.db.execute("DELETE FROM payment_attempts WHERE id='pending'", [])?;
            s.db.execute(
                "UPDATE wallets SET unresolved_exposure='0' WHERE id=?1",
                [wallet],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let pool = Arc::new(
        ManagedPool::new(
            f.store.clone(),
            f.pool.clone(),
            f.base.clone(),
            SpendPolicy::dollars("5").unwrap(),
        )
        .unwrap(),
    );
    pool.reconcile().await.unwrap();
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| {
            let flag: Option<String> = s.db.query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [wallet],
                |r| r.get(0),
            )?;
            assert_eq!(flag, None);
            assert_eq!(
                s.db.query_row("SELECT COUNT(*) FROM wallet_evidence_repairs", [], |r| r
                    .get::<_, i64>(0))?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
}
