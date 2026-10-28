//! Continue past invalidation: original liabilities, crash safety and new proofs.
use super::*;
use std::sync::atomic::Ordering;

fn dispositions(f: &Fixture) -> (i64, i64, String, String) {
    let (current, historical, state, wallet): (i64, i64, String, String) = f.store.db.query_row("SELECT (SELECT COUNT(*) FROM payment_resolutions),(SELECT COUNT(*) FROM payment_resolution_history),state,wallet_id FROM payment_attempts WHERE id='proof'", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    (
        current,
        historical,
        state,
        exposure(&f.store.db, &wallet).unwrap().to_string(),
    )
}
fn wallet_query(f: &Fixture) -> ChainQuery {
    f.store
        .scoped_chain_query(&f.pool, vec![f.wallets[0].clone()], vec![])
        .unwrap()
}

async fn resolve(f: &mut Fixture, auth: PendingAuthorization) {
    let query = f
        .store
        .scoped_chain_query(&f.pool, vec![], vec![auth])
        .unwrap();
    let view = f.rpc.view(query).await.unwrap();
    f.store.publish_scoped_evidence(&f.pool, view).unwrap();
}

#[tokio::test]
async fn replaced_resolution_restores_original_liability_and_finishes_after_restart() {
    for was_used in [true, false] {
        let mut f = Fixture::new().await;
        let auth = f.authorization();
        if !was_used {
            f.nonce.store(0, Ordering::SeqCst);
            f.stamp.store(auth.valid_before + 1, Ordering::SeqCst);
        }
        resolve(&mut f, auth.clone()).await;
        let original: (String, String) = f
            .store
            .db
            .query_row(
                "SELECT outcome,support FROM payment_resolutions WHERE attempt_id='proof'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let stale = f.rpc.view(f.query()).await.unwrap();
        f.store.prepare_evidence_view(stale.clone()).unwrap();
        f.chain.lock().unwrap().2 = Some(88);
        f.nonce.store(0, Ordering::SeqCst);
        f.stamp.store(auth.valid_before - 1, Ordering::SeqCst);
        let failure = f.rpc.view(wallet_query(&f)).await.err().unwrap();
        f.store.record_base_failure(&failure).unwrap();
        assert_eq!(
            dispositions(&f),
            (0, 1, "POSSIBLY_SUBMITTED".into(), "7".into())
        );
        let archived: (String, String) = f
            .store
            .db
            .query_row(
                "SELECT outcome,support FROM payment_resolution_history WHERE attempt_id='proof'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(archived, original, "preserve exact original proof bytes");
        assert_eq!(
            f.store
                .prepare_evidence_view(stale)
                .err()
                .unwrap()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::Invalidated)
        );
        // Release only this temporary fixture's owner and reopen the committed
        // state, just as a process restart between recovery pages would do.
        let id = f.store.id().to_owned();
        let scratch = tempfile::tempdir().unwrap();
        f.store = Store::create(
            &scratch.path().join("state"),
            &scratch.path().join("key"),
            1,
            b"scratch",
        )
        .unwrap();
        f.store = Store::open(
            &f._directory.path().join("state"),
            &f._directory.path().join("key"),
            &id,
        )
        .unwrap();
        resolve(&mut f, auth.clone()).await;
        assert_eq!(
            dispositions(&f),
            (0, 1, "POSSIBLY_SUBMITTED".into(), "7".into()),
            "replacement chain time cannot inherit orphaned expiry"
        );
        f.chain.lock().unwrap().0 = 101;
        if was_used {
            f.stamp.store(auth.valid_before + 1, Ordering::SeqCst);
        } else {
            f.nonce.store(1, Ordering::SeqCst);
        }
        resolve(&mut f, auth).await;
        assert_eq!(dispositions(&f).0, 1);
        assert_eq!(exposure(&f.store.db, &f.wallets[0]).unwrap(), U256::ZERO);
        let outcome: String = f
            .store
            .db
            .query_row("SELECT outcome FROM payment_resolutions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outcome, if was_used { "EXPIRED_UNUSED" } else { "USED" });
        f.store.record_base_failure(&failure).unwrap();
        assert_eq!(
            dispositions(&f).0,
            1,
            "late old acquisition failure cannot reopen a repaired disposition"
        );
        assert_eq!(dispositions(&f).1, 1);
        assert_eq!(
            f.store
                .db
                .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1,
            "reconciliation never creates another authorization"
        );
    }
}

#[tokio::test]
async fn failed_atomic_reopening_keeps_original_proof_and_retries_without_partial_history() {
    let mut f = Fixture::new().await;
    let auth = f.authorization();
    resolve(&mut f, auth).await;
    f.chain.lock().unwrap().2 = Some(88);
    let failure = f.rpc.view(wallet_query(&f)).await.err().unwrap();
    f.store.db.execute_batch("CREATE TRIGGER reject_projection_reset BEFORE UPDATE OF financial_height ON wallets BEGIN SELECT RAISE(ABORT,'synthetic failed commit'); END;").unwrap();
    assert!(f.store.record_base_failure(&failure).is_err());
    assert_eq!(dispositions(&f).0, 1);
    assert_eq!(dispositions(&f).1, 0);
    assert_eq!(dispositions(&f).2, "RESOLVED");
    assert_eq!(generation(&f.store.db, "wallet", &f.wallets[0]).unwrap(), 0);
    f.store
        .db
        .execute_batch("DROP TRIGGER reject_projection_reset;")
        .unwrap();
    f.store.record_base_failure(&failure).unwrap();
    assert_eq!(
        dispositions(&f),
        (0, 1, "POSSIBLY_SUBMITTED".into(), "7".into())
    );
    f.store.record_base_failure(&failure).unwrap();
    assert_eq!(
        dispositions(&f).1,
        1,
        "repeated invalidation preserves one original archived proof"
    );
}

#[tokio::test]
async fn rebuilding_legacy_projection_ignores_invalidated_disposable_cache() {
    let mut f = Fixture::new().await;
    let saved = f.rpc.view(f.query()).await.unwrap();
    f.store.prepare_evidence_view(saved.clone()).unwrap();
    let wallet = f.wallets[0].clone();
    f.store.db.execute("UPDATE wallets SET financial_height=1,financial_hash=NULL,evidence_repair='chain_repair_required' WHERE id=?1",[&wallet]).unwrap();
    f.chain.lock().unwrap().2 = Some(88);
    let query = wallet_query(&f);
    assert!(query.captures[&wallet].previous.is_none());
    let view = f.rpc.view(query).await.unwrap();
    f.store.prepare_evidence_view(view).unwrap();
    assert_eq!(
        generation(&f.store.db, "wallet", &wallet).unwrap(),
        1,
        "invalid cache must not invalidate the repaired capture again"
    );
    assert_eq!(generation(&f.store.db, "wallet", &f.wallets[1]).unwrap(), 0);
}

#[tokio::test]
async fn retired_authorizations_reopen_and_finish_in_existing_pages_across_restart() {
    let mut f = Fixture::new().await;
    let auth = f.authorization();
    for sequence in 2..=66 {
        f.store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) VALUES(?1,?2,?3,0,'7','fixture','POSSIBLY_SUBMITTED',?4,?5,?6,?7)",params![format!("proof-{sequence}"),f.pool,auth.wallet,auth.payer,format!("0x{sequence:064x}"),i64::try_from(auth.valid_before).unwrap(),sequence]).unwrap();
    }
    let mut after = 0;
    while after < 66 {
        let page = f
            .store
            .retirement_pending_page(&f.pool, &auth.wallet, after, 66)
            .unwrap();
        after = page.last().unwrap().0;
        let query = f
            .store
            .scoped_chain_query(&f.pool, vec![], page.into_iter().map(|(_, a)| a).collect())
            .unwrap();
        f.store
            .publish_scoped_evidence(&f.pool, f.rpc.view(query).await.unwrap())
            .unwrap();
    }
    f.store
        .db
        .execute(
            "UPDATE wallets SET role='RETIRED' WHERE id=?1",
            [&auth.wallet],
        )
        .unwrap();
    f.chain.lock().unwrap().2 = Some(88);
    let failure = f.rpc.view(wallet_query(&f)).await.err().unwrap();
    f.store.record_base_failure(&failure).unwrap();
    assert_eq!(
        exposure(&f.store.db, &auth.wallet).unwrap(),
        U256::from(66 * 7)
    );
    f.nonce.store(0, Ordering::SeqCst);
    f.stamp.store(auth.valid_before + 1, Ordering::SeqCst);
    let page = f
        .store
        .retirement_pending_page(&f.pool, &auth.wallet, 0, 66)
        .unwrap();
    assert_eq!(page.len(), 64);
    let after = page.last().unwrap().0;
    let query = f
        .store
        .scoped_chain_query(&f.pool, vec![], page.into_iter().map(|(_, a)| a).collect())
        .unwrap();
    f.store
        .publish_scoped_evidence(&f.pool, f.rpc.view(query).await.unwrap())
        .unwrap();
    let id = f.store.id().to_owned();
    let scratch = tempfile::tempdir().unwrap();
    f.store = Store::create(
        &scratch.path().join("state"),
        &scratch.path().join("key"),
        1,
        b"scratch",
    )
    .unwrap();
    f.store = Store::open(
        &f._directory.path().join("state"),
        &f._directory.path().join("key"),
        &id,
    )
    .unwrap();
    let page = f
        .store
        .retirement_pending_page(&f.pool, &auth.wallet, after, 66)
        .unwrap();
    assert_eq!(page.len(), 2);
    let query = f
        .store
        .scoped_chain_query(&f.pool, vec![], page.into_iter().map(|(_, a)| a).collect())
        .unwrap();
    f.store
        .publish_scoped_evidence(&f.pool, f.rpc.view(query).await.unwrap())
        .unwrap();
    let (role,pending,resolved,history,attempts):(String,i64,i64,i64,i64)=f.store.db.query_row("SELECT role,(SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'),(SELECT COUNT(*) FROM payment_resolutions),(SELECT COUNT(*) FROM payment_resolution_history),(SELECT COUNT(*) FROM payment_attempts) FROM wallets WHERE id=?1",[&auth.wallet],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(
        (role.as_str(), pending, resolved, history, attempts),
        ("RETIRED", 0, 66, 66, 66)
    );
    assert!(
        f.store
            .advisory_candidate(&f.pool, U256::from(1), Some(&auth.wallet), i64::MAX, &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.store.funding_jobs().unwrap().len(),
        2,
        "recovery cannot allocate or top up a wallet"
    );
}

#[tokio::test]
async fn encountered_malformed_resolutions_reopen_and_finish_at_the_same_anchor() {
    for corruption in [
        "UPDATE payment_resolutions SET height='not-a-height'",
        "UPDATE payment_resolutions SET height=x'3838'",
        "UPDATE payment_resolutions SET height=-1",
        "UPDATE payment_resolutions SET hash=x'626164'",
        "UPDATE payment_resolutions SET hash='invalid-hash'",
        "UPDATE payment_resolutions SET block_time='not-a-time'",
        "UPDATE payment_resolutions SET block_time=x'313030'",
        "UPDATE payment_resolutions SET block_time=-1",
        "UPDATE payment_resolutions SET outcome='EXPIRED_UNUSED',block_time=1",
        "UPDATE payment_resolutions SET height='not-a-height'; UPDATE wallets SET financial_height=1,financial_hash=NULL WHERE id=(SELECT wallet_id FROM payment_attempts WHERE id='proof')",
    ] {
        let mut f = Fixture::new().await;
        let auth = f.authorization();
        // Two real pending acquisitions can overlap. Keep the first proof while
        // the second resolves the original, then encounter its damaged row.
        let query = f
            .store
            .scoped_chain_query(&f.pool, vec![], vec![auth.clone()])
            .unwrap();
        assert_eq!(query.pending.len(), 1);
        let view = f.rpc.view(query).await.unwrap();
        assert!(view.resolution_certificates.contains_key("proof"));
        resolve(&mut f, auth.clone()).await;
        let original: (String, String, String, i64) = f
            .store
            .db
            .query_row(
                "SELECT payer,nonce,amount,valid_before FROM payment_attempts WHERE id='proof'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        let original_checkpoint = checkpoint(&f.store.db, &auth.wallet).unwrap().unwrap();
        f.store.db.execute_batch(corruption).unwrap();
        let damaged: Option<Vec<rusqlite::types::Value>> = f.store.db.query_row(
            "SELECT outcome,height,hash,block_time,support FROM payment_resolutions WHERE attempt_id='proof'",
            [], |r| (0..5).map(|i| r.get(i)).collect(),
        ).optional().unwrap();
        if corruption.contains("financial_height") {
            // Existing checkpoint repair must not move the broken tuple into
            // an archive that vetoes the next proof at the same block.
            let query = wallet_query(&f);
            assert!(query.captures[&auth.wallet].checkpoint.is_none());
        } else {
            assert_eq!(
                f.store
                    .publish_scoped_evidence(&f.pool, view)
                    .err()
                    .unwrap()
                    .downcast_ref::<EvidenceFailure>(),
                Some(&EvidenceFailure::InvalidTuple)
            );
        }
        assert_eq!(
            dispositions(&f),
            (
                0,
                i64::from(damaged.is_some()),
                "POSSIBLY_SUBMITTED".into(),
                "7".into()
            )
        );
        assert_eq!(generation(&f.store.db, "wallet", &auth.wallet).unwrap(), 1);
        if let Some(damaged) = damaged {
            let archived: Vec<rusqlite::types::Value> = f.store.db.query_row(
                "SELECT outcome,height,hash,block_time,support FROM payment_resolution_history WHERE attempt_id='proof'",
                [], |r| (0..5).map(|i| r.get(i)).collect(),
            ).unwrap();
            assert_eq!(archived, damaged, "retain exact damaged audit fields");
        }
        resolve(&mut f, auth.clone()).await;
        assert_eq!(dispositions(&f).0, 1, "{corruption}");
        assert_eq!(dispositions(&f).2, "RESOLVED");
        assert_eq!(exposure(&f.store.db, &auth.wallet).unwrap(), U256::ZERO);
        assert_eq!(
            checkpoint(&f.store.db, &auth.wallet).unwrap().unwrap(),
            original_checkpoint
        );
        let retained: (String, String, String, i64) = f
            .store
            .db
            .query_row(
                "SELECT payer,nonce,amount,valid_before FROM payment_attempts WHERE id='proof'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(retained, original);
        assert_eq!(
            f.store
                .db
                .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(f.store.funding_jobs().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn archived_resolution_cannot_veto_fresh_canonical_original_resolution() {
    for conflict_first in [false, true] {
        let mut f = Fixture::new().await;
        let auth = f.authorization();
        let hash = format!("0x{:064x}", 88);
        let insert_damaged = |db: &Connection| {
            db.execute("INSERT INTO payment_resolution_history VALUES ('proof','UNKNOWN',x'3838',?1,'not-a-time',x'6578616374')", [&hash]).unwrap();
        };
        if !conflict_first {
            insert_damaged(&f.store.db);
        }
        f.store.db.execute("INSERT INTO payment_resolution_history VALUES ('proof','EXPIRED_UNUSED',88,?1,?2,NULL)", params![hash,i64::try_from(auth.valid_before+1).unwrap()]).unwrap();
        if conflict_first {
            insert_damaged(&f.store.db);
        }
        let query = f
            .store
            .scoped_chain_query(&f.pool, vec![], vec![auth.clone()])
            .unwrap();
        assert_eq!(query.pending.len(), 1);
        let view = f.rpc.view(query).await.unwrap();
        assert!(view.resolution_certificates.contains_key("proof"));
        f.store.publish_scoped_evidence(&f.pool, view).unwrap();
        assert_eq!(dispositions(&f), (1, 2, "RESOLVED".into(), "0".into()));
        assert_eq!(exposure(&f.store.db, &auth.wallet).unwrap(), U256::ZERO);
        let damaged: (Vec<u8>, String, Vec<u8>) = f
            .store
            .db
            .query_row(
                "SELECT height,block_time,support FROM payment_resolution_history WHERE outcome='UNKNOWN'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            damaged,
            (b"88".to_vec(), "not-a-time".into(), b"exact".to_vec())
        );
        assert_eq!(
            f.store
                .db
                .query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn malformed_original_expiry_is_not_repaired_as_a_derived_resolution() {
    let mut f = Fixture::new().await;
    let auth = f.authorization();
    let query = f
        .store
        .scoped_chain_query(&f.pool, vec![], vec![auth.clone()])
        .unwrap();
    assert_eq!(query.pending.len(), 1);
    let view = f.rpc.view(query).await.unwrap();
    assert!(view.resolution_certificates.contains_key("proof"));
    f.store
        .db
        .execute(
            "UPDATE payment_attempts SET valid_before=x'313030' WHERE id='proof'",
            [],
        )
        .unwrap();
    let error = f
        .store
        .publish_scoped_evidence(&f.pool, view)
        .err()
        .unwrap();
    assert!(matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::InvalidColumnType(..))
    ));
    assert_eq!(exposure(&f.store.db, &auth.wallet).unwrap(), U256::from(7));
    assert_eq!(
        dispositions(&f),
        (0, 0, "POSSIBLY_SUBMITTED".into(), "7".into())
    );
    assert_eq!(generation(&f.store.db, "wallet", &auth.wallet).unwrap(), 0);
}
