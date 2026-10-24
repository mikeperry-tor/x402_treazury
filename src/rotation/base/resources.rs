//! Shared bounds protect query memory, active sockets and queued RPC callers.
//! They never expire useful work or confer financial authority.
use anyhow::{Context, Result};
use tokio::sync::{Semaphore, SemaphorePermit};

static QUERIES: Semaphore = Semaphore::const_new(64);
static RPC_CALLERS: Semaphore = Semaphore::const_new(128);
static ACTIVE_RPCS: Semaphore = Semaphore::const_new(32);

pub(super) fn query() -> Result<SemaphorePermit<'static>> {
    crate::limits::try_capacity(&QUERIES, "base_query_busy", 64)
        .context("Base query capacity unavailable")
}

pub(super) fn rpc_caller() -> Result<SemaphorePermit<'static>> {
    // Admission precedes waiting, preventing an unbounded semaphore waiter list.
    crate::limits::try_capacity(&RPC_CALLERS, "base_rpc_callers_busy", 128)
        .context("Base RPC queue capacity unavailable")
}

pub(super) async fn active_rpc() -> Result<SemaphorePermit<'static>> {
    crate::limits::acquire_capacity(&ACTIVE_RPCS, "base_active_rpcs_busy", Some(32), "hardcoded")
        .await
        .context("Base RPC owner closed")
}
