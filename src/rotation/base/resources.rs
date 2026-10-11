//! Shared bounds protect query memory, active sockets and queued RPC callers.
//! They never expire useful work or confer financial authority.
use anyhow::{Context, Result};
use tokio::sync::{Semaphore, SemaphorePermit};

static QUERIES: Semaphore = Semaphore::const_new(64);
static RPC_CALLERS: Semaphore = Semaphore::const_new(128);
static ACTIVE_RPCS: Semaphore = Semaphore::const_new(32);

pub(super) fn query() -> Result<SemaphorePermit<'static>> {
    QUERIES
        .try_acquire()
        .context("Base query capacity unavailable")
}

pub(super) async fn rpc() -> Result<(SemaphorePermit<'static>, SemaphorePermit<'static>)> {
    // Admission precedes waiting, preventing an unbounded semaphore waiter list.
    let caller = RPC_CALLERS
        .try_acquire()
        .context("Base RPC queue capacity unavailable")?;
    let active = ACTIVE_RPCS
        .acquire()
        .await
        .context("Base RPC owner closed")?;
    Ok((caller, active))
}
