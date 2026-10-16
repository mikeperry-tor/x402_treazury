use super::*;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::Mutex;

struct Chain {
    head: u64,
    used: bool,
    requests: Vec<Value>,
    forbid_old: bool,
}
struct Fixture {
    store: Store,
    base: BaseRpc,
    wallet: String,
    pool: String,
    chain: Arc<Mutex<Chain>>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(count: usize, original_used: bool) -> Self {
        let stamp = crate::rotation::base::now().unwrap();
        let chain = Arc::new(Mutex::new(Chain {
            head: 100,
            used: original_used,
            requests: vec![],
            forbid_old: false,
        }));
        let state = chain.clone();
        let app=Router::new().route("/",post(move |Json(request):Json<Value>| {
            let state=state.clone();
            async move {
                let mut s=state.lock().unwrap();
                s.requests.push(request.clone());
                let value=match request["method"].as_str().unwrap() {
                    "eth_chainId"=>json!("0x2105"),
                    "eth_getBlockByNumber"=>{
                        let tag=request["params"][0].as_str().unwrap();
                        let h=if tag=="latest" {s.head}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                        assert!(!s.forbid_old || h>=108,"old original checkpoint must never be fetched");
                        json!({"number":format!("0x{h:x}"),"hash":format!("0x{h:064x}"),"timestamp":format!("0x{stamp:x}")})
                    },
                    "eth_call"=>{
                        assert_eq!(request["params"][1]["requireCanonical"],true);
                        let balance=request["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231");
                        json!(format!("0x{:064x}",if balance {1_000u64}else{u64::from(s.used)}))
                    },
                    _=>panic!("replacement must remain read-only")
                };
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":value}))
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
            "replacement",
            "1",
            None,
            0,
        )
        .unwrap();
        let address = store.status().unwrap().pools.remove(0).addresses.remove(0);
        let wallet = address.id;
        let mut auths = vec![];
        for index in 0..count {
            let id = format!("resolved-{index:04}");
            let nonce = format!("0x{:064x}", index + 1);
            store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence) VALUES(?1,?2,?3,0,'1','fixture','POSSIBLY_SUBMITTED',?4,?4,?5,0,?6,?7)",params![id,pool,wallet,address.address,nonce,i64::try_from(stamp-1).unwrap(),index as i64+1]).unwrap();
            auths.push(PendingAuthorization {
                id,
                wallet: wallet.clone(),
                payer: address.address.clone(),
                nonce,
                valid_before: stamp - 1,
            });
        }
        for page in auths.chunks(PAGE) {
            let query = store
                .scoped_chain_query(&pool, vec![], page.to_vec())
                .unwrap();
            store
                .publish_scoped_evidence(&pool, base.view(query).await.unwrap())
                .unwrap();
        }
        store
            .invalidate_wallet_evidence(
                &wallet,
                &Anchor {
                    height: 88,
                    hash: format!("0x{:064x}", 88),
                },
                true,
                false,
            )
            .unwrap();
        // The canonical path does not require original readiness, roles, or
        // admission balance records, and never restores these separate disputes.
        store.db.execute("UPDATE funding_jobs SET readiness_disputed='chain_repair_required' WHERE wallet_id=?1",[&wallet]).unwrap();
        {
            let mut s = chain.lock().unwrap();
            s.head = 120;
            s.used = !original_used;
            s.forbid_old = true;
            s.requests.clear();
        }
        Self {
            store,
            base,
            wallet,
            pool,
            chain,
            server,
            _dir: dir,
        }
    }
    async fn stage(&mut self) -> (Snapshot, Arc<ReplacementPass>, String, usize) {
        let snapshot = self
            .store
            .begin_replacement_staging(&self.pool, &self.wallet, Uuid::new_v4().to_string())
            .unwrap()
            .unwrap();
        let pass = Arc::new(
            self.base
                .begin_replacement(
                    snapshot.capture.clone(),
                    snapshot.address.clone(),
                    snapshot.debit.as_ref().map(|a| a.height),
                )
                .await
                .unwrap(),
        );
        let mut cursor = String::new();
        let mut digest = Sha256::new();
        let mut count = 0;
        loop {
            let originals = rows(&self.store.db, &self.wallet, &cursor).unwrap();
            if originals.is_empty() {
                break;
            }
            let auths = originals
                .iter()
                .map(|(_, raw)| {
                    serde_json::from_str::<Row>(raw)
                        .unwrap()
                        .authorization(&snapshot)
                        .unwrap()
                })
                .collect();
            let page = self.base.replacement_page(&pass, auths).await.unwrap();
            assert!(page.unresolved().is_empty());
            for (id, raw) in &originals {
                cursor = id.clone();
                digest_row(&mut digest, raw);
                count += 1;
            }
            self.store
                .stage_replacement_page(&snapshot, &pass, originals, page)
                .unwrap();
        }
        (snapshot, pass, format!("{:x}", digest.finalize()), count)
    }
}

#[tokio::test]
async fn canonical_replacement_preserves_originals_releases_only_complete_set_and_keeps_pending() {
    for original_used in [true, false] {
        let mut f = Fixture::new(65, original_used).await;
        let originals: Vec<String> = f
            .store
            .db
            .prepare("SELECT support FROM payment_resolutions ORDER BY attempt_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let (snapshot, pass, digest, count) = f.stage().await;
        f.store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES('new-pending',?1,?2,0,'7','fixture','POSSIBLY_SUBMITTED')",params![f.pool,f.wallet]).unwrap();
        let fresh = f.base.finish_replacement(&pass).await.unwrap();
        f.store
            .commit_replacement(snapshot, pass, fresh, digest, count)
            .unwrap();
        assert_eq!(exposure(&f.store.db, &f.wallet).unwrap(), U256::from(7));
        let after: Vec<String> = f
            .store
            .db
            .prepare("SELECT support FROM payment_resolutions ORDER BY attempt_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(originals, after);
        let (count, outcome): (i64, String) = f
            .store
            .db
            .query_row(
                "SELECT COUNT(*),MIN(outcome) FROM payment_resolution_replacements",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(!f.chain.lock().unwrap().requests.is_empty());
        assert_eq!(count, 65);
        assert_eq!(
            outcome,
            if original_used {
                "EXPIRED_UNUSED"
            } else {
                "USED"
            }
        );
        assert_eq!(
            checkpoint(&f.store.db, &f.wallet).unwrap().unwrap().height,
            108
        );
        assert_eq!(
            debit_checkpoint(&f.store.db, &f.wallet)
                .unwrap()
                .unwrap()
                .height,
            108
        );
        let repair: Option<String> = f
            .store
            .db
            .query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [&f.wallet],
                |row| row.get(0),
            )
            .unwrap();
        assert!(repair.is_none());
        let funding: String = f
            .store
            .db
            .query_row(
                "SELECT readiness_disputed FROM funding_jobs WHERE wallet_id=?1",
                [&f.wallet],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(funding, "chain_repair_required");
        let staged: i64 = f
            .store
            .db
            .query_row(
                "SELECT COUNT(*) FROM resolution_replacement_stage",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(staged, 0);
        let query = f
            .store
            .scoped_chain_query(&f.pool, vec![f.wallet.clone()], vec![])
            .unwrap();
        assert!(
            f.store
                .prepare_evidence_view(f.base.view(query).await.unwrap())
                .is_ok()
        );
    }
}

#[tokio::test]
async fn canonical_replacement_rejects_changed_membership_and_checkpoint_without_partial_release() {
    for change in 0..5 {
        let mut f = Fixture::new(2, true).await;
        let (snapshot, pass, digest, count) = f.stage().await;
        let fresh = f.base.finish_replacement(&pass).await.unwrap();
        match change {
            0 => {
                f.store
                    .db
                    .execute(
                        "UPDATE payment_attempts SET amount='2' WHERE id='resolved-0000'",
                        [],
                    )
                    .unwrap();
            }
            1 => {
                f.store
                    .db
                    .execute(
                        "UPDATE wallets SET debit_hash=?2 WHERE id=?1",
                        params![f.wallet, format!("0x{:064x}", 90)],
                    )
                    .unwrap();
            }
            2 => {
                f.store.db.execute("UPDATE payment_attempts SET resolution_disputed=0 WHERE id='resolved-0000'",[]).unwrap();
            }
            3 => {
                f.store
                    .invalidate_endpoint_evidence(&pass.endpoint())
                    .unwrap();
            }
            _ => {
                f.store.db.execute_batch("CREATE TEMP TRIGGER reject_adoption BEFORE UPDATE OF evidence_repair ON wallets BEGIN SELECT RAISE(ABORT,'fixture rollback'); END;").unwrap();
            }
        }
        assert!(
            f.store
                .commit_replacement(snapshot, pass, fresh, digest, count)
                .is_err()
        );
        let replacements: i64 = f
            .store
            .db
            .query_row(
                "SELECT COUNT(*) FROM payment_resolution_replacements",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(replacements, 0);
        let repairs: i64 = f
            .store
            .db
            .query_row(
                "SELECT COUNT(*) FROM payment_resolution_repairs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(repairs, 0);
        let flag: Option<String> = f
            .store
            .db
            .query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [&f.wallet],
                |row| row.get(0),
            )
            .unwrap();
        assert!(flag.is_some());
    }
}

#[tokio::test]
async fn staging_guard_removes_cancelled_owner_and_preserves_accepted_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let pool = crate::rotation::store::test_support::declare_two_reserves(
        &mut store, "cleanup", "1", None, 0,
    )
    .unwrap();
    let wallet = store.status().unwrap().pools[0].addresses[0].id.clone();
    store
        .db
        .execute(
            "UPDATE wallets SET evidence_repair='chain_repair_required' WHERE id=?1",
            [&wallet],
        )
        .unwrap();
    let (handle, task) = StoreHandle::spawn(store);
    let id = Uuid::new_v4().to_string();
    let owner = StagingOwner {
        store: handle.clone(),
        id: id.clone(),
    };
    let begin = id.clone();
    handle
        .call(move |s| s.begin_replacement_staging(&pool, &wallet, begin))
        .await
        .unwrap();
    drop(owner);
    tokio::task::yield_now().await;
    let mut removed = false;
    for _ in 0..20 {
        removed = handle
            .call(|s| {
                Ok(s.db.query_row(
                    "SELECT COUNT(*)=0 FROM resolution_replacement_passes",
                    [],
                    |row| row.get::<_, bool>(0),
                )?)
            })
            .await
            .unwrap();
        if removed {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(removed);
    drop(handle);
    task.await.unwrap();
}

#[tokio::test]
async fn replacement_does_not_require_obsolete_proof_payloads_but_preserves_them() {
    for old in [Some("{broken"), None] {
        let mut f = Fixture::new(1, true).await;
        f.store
            .db
            .execute("UPDATE payment_resolutions SET support=?1", [old])
            .unwrap();
        let (snapshot, pass, digest, count) = f.stage().await;
        let fresh = f.base.finish_replacement(&pass).await.unwrap();
        f.store
            .commit_replacement(snapshot, pass, fresh, digest, count)
            .unwrap();
        let saved: Option<String> = f
            .store
            .db
            .query_row("SELECT support FROM payment_resolutions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(saved.as_deref(), old);
        assert_eq!(exposure(&f.store.db, &f.wallet).unwrap(), U256::ZERO);
    }
}

#[tokio::test]
async fn replacement_rejects_missing_authoritative_rows_and_ambiguous_amounts() {
    for sql in [
        "DELETE FROM payment_resolutions",
        "UPDATE payment_attempts SET amount='malformed'",
        "UPDATE payment_attempts SET amount='0'",
    ] {
        let mut f = Fixture::new(1, true).await;
        f.store.db.execute(sql, []).unwrap();
        assert!(
            f.store
                .begin_replacement_staging(&f.pool, &f.wallet, Uuid::new_v4().to_string())
                .is_err()
        );
    }
    let mut f = Fixture::new(1, true).await;
    f.store
        .db
        .execute("DELETE FROM payment_resolutions", [])
        .unwrap();
    f.store
        .db
        .execute("DELETE FROM payment_attempts", [])
        .unwrap();
    let (snapshot, pass, digest, count) = f.stage().await;
    assert_eq!(count, 0);
    let fresh = f.base.finish_replacement(&pass).await.unwrap();
    assert_eq!(
        f.store
            .commit_replacement(snapshot, pass, fresh, digest, count)
            .unwrap_err()
            .downcast_ref::<EvidenceFailure>(),
        Some(&EvidenceFailure::MalformedSupport)
    );
}

#[tokio::test]
async fn production_repair_owner_adopts_after_reopen_and_uses_effective_provenance() {
    let mut f = Fixture::new(2, true).await;
    let scratch = tempfile::tempdir().unwrap();
    let dummy = Store::create(
        &scratch.path().join("state"),
        &scratch.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let original = std::mem::replace(&mut f.store, dummy);
    let treasury = original.id().to_owned();
    drop(original);
    let reopened = Store::open(
        &f._dir.path().join("state"),
        &f._dir.path().join("key"),
        &treasury,
    )
    .unwrap();
    let (store, worker) = StoreHandle::spawn(reopened);
    assert!(
        super::super::repair::repair_wallet(
            store.clone(),
            f.base.clone(),
            f.pool.clone(),
            f.wallet.clone()
        )
        .await
        .unwrap()
    );
    let wallet = f.wallet.clone();
    let query = store
        .call(move |s| {
            assert_eq!(exposure(&s.db, &wallet)?, U256::ZERO);
            let row: (String, String, u64) = s.db.query_row(
                "SELECT payer,nonce,valid_before FROM payment_attempts WHERE id='resolved-0000'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, sql_u64(row, 2)?)),
            )?;
            Ok(crate::rotation::base::ChainQuery {
                wallets: vec![],
                anchor: None,
                captures: std::collections::BTreeMap::from([(
                    wallet.clone(),
                    s.capture_evidence(&wallet)?,
                )]),
                pending: vec![PendingAuthorization {
                    id: "resolved-0000".into(),
                    wallet,
                    payer: row.0,
                    nonce: row.1,
                    valid_before: row.2,
                }],
            })
        })
        .await
        .unwrap();
    let view = f.base.view(query).await.unwrap();
    let endpoint = view.resolution_certificates["resolved-0000"]
        .endpoint()
        .to_owned();
    let pool = f.pool.clone();
    let wallet = f.wallet.clone();
    store.call(move |s| {
        s.publish_scoped_evidence(&pool, view)?;
        let reason: Option<String> = s.db.query_row("SELECT evidence_repair FROM wallets WHERE id=?1", [&wallet], |row|row.get(0))?;
        assert!(reason.is_none(), "duplicate new outcome must use adopted proof, not orphaned original");
        s.invalidate_endpoint_evidence(&endpoint)?;
        assert_eq!(exposure(&s.db, &wallet)?, U256::from(2));
        let original: String = s.db.query_row("SELECT outcome FROM payment_resolutions WHERE attempt_id='resolved-0000'", [], |row|row.get(0))?;
        let effective: String = s.db.query_row("SELECT outcome FROM effective_payment_resolutions WHERE attempt_id='resolved-0000'", [], |row|row.get(0))?;
        assert_eq!(original, "USED");assert_eq!(effective, "EXPIRED_UNUSED");
        Ok(())
    }).await.unwrap();
    drop(store);
    worker.await.unwrap();
}

#[tokio::test]
async fn adoption_rejects_same_hash_amount_contradiction_before_releasing_liability() {
    let mut f = Fixture::new(1, true).await;
    let (snapshot, pass, digest, count) = f.stage().await;
    let fresh = f.base.finish_replacement(&pass).await.unwrap();
    let mut observation = fresh.view().observations[&f.wallet].clone();
    observation.confirmed.amount = U256::from(999);
    observation.latest.amount = U256::from(999);
    let support = BalanceSupport {
        observation: observation.clone(),
        checkpoint: None,
    };
    f.store.db.execute("INSERT INTO payment_admission_evidence(attempt_id,block_height,block_hash,balance,reserved_before,valid_until,support,latest_height,latest_hash,endpoint,payer_role) VALUES('resolved-0000',?1,?2,'999','0',?3,?4,?5,?6,?7,'ACTIVE')",params![i64::try_from(observation.confirmed.anchor.height).unwrap(),observation.confirmed.anchor.hash,observation.valid_until.to_string(),serde_json::to_string(&support).unwrap(),i64::try_from(observation.latest.anchor.height).unwrap(),observation.latest.anchor.hash,observation.endpoint]).unwrap();
    assert_eq!(
        f.store
            .commit_replacement(snapshot, pass, fresh, digest, count)
            .unwrap_err()
            .downcast_ref::<EvidenceFailure>(),
        Some(&EvidenceFailure::AmountContradiction)
    );
    assert_eq!(exposure(&f.store.db, &f.wallet).unwrap(), U256::from(1));
    let reason: String = f
        .store
        .db
        .query_row(
            "SELECT evidence_repair FROM wallets WHERE id=?1",
            [&f.wallet],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reason, "chain_repair_required");
    let repairs: i64 = f
        .store
        .db
        .query_row(
            "SELECT COUNT(*) FROM payment_resolution_repairs",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repairs, 0);
}

#[tokio::test]
async fn unused_replacement_preserves_absent_debit_and_unexpired_nonce_cannot_release() {
    let mut f = Fixture::new(1, false).await;
    f.chain.lock().unwrap().used = false;
    let (snapshot, pass, digest, count) = f.stage().await;
    let fresh = f.base.finish_replacement(&pass).await.unwrap();
    f.store
        .commit_replacement(snapshot, pass, fresh, digest, count)
        .unwrap();
    assert!(debit_checkpoint(&f.store.db, &f.wallet).unwrap().is_none());

    let mut f = Fixture::new(1, true).await;
    f.store
        .db
        .execute(
            "UPDATE payment_attempts SET valid_before=?1",
            [i64::try_from(crate::rotation::base::now().unwrap() + 500).unwrap()],
        )
        .unwrap();
    let snapshot = f
        .store
        .begin_replacement_staging(&f.pool, &f.wallet, Uuid::new_v4().to_string())
        .unwrap()
        .unwrap();
    let pass = f
        .base
        .begin_replacement(
            snapshot.capture.clone(),
            snapshot.address.clone(),
            snapshot.debit.as_ref().map(|a| a.height),
        )
        .await
        .unwrap();
    let row: Row = serde_json::from_str(&rows(&f.store.db, &f.wallet, "").unwrap()[0].1).unwrap();
    let page = f
        .base
        .replacement_page(&pass, vec![row.authorization(&snapshot).unwrap()])
        .await
        .unwrap();
    assert_eq!(page.unresolved().len(), 1);
    assert!(page.resolutions().is_empty());
    assert_eq!(exposure(&f.store.db, &f.wallet).unwrap(), U256::from(1));
    f.store.clear_replacement_staging(&snapshot.id).unwrap();
}

#[tokio::test]
async fn retired_repair_releases_disputed_liability_without_restoring_payment_authority() {
    let mut f = Fixture::new(2, true).await;
    f.store
        .db
        .execute(
            "UPDATE reserve_claims SET wallet_id=NULL WHERE wallet_id=?1",
            [&f.wallet],
        )
        .unwrap();
    f.store
        .db
        .execute("UPDATE wallets SET role='RETIRED' WHERE id=?1", [&f.wallet])
        .unwrap();
    f.store
        .db
        .execute(
            "UPDATE pools SET payment_limit='1000000',enabled=1 WHERE id=?1",
            [&f.pool],
        )
        .unwrap();
    assert_eq!(exposure(&f.store.db, &f.wallet).unwrap(), U256::from(2));
    let before: String = f.store.db.query_row("SELECT json_object('requirement',readiness_requirement,'support',readiness_support,'dispute',readiness_disputed,'state',state) FROM funding_jobs WHERE wallet_id=?1",[&f.wallet],|r|r.get(0)).unwrap();
    let boundary = f.store.wallet_boundary(&f.pool).unwrap();
    assert!(
        f.store
            .repair_wallet_page(&f.pool, -1, boundary, PAGE)
            .unwrap()
            .iter()
            .any(|(_, id)| id.as_deref() == Some(&f.wallet))
    );
    let scratch = tempfile::tempdir().unwrap();
    let dummy = Store::create(
        &scratch.path().join("state"),
        &scratch.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let original = std::mem::replace(&mut f.store, dummy);
    let (store, worker) = StoreHandle::spawn(original);
    assert!(
        super::super::repair::repair_wallet(
            store.clone(),
            f.base.clone(),
            f.pool.clone(),
            f.wallet.clone()
        )
        .await
        .unwrap()
    );
    let pool = f.pool.clone();
    let wallet = f.wallet.clone();
    let query = store.call(move |s| {
        assert_eq!(exposure(&s.db,&wallet)?,U256::ZERO);
        let (role,repair):(String,Option<String>) = s.db.query_row("SELECT role,evidence_repair FROM wallets WHERE id=?1",[&wallet],|r|Ok((r.get(0)?,r.get(1)?)))?;
        assert_eq!(role,"RETIRED");
        assert!(repair.is_none());
        assert_eq!(s.db.query_row("SELECT COUNT(*) FROM reserve_claims WHERE wallet_id=?1",[&wallet],|r|r.get::<_,i64>(0))?,0);
        let after:String = s.db.query_row("SELECT json_object('requirement',readiness_requirement,'support',readiness_support,'dispute',readiness_disputed,'state',state) FROM funding_jobs WHERE wallet_id=?1",[&wallet],|r|r.get(0))?;
        assert_eq!(before,after);
        assert!(!s.repair_wallet_page(&pool,-1,s.wallet_boundary(&pool)?,PAGE)?.iter().any(|(_,id)|id.as_deref()==Some(&wallet)));
        s.payment_balance_query(&pool,vec![wallet])
    }).await.unwrap();
    let view = f.base.view(query).await.unwrap();
    let pool = f.pool.clone();
    let wallet = f.wallet.clone();
    store
        .call(move |s| {
            assert!(
                s.payment_candidate_with_evidence(
                    &pool,
                    U256::from(1),
                    Some(&wallet),
                    i64::MAX,
                    &[view]
                )?
                .is_none(),
                "fresh positive balance cannot restore a retired payer"
            );
            assert!(
                s.wallet_page(&pool, -1, s.wallet_boundary(&pool)?, PAGE)?
                    .iter()
                    .all(|(_, id, _)| id != &wallet)
            );
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    worker.await.unwrap();
}
