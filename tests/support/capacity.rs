#![allow(dead_code, unused_macros, unused_imports)]
//! Explicit unfunded setup shared by integration fixtures. Never publishes D.
use x402_treazury::rotation::config::{ReserveConfig, ReservesConfig};
pub(crate) fn small_reserve(target: &str) -> ReservesConfig {
    ReservesConfig {
        small: ReserveConfig {
            funding_amount_usdc: target.into(),
            max_funding_amount_usdc: None,
        },
        large: None,
    }
}
// Macro permits normal receiver reborrowing for both Store and &mut Store.
// Every caller supplies the cap and remainder tolerance explicitly.
macro_rules! declare_and_allocate {
    ($store:expr, $name:expr, $reserves:expr, $cap:expr, $tolerance:expr) => {{
        (|| -> anyhow::Result<String> {
            let pool = $store.ensure_capacity_group($name, &$reserves, $cap, $tolerance)?;
            $store.allocate_capacity_kind(
                &pool,
                x402_treazury::rotation::store::AllocationKind::Initial,
                true,
            )?;
            $store.allocate_capacity_kind(
                &pool,
                x402_treazury::rotation::store::AllocationKind::Small,
                true,
            )?;
            Ok(pool)
        })()
    }};
}
pub(crate) use declare_and_allocate;

#[cfg(feature = "zcash")]
pub(crate) async fn treasury_pair(
    owner: &x402_treazury::treasury::Treasury,
    name: &str,
    target: &str,
    cap: Option<alloy_primitives::U256>,
    tolerance: u64,
) -> anyhow::Result<String> {
    let pool = owner
        .ensure_capacity_group(name.into(), small_reserve(target), cap, tolerance)
        .await?;
    owner
        .allocate_capacity_kind(
            pool.clone(),
            x402_treazury::rotation::store::AllocationKind::Initial,
            true,
        )
        .await?;
    owner
        .allocate_capacity_kind(
            pool.clone(),
            x402_treazury::rotation::store::AllocationKind::Small,
            true,
        )
        .await?;
    Ok(pool)
}
