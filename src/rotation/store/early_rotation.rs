//! Advisory early rotation at ordinary refill opportunities. No network work.
use super::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;

const DOMAIN: &[u8] = b"x402_treazury/early-rotation/v1";

fn random_block(key: &[u8; 32], ids: [&str; 3], counter: u64) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    mac.update(DOMAIN);
    for id in ids {
        mac.update(&(id.len() as u64).to_be_bytes());
        mac.update(id.as_bytes());
    }
    mac.update(&counter.to_be_bytes());
    mac.finalize().into_bytes().into()
}

// Discard the incomplete residue bucket at the bottom of the 256-bit range.
fn uniform_offset(sample: U256, width: U256) -> Option<U256> {
    let reject_below = (U256::MAX % width + U256::from(1)) % width;
    (sample >= reject_below).then(|| sample % width)
}

fn threshold(key: &[u8; 32], ids: [&str; 3], credit: U256, cap: U256) -> Result<U256> {
    let upper = cap.min(credit / U256::from(4));
    let lower = upper / U256::from(5);
    if lower == upper {
        return Ok(lower);
    }
    let width = upper - lower + U256::from(1);
    let mut counter = 0u64;
    loop {
        let sample = U256::from_be_bytes(random_block(key, ids, counter));
        if let Some(offset) = uniform_offset(sample, width) {
            return Ok(lower + offset);
        }
        counter = counter
            .checked_add(1)
            .context("threshold stream exhausted")?;
    }
}

