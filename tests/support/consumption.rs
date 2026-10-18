//! Initial-funding and immutable journal correlation helpers for local HTTP fixtures.
use super::*;
use x402_treazury::rotation::store::AllocationKind;

pub(super) async fn fill(
    h: &Harness,
    worker: &mut x402_treazury::rotation::funding::FundingWorker<super::lifecycle::Funding>,
    instant: &mut u64,
) {
    let pool = h.pool.clone();
    h.store
        .call(move |s| {
            for kind in [AllocationKind::Small, AllocationKind::Large] {
                s.allocate_capacity_kind(&pool, kind, true)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    for _ in 0..30 {
        super::lifecycle::tick(worker, instant).await;
        let pool = h.pool.clone();
        let complete = h
            .store
            .call(move |s| {
                Ok(s.status()?
                    .funding_jobs
                    .iter()
                    .filter(|j| j.pool_id == pool)
                    .all(|j| {
                        j.phase == x402_treazury::rotation::store::funding::FundingPhase::Complete
                    }))
            })
            .await
            .unwrap();
        if complete {
            return;
        }
    }
    panic!("funding did not complete within fixture passes");
}

pub(super) async fn paid_and_settled(h: &Harness, amount: u64) {
    *h.f.challenge.lock().unwrap() = challenge(&amount.to_string());
    let before = h.f.signed.lock().unwrap().len();
    let unsigned_before =
        h.f.traces
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, kind, _)| kind == "unsigned")
            .count();
    let inventory = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(h.client.execute(h.route()).await.unwrap(), "paid");
    let signed = h.f.signed.lock().unwrap().clone();
    assert_eq!(signed.len(), before + 1);
    let unsigned =
        h.f.traces
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, kind, _)| kind == "unsigned")
            .count()
            - unsigned_before;
    assert!(
        (1..=2).contains(&unsigned),
        "one original request and at most one unsigned handoff"
    );
    assert_original_journal_claim(h, &inventory, signed.last().unwrap());
    settle_last(h, amount).await;
}

fn assert_original_journal_claim(
    h: &Harness,
    inventory: &x402_treazury::rotation::store::Status,
    signed: &Value,
) {
    let payer = signed["payload"]["authorization"]["from"]
        .as_str()
        .unwrap()
        .parse::<Address>()
        .unwrap()
        .to_string();
    let group = inventory.pools.iter().find(|p| p.id == h.pool).unwrap();
    let wallet = group.addresses.iter().find(|w| w.address == payer).unwrap();
    let claim = group
        .reserve_claims
        .iter()
        .find(|c| c.wallet_id.as_deref() == Some(wallet.id.as_str()));
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    let audit: (String, Option<String>, Option<i64>) = db
        .query_row(
            "SELECT e.payer_role,e.reserve_slot,e.claim_generation FROM payment_admission_evidence e JOIN payment_attempts p ON p.id=e.attempt_id WHERE p.nonce=?1",
            [signed["payload"]["authorization"]["nonce"]
                .as_str()
                .unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(audit.0, wallet.role);
    if wallet.role == "READY" {
        let claim = claim.unwrap();
        assert_eq!(
            (audit.1.as_deref(), audit.2),
            (Some(claim.slot.as_str()), Some(claim.generation))
        );
    } else {
        assert_eq!((audit.1, audit.2), (None, None));
    }
}

pub(super) async fn settle_last(h: &Harness, amount: u64) {
    let signed = h.f.signed.lock().unwrap().clone();
    let payer = signed.last().unwrap()["payload"]["authorization"]["from"]
        .as_str()
        .unwrap()
        .parse::<Address>()
        .unwrap()
        .to_string();
    {
        let mut balances = h.f.balances.lock().unwrap();
        let balance = balances.get_mut(&payer).unwrap();
        *balance = balance
            .checked_sub(amount)
            .expect("one actual wallet pays the full amount");
    }
    h.f.used.store(true, Ordering::SeqCst);
    reconcile_complete_history(h).await;
    // Refresh current balances after the independently committed debit checkpoint.
    assert_balance_observation_outcome(
        make_pool(h.store.clone(), h.pool.clone(), &h.base)
            .reconcile()
            .await,
    );
}
