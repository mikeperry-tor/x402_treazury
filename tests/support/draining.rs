//! Deterministic retained-capacity workloads through real HTTP, hints and Base proofs.
use super::*;
use x402_treazury::rotation::config::{ReserveConfig, ReservesConfig};
use x402_treazury::{catalog::ToolSpec, catalog_state::BoundTool};

fn binding(h: &Harness, method: &str) -> BoundTool {
    BoundTool {
        tool: ToolSpec {
            response_mapping: None,
            image_limits: Default::default(),
            name: method.into(),
            description: "draining fixture".into(),
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
        source: Some(("retained-draining".into(), 1)),
        help: Arc::new(tokio::sync::OnceCell::new()),
    }
}
async fn publish(h: &Harness) {
    assert_balance_observation_outcome(
        make_pool(h.store.clone(), h.pool.clone(), &h.base)
            .reconcile()
            .await,
    );
}
async fn group(h: &Harness) -> x402_treazury::rotation::store::PoolStatus {
    let pool = h.pool.clone();
    h.store
        .call(move |s| {
            Ok(s.status()?
                .pools
                .into_iter()
                .find(|p| p.id == pool)
                .unwrap())
        })
        .await
        .unwrap()
}
fn payer(h: &Harness) -> String {
    h.f.signed.lock().unwrap().last().unwrap()["payload"]["authorization"]["from"]
        .as_str()
        .unwrap()
        .parse::<Address>()
        .unwrap()
        .to_string()
}
async fn retained_inventory(two: bool) -> Harness {
    retained_inventory_setup(two, true).await
}
async fn retained_inventory_setup(two: bool, paused: bool) -> Harness {
    let h = Harness::new().await;
    h.store.call(super::lifecycle::synced).await.unwrap();
    let mut worker = super::lifecycle::funder(&h, Arc::default());
    let mut instant = now().unwrap();
    for _ in 0..if two { 2 } else { 1 } {
        let state = group(&h).await;
        let active = state.addresses.iter().find(|w| w.role == "ACTIVE").unwrap();
        h.f.balances
            .lock()
            .unwrap()
            .insert(active.address.clone(), 600_000);
        publish(&h).await;
        super::consumption::paid_and_settled(&h, 1_000_000).await;
        super::consumption::fill(&h, &mut worker, &mut instant).await;
    }
    let pool = h.pool.clone();
    h.store
        .call(move |s| {
            s.configure_capacity_group(
                &pool,
                Some(alloy_primitives::U256::from(10_000_000)),
                0,
                &ReservesConfig {
                    small: ReserveConfig {
                        funding_amount_usdc: "5".into(),
                        max_funding_amount_usdc: None,
                    },
                    large: Some(ReserveConfig {
                        funding_amount_usdc: "5".into(),
                        max_funding_amount_usdc: None,
                    }),
                },
            )
        })
        .await
        .unwrap();
    super::consumption::fill(&h, &mut worker, &mut instant).await;
    drop(worker);
    let state = group(&h).await;
    assert_eq!(
        state
            .addresses
            .iter()
            .filter(|w| w.role == "RETAINED")
            .count(),
        if two { 2 } else { 1 }
    );
    if paused {
        h.store
            .call(|s| {
                s.pause_new_funding();
                Ok(())
            })
            .await
            .unwrap();
    }
    h
}
pub(super) async fn reopen(h: Harness) -> Harness {
    let Harness {
        dir,
        f,
        store,
        worker,
        pool,
        client,
        url,
        base,
        server,
    } = h;
    let id = store.call(|s| Ok(s.id().to_owned())).await.unwrap();
    drop(client);
    drop(store);
    worker.await.unwrap();
    let mut state = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
    state.pause_new_funding();
    let (store, worker) = StoreHandle::spawn(state);
    let client = make_client(store.clone(), pool.clone(), &base);
    Harness {
        dir,
        f,
        store,
        worker,
        pool,
        client,
        url,
        base,
        server,
    }
}
async fn cold_retained(method: &str) {
    let h = retained_inventory(false).await;
    let before = group(&h).await;
    let active = before
        .addresses
        .iter()
        .find(|w| w.role == "ACTIVE")
        .unwrap();
    let retained = before
        .addresses
        .iter()
        .find(|w| w.role == "RETAINED")
        .unwrap()
        .address
        .clone();
    h.f.balances
        .lock()
        .unwrap()
        .insert(active.address.clone(), 50_000);
    let h = reopen(h).await; // Both runtime hints and cached Base certificates start empty.
    *h.f.challenge.lock().unwrap() = challenge("100000");
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    *h.f.unsigned_gate.lock().unwrap() = Some(UnsignedGate {
        barrier: barrier.clone(),
        remaining: 1,
    });
    let tool = binding(&h, method);
    let trace_start = h.f.traces.lock().unwrap().len();
    let call = tokio::spawn(async move { tool.invoke(&Default::default()).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while h
            .f
            .unsigned_gate
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .remaining
            != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    publish(&h).await; // Retained proof is compatible and complete before final selection.
    barrier.wait().await;
    assert_eq!(call.await.unwrap().unwrap(), "paid");
    assert_eq!(payer(&h), retained);
    {
        let traces = h.f.traces.lock().unwrap();
        assert_eq!(
            traces[trace_start..]
                .iter()
                .filter(|(_, kind, _)| kind == "unsigned")
                .count(),
            2
        );
        assert_eq!(
            traces[trace_start..]
                .iter()
                .filter(|(_, kind, _)| kind == "signed")
                .count(),
            1
        );
    }
    let after = group(&h).await;
    assert_eq!(before.generation, after.generation);
    assert_eq!(
        serde_json::to_value(before.reserve_claims).unwrap(),
        serde_json::to_value(after.reserve_claims).unwrap()
    );
    assert_eq!(before.addresses.len(), after.addresses.len());
    super::consumption::settle_last(&h, 100_000).await;
    h.close().await;
}
#[tokio::test]
async fn cold_get_hands_off_once_to_published_retained_capacity_without_funding() {
    cold_retained("GET").await;
}
#[tokio::test]
async fn cold_post_hands_off_once_to_published_retained_capacity_without_funding() {
    cold_retained("POST").await;
}

#[tokio::test]
async fn learned_get_post_drain_two_retained_wallets_without_allocating_or_changing_active() {
    let h = retained_inventory(true).await;
    let before = group(&h).await;
    let active = before
        .addresses
        .iter()
        .find(|w| w.role == "ACTIVE")
        .unwrap()
        .id
        .clone();
    let mut retained: Vec<_> = before
        .addresses
        .iter()
        .filter(|w| w.role == "RETAINED")
        .map(|w| (w.id.clone(), w.address.clone()))
        .collect();
    retained.sort();
    let retained: Vec<_> = retained.into_iter().map(|(_, address)| address).collect();
    let get = binding(&h, "GET");
    let post = binding(&h, "POST");
    *h.f.challenge.lock().unwrap() = challenge("100000");
    for tool in [&get, &post] {
        assert_eq!(tool.invoke(&Default::default()).await.unwrap(), "paid");
        super::consumption::settle_last(&h, 100_000).await;
    }
    let mut paid = [0usize; 2];
    for n in 0..12 {
        let expected = retained
            .iter()
            .enumerate()
            .filter_map(|(i, address)| {
                let value = *h.f.balances.lock().unwrap().get(address).unwrap();
                (value >= 100_000).then_some((value, i, address.clone()))
            })
            .min()
            .unwrap();
        let start = h.f.traces.lock().unwrap().len();
        let tool = if n % 2 == 0 { &get } else { &post };
        assert_eq!(tool.invoke(&Default::default()).await.unwrap(), "paid");
        assert_eq!(payer(&h), expected.2);
        paid[expected.1] += 1;
        {
            let traces = h.f.traces.lock().unwrap();
            assert_eq!(
                traces[start..]
                    .iter()
                    .filter(|(_, kind, _)| kind == "unsigned")
                    .count(),
                1
            );
            assert_eq!(
                traces[start..]
                    .iter()
                    .filter(|(_, kind, _)| kind == "signed")
                    .count(),
                1
            );
        }
        super::consumption::settle_last(&h, 100_000).await;
    }
    assert_eq!(paid, [6, 6]);
    let after = group(&h).await;
    assert_eq!(
        after
            .addresses
            .iter()
            .find(|w| w.role == "ACTIVE")
            .unwrap()
            .id,
        active
    );
    assert_eq!(before.addresses.len(), after.addresses.len());
    assert_eq!(
        serde_json::to_value(before.reserve_claims).unwrap(),
        serde_json::to_value(after.reserve_claims).unwrap()
    );
    drop(get);
    drop(post);
    h.close().await;
}

#[tokio::test]
async fn cold_get_learns_while_history_is_slow_then_drains_published_available_retained() {
    cold_learning_with_late_retained_publication("GET").await;
}

#[tokio::test]
async fn cold_post_learns_while_history_is_slow_then_drains_published_available_retained() {
    cold_learning_with_late_retained_publication("POST").await;
}

async fn cold_learning_with_late_retained_publication(method: &str) {
    let h = retained_inventory_setup(true, false).await;
    // A distinct group's real journal supplies slow history without racing the
    // learning payer's debit watermark. Its owner remains independent.
    let history_pool = h
        .store
        .call(|s| {
            let reserves = ReservesConfig {
                small: ReserveConfig {
                    funding_amount_usdc: "5".into(),
                    max_funding_amount_usdc: None,
                },
                large: None,
            };
            let pool = s.ensure_capacity_group(
                "unrelated_history",
                &reserves,
                Some(alloy_primitives::U256::from(5_000_000u64)),
                0,
            )?;
            for kind in [
                x402_treazury::rotation::store::AllocationKind::Initial,
                x402_treazury::rotation::store::AllocationKind::Small,
            ] {
                s.allocate_capacity_kind(&pool, kind, true)?;
            }
            Ok(pool)
        })
        .await
        .unwrap();
    let addresses = h
        .store
        .call(|s| {
            Ok(s.status()?
                .pools
                .into_iter()
                .find(|p| p.name == "unrelated_history")
                .unwrap()
                .addresses)
        })
        .await
        .unwrap();
    for wallet in addresses {
        h.f.balances
            .lock()
            .unwrap()
            .insert(wallet.address, 5_000_000);
    }
    assert_balance_observation_outcome(
        make_pool(h.store.clone(), history_pool.clone(), &h.base)
            .reconcile()
            .await,
    );
    let history_client = make_client(h.store.clone(), history_pool.clone(), &h.base);
    *h.f.challenge.lock().unwrap() = challenge("100000");
    assert_eq!(history_client.execute(h.route()).await.unwrap(), "paid");
    drop(history_client);
    let original_payer = payer(&h);
    *h.f.balances
        .lock()
        .unwrap()
        .get_mut(&original_payer)
        .unwrap() -= 100_000;
    h.f.used.store(true, Ordering::SeqCst);
    let h = reopen(h).await;
    let before = group(&h).await;
    let mut retained: Vec<_> = before
        .addresses
        .iter()
        .filter(|w| w.role == "RETAINED")
        .map(|w| (w.id.clone(), w.address.clone()))
        .collect();
    retained.sort();
    let unavailable = retained[0].1.clone();
    let available = retained[1].1.clone();
    h.f.balance_unavailable
        .lock()
        .unwrap()
        .extend([unavailable, available.clone()]);
    h.f.balance_delays
        .lock()
        .unwrap()
        .insert(available.clone(), 25);
    // READY reads cannot win the first challenge before active evidence. Closing
    // these test-only gates after the paid response lets owned reads finish.
    let ready_gates: Vec<_> = before
        .addresses
        .iter()
        .filter(|w| w.role == "READY")
        .map(|w| {
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            h.f.balance_gates
                .lock()
                .unwrap()
                .insert(w.address.clone(), gate.clone());
            gate
        })
        .collect();
    h.f.nonce_delay.store(true, Ordering::SeqCst);
    let manager = make_pool(h.store.clone(), history_pool, &h.base);
    let background = tokio::spawn(async move { manager.reconcile().await });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while h.f.nonce_delay.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let learned = binding(&h, method);
    // This first real challenge teaches the operation hint. Faster active evidence
    // suffices even while history is retained and another wallet is unavailable.
    assert_eq!(learned.invoke(&Default::default()).await.unwrap(), "paid");
    let active = before
        .addresses
        .iter()
        .find(|w| w.role == "ACTIVE")
        .unwrap()
        .address
        .clone();
    assert_eq!(payer(&h), active);
    h.f.balance_gates.lock().unwrap().clear();
    for gate in ready_gates {
        gate.close();
    }
    allow_one_unavailable(background.await.unwrap());
    h.f.balance_delays.lock().unwrap().clear();
    // Only after learning completes can the ordinary owner acquire this retained
    // wallet's evidence. The other retained wallet stays unavailable throughout.
    h.f.balance_unavailable.lock().unwrap().remove(&available);
    settle_with_unavailable(&h, 100_000).await;
    assert_eq!(
        *h.f.balances.lock().unwrap().get(&available).unwrap(),
        600_000
    );
    for _ in 0..6 {
        let start = h.f.traces.lock().unwrap().len();
        assert_eq!(learned.invoke(&Default::default()).await.unwrap(), "paid");
        assert_eq!(payer(&h), available);
        assert_eq!(
            h.f.traces.lock().unwrap()[start..]
                .iter()
                .filter(|(_, kind, _)| kind == "unsigned")
                .count(),
            1
        );
        settle_with_unavailable(&h, 100_000).await;
    }
    assert_eq!(*h.f.balances.lock().unwrap().get(&available).unwrap(), 0);
    let after = group(&h).await;
    assert_eq!(before.addresses.len(), after.addresses.len());
    assert_eq!(
        serde_json::to_value(before.reserve_claims).unwrap(),
        serde_json::to_value(after.reserve_claims).unwrap()
    );
    drop(learned);
    h.close().await;
}

fn allow_one_unavailable(result: anyhow::Result<()>) {
    if let Err(error) = result {
        if error
            .chain()
            .any(|cause| cause.to_string().contains("RPC code -32005"))
        {
            return;
        }
        assert_balance_observation_outcome(Err(error));
    }
}
async fn settle_with_unavailable(h: &Harness, amount: u64) {
    let who = payer(h);
    {
        let mut balances = h.f.balances.lock().unwrap();
        let balance = balances.get_mut(&who).unwrap();
        *balance = balance.checked_sub(amount).unwrap();
    }
    h.f.used.store(true, Ordering::SeqCst);
    allow_one_unavailable(
        make_pool(h.store.clone(), h.pool.clone(), &h.base)
            .reconcile()
            .await,
    );
    let db = rusqlite::Connection::open(&h.f.db).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    allow_one_unavailable(
        make_pool(h.store.clone(), h.pool.clone(), &h.base)
            .reconcile()
            .await,
    );
}

#[tokio::test]
async fn learned_cross_role_drain_consumes_small_once_and_reuses_large_remainder_with_refill_denied()
 {
    let h = retained_inventory(false).await;
    let before = group(&h).await;
    let old_active = before
        .addresses
        .iter()
        .find(|w| w.role == "ACTIVE")
        .unwrap()
        .address
        .clone();
    let retained = before
        .addresses
        .iter()
        .find(|w| w.role == "RETAINED")
        .unwrap()
        .address
        .clone();
    let small_id = before
        .reserve_claims
        .iter()
        .find(|c| c.slot == "small")
        .unwrap()
        .wallet_id
        .clone()
        .unwrap();
    let small = before
        .addresses
        .iter()
        .find(|w| w.id == small_id)
        .unwrap()
        .address
        .clone();
    for wallet in &before.addresses {
        h.f.balances.lock().unwrap().insert(
            wallet.address.clone(),
            if wallet.address == retained {
                400_000
            } else if wallet.address == small {
                2_000_000
            } else {
                8_000_000
            },
        );
    }
    publish(&h).await;
    let expensive = binding(&h, "GET");
    let cheap = binding(&h, "POST");
    *h.f.challenge.lock().unwrap() = challenge("6000000");
    assert_eq!(expensive.invoke(&Default::default()).await.unwrap(), "paid");
    assert_eq!(payer(&h), old_active);
    super::consumption::settle_last(&h, 6_000_000).await;
    // Explicit fixture credit establishes the measured starting $8 balance after
    // genuine expensive-key learning; it is not assumed by the selector.
    h.f.balances
        .lock()
        .unwrap()
        .insert(old_active.clone(), 8_000_000);
    publish(&h).await;
    *h.f.challenge.lock().unwrap() = challenge("100000");
    assert_eq!(cheap.invoke(&Default::default()).await.unwrap(), "paid");
    super::consumption::settle_last(&h, 100_000).await;
    for _ in 0..4 {
        let start = h.f.traces.lock().unwrap().len();
        assert_eq!(cheap.invoke(&Default::default()).await.unwrap(), "paid");
        assert_eq!(payer(&h), retained);
        assert_eq!(
            h.f.traces.lock().unwrap()[start..]
                .iter()
                .filter(|(_, kind, _)| kind == "unsigned")
                .count(),
            1
        );
        super::consumption::settle_last(&h, 100_000).await;
    }
    assert_eq!(cheap.invoke(&Default::default()).await.unwrap(), "paid");
    assert_eq!(payer(&h), small);
    super::consumption::settle_last(&h, 100_000).await;
    let consumed = group(&h).await;
    let claim = consumed
        .reserve_claims
        .iter()
        .find(|c| c.slot == "small")
        .unwrap();
    assert_eq!(
        claim.generation,
        before
            .reserve_claims
            .iter()
            .find(|c| c.slot == "small")
            .unwrap()
            .generation
            + 1
    );
    assert!(claim.wallet_id.is_none());
    let pool = h.pool.clone();
    assert!(
        h.store
            .call(move |s| s.allocate_capacity_kind(
                &pool,
                x402_treazury::rotation::store::AllocationKind::Small,
                true
            ))
            .await
            .unwrap_err()
            .to_string()
            .contains("funding_disabled")
    );
    *h.f.challenge.lock().unwrap() = challenge("6000000");
    let start = h.f.traces.lock().unwrap().len();
    assert_eq!(expensive.invoke(&Default::default()).await.unwrap(), "paid");
    assert_eq!(payer(&h), old_active);
    assert_eq!(
        h.f.traces.lock().unwrap()[start..]
            .iter()
            .filter(|(_, kind, _)| kind == "unsigned")
            .count(),
        1
    );
    super::consumption::settle_last(&h, 6_000_000).await;
    let after = group(&h).await;
    assert_eq!(
        after
            .addresses
            .iter()
            .find(|w| w.role == "ACTIVE")
            .unwrap()
            .address,
        small
    );
    assert_eq!(before.addresses.len(), after.addresses.len());
    drop(expensive);
    drop(cheap);
    h.close().await;
}

#[tokio::test]
async fn learned_cheap_suffix_leaves_large_active_for_smaller_retained_capacity() {
    let h = retained_inventory(true).await;
    let before = group(&h).await;
    let large_id = before
        .reserve_claims
        .iter()
        .find(|c| c.slot == "large")
        .unwrap()
        .wallet_id
        .clone()
        .unwrap();
    let large = before
        .addresses
        .iter()
        .find(|w| w.id == large_id)
        .unwrap()
        .address
        .clone();
    h.f.balances
        .lock()
        .unwrap()
        .insert(large.clone(), 8_000_000);
    publish(&h).await;
    let cheap = binding(&h, "POST");
    let expensive = binding(&h, "GET");
    *h.f.challenge.lock().unwrap() = challenge("100000");
    assert_eq!(cheap.invoke(&Default::default()).await.unwrap(), "paid");
    super::consumption::settle_last(&h, 100_000).await;
    *h.f.challenge.lock().unwrap() = challenge("6000000");
    assert_eq!(expensive.invoke(&Default::default()).await.unwrap(), "paid");
    assert_eq!(payer(&h), large);
    super::consumption::settle_last(&h, 6_000_000).await;
    *h.f.challenge.lock().unwrap() = challenge("100000");
    let retained: Vec<_> = before
        .addresses
        .iter()
        .filter(|w| w.role == "RETAINED")
        .map(|w| (w.id.clone(), w.address.clone()))
        .collect();
    for _ in 0..12 {
        let expected = retained
            .iter()
            .filter_map(|(id, address)| {
                let balance = *h.f.balances.lock().unwrap().get(address).unwrap();
                (balance >= 100_000).then_some((balance, id, address))
            })
            .min()
            .unwrap();
        let start = h.f.traces.lock().unwrap().len();
        assert_eq!(cheap.invoke(&Default::default()).await.unwrap(), "paid");
        assert_eq!(payer(&h), *expected.2);
        assert_eq!(
            h.f.traces.lock().unwrap()[start..]
                .iter()
                .filter(|(_, kind, _)| kind == "unsigned")
                .count(),
            1
        );
        super::consumption::settle_last(&h, 100_000).await;
    }
    let after = group(&h).await;
    assert_eq!(
        after
            .addresses
            .iter()
            .find(|w| w.role == "ACTIVE")
            .unwrap()
            .address,
        large
    );
    assert_eq!(
        *h.f.balances.lock().unwrap().get(&large).unwrap(),
        2_000_000
    );
    assert_eq!(before.addresses.len(), after.addresses.len());
    drop(cheap);
    drop(expensive);
    h.close().await;
}
