//! Settled local HTTP/RPC workloads. No live treasury or provider is used.
use super::*;
use x402_treazury::rotation::{
    config::{ReserveConfig, ReservesConfig},
    store::AllocationKind,
};
const USDC_UNIT: u64 = 1_000_000;

async fn inventory() -> (
    Harness,
    x402_treazury::rotation::funding::FundingWorker<super::lifecycle::Funding>,
    u64,
) {
    let mut h = Harness::new().await;
    let pool = h
        .store
        .call(|s| {
            let reserves = ReservesConfig {
                small: ReserveConfig {
                    funding_amount_usdc: "2".into(),
                    max_funding_amount_usdc: None,
                },
                large: Some(ReserveConfig {
                    funding_amount_usdc: "10".into(),
                    max_funding_amount_usdc: None,
                }),
            };
            let pool = s.ensure_capacity_group(
                "consumption",
                &reserves,
                Some(alloy_primitives::U256::from(10 * USDC_UNIT)),
                10_000,
            )?;
            for kind in [
                AllocationKind::Initial,
                AllocationKind::Small,
                AllocationKind::Large,
            ] {
                s.allocate_capacity_kind(&pool, kind, true)?;
            }
            super::lifecycle::synced(s)?;
            Ok(pool)
        })
        .await
        .unwrap();
    h.pool = pool;
    h.client = make_client(h.store.clone(), h.pool.clone(), &h.base);
    let worker = super::lifecycle::funder(&h, Arc::default());
    (h, worker, now().unwrap())
}
pub(super) async fn fill(
    h: &Harness,
    worker: &mut x402_treazury::rotation::funding::FundingWorker<super::lifecycle::Funding>,
    instant: &mut u64,
) {
    let pool = h.pool.clone();
    h.store
        .call(move |s| {
            for kind in [
                AllocationKind::Initial,
                AllocationKind::Small,
                AllocationKind::Large,
            ] {
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

fn binding(h: &Harness, method: &str, source: &str) -> x402_treazury::catalog_state::BoundTool {
    use x402_treazury::{catalog::ToolSpec, catalog_state::BoundTool};
    BoundTool {
        tool: ToolSpec {
            response_mapping: None,
            image_limits: Default::default(),
            name: method.into(),
            description: "fixture".into(),
            method: method.into(),
            path: "/pay".into(),
            input_schema: json!({"type":"object"}),
            param_routes: Default::default(),
            has_body: false,
            help_url: None,
            body_names: Default::default(),
        },
        client: h.client.clone(),
        base: h.base.clone(),
        source: Some((source.into(), 1)),
        help: Arc::new(tokio::sync::OnceCell::new()),
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
async fn accounting(h: &Harness, gross: u64, spent: u64, holdings: u64) {
    let pool = h.pool.clone();
    let state = h.store.call(|s| s.status()).await.unwrap();
    let group = state.pools.iter().find(|p| p.id == pool).unwrap();
    let allocated: u64 = state
        .funding_jobs
        .iter()
        .filter(|j| {
            j.pool_id == pool
                && j.phase == x402_treazury::rotation::store::funding::FundingPhase::Complete
        })
        .map(|j| j.target.parse::<u64>().unwrap())
        .sum();
    let held: u64 = group
        .addresses
        .iter()
        .map(|w| {
            h.f.balances
                .lock()
                .unwrap()
                .get(&w.address)
                .copied()
                .unwrap_or(0)
        })
        .sum();
    assert_eq!((allocated, held), (gross, holdings));
    assert_eq!(allocated - held, spent);
    for wallet in &group.addresses {
        if wallet.role == "RETIRED" {
            assert_eq!(
                h.f.balances
                    .lock()
                    .unwrap()
                    .get(&wallet.address)
                    .copied()
                    .unwrap_or(0),
                0
            );
        }
    }
}
#[tokio::test]
async fn expensive_only_refills_past_eleven_wallets_without_retirement() {
    let (h, mut worker, mut instant) = inventory().await;
    for calls in 1..=12 {
        fill(&h, &mut worker, &mut instant).await;
        paid_and_settled(&h, 6 * USDC_UNIT).await;
        accounting(
            &h,
            (4 + 10 * calls) * USDC_UNIT,
            6 * calls * USDC_UNIT,
            (4 + 4 * calls) * USDC_UNIT,
        )
        .await;
        let state = h.store.call(|s| s.status()).await.unwrap();
        let group = state.pools.iter().find(|p| p.id == h.pool).unwrap();
        assert!(group.addresses.iter().all(|w| w.role != "RETIRED"));
        if calls == 9 {
            accounting(&h, 94 * USDC_UNIT, 54 * USDC_UNIT, 40 * USDC_UNIT).await;
            assert_eq!(
                group
                    .addresses
                    .iter()
                    .filter(|w| w.role == "RETAINED")
                    .count(),
                9
            );
            assert_eq!(
                group
                    .addresses
                    .iter()
                    .filter(|w| w.role == "ACTIVE")
                    .count(),
                1
            );
            assert_eq!(
                group.addresses.iter().filter(|w| w.role == "READY").count(),
                1
            );
        }
        if calls == 10 {
            accounting(&h, 104 * USDC_UNIT, 60 * USDC_UNIT, 44 * USDC_UNIT).await;
        }
    }
    h.store
        .call(|s| {
            s.pause_new_funding();
            Ok(())
        })
        .await
        .unwrap();
    let before = h.store.call(|s| s.status()).await.unwrap();
    let group = before.pools.iter().find(|p| p.id == h.pool).unwrap();
    assert!(
        group
            .addresses
            .iter()
            .filter(|w| w.role == "RETAINED")
            .count()
            >= 9
    );
    let expected = group
        .addresses
        .iter()
        .find(|w| w.role == "RETAINED" && w.target == "2000000")
        .unwrap()
        .address
        .clone();
    let cheap = binding(&h, "POST", "many-retained-paused");
    for learning in [true, false] {
        *h.f.challenge.lock().unwrap() = challenge("100000");
        let unsigned_before =
            h.f.traces
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, k, _)| k == "unsigned")
                .count();
        let inventory = h.store.call(|s| s.status()).await.unwrap();
        assert_eq!(cheap.invoke(&serde_json::Map::new()).await.unwrap(), "paid");
        let signed = h.f.signed.lock().unwrap().last().unwrap().clone();
        assert_original_journal_claim(&h, &inventory, &signed);
        if !learning {
            assert_eq!(
                signed["payload"]["authorization"]["from"]
                    .as_str()
                    .unwrap()
                    .parse::<Address>()
                    .unwrap()
                    .to_string(),
                expected
            );
            assert_eq!(
                h.f.traces
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(_, k, _)| k == "unsigned")
                    .count()
                    - unsigned_before,
                1
            );
        }
        settle_last(&h, 100000).await;
    }
    let after = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(before.funding_jobs.len(), after.funding_jobs.len());
    let after_group = after.pools.iter().find(|p| p.id == h.pool).unwrap();
    assert_eq!(
        serde_json::to_value(&group.reserve_claims).unwrap(),
        serde_json::to_value(&after_group.reserve_claims).unwrap()
    );
    assert_eq!(
        group
            .addresses
            .iter()
            .find(|w| w.role == "ACTIVE")
            .unwrap()
            .id,
        after_group
            .addresses
            .iter()
            .find(|w| w.role == "ACTIVE")
            .unwrap()
            .id
    );
    drop(cheap);
    drop(worker);
    h.close().await;
}
#[tokio::test]
async fn naturally_expired_mixed_price_hints_preserve_fragmentation_and_continue_filling() {
    let (h, mut worker, mut instant) = inventory().await;
    let expensive = binding(&h, "GET", "consumption-expired");
    let cheap = binding(&h, "POST", "consumption-expired");
    for wallets in 1..=10 {
        fill(&h, &mut worker, &mut instant).await;
        for _ in 0..3 {
            let mut expensive_payer = None;
            for (tool, price) in [(&expensive, 2_250_000u64), (&cheap, 500_000)] {
                let inventory = h.store.call(|s| s.status()).await.unwrap();
                *h.f.challenge.lock().unwrap() = challenge(&price.to_string());
                let signed_before = h.f.signed.lock().unwrap().len();
                assert_eq!(tool.invoke(&serde_json::Map::new()).await.unwrap(), "paid");
                let signed = h.f.signed.lock().unwrap().clone();
                assert_eq!(signed.len(), signed_before + 1);
                let payment = signed.last().unwrap();
                assert_original_journal_claim(&h, &inventory, payment);
                let payer = payment["payload"]["authorization"]["from"]
                    .as_str()
                    .unwrap()
                    .parse::<Address>()
                    .unwrap()
                    .to_string();
                if price == 2_250_000 {
                    expensive_payer = Some(payer);
                } else {
                    assert_eq!(
                        Some(payer),
                        expensive_payer,
                        "expired hint preserves sufficient bound active despite smaller affordable retained capacity"
                    );
                }
                settle_last(&h, price).await;
                // Every real challenge learned its key. Only the monotonic hint
                // clock advances here; financial block time and evidence remain
                // real, and localhost I/O always runs with time resumed.
                tokio::time::pause();
                tokio::time::advance(std::time::Duration::from_secs(301)).await;
                tokio::time::resume();
            }
        }
        let spent = wallets * 8_250_000;
        if wallets == 9 {
            accounting(&h, 94_000_000, 74_250_000, 19_750_000).await;
        }
        if wallets == 10 {
            accounting(&h, 104_000_000, 82_500_000, 21_500_000).await;
        }
        accounting(
            &h,
            (4 + 10 * wallets) * USDC_UNIT,
            spent,
            (4 + 10 * wallets) * USDC_UNIT - spent,
        )
        .await;
    }
    drop(expensive);
    drop(cheap);
    drop(worker);
    h.close().await;
}
#[test]
fn settled_fixed_price_sizing_bounds_use_integer_micro_usdc() {
    assert_eq!((10 * USDC_UNIT) % (6 * USDC_UNIT), 4 * USDC_UNIT); // outside recommendation
    assert_eq!((12 * USDC_UNIT) % (6 * USDC_UNIT), 0);
    assert_eq!((24 * USDC_UNIT) % (6 * USDC_UNIT), 0);
    assert_eq!(
        [5_000_000, 4_000_000, 3_340_000].map(|p| 10_000_000 % p),
        [0, 2_000_000, 3_320_000]
    );
    for divisor in [2u64, 3, 4, 10] {
        for target in [9_999_999u64, 10_000_000, 10_000_001] {
            let boundary = target / divisor;
            for price in [boundary - 1, boundary, boundary + 1] {
                let remainder = target % price;
                assert!(remainder < price);
                if price <= boundary {
                    assert!(remainder * divisor < target);
                }
            }
        }
    }
}

#[tokio::test]
async fn actual_spendable_exhaustion_preserves_claim_and_resumes_after_capacity_arrives() {
    let (h, mut worker, mut instant) = inventory().await;
    worker
        .backend
        .lock()
        .await
        .capital
        .store(300, Ordering::SeqCst);
    h.store
        .call(|s| super::lifecycle::synced_capacity(s, 300))
        .await
        .unwrap();
    fill(&h, &mut worker, &mut instant).await;
    paid_and_settled(&h, 6 * USDC_UNIT).await;
    let pool = h.pool.clone();
    h.store
        .call(move |s| {
            s.allocate_capacity_kind(&pool, AllocationKind::Large, true)?;
            Ok(())
        })
        .await
        .unwrap();
    let before = h.store.call(|s| s.status()).await.unwrap();
    for _ in 0..5 {
        super::lifecycle::tick(&mut worker, &mut instant).await;
    }
    let paused = h.store.call(|s| s.status()).await.unwrap();
    assert_eq!(
        before
            .funding_jobs
            .iter()
            .map(|j| (&j.id, &j.wallet_id))
            .collect::<Vec<_>>(),
        paused
            .funding_jobs
            .iter()
            .map(|j| (&j.id, &j.wallet_id))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        before.treasury_operations.len(),
        paused.treasury_operations.len(),
        "zero spendable capital grants no new preparation"
    );
    accounting(&h, 14 * USDC_UNIT, 6 * USDC_UNIT, 8 * USDC_UNIT).await;
    worker
        .backend
        .lock()
        .await
        .capital
        .store(100, Ordering::SeqCst);
    h.store
        .call(|s| super::lifecycle::synced_capacity(s, 100))
        .await
        .unwrap();
    fill(&h, &mut worker, &mut instant).await;
    paid_and_settled(&h, 6 * USDC_UNIT).await;
    accounting(&h, 24 * USDC_UNIT, 12 * USDC_UNIT, 12 * USDC_UNIT).await;
    drop(worker);
    h.close().await;
}

#[tokio::test]
async fn learned_mixed_operation_keys_choose_actual_smallest_available_wallet() {
    let (h, mut worker, mut instant) = inventory().await;
    fill(&h, &mut worker, &mut instant).await;
    let expensive = binding(&h, "GET", "consumption-mixed");
    let cheap = binding(&h, "POST", "consumption-mixed");
    let mut spent = 0u64;
    let mut retained_wins = 0;
    for pair in 0..12 {
        fill(&h, &mut worker, &mut instant).await;
        for (tool, price) in [(&expensive, 2_250_000u64), (&cheap, 500_000)] {
            let state = h.store.call(|s| s.status()).await.unwrap();
            let group = state.pools.iter().find(|p| p.id == h.pool).unwrap();
            let mut choices = group
                .addresses
                .iter()
                .enumerate()
                .filter_map(|(sequence, w)| {
                    let role = match w.role.as_str() {
                        "ACTIVE" => 0,
                        "RETAINED" => 1,
                        "READY" => 2,
                        _ => return None,
                    };
                    let amount =
                        h.f.balances
                            .lock()
                            .unwrap()
                            .get(&w.address)
                            .copied()
                            .unwrap_or(0);
                    (amount >= price).then_some((amount, role, sequence, w.address.clone()))
                })
                .collect::<Vec<_>>();
            choices.sort();
            let expected = choices.first().unwrap().clone();
            *h.f.challenge.lock().unwrap() = challenge(&price.to_string());
            let unsigned_before =
                h.f.traces
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(_, kind, _)| kind == "unsigned")
                    .count();
            let signed_before = h.f.signed.lock().unwrap().len();
            assert_eq!(tool.invoke(&serde_json::Map::new()).await.unwrap(), "paid");
            let signed = h.f.signed.lock().unwrap().clone();
            assert_eq!(signed.len(), signed_before + 1);
            assert_original_journal_claim(&h, &state, signed.last().unwrap());
            let payer = signed.last().unwrap()["payload"]["authorization"]["from"]
                .as_str()
                .unwrap()
                .parse::<Address>()
                .unwrap()
                .to_string();
            if pair > 0 {
                assert_eq!(
                    payer, expected.3,
                    "learned operation must choose actual smallest sufficient available capacity"
                );
                let unsigned_after =
                    h.f.traces
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(_, kind, _)| kind == "unsigned")
                        .count();
                assert_eq!(unsigned_after - unsigned_before, 1);
                retained_wins += usize::from(expected.1 == 1);
            }
            settle_last(&h, price).await;
            spent += price;
            let state = h.store.call(|s| s.status()).await.unwrap();
            let gross = state
                .funding_jobs
                .iter()
                .filter(|j| {
                    j.pool_id == h.pool
                        && j.phase
                            == x402_treazury::rotation::store::funding::FundingPhase::Complete
                })
                .map(|j| j.target.parse::<u64>().unwrap())
                .sum();
            accounting(&h, gross, spent, gross - spent).await;
        }
    }
    assert!(
        retained_wins > 0,
        "fixture must include genuine smallest-fit retained opportunities"
    );
    drop(expensive);
    drop(cheap);
    drop(worker);
    h.close().await;
}

// Deliberately independent of selection: the model accepts the actual payer,
// applies the financial event, then checks every wallet and claim. It never
// predicts the implementation's search, RPC cache, or scheduling decisions.
struct FinancialModel {
    balances: BTreeMap<String, u64>,
    roles: BTreeMap<String, String>,
    claims: BTreeMap<String, (Option<String>, i64)>,
    pending: Option<(String, u64)>,
    credited: u64,
    debited: u64,
}
impl FinancialModel {
    async fn initial(h: &Harness) -> Self {
        let state = h.store.call(|s| s.status()).await.unwrap();
        let group = state.pools.iter().find(|p| p.id == h.pool).unwrap();
        let balances: BTreeMap<_, _> = group
            .addresses
            .iter()
            .map(|w| (w.id.clone(), w.target.parse::<u64>().unwrap()))
            .collect();
        Self {
            credited: balances.values().sum(),
            balances,
            roles: group
                .addresses
                .iter()
                .map(|w| (w.id.clone(), w.role.clone()))
                .collect(),
            claims: group
                .reserve_claims
                .iter()
                .map(|c| (c.slot.clone(), (c.wallet_id.clone(), c.generation)))
                .collect(),
            pending: None,
            debited: 0,
        }
    }
    async fn submitted(&mut self, h: &Harness, amount: u64) {
        assert!(self.pending.is_none());
        let signed = h.f.signed.lock().unwrap().last().unwrap().clone();
        let address = signed["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .parse::<Address>()
            .unwrap()
            .to_string();
        let state = h.store.call(|s| s.status()).await.unwrap();
        let group = state.pools.iter().find(|p| p.id == h.pool).unwrap();
        let payer = group
            .addresses
            .iter()
            .find(|w| w.address == address)
            .unwrap()
            .id
            .clone();
        assert!(self.balances[&payer] >= amount);
        if self.roles[&payer] == "READY" {
            for role in self.roles.values_mut() {
                if role == "ACTIVE" {
                    *role = "RETAINED".into();
                }
            }
            self.roles.insert(payer.clone(), "ACTIVE".into());
            let claim = self
                .claims
                .values_mut()
                .find(|(w, _)| w.as_ref() == Some(&payer))
                .unwrap();
            claim.0 = None;
            claim.1 += 1;
        }
        self.pending = Some((payer, amount));
        self.check(h).await;
    }
    fn resolved(&mut self, used: bool) {
        let (payer, amount) = self.pending.take().unwrap();
        if used {
            *self.balances.get_mut(&payer).unwrap() -= amount;
            self.debited += amount;
        }
        for (id, role) in &mut self.roles {
            if role == "RETAINED" && self.balances[id] == 0 {
                *role = "RETIRED".into();
            }
        }
    }
    async fn check(&self, h: &Harness) {
        let state = h.store.call(|s| s.status()).await.unwrap();
        let group = state.pools.iter().find(|p| p.id == h.pool).unwrap();
        assert_eq!(group.addresses.len(), self.balances.len());
        for wallet in &group.addresses {
            assert_eq!(wallet.role, self.roles[&wallet.id]);
            assert_eq!(
                h.f.balances.lock().unwrap()[&wallet.address],
                self.balances[&wallet.id]
            );
            let exposure = self
                .pending
                .as_ref()
                .filter(|(id, _)| id == &wallet.id)
                .map_or(0, |(_, amount)| *amount);
            assert_eq!(
                wallet
                    .signed_exposure_atomic
                    .as_deref()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap(),
                exposure
            );
            if wallet.role == "RETIRED" {
                assert!(wallet.retirement.is_some());
            }
        }
        for claim in &group.reserve_claims {
            assert_eq!(
                (claim.wallet_id.clone(), claim.generation),
                self.claims[&claim.slot]
            );
        }
        assert_eq!(
            self.credited - self.debited,
            self.balances.values().sum::<u64>()
        );
    }
}

#[tokio::test]
async fn seeded_financial_model_preserves_claims_liabilities_retirement_and_reopen() {
    let (mut h, mut worker, mut instant) = inventory().await;
    fill(&h, &mut worker, &mut instant).await;
    drop(worker);
    h.store
        .call(|s| {
            s.pause_new_funding();
            Ok(())
        })
        .await
        .unwrap();
    let mut model = FinancialModel::initial(&h).await;
    // First create genuine uncertain seller work; reopening cannot release it.
    *h.f.challenge.lock().unwrap() = challenge("6000000");
    h.f.challenge.lock().unwrap()["accepts"][0]["maxTimeoutSeconds"] = json!(5);
    h.f.reject.store(true, Ordering::SeqCst);
    assert!(h.client.execute(h.route()).await.is_err());
    model.submitted(&h, 6_000_000).await;
    h = super::draining::reopen(h).await;
    model.check(&h).await;
    let expiry =
        h.f.signed.lock().unwrap().last().unwrap()["payload"]["authorization"]["validBefore"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
    while now().unwrap() <= expiry {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    h.f.reject.store(false, Ordering::SeqCst);
    reconcile_complete_history(&h).await;
    model.resolved(false);
    {
        let db = rusqlite::Connection::open(&h.f.db).unwrap();
        let expired: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM payment_resolutions WHERE outcome='EXPIRED_UNUSED'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(expired, 1);
    }
    model.check(&h).await;
    let tool = binding(&h, "POST", "seeded-variable-price");
    // The same trusted key changes from $6 to $0.10 to $2, then seeded
    // prices. Learning the cheap price selects the retained $2 wallet on
    // the third call, whose actual larger challenge drains it exactly.
    let mut seed = 0x402_cafe_u64;
    let mut prices = vec![6_000_000, 100_000, 2_000_000];
    for _ in 0..8 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        prices.push((1 + ((seed >> 32) % 3)) * 100_000);
    }
    for amount in prices {
        *h.f.challenge.lock().unwrap() = challenge(&amount.to_string());
        let before = h.f.signed.lock().unwrap().len();
        assert_eq!(tool.invoke(&serde_json::Map::new()).await.unwrap(), "paid");
        assert_eq!(h.f.signed.lock().unwrap().len(), before + 1);
        model.submitted(&h, amount).await;
        settle_last(&h, amount).await;
        model.resolved(true);
        model.check(&h).await;
    }
    assert!(model.roles.values().any(|role| role == "RETIRED"));
    drop(tool);
    h = super::draining::reopen(h).await;
    model.check(&h).await;
    h.close().await;
}
