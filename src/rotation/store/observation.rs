//! Shared scalar observation ownership. Cadence grants no financial authority.
use super::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

pub(crate) const CADENCE: Duration = Duration::from_secs(60);
#[cfg(test)]
mod tests;
#[derive(Default)]
pub(super) struct Observation {
    due: Option<Instant>,
    failures: u32,
    value: Option<U256>,
}
type Owner = Arc<tokio::sync::Mutex<Observation>>;
pub(super) type Observers = Arc<Mutex<BTreeMap<String, Owner>>>;
impl StoreHandle {
    async fn balance_acquisition_owner(&self, wallet: &str) -> Owner {
        let (owner, expired_failures) = {
            let mut observers = self.balance_observers.lock().expect("balance observers");
            let mut expired_failures = Vec::new();
            observers.retain(|id, owner| {
                if Arc::strong_count(owner) == 1
                    && let Ok(state) = owner.try_lock()
                    && state.due.is_none_or(|due| due <= Instant::now())
                {
                    if state.failures == 0 {
                        return false;
                    }
                    expired_failures.push(id.clone());
                }
                true
            });
            let owner = observers.entry(wallet.to_owned()).or_default().clone();
            (owner, expired_failures)
        };
        if expired_failures.is_empty() {
            return owner;
        }
        // Live failures own their consecutive backoff even after becoming
        // due. Only a terminal wallet role permits discarding that history.
        // Cleanup is best effort and never gates the actual observation.
        let Ok(retired) = self
            .call(move |s| {
                let mut query =
                    s.db.prepare_cached("SELECT role='RETIRED' FROM wallets WHERE id=?1")?;
                let mut retired = Vec::new();
                for id in expired_failures {
                    if query
                        .query_row([&id], |r| r.get::<_, bool>(0))
                        .optional()?
                        .unwrap_or(false)
                    {
                        retired.push(id);
                    }
                }
                Ok(retired)
            })
            .await
        else {
            return owner;
        };
        let mut observers = self.balance_observers.lock().expect("balance observers");
        for id in retired {
            let removable = observers.get(&id).is_some_and(|owner| {
                Arc::strong_count(owner) == 1
                    && owner
                        .try_lock()
                        .is_ok_and(|state| state.due.is_some_and(|due| due <= Instant::now()))
            });
            if removable {
                observers.remove(&id);
            }
        }
        owner
    }
    fn balance_owner(&self, wallet: &str) -> Owner {
        self.balance_observers
            .lock()
            .expect("balance observers")
            .entry(wallet.to_owned())
            .or_default()
            .clone()
    }
    /// An active acquisition retains the wallet mutex; callers can wait for its
    /// completion without starting another request. Scheduling checks never probe.
    pub(crate) async fn balance_due(&self, wallet: &str) -> Instant {
        self.balance_owner(wallet)
            .try_lock()
            .ok()
            .and_then(|state| state.due)
            .unwrap_or_else(Instant::now)
    }
    /// Cancellation drops only the waiter. The accepted unsigned acquisition and
    /// its durable result finish under the same wallet owner. Calls arriving in
    /// the cooldown reuse its outcome; pending refresh requests remain pending.
    pub(crate) async fn observe_balance(
        &self,
        pool: String,
        wallet: String,
        base: super::super::base::BaseRpc,
    ) -> Result<U256> {
        let store = self.clone();
        let owner = self.balance_acquisition_owner(&wallet).await;
        tokio::spawn(async move {
            let mut observation = owner.lock_owned().await;
            if observation.due.is_some_and(|due| Instant::now() < due) {
                return observation
                    .value
                    .context("advisory balance observation unavailable");
            }
            let (group, id) = (pool.clone(), wallet.clone());
            let result = match store.call(move |s| s.begin_balance(&group, &id)).await {
                Ok(address) => base.current_balance(&address).await,
                Err(error) => Err(error),
            };
            let value = result.as_ref().ok().copied();
            let published = store
                .call(move |s| s.finish_balance(&pool, &wallet, value))
                .await;
            observation.failures = if result.is_ok() && published.is_ok() {
                0
            } else {
                observation.failures.saturating_add(1)
            };
            let seconds = if observation.failures == 0 {
                CADENCE.as_secs()
            } else {
                CADENCE
                    .as_secs()
                    .saturating_mul(1u64 << observation.failures.saturating_sub(1).min(3))
                    .min(300)
            };
            observation.due = Some(Instant::now() + Duration::from_secs(seconds));
            observation.value = if published.is_ok() { value } else { None };
            published?;
            result
        })
        .await
        .context("balance observation owner stopped")?
    }
}
