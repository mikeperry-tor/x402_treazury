//! Wallet chain repair uses the single bounded canonical replacement owner.
//! Original proofs remain immutable audit evidence; an undisputed journal cannot
//! explain a chain block created by the supported production invalidation paths.
use super::*;
use crate::rotation::base::BaseRpc;

pub(crate) async fn repair_wallet(
    store: StoreHandle,
    base: BaseRpc,
    pool: String,
    wallet: String,
) -> Result<bool> {
    super::replacement::replace_wallet(store, base, pool, wallet).await
}

#[cfg(test)]
mod tests;
