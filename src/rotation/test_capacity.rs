#![allow(dead_code, unused_macros, unused_imports)]
//! Explicit unfunded setup shared by integration fixtures. Never publishes D.
use crate::rotation::config::{ReserveConfig, ReservesConfig};
pub(crate) fn small_reserve(target: &str) -> ReservesConfig {
    ReservesConfig {
        small: ReserveConfig {
            funding_amount_usdc: target.into(),
            max_funding_amount_usdc: None,
        },
        large: None,
    }
}
pub(crate) fn two_reserves(target: &str) -> ReservesConfig {
    let mut config = small_reserve(target);
    config.large = Some(config.small.clone());
    config
}
// Macro permits normal receiver reborrowing for both Store and &mut Store.
// Every caller supplies the cap and remainder tolerance explicitly.
macro_rules! declare_and_allocate {
    ($store:expr, $name:expr, $reserves:expr, $cap:expr, $tolerance:expr) => {{
        (|| -> anyhow::Result<String> {
            let pool = $store.ensure_capacity_group($name, &$reserves, $cap, $tolerance)?;
            $store.allocate_capacity_kind(
                &pool,
                crate::rotation::store::AllocationKind::Small,
                true,
            )?;
            $store.allocate_capacity_kind(
                &pool,
                crate::rotation::store::AllocationKind::Large,
                true,
            )?;
            Ok(pool)
        })()
    }};
}
pub(crate) use declare_and_allocate;

#[cfg(feature = "zcash")]
pub(crate) async fn treasury_pair(
    owner: &crate::treasury::Treasury,
    name: &str,
    target: &str,
    cap: Option<alloy_primitives::U256>,
    tolerance: u64,
) -> anyhow::Result<String> {
    let pool = owner
        .ensure_capacity_group(name.into(), two_reserves(target), cap, tolerance)
        .await?;
    owner
        .allocate_capacity_kind(
            pool.clone(),
            crate::rotation::store::AllocationKind::Small,
            true,
        )
        .await?;
    owner
        .allocate_capacity_kind(
            pool.clone(),
            crate::rotation::store::AllocationKind::Large,
            true,
        )
        .await?;
    Ok(pool)
}
