//! Proof retries remain independent of advisory successes and maintenance owners.
use super::*;
use std::time::Duration;

fn add_retained_sibling(f: &Fixture, wallet: &str, address: u64, balance: u64) {
    // These public unfunded rows need no signer or source operation. Give each
    // its own consumed initial claim so the destination guard remains real.
    let db = f.db();
    db.execute(
        "INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance) SELECT ?1,pool_id,(SELECT MAX(sequence)+1 FROM wallets WHERE pool_id=w.pool_id),?2,X'00','1','RETAINED',?3 FROM wallets w WHERE id=?4",
        rusqlite::params![wallet, format!("0x{address:040x}"), balance.to_string(), f.wallet],
    ).unwrap();
    db.execute(
        "INSERT INTO funding_jobs(id,wallet_id,state,target,pool_id,allocation_kind,claim_generation,readiness_requirement,readiness_capacity) SELECT ?1,id,'COMPLETE','1',pool_id,'small',0,'1','1' FROM wallets WHERE id=?2",
        rusqlite::params![format!("job-{wallet}"), wallet],
    ).unwrap();
}

async fn proof_due(f: &Fixture) -> tokio::time::Instant {
    let wallet = f.wallet.clone();
    f.store
        .call(move |s| Ok(s.retirement_due(&wallet)))
        .await
        .unwrap()
        .expect("failed proof retains its request")
}

#[tokio::test]
async fn concurrent_canonical_publication_owns_retirement_completion_in_both_directions() {
    for newer_latest_high in [false, true] {
        let f = Fixture::new(1_000, true, true, 0).await;
        let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
        f.store
            .call(move |s| s.finish_balance(&group, &wallet, Some(U256::ZERO)))
            .await
            .unwrap();
        {
            let mut rpc = f.rpc.state.lock().unwrap();
            (rpc.confirmed, rpc.latest) = if newer_latest_high {
                (2_000, 0)
            } else {
                (0, 2_000)
            };
            rpc.block_latest_balance = true;
        }
        let retirement = {
            let pool = f.pool.clone();
            let wallet = f.wallet.clone();
            tokio::spawn(async move { pool.retire_wallet(wallet).await })
        };
        // The response owns the old exact amount before another canonical caller
        // publishes a newer coherent pair. Scalar demand is deliberately absent.
        f.rpc.started.notified().await;
        {
            let mut rpc = f.rpc.state.lock().unwrap();
            rpc.block_latest_balance = false;
            rpc.head = 222;
            (rpc.confirmed, rpc.latest) = if newer_latest_high {
                (0, 2_000)
            } else {
                (2_000, 0)
            };
        }
        let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
        let query = f
            .store
            .call(move |s| s.scoped_chain_query(&group, vec![wallet], vec![]))
            .await
            .unwrap();
        let newer = f.pool.base.view(query).await.unwrap();
        let wallet = f.wallet.clone();
        f.store
            .call(move |s| s.record_verified_credit(&wallet, newer).map(|_| ()))
            .await
            .unwrap();
        f.rpc.release.add_permits(1);
        assert_eq!(retirement.await.unwrap().unwrap(), newer_latest_high);
        assert_eq!(f.role(), "RETAINED");
        let wallet = f.wallet.clone();
        assert_eq!(
            f.store
                .call(move |s| Ok(s.retirement_requested(&wallet)))
                .await
                .unwrap(),
            !newer_latest_high
        );

        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(61)).await;
        let group = f.pool.pool.clone();
        f.store
            .call(move |s| s.observation_page(&group, -1).map(|_| ()))
            .await
            .unwrap();
        if newer_latest_high {
            let calls = f.counts();
            assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
            assert_eq!(
                f.counts(),
                calls,
                "newer latest-high evidence completed demand"
            );
        } else {
            {
                let mut rpc = f.rpc.state.lock().unwrap();
                rpc.head = 244;
                rpc.confirmed = 0;
                rpc.latest = 0;
            }
            assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
            assert_eq!(
                f.role(),
                "RETIRED",
                "confirmation progress must remain owned"
            );
        }
        f.close().await;
        tokio::time::resume();
    }
}

