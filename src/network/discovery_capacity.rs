//! Discovery admission counts active Tor isolation scopes, shared by all their requests.
use super::{HttpTransport, IsolationId};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{AcquireError, OnceCell, OwnedSemaphorePermit, Semaphore};

pub struct DiscoveryCapacity {
    slots: Arc<Semaphore>,
    scopes: Mutex<HashMap<IsolationId, Weak<Scope>>>,
}

struct Scope {
    owner: Weak<DiscoveryCapacity>,
    key: IsolationId,
    permit: OnceCell<OwnedSemaphorePermit>,
}

pub(crate) struct DiscoveryPermit {
    _scope: Arc<Scope>,
}

impl DiscoveryCapacity {
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(limit)),
            scopes: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
        transport: &HttpTransport,
    ) -> Result<Option<DiscoveryPermit>, AcquireError> {
        if !transport.uses_tor() {
            return Ok(None);
        }
        let scope = {
            let mut scopes = self.scopes.lock().expect("discovery capacity poisoned");
            if let Some(scope) = scopes.get(&transport.scope).and_then(Weak::upgrade) {
                scope
            } else {
                let scope = Arc::new(Scope {
                    owner: Arc::downgrade(self),
                    key: transport.scope.clone(),
                    permit: OnceCell::new(),
                });
                scopes.insert(transport.scope.clone(), Arc::downgrade(&scope));
                scope
            }
        };
        // Tokio resets initialization when its initializer is cancelled or fails;
        // surviving same-scope callers can then acquire without a detached task.
        scope
            .permit
            .get_or_try_init(|| {
                crate::limits::acquire_owned_capacity(
                    self.slots.clone(),
                    "discovery_tor_scopes_busy",
                    None,
                    "startup.catalog_concurrency",
                )
            })
            .await?;
        Ok(Some(DiscoveryPermit { _scope: scope }))
    }

    #[cfg(test)]
    pub(crate) fn available_permits(&self) -> usize {
        self.slots.available_permits()
    }

    #[cfg(test)]
    pub(crate) fn live_scope_count(&self) -> usize {
        self.scopes
            .lock()
            .expect("discovery capacity poisoned")
            .values()
            .filter(|scope| scope.strong_count() != 0)
            .count()
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            let mut scopes = owner.scopes.lock().expect("discovery capacity poisoned");
            // Another caller can replace our expired weak reference before this
            // destructor obtains the lock. Only remove our own entry.
            if scopes
                .get(&self.key)
                .is_some_and(|stored| std::ptr::eq(stored.as_ptr(), self))
            {
                scopes.remove(&self.key);
            }
        }
        // The map lock has been released before the permit field is dropped.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{HttpPolicy, Mode, NetworkContext, NetworkPolicy};
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll, Waker},
    };

    fn tor() -> NetworkContext {
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some("127.0.0.1:9150".parse().unwrap()),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
    }

    fn transport(context: &NetworkContext, url: &str) -> Arc<HttpTransport> {
        context
            .http_participating(
                &IsolationId::discovery(url).unwrap(),
                url,
                None,
                true,
                HttpPolicy::default(),
            )
            .unwrap()
    }

    fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[tokio::test]
    async fn active_scope_joins_ahead_of_queued_distinct_scope() {
        let context = tor();
        let first = transport(&context, "https://first.example.com/spec");
        let alias = transport(&context, "https://first.example.com/prices");
        assert_eq!(first.scope, alias.scope);
        let second = transport(&context, "https://second.example.com/spec");
        let third = transport(&context, "https://third.example.com/spec");
        let gate = Arc::new(DiscoveryCapacity::new(1));
        let first_guard = gate.acquire(&first).await.unwrap();
        let mut second_wait = Box::pin(gate.acquire(&second));
        let mut third_wait = Box::pin(gate.acquire(&third));
        assert!(poll(second_wait.as_mut()).is_pending());
        assert!(poll(third_wait.as_mut()).is_pending());
        let alias_guard = match poll(Box::pin(gate.acquire(&alias)).as_mut()) {
            Poll::Ready(Ok(Some(guard))) => guard,
            _ => panic!("active scope must join immediately despite queued scopes"),
        };
        assert_eq!(gate.available_permits(), 0);
        drop(first_guard);
        assert!(poll(second_wait.as_mut()).is_pending());
        drop(alias_guard);
        // Distinct scopes retain the semaphore's arrival ordering.
        assert!(poll(third_wait.as_mut()).is_pending());
        let second_guard = second_wait.await.unwrap();
        assert!(poll(third_wait.as_mut()).is_pending());
        drop(second_guard);
        drop(third_wait.await.unwrap());
        assert_eq!(gate.available_permits(), 1);
        assert_eq!(gate.live_scope_count(), 0);
        assert!(gate.scopes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn same_domain_fresh_retry_scope_waits_for_capacity() {
        let context = tor();
        let initial = transport(&context, "https://first.example.com/spec");
        let retry = context.http_candidate(&initial).unwrap();
        assert_eq!(initial.origin(), retry.origin());
        assert_eq!(initial.identity, retry.identity);
        assert_ne!(initial.scope, retry.scope);
        let gate = Arc::new(DiscoveryCapacity::new(1));
        let initial_guard = gate.acquire(&initial).await.unwrap();
        let mut retry_wait = Box::pin(gate.acquire(&retry));
        assert!(poll(retry_wait.as_mut()).is_pending());
        drop(initial_guard);
        drop(retry_wait.await.unwrap());
        assert_eq!(gate.available_permits(), 1);
        assert!(gate.scopes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancelled_initializer_leaves_sibling_able_to_acquire() {
        let context = tor();
        let occupied = transport(&context, "https://occupied.example.com/spec");
        let waiting = transport(&context, "https://waiting.example.com/spec");
        let gate = Arc::new(DiscoveryCapacity::new(1));
        let occupied_guard = gate.acquire(&occupied).await.unwrap();
        let mut initializer = Box::pin(gate.acquire(&waiting));
        let mut sibling = Box::pin(gate.acquire(&waiting));
        assert!(poll(initializer.as_mut()).is_pending());
        assert!(poll(sibling.as_mut()).is_pending());
        assert_eq!(gate.live_scope_count(), 2);
        drop(initializer);
        assert!(poll(sibling.as_mut()).is_pending());
        drop(occupied_guard);
        drop(sibling.await.unwrap());
        assert_eq!(gate.available_permits(), 1);
        assert!(gate.scopes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn last_waiter_cancellation_removes_scope_and_queue_entry() {
        let context = tor();
        let occupied = transport(&context, "https://occupied.example.com/spec");
        let waiting = transport(&context, "https://waiting.example.com/spec");
        let gate = Arc::new(DiscoveryCapacity::new(1));
        let occupied_guard = gate.acquire(&occupied).await.unwrap();
        let mut cancelled = Box::pin(gate.acquire(&waiting));
        assert!(poll(cancelled.as_mut()).is_pending());
        assert_eq!(gate.live_scope_count(), 2);
        drop(cancelled);
        assert_eq!(gate.live_scope_count(), 1);
        assert_eq!(gate.scopes.lock().unwrap().len(), 1);
        drop(occupied_guard);
        assert_eq!(gate.available_permits(), 1);
        drop(gate.acquire(&waiting).await.unwrap());
        assert!(gate.scopes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn direct_transport_bypasses_saturated_tor_capacity() {
        let context = tor();
        let tor_transport = transport(&context, "https://first.example.com/spec");
        let direct = NetworkContext::new(NetworkPolicy::default()).unwrap();
        let direct_transport = transport(&direct, "https://first.example.com/spec");
        let gate = Arc::new(DiscoveryCapacity::new(1));
        let tor_guard = gate.acquire(&tor_transport).await.unwrap();
        assert!(matches!(
            poll(Box::pin(gate.acquire(&direct_transport)).as_mut()),
            Poll::Ready(Ok(None))
        ));
        assert_eq!(gate.available_permits(), 0);
        assert_eq!(gate.live_scope_count(), 1);
        drop(tor_guard);
        assert!(gate.scopes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_initialization_retains_no_scope_history() {
        let context = tor();
        let transport = transport(&context, "https://first.example.com/spec");
        let gate = Arc::new(DiscoveryCapacity::new(1));
        gate.slots.close();
        assert!(gate.acquire(&transport).await.is_err());
        assert!(gate.scopes.lock().unwrap().is_empty());
        assert!(gate.acquire(&transport).await.is_err());
        assert!(gate.scopes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn old_scope_drop_cannot_remove_replacement_weak_entry() {
        let context = tor();
        let transport = transport(&context, "https://first.example.com/spec");
        let gate = Arc::new(DiscoveryCapacity::new(1));
        let old_guard = gate.acquire(&transport).await.unwrap();
        // Model the replacement inserted after Weak::upgrade observes the old
        // scope's final Arc gone but before its destructor obtains the map lock.
        let replacement = Arc::new(Scope {
            owner: Arc::downgrade(&gate),
            key: transport.scope.clone(),
            permit: OnceCell::new(),
        });
        gate.scopes
            .lock()
            .unwrap()
            .insert(transport.scope.clone(), Arc::downgrade(&replacement));
        drop(old_guard);
        let stored = gate.scopes.lock().unwrap()[&transport.scope]
            .upgrade()
            .unwrap();
        assert!(Arc::ptr_eq(&stored, &replacement));
        assert_eq!(gate.available_permits(), 1);
        drop(stored);
        drop(replacement);
        assert!(gate.scopes.lock().unwrap().is_empty());
    }
}
