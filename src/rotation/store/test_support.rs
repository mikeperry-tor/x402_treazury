//! Explicit unfunded fixture setup; no synthetic readiness or payment authority.
use super::*;
pub(crate) fn reserves(target: &str) -> crate::rotation::config::ReservesConfig {
    crate::rotation::config::ReservesConfig {
        small: crate::rotation::config::ReserveConfig {
            funding_amount_usdc: target.into(),
            max_funding_amount_usdc: None,
        },
        large: None,
    }
}
pub(crate) fn declare_and_allocate(
    store: &mut Store,
    name: &str,
    target: &str,
    cap: Option<U256>,
    tolerance: u64,
) -> Result<String> {
    let pool = store.ensure_capacity_group(name, &reserves(target), cap, tolerance)?;
    store.allocate_capacity_kind(&pool, AllocationKind::Initial, true)?;
    store.allocate_capacity_kind(&pool, AllocationKind::Small, true)?;
    Ok(pool)
}
