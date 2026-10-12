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
/// Two independent reserve operations for scheduler/concurrency fixtures.
pub(crate) fn declare_two_reserves(
    store: &mut Store,
    name: &str,
    target: &str,
    cap: Option<U256>,
    tolerance: u64,
) -> Result<String> {
    let mut settings = reserves(target);
    settings.large = Some(settings.small.clone());
    let pool = store.ensure_capacity_group(name, &settings, cap, tolerance)?;
    store.allocate_capacity_kind(&pool, AllocationKind::Small, true)?;
    store.allocate_capacity_kind(&pool, AllocationKind::Large, true)?;
    Ok(pool)
}
