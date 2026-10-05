//! Identity-bound episode registry. Ownership outlives HTTP pool eviction.
use super::{Config, Limits, episode::Episode};
use crate::network::{HttpPolicy, IsolationId};
use anyhow::{Result, ensure};
use rand::Rng;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::time::Instant;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Owner {
    pub runtime: tokio::runtime::Id,
    pub identity: IsolationId,
    pub origin: String,
    pub transport: HttpPolicy,
    pub public_only: bool,
    pub timeout_ms: u64,
}
#[derive(Clone, Debug)]
pub enum Capability {
    Unknown,
    Checking,
    Available {
        length: u64,
        validator: Option<String>,
    },
    Unavailable(&'static str),
}
pub struct OwnerState {
    pub config: Arc<Config>,
    pub capability: Capability,
    pub episode: Option<Arc<Mutex<Episode>>>,
}
pub struct Registry {
    pub owners: HashMap<Owner, OwnerState>,
    limits: Limits,
}
impl Registry {
    pub fn new(limits: Limits) -> Self {
        Self {
            owners: HashMap::new(),
            limits,
        }
    }
    pub fn attach(
        &mut self,
        owner: Owner,
        config: Arc<Config>,
        now: Instant,
        rng: &mut impl Rng,
    ) -> Result<(Arc<Mutex<Episode>>, bool)> {
        if let Some(state) = self.owners.get(&owner) {
            ensure!(
                *state.config == *config,
                "cover_owner_configuration_conflict"
            );
            if let Some(episode) = &state.episode {
                let mut e = episode.lock().expect("cover episode poisoned");
                // Keep stopped episodes attached until all real calls and stream cleanup end.
                if e.calls > 0 || e.streams > 0 || !e.exhausted(now) {
                    e.attach();
                    return Ok((episode.clone(), false));
                }
            }
        } else {
            ensure!(
                self.owners.len() < self.limits.max_owners,
                "cover_owner_limit"
            );
        }
        let active = self
            .owners
            .values()
            .filter(|state| {
                state.episode.as_ref().is_some_and(|e| {
                    let e = e.lock().unwrap();
                    e.calls > 0 || e.streams > 0 || !e.exhausted(now)
                })
            })
            .count();
        ensure!(
            active < self.limits.max_active_episodes,
            "cover_active_episode_limit"
        );
        let e = Arc::new(Mutex::new(Episode::new(&config, now, rng)?));
        let state = self.owners.entry(owner).or_insert_with(|| OwnerState {
            config,
            capability: Capability::Unknown,
            episode: None,
        });
        state.episode = Some(e.clone());
        Ok((e, true))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    #[tokio::test]
    async fn merge_conflicts_limits_and_negative_cache_survive_episodes() {
        let mut r = Registry::new(Limits {
            max_active_episodes: 1,
            max_owners: 2,
            ..Default::default()
        });
        let owner = Owner {
            runtime: tokio::runtime::Handle::current().id(),
            identity: IsolationId::discovery("https://example.com").unwrap(),
            origin: "https://example.com".into(),
            transport: Default::default(),
            public_only: false,
            timeout_ms: 1000,
        };
        let config = Arc::new(crate::cover::tests::example_config());
        let mut rng = StdRng::seed_from_u64(4);
        let now = Instant::now();
        let (a, fresh) = r
            .attach(owner.clone(), config.clone(), now, &mut rng)
            .unwrap();
        assert!(fresh);
        let (b, fresh) = r
            .attach(owner.clone(), config.clone(), now, &mut rng)
            .unwrap();
        assert!(!fresh && Arc::ptr_eq(&a, &b));
        assert_eq!(a.lock().unwrap().calls, 2);
        let mut changed = (*config).clone();
        changed.concurrency = 1;
        assert!(
            r.attach(owner.clone(), Arc::new(changed), now, &mut rng)
                .is_err()
        );
        let mut other = owner.clone();
        other.identity = IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
        assert!(r.attach(other, config.clone(), now, &mut rng).is_err());
        r.owners.get_mut(&owner).unwrap().capability = Capability::Unavailable("range_ignored");
        {
            let mut e = a.lock().unwrap();
            e.detach(now);
            e.detach(now);
            e.stop("deadline");
        }
        let (c, fresh) = r.attach(owner.clone(), config, now, &mut rng).unwrap();
        assert!(fresh && !Arc::ptr_eq(&a, &c));
        assert!(matches!(
            r.owners[&owner].capability,
            Capability::Unavailable("range_ignored")
        ));
    }
}
