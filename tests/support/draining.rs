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
    // Establish ACTIVE through a real first payment before exercising retained history.
    super::consumption::paid_and_settled(&h, 1).await;
    super::consumption::fill(&h, &mut worker, &mut instant).await;
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