impl Store {
    /// One current ACTIVE transition only; its replacement must first be used
    /// before it can qualify at a later opportunity. Estimates grant no authority.
    pub(crate) fn promote_low_active(&mut self, name: &str) -> Result<bool> {
        if self.funding_restriction != super::super::restriction::FundingRestriction::Ordinary {
            return Ok(false);
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        type Active = (String, String, Option<String>, Option<String>, String);
        let active: Option<Active> = tx.query_row(
            "SELECT p.id,w.id,w.balance,f.readiness_requirement,p.payment_limit FROM pools p JOIN wallets w ON w.pool_id=p.id LEFT JOIN funding_jobs f ON f.wallet_id=w.id WHERE p.name=?1 AND p.enabled=1 AND p.payment_limit IS NOT NULL AND w.role='ACTIVE' AND EXISTS(SELECT 1 FROM payment_attempts a WHERE a.wallet_id=w.id AND a.pool_id=p.id)",
            [name], |r| Ok((r.get(0)?,r.get(1)?,r.get_ref(2)?.as_str().ok().map(str::to_owned),r.get_ref(3)?.as_str().ok().map(str::to_owned),r.get(4)?))
        ).optional()?;
        let Some((pool, wallet, balance, credit, cap)) = active else {
            return Ok(false);
        };
        let (Some(balance), Some(credit)) = (
            balance.as_deref().and_then(|v| amount(v).ok()),
            credit
                .as_deref()
                .and_then(|v| amount(v).ok())
                .filter(|v| *v > U256::ZERO),
        ) else {
            return Ok(false);
        };
        if balance > threshold(&self.key, [&self.id, &pool, &wallet], credit, amount(&cap)?)? {
            return Ok(false);
        }
        // payment_ownership verifies the enabled exact claim/generation. The
        // transaction prevents another admission from consuming it concurrently.
        let mut query = tx.prepare("SELECT w.id,w.address FROM reserve_claims c JOIN wallets w ON w.id=c.wallet_id WHERE c.pool_id=?1 AND c.enabled=1 AND w.role='READY' ORDER BY CASE c.slot WHEN 'small' THEN 0 ELSE 1 END")?;
        let candidates = query
            .query_map([&pool], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get_ref(1)?.as_str().ok().map(str::to_owned),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(query);
        for (ready, address) in candidates {
            if address
                .as_deref()
                .is_none_or(|value| value.parse::<alloy_primitives::Address>().is_err())
            {
                tracing::warn!(
                    wallet = name,
                    category = "malformed_advisory_wallet_address",
                    "A reserve identity is unreadable; early rotation will try another funded reserve"
                );
                continue;
            }
            if let Some(ownership) = capacity::payment_ownership(&tx, &pool, &ready)? {
                capacity::promote_ready(&tx, &pool, &ready, ownership)?;
                tx.commit()?;
                self.notify_capacity_changed(&pool);
                tracing::info!(
                    wallet = name,
                    "A funded reserve became active; the previous payer remains available for affordable calls"
                );
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_threshold_vectors_bounds_and_rejection() {
        let ids = ["treasury", "pool", "wallet"];
        let block = random_block(&[7; 32], ids, 0);
        // Independently generated with Python's standard hmac/hashlib.
        assert_eq!(
            alloy_primitives::hex::encode(block),
            "71076212e41c4b6a0c0cb4a1f030ca740abfcd97d97853b804a9ed77cb896527"
        );
        let draw = threshold(&[7; 32], ids, U256::from(4_000_000), U256::from(1_000_000)).unwrap();
        assert_eq!(draw, U256::from(908562));
        assert_eq!(
            draw,
            threshold(&[7; 32], ids, U256::from(25_000_000), U256::from(1_000_000)).unwrap()
        );
        assert_ne!(block, random_block(&[8; 32], ids, 0));
        assert_ne!(
            block,
            random_block(&[7; 32], ["treasury", "po", "olwallet"], 0)
        );
        assert_ne!(block, random_block(&[7; 32], ids, 1));
        for credit in [
            U256::ZERO,
            U256::from(1),
            U256::from(4),
            U256::from(25_000_000),
            U256::MAX,
        ] {
            for cap in [U256::ZERO, U256::from(1), U256::from(1_000_000), U256::MAX] {
                let upper = cap.min(credit / U256::from(4));
                let t = threshold(&[7; 32], ids, credit, cap).unwrap();
                assert!(t >= upper / U256::from(5) && t <= upper);
            }
        }
        let width = U256::from(10);
        for sample in 0..6 {
            assert_eq!(uniform_offset(U256::from(sample), width), None);
        }
        for sample in 6..26 {
            assert_eq!(
                uniform_offset(U256::from(sample), width),
                Some(U256::from(sample % 10))
            );
        }
    }

    fn funded(store: &mut Store, job: &funding::FundingJob, balance: u64) {
        store
            .record_funding_balance(
                &job.id,
                &job.operation_id,
                &job.wallet_id,
                &job.recipient,
                U256::from(balance),
            )
            .unwrap();
    }

    #[test]
    fn promotion_uses_saved_credit_and_journal_without_allocating_or_reopening_claims() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let reserves = crate::rotation::test_capacity::two_reserves("4");
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "pool",
            reserves,
            Some(U256::from(1_000_000)),
            0
        )
        .unwrap();
        let jobs = store.funding_jobs().unwrap();
        for job in &jobs {
            funded(&mut store, job, 4_000_000);
        }
        let small = jobs
            .iter()
            .find(|j| store.funding_wallet_type(&j.id).unwrap() == "small")
            .unwrap()
            .clone();
        let tx = store.db.transaction().unwrap();
        capacity::consume_reserve(&tx, &pool, &small.wallet_id).unwrap();
        tx.commit().unwrap();
        store.fill_funding_vacancies("pool").unwrap();
        let replacement = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| !jobs.iter().any(|old| old.id == j.id))
            .unwrap();
        funded(&mut store, &replacement, 4_000_000);
        let t = threshold(
            &store.key,
            [&store.id, &pool, &small.wallet_id],
            U256::from(4_000_000),
            U256::from(1_000_000),
        )
        .unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET balance=?2 WHERE id=?1",
                params![small.wallet_id, t.to_string()],
            )
            .unwrap();
        assert!(
            !store.promote_low_active("pool").unwrap(),
            "an unused ACTIVE must not churn"
        );
        store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES ('used',?1,?2,0,'1','fixture','POSSIBLY_SUBMITTED')", params![pool,small.wallet_id]).unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET balance='unknown' WHERE id=?1",
                [&small.wallet_id],
            )
            .unwrap();
        assert!(!store.promote_low_active("pool").unwrap());
        store
            .db
            .execute(
                "UPDATE wallets SET balance=?2 WHERE id=?1",
                params![small.wallet_id, (t + U256::from(1)).to_string()],
            )
            .unwrap();
        assert!(!store.promote_low_active("pool").unwrap());
        store
            .db
            .execute(
                "UPDATE wallets SET balance=?2 WHERE id=?1",
                params![small.wallet_id, t.to_string()],
            )
            .unwrap();
        let before = store.funding_jobs().unwrap().len();
        // Restrictions disable only this optional capability, not payer eligibility.
        store.pause_new_funding();
        assert!(!store.promote_low_active("pool").unwrap());
        store.funding_restriction = crate::rotation::restriction::FundingRestriction::Ordinary;
        store
            .db
            .execute(
                "UPDATE reserve_claims SET enabled=0 WHERE pool_id=?1 AND slot='large'",
                [&pool],
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET address='malformed' WHERE id=?1",
                [&replacement.wallet_id],
            )
            .unwrap();
        assert!(
            !store.promote_low_active("pool").unwrap(),
            "no usable READY means no rotation"
        );
        store
            .db
            .execute(
                "UPDATE wallets SET address=?2 WHERE id=?1",
                params![replacement.wallet_id, replacement.recipient],
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE reserve_claims SET enabled=1 WHERE pool_id=?1 AND slot='large'",
                [&pool],
            )
            .unwrap();
        // Capture/signing can precede the background transition. The real journal
        // must then accept either original identity under its current role.
        let captured = [&small, &replacement].map(|job| {
            store
                .prepare_advisory_payment(
                    &pool,
                    U256::from(1),
                    "fixture",
                    &crate::rotation::manager::PaymentCandidateHandle {
                        wallet: job.wallet_id.clone(),
                        address: job.recipient.clone(),
                    },
                )
                .unwrap()
        });
        assert!(store.promote_low_active("pool").unwrap());
        assert_eq!(
            capacity::payment_ownership(&store.db, &pool, &small.wallet_id)
                .unwrap()
                .unwrap()
                .role,
            "RETAINED"
        );
        assert_eq!(
            capacity::payment_ownership(&store.db, &pool, &replacement.wallet_id)
                .unwrap()
                .unwrap()
                .role,
            "ACTIVE",
            "small reserve precedes large"
        );
        assert!(
            !store.promote_low_active("pool").unwrap(),
            "replacement is unused"
        );
        assert_eq!(
            store.funding_jobs().unwrap().len(),
            before,
            "promotion never allocates"
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM payment_attempts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT readiness_requirement FROM funding_jobs WHERE id=?1",
                    [&small.id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "4000000"
        );
        let generation: i64 = store
            .db
            .query_row(
                "SELECT generation FROM reserve_claims WHERE pool_id=?1 AND slot='small'",
                [&pool],
                |r| r.get(0),
            )
            .unwrap();
        for (index, admission) in captured.iter().enumerate() {
            let now = crate::rotation::base::now().unwrap();
            store
                .journal_authorization(
                    admission,
                    Authorization {
                        payer: admission.address.clone(),
                        payee: "fixture".into(),
                        nonce: format!("nonce-{index}"),
                        valid_after: now,
                        valid_before: now + 300,
                    },
                    None,
                )
                .unwrap();
        }
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT generation FROM reserve_claims WHERE pool_id=?1 AND slot='small'",
                    [&pool],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            generation
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM payment_attempts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            3
        );
        let expected = threshold(
            &store.key,
            [&store.id, &pool, &small.wallet_id],
            U256::from(4_000_000),
            U256::from(1_000_000),
        )
        .unwrap();
        store.rename_capacity_group("pool", "renamed").unwrap();
        let id = store.id.clone();
        drop(store);
        let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        assert_eq!(
            expected,
            threshold(
                &store.key,
                [&store.id, &pool, &small.wallet_id],
                U256::from(4_000_000),
                U256::from(1_000_000)
            )
            .unwrap()
        );
    }