#[tokio::test]
async fn malformed_stored_capture_stops_its_request_while_healthy_sibling_retires() {
    for corruption in ["address='private-invalid-wallet-address'"] {
        let f = Fixture::new(1_000, true, true, 0).await;
        let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
        f.store
            .call(move |s| s.finish_balance(&group, &wallet, Some(U256::ZERO)))
            .await
            .unwrap();
        f.db()
            .execute(
                &format!("UPDATE wallets SET {corruption} WHERE id=?1"),
                [&f.wallet],
            )
            .unwrap();
        add_retained_sibling(&f, "healthy-sibling", 90, 0);
        assert!(f.pool.reconcile().await.is_err());
        assert_eq!(f.role(), "RETAINED");
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT role FROM wallets WHERE id='healthy-sibling'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "RETIRED"
        );
        let wallet = f.wallet.clone();
        assert!(
            !f.store
                .call(move |s| Ok(s.retirement_requested(&wallet)))
                .await
                .unwrap()
        );
        let calls = f.counts();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(600)).await;
        let group = f.pool.pool.clone();
        f.store
            .call(move |s| s.observation_page(&group, -1).map(|_| ()))
            .await
            .unwrap();
        assert!(f.pool.retire_wallet(f.wallet.clone()).await.is_err());
        assert_eq!(
            f.counts(),
            calls,
            "local corruption must not request repeated proofs"
        );
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT COUNT(*) FROM wallet_retirements WHERE wallet_id=?1",
                    [&f.wallet],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        f.close().await;
        tokio::time::resume();
    }
}

