//! Shared bounds protect query memory, active sockets and queued RPC callers.
//! Historical subsets leave one slot at each boundary for current evidence.
//! They never expire useful work or confer financial authority.
use anyhow::{Context, Result};
use tokio::sync::{Semaphore, SemaphorePermit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Current,
    Historical,
}
const QUERY_LIMIT: usize = 64;
const CALLER_LIMIT: usize = 128;
const ACTIVE_LIMIT: usize = 32;
static QUERIES: Semaphore = Semaphore::const_new(QUERY_LIMIT);
static RPC_CALLERS: Semaphore = Semaphore::const_new(CALLER_LIMIT);
static ACTIVE_RPCS: Semaphore = Semaphore::const_new(ACTIVE_LIMIT);
static HISTORICAL_QUERIES: Semaphore = Semaphore::const_new(QUERY_LIMIT - 1);
static HISTORICAL_CALLERS: Semaphore = Semaphore::const_new(CALLER_LIMIT - 1);
static HISTORICAL_ACTIVE: Semaphore = Semaphore::const_new(ACTIVE_LIMIT - 1);

pub(super) struct QueryPermit {
    _historical: Option<SemaphorePermit<'static>>,
    _aggregate: SemaphorePermit<'static>,
}
pub(super) fn query(class: Class) -> Result<QueryPermit> {
    let historical = if class == Class::Historical {
        Some(
            HISTORICAL_QUERIES
                .try_acquire()
                .context("Base historical query capacity unavailable")?,
        )
    } else {
        None
    };
    let aggregate = QUERIES
        .try_acquire()
        .context("Base query capacity unavailable")?;
    Ok(QueryPermit {
        _historical: historical,
        _aggregate: aggregate,
    })
}

pub(super) struct RpcPermits {
    _historical_caller: Option<SemaphorePermit<'static>>,
    _caller: SemaphorePermit<'static>,
    _historical_active: Option<SemaphorePermit<'static>>,
    _active: SemaphorePermit<'static>,
}
pub(super) async fn rpc(class: Class) -> Result<RpcPermits> {
    // Bound waiting owners before awaiting sockets. Historical waiters cannot
    // consume the last aggregate caller or active permit while queued.
    let historical_caller = if class == Class::Historical {
        Some(
            HISTORICAL_CALLERS
                .try_acquire()
                .context("Base historical RPC queue capacity unavailable")?,
        )
    } else {
        None
    };
    let caller = RPC_CALLERS
        .try_acquire()
        .context("Base RPC queue capacity unavailable")?;
    let historical_active = if class == Class::Historical {
        Some(
            HISTORICAL_ACTIVE
                .acquire()
                .await
                .context("Base historical RPC owner closed")?,
        )
    } else {
        None
    };
    let active = ACTIVE_RPCS
        .acquire()
        .await
        .context("Base RPC owner closed")?;
    Ok(RpcPermits {
        _historical_caller: historical_caller,
        _caller: caller,
        _historical_active: historical_active,
        _active: active,
    })
}

#[cfg(test)]
pub(crate) fn available_queries() -> usize {
    QUERIES.available_permits()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_subsets_preserve_current_capacity_at_all_three_boundaries() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "rotation::base::resources::tests::partition_child",
                "--ignored",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[tokio::test]
    #[ignore = "isolated process-wide resource accounting"]
    async fn partition_child() {
        let history: Vec<_> = (0..QUERY_LIMIT - 1)
            .map(|_| query(Class::Historical).unwrap())
            .collect();
        assert!(query(Class::Historical).is_err());
        let current = query(Class::Current).unwrap();
        assert!(query(Class::Current).is_err());
        let mut calls = Vec::new();
        for _ in 0..CALLER_LIMIT - 1 {
            calls.push(tokio::spawn(async {
                let _permits = rpc(Class::Historical).await.unwrap();
                std::future::pending::<()>().await;
            }));
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while RPC_CALLERS.available_permits() != 1 || ACTIVE_RPCS.available_permits() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(rpc(Class::Historical).await.is_err());
        let current_rpc =
            tokio::time::timeout(std::time::Duration::from_secs(1), rpc(Class::Current))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(RPC_CALLERS.available_permits(), 0);
        assert_eq!(ACTIVE_RPCS.available_permits(), 0);
        assert!(rpc(Class::Current).await.is_err());
        // Cancellation of waiting and active unsigned calls returns both subsets
        // and aggregate permits; no second owner or timeout is necessary.
        for call in &calls {
            call.abort();
        }
        for call in calls {
            assert!(call.await.unwrap_err().is_cancelled());
        }
        drop(current_rpc);
        drop(current);
        drop(history);
        assert_eq!(QUERIES.available_permits(), QUERY_LIMIT);
        assert_eq!(RPC_CALLERS.available_permits(), CALLER_LIMIT);
        assert_eq!(ACTIVE_RPCS.available_permits(), ACTIVE_LIMIT);
        assert_eq!(HISTORICAL_QUERIES.available_permits(), QUERY_LIMIT - 1);
        assert_eq!(HISTORICAL_CALLERS.available_permits(), CALLER_LIMIT - 1);
        assert_eq!(HISTORICAL_ACTIVE.available_permits(), ACTIVE_LIMIT - 1);
    }
}