    #[tokio::test]
    async fn queued_first_use_and_early_rotation_share_one_claim_in_both_orders() {
        fn prepare(store: &Store, pool: &str, job: &funding::FundingJob) -> PreparedPayment {
            store
                .prepare_advisory_payment(
                    pool,
                    U256::from(1),
                    "race-requirements",
                    &crate::rotation::manager::PaymentCandidateHandle {
                        wallet: job.wallet_id.clone(),
                        address: job.recipient.clone(),
                    },
                )
                .unwrap()
        }
        fn authorization(payment: &PreparedPayment, nonce: &str) -> Authorization {
            let now = crate::rotation::base::now().unwrap();
            Authorization {
                payer: payment.address.clone(),
                payee: "race-payee".into(),
                nonce: nonce.into(),
                valid_after: now,
                valid_before: now + 300,
            }
        }
        fn liability(store: &Store, id: &str) -> Vec<rusqlite::types::Value> {
            store.db.query_row(
                "SELECT id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence FROM payment_attempts WHERE id=?1",
                [id], |row| (0..13).map(|i| row.get(i)).collect(),
            ).unwrap()
        }
        fn readiness(store: &Store, job: &funding::FundingJob) -> (String, String, String) {
            store.db.query_row(
                "SELECT readiness_operation,readiness_requirement,readiness_capacity FROM funding_jobs WHERE id=?1",
                [&job.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).unwrap()
        }
        for journal_first in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = Store::create(
                &dir.path().join("state"),
                &dir.path().join("key"),
                1,
                b"fixture",
            )
            .unwrap();
            let reserves = crate::rotation::test_capacity::two_reserves("4");
            let pool = crate::rotation::test_capacity::declare_and_allocate!(
                store,
                "pool",
                reserves,
                Some(U256::from(1_000_000)),
                0
            )
            .unwrap();
            let jobs = store.funding_jobs().unwrap();
            for job in &jobs {
                funded(&mut store, job, 4_000_000);
            }
            let active = jobs
                .iter()
                .find(|job| store.funding_wallet_type(&job.id).unwrap() == "small")
                .unwrap()
                .clone();
            let initial = prepare(&store, &pool, &active);
            store
                .journal_authorization(&initial, authorization(&initial, "initial-use"), None)
                .unwrap();
            let initial_liability = liability(&store, &initial.id);
            store.fill_funding_vacancies("pool").unwrap();
            let ready = store
                .funding_jobs()
                .unwrap()
                .into_iter()
                .find(|job| !jobs.iter().any(|old| old.id == job.id))
                .unwrap();
            funded(&mut store, &ready, 4_000_000);
            let threshold = threshold(
                &store.key,
                [&store.id, &pool, &active.wallet_id],
                U256::from(4_000_000),
                U256::from(1_000_000),
            )
            .unwrap();
            store
                .db
                .execute(
                    "UPDATE wallets SET balance=?2 WHERE id=?1",
                    params![active.wallet_id, threshold.to_string()],
                )
                .unwrap();
            // Capture both signing identities before either competing mutation.
            let active_payment = prepare(&store, &pool, &active);
            let ready_payment = prepare(&store, &pool, &ready);
            let ready_id = ready_payment.id.clone();
            let ready_auth = authorization(&ready_payment, "ready-first-use");
            let expected_ready = (
                ready_payment.id.clone(),
                pool.clone(),
                ready.wallet_id.clone(),
                "1".to_owned(),
                "race-requirements".to_owned(),
                "POSSIBLY_SUBMITTED".to_owned(),
                ready.recipient.clone(),
                ready_auth.payee.clone(),
                ready_auth.nonce.clone(),
                i64::try_from(ready_auth.valid_after).unwrap(),
                i64::try_from(ready_auth.valid_before).unwrap(),
            );
            let generation: i64 = store
                .db
                .query_row(
                    "SELECT generation FROM reserve_claims WHERE pool_id=?1 AND slot='small'",
                    [&pool],
                    |row| row.get(0),
                )
                .unwrap();
            let originals = store.funding_jobs().unwrap();
            let snapshots: Vec<_> = originals
                .iter()
                .map(|job| (serde_json::to_value(job).unwrap(), readiness(&store, job)))
                .collect();
            let (handle, worker) = StoreHandle::spawn(store);
            let (entered, entrance) = tokio::sync::oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel();
            let blocking_handle = handle.clone();
            let blocker = tokio::spawn(async move {
                blocking_handle
                    .call(move |_| {
                        entered.send(()).unwrap();
                        wait.recv().unwrap();
                        Ok(())
                    })
                    .await
            });
            entrance.await.unwrap();
            let pending;
            let (journal_result, background_result) = {
                let journal = handle.call(move |store| {
                    store.journal_authorization(&ready_payment, ready_auth, None)
                });
                let background = handle.call(|store| {
                    let promoted = store.promote_low_active("pool")?;
                    store.fill_funding_vacancies("pool")?;
                    Ok(promoted)
                });
                let mut journal = std::pin::pin!(journal);
                let mut background = std::pin::pin!(background);
                // With the owner blocked and two free queue slots, each first poll
                // enqueues its real Store call and waits for its reply. Both calls
                // are pending together; poll order fixes FIFO order without sleeps.
                if journal_first {
                    pending = [
                        futures_util::poll!(journal.as_mut()).is_pending(),
                        futures_util::poll!(background.as_mut()).is_pending(),
                    ];
                } else {
                    pending = [
                        futures_util::poll!(background.as_mut()).is_pending(),
                        futures_util::poll!(journal.as_mut()).is_pending(),
                    ];
                }
                release.send(()).unwrap();
                tokio::join!(journal, background)
            };
            blocker.await.unwrap().unwrap();
            assert_eq!(pending, [true, true]);
            journal_result.unwrap();
            assert_eq!(background_result.unwrap(), !journal_first);
            let checked = handle.call(move |store| {
                let ready_liability = liability(store, &ready_id);
                let actual = store.db.query_row(
                    "SELECT id,pool_id,wallet_id,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before FROM payment_attempts WHERE id=?1",
                    [&ready_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,i64>(9)?,r.get::<_,i64>(10)?)),
                )?;
                assert_eq!(actual, expected_ready);
                let evidence: (String, Option<String>, Option<i64>) = store.db.query_row(
                    "SELECT payer_role,reserve_slot,claim_generation FROM payment_admission_evidence WHERE attempt_id=?1",
                    [&ready_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
                )?;
                assert_eq!(evidence, if journal_first {
                    ("READY".into(), Some("small".into()), Some(generation))
                } else {
                    ("ACTIVE".into(), None, None)
                });
                assert_eq!(capacity::payment_ownership(&store.db, &pool, &active.wallet_id)?.unwrap().role, "RETAINED");
                assert_eq!(capacity::payment_ownership(&store.db, &pool, &ready.wallet_id)?.unwrap().role, "ACTIVE");
                // A captured former ACTIVE remains chargeable after its role changes.
                store.journal_authorization(&active_payment, authorization(&active_payment, "retained-use"), None)?;
                let retained_liability = liability(store, &active_payment.id);
                for _ in 0..2 {
                    assert!(!store.promote_low_active("pool")?);
                    store.fill_funding_vacancies("pool")?;
                }
                for job in [&active, &ready] {
                    let balance: String = store.db.query_row("SELECT balance FROM wallets WHERE id=?1", [&job.wallet_id], |r| r.get(0))?;
                    assert_eq!(balance, if job.id == active.id { (threshold - U256::from(1)).to_string() } else { "3999999".into() });
                    funded(store, job, 0);
                    funded(store, job, 7);
                    assert!(!capacity::funding_claim_present(&store.db, &job.id)?);
                }
                // Repeated observations cannot restore consumed reserve ownership
                // or allocate another replacement for an already filled vacancy.
                store.fill_funding_vacancies("pool")?;
                assert_eq!(capacity::payment_ownership(&store.db, &pool, &active.wallet_id)?.unwrap().role, "RETAINED");
                assert_eq!(capacity::payment_ownership(&store.db, &pool, &ready.wallet_id)?.unwrap().role, "ACTIVE");
                let (replacement, next): (String, i64) = store.db.query_row(
                    "SELECT wallet_id,generation FROM reserve_claims WHERE pool_id=?1 AND slot='small'",
                    [&pool], |r| Ok((r.get(0)?,r.get(1)?)),
                )?;
                assert_eq!(next, generation + 1);
                assert!(!originals.iter().any(|job| job.wallet_id == replacement));
                assert_eq!(store.funding_jobs()?.len(), originals.len() + 1);
                assert_eq!(store.db.query_row("SELECT count(*) FROM wallets", [], |r| r.get::<_,i64>(0))?, i64::try_from(originals.len() + 1).unwrap());
                for (job, (snapshot, credit)) in originals.iter().zip(snapshots) {
                    assert_eq!(serde_json::to_value(store.funding_job(&job.id)?.unwrap())?, snapshot);
                    assert_eq!(readiness(store, job), credit);
                }
                assert_eq!(liability(store, &initial.id), initial_liability);
                assert_eq!(liability(store, &ready_id), ready_liability);
                assert_eq!(liability(store, &active_payment.id), retained_liability);
                assert_eq!(store.db.query_row("SELECT count(*) FROM payment_attempts", [], |r| r.get::<_,i64>(0))?, 3);
                Ok(())
            }).await;
            drop(handle);
            worker.await.unwrap();
            checked.unwrap();
        }
    }
}