#[tokio::test]
async fn stopped_request_stays_failed_without_proofs_until_new_demand_or_ineligibility() {
    let f = Fixture::new(1_000, true, true, 1).await;
    f.db()
        .execute(
            "UPDATE payment_attempts SET nonce='private-invalid-nonce'",
            [],
        )
        .unwrap();
    add_retained_sibling(&f, "healthy-sibling", 90, 0);
    assert!(f.pool.reconcile().await.is_err());
    let calls = f.counts();
    assert!(
        f.pool.reconcile().await.is_err(),
        "a stopped request must not become successful on a second finite pass"
    );
    assert_eq!(
        f.counts(),
        calls,
        "cooldown reuse must not renew proof demand"
    );
    assert_eq!(
        f.db()
            .query_row(
                "SELECT role FROM wallets WHERE id='healthy-sibling'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "RETIRED"
    );

    // Current ACTIVE ownership makes the historical retirement stop irrelevant.
    f.db()
        .execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1", [&f.wallet])
        .unwrap();
    assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert_eq!(f.counts(), calls);
    // A relevant role transition legitimately requests work again. The original
    // authorization is repaired in place and is resolved, never replayed.
    f.db()
        .execute(
            "UPDATE wallets SET role='RETAINED' WHERE id=?1",
            [&f.wallet],
        )
        .unwrap();
    f.db()
        .execute(
            "UPDATE payment_attempts SET nonce=?1",
            [format!("0x{:064x}", 1)],
        )
        .unwrap();
    f.rpc.state.lock().unwrap().used.insert(1);
    assert!(
        !f.pool.retire_wallet(f.wallet.clone()).await.unwrap(),
        "role changes preserve the existing completion floor"
    );
    assert_eq!(f.counts(), calls);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert_eq!(f.role(), "RETIRED");
    assert_eq!(
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM payment_attempts WHERE state='RESOLVED'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    f.close().await;
}

#[tokio::test]
async fn fresh_scalar_demand_retries_an_operator_stop_on_its_existing_schedule() {
    let f = Fixture::new(1_000, true, true, 1).await;
    f.db()
        .execute(
            "UPDATE payment_attempts SET nonce='private-invalid-nonce'",
            [],
        )
        .unwrap();
    assert!(f.pool.reconcile().await.is_err());
    let calls = f.counts();
    f.db()
        .execute(
            "UPDATE payment_attempts SET nonce=?1",
            [format!("0x{:064x}", 1)],
        )
        .unwrap();
    f.rpc.state.lock().unwrap().used.insert(1);
    let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
    f.store
        .call(move |s| s.finish_balance(&group, &wallet, Some(U256::ZERO)))
        .await
        .unwrap();
    assert!(!f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert_eq!(
        f.counts(),
        calls,
        "fresh demand cannot bypass the proof floor"
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert_eq!(f.role(), "RETIRED");
    f.close().await;
}

#[tokio::test]
async fn transient_publication_lock_retains_original_authorization_and_retry() {
    let f = Fixture::new(1_000, true, true, 1).await;
    let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
    f.store
        .call(move |s| s.finish_balance(&group, &wallet, Some(U256::ZERO)))
        .await
        .unwrap();
    {
        let mut rpc = f.rpc.state.lock().unwrap();
        rpc.used.insert(1);
        rpc.block_nonce = true;
    }
    let retirement = {
        let pool = f.pool.clone();
        let wallet = f.wallet.clone();
        tokio::spawn(async move { pool.retire_wallet(wallet).await })
    };
    f.rpc.started.notified().await;
    let db = f.db();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    f.rpc.state.lock().unwrap().block_nonce = false;
    f.rpc.release.add_permits(1);
    let error = retirement.await.unwrap().unwrap_err();
    assert!(crate::rotation::store::inventory::transient_read(&error));
    assert!(!super::super::operator_required(&error));
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(f.role(), "RETAINED");
    assert_eq!(
        db.query_row("SELECT state FROM payment_attempts", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "POSSIBLY_SUBMITTED"
    );
    proof_due(&f).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    assert!(f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert_eq!(f.role(), "RETIRED");
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    f.close().await;
}

#[tokio::test]
async fn proof_failure_backoff_survives_scalar_success_and_independent_wallet_progress() {
    let f = Fixture::new(1_000, true, true, 0).await;
    f.store
        .observe_balance(f.pool.pool.clone(), f.wallet.clone(), f.pool.base.clone())
        .await
        .unwrap();
    f.rpc.state.lock().unwrap().fail = true;
    assert!(f.pool.retire_wallet(f.wallet.clone()).await.is_err());
    let first_due = proof_due(&f).await;
    assert!(first_due >= tokio::time::Instant::now() + Duration::from_secs(59));
    let calls = f.rpc.state.lock().unwrap().total_calls;
    assert_eq!(
        f.store
            .observe_balance(f.pool.pool.clone(), f.wallet.clone(), f.pool.base.clone())
            .await
            .unwrap(),
        U256::ZERO,
        "a proof failure cannot poison the successful scalar outcome"
    );
    assert_eq!(f.rpc.state.lock().unwrap().total_calls, calls);

    f.rpc.state.lock().unwrap().fail = false;
    add_retained_sibling(&f, "healthy-sibling", 90, 0);
    assert!(
        f.pool
            .retire_wallet("healthy-sibling".into())
            .await
            .unwrap()
    );
    let role: String = f
        .db()
        .query_row(
            "SELECT role FROM wallets WHERE id='healthy-sibling'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(role, "RETIRED", "a failed wallet cannot block its sibling");
    add_retained_sibling(&f, "above-tolerance", 91, 2_000);
    let calls = f.rpc.state.lock().unwrap().total_calls;
    for _ in 0..3 {
        let group = f.pool.pool.clone();
        f.store
            .call(move |s| {
                s.observation_page(&group, -1)?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            f.pool
                .retire_wallet("above-tolerance".into())
                .await
                .unwrap()
        );
        assert!(!f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    }
    assert_eq!(
        f.rpc.state.lock().unwrap().total_calls,
        calls,
        "capacity wakes and above-tolerance siblings issue no collateral proofs"
    );
    assert_eq!(proof_due(&f).await, first_due);

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    f.rpc.state.lock().unwrap().fail = true;
    assert!(f.pool.retire_wallet(f.wallet.clone()).await.is_err());
    let second_due = proof_due(&f).await;
    assert!(
        second_due >= tokio::time::Instant::now() + Duration::from_secs(119),
        "the second consecutive proof failure must retain its 120-second backoff"
    );
    f.rpc.state.lock().unwrap().fail = false;
    let scalar_before = f.counts().0;
    assert_eq!(
        f.store
            .observe_balance(f.pool.pool.clone(), f.wallet.clone(), f.pool.base.clone())
            .await
            .unwrap(),
        U256::ZERO,
    );
    assert_eq!(
        f.counts().0,
        scalar_before + 1,
        "exercise a fresh scalar success"
    );
    assert_eq!(
        proof_due(&f).await,
        second_due,
        "fresh scalar demand cannot reset the proof failure streak or due time"
    );
    tokio::time::advance(Duration::from_secs(60)).await;
    let calls = f.rpc.state.lock().unwrap().total_calls;
    assert!(!f.pool.retire_wallet(f.wallet.clone()).await.unwrap());
    assert_eq!(f.rpc.state.lock().unwrap().total_calls, calls);
    tokio::time::advance(Duration::from_secs(61)).await;
    f.rpc.state.lock().unwrap().fail = true;
    assert!(f.pool.retire_wallet(f.wallet.clone()).await.is_err());
    assert!(
        proof_due(&f).await >= tokio::time::Instant::now() + Duration::from_secs(239),
        "scalar and sibling successes cannot reset the third proof failure to 60 seconds"
    );
    f.close().await;
}

#[tokio::test]
async fn replacing_maintenance_owner_joins_detached_authorization_work() {
    let mut f = Fixture::new(1_000, true, true, 1).await;
    {
        let mut rpc = f.rpc.state.lock().unwrap();
        rpc.used.insert(1);
        rpc.block_nonce = true;
    }
    let entered = Arc::new(Notify::new());
    let notify = entered.clone();
    Arc::get_mut(&mut f.pool).unwrap().reconciliation_test_hook =
        Some(Arc::new(move || notify.notify_one()));
    let first_stop = tokio_util::sync::CancellationToken::new();
    let first = {
        let pool = f.pool.clone();
        let stop = first_stop.clone();
        tokio::spawn(async move { pool.run_reconciliation(stop).await })
    };
    entered.notified().await;
    f.rpc.started.notified().await;
    first_stop.cancel();
    first.await.unwrap().unwrap();
    assert_eq!(f.counts().2, 1);
    let second_stop = tokio_util::sync::CancellationToken::new();
    let second = {
        let pool = f.pool.clone();
        let stop = second_stop.clone();
        tokio::spawn(async move { pool.run_reconciliation(stop).await })
    };
    entered.notified().await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        f.counts().2,
        1,
        "a new maintenance owner must share the still-active wallet acquisition"
    );
    f.rpc.state.lock().unwrap().block_nonce = false;
    f.rpc.release.add_permits(1);
    while f.role() != "RETIRED" {
        tokio::task::yield_now().await;
    }
    second_stop.cancel();
    second.await.unwrap().unwrap();
    assert_eq!(
        f.counts().2,
        1,
        "the original nonce proof must publish exactly once"
    );
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM payment_resolutions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    f.close().await;
}

#[tokio::test]
async fn malformed_derived_checkpoints_rebuild_without_stopping_retirement() {
    for corruption in [
        "financial_height=1,financial_hash=NULL",
        "financial_height=NULL,financial_hash='private-invalid-checkpoint'",
        "financial_height=-1,financial_hash='0x0000000000000000000000000000000000000000000000000000000000000001'",
        "financial_height=1,financial_hash='private-invalid-checkpoint'",
        "debit_height=999,debit_hash='0x0000000000000000000000000000000000000000000000000000000000000001'",
        "evidence_repair='chain_repair_required'",
    ] {
        let f = Fixture::new(1_000, true, true, 0).await;
        let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
        f.store
            .call(move |s| s.finish_balance(&group, &wallet, Some(U256::ZERO)))
            .await
            .unwrap();
        f.db()
            .execute(
                &format!("UPDATE wallets SET {corruption} WHERE id=?1"),
                [&f.wallet],
            )
            .unwrap();
        assert!(f.pool.reconcile().await.is_ok(), "{corruption}");
        assert_eq!(f.role(), "RETIRED", "{corruption}");
        f.close().await;
    }
}

#[tokio::test]
async fn completed_never_accepted_destination_retires_without_fabricating_initial_credit() {
    let f = Fixture::new(1_000, false, true, 0).await;
    f.db()
        .execute(
            "UPDATE funding_jobs SET state='COMPLETE' WHERE wallet_id=?1",
            [&f.wallet],
        )
        .unwrap();
    f.db()
        .execute(
            "UPDATE wallets SET role='RETAINED' WHERE id=?1",
            [&f.wallet],
        )
        .unwrap();
    let (group, wallet) = (f.pool.pool.clone(), f.wallet.clone());
    f.store
        .call(move |s| s.finish_balance(&group, &wallet, Some(U256::ZERO)))
        .await
        .unwrap();
    assert!(f.pool.reconcile().await.is_ok());
    assert_eq!(f.role(), "RETIRED");
    let (credited,jobs,attempts):(bool,i64,i64)=f.db().query_row("SELECT readiness_requirement IS NOT NULL,(SELECT COUNT(*) FROM funding_jobs),(SELECT COUNT(*) FROM payment_attempts) FROM funding_jobs WHERE wallet_id=?1",[&f.wallet],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(!credited);
    assert_eq!((jobs, attempts), (1, 0));
    f.close().await;
}
