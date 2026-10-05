//! Pure episode accounting; the runtime owns task cancellation and transport slots.
use super::{Config, sampling::Unit};
use anyhow::{Result, ensure};
use rand::Rng;
use std::time::Duration;
use tokio::time::Instant;

pub struct Episode {
    pub started: Instant,
    pub deadline: Instant,
    pub first_dispatch: Instant,
    pub tail_deadline: Option<Instant>,
    pub target: u64,
    pub received: u64,
    pub reserved: u64,
    pub requests: usize,
    pub streams: usize,
    pub calls: usize,
    pub padding: u64,
    pub stopped: Option<&'static str>,
    tail: Duration,
}
impl Episode {
    pub fn new(config: &Config, now: Instant, rng: &mut impl Rng) -> Result<Self> {
        let target = config.volume.sample(Unit::Bytes, rng)?;
        let start = config.start_delay.sample(Unit::Milliseconds, rng)?;
        let tail = config.tail.sample(Unit::Milliseconds, rng)?;
        Ok(Self {
            started: now,
            deadline: now + Duration::from_millis(config.max_episode_ms),
            first_dispatch: now + Duration::from_millis(start),
            tail_deadline: None,
            target,
            received: 0,
            reserved: 0,
            requests: 0,
            streams: 0,
            calls: 1,
            padding: 0,
            stopped: None,
            tail: Duration::from_millis(tail),
        })
    }
    pub fn attach(&mut self) {
        self.calls += 1;
        self.tail_deadline = None;
    }
    pub fn detach(&mut self, now: Instant) {
        self.calls = self.calls.saturating_sub(1);
        if self.calls == 0 {
            self.tail_deadline = Some((now + self.tail).min(self.deadline));
        }
    }
    pub fn stop(&mut self, reason: &'static str) {
        self.stopped = Some(reason);
    }
    pub fn exhausted(&self, now: Instant) -> bool {
        self.stopped.is_some()
            || now >= self.deadline
            || self.tail_deadline.is_some_and(|t| now >= t)
    }
    pub fn reserve(&mut self, config: &Config, now: Instant, requested: u64) -> Result<u64> {
        ensure!(!self.exhausted(now), "cover_episode_stopped");
        ensure!(now >= self.first_dispatch, "cover_start_delay");
        ensure!(
            self.streams < config.concurrency,
            "cover_episode_stream_limit"
        );
        ensure!(
            self.requests < config.max_requests_per_episode,
            "cover_episode_request_limit"
        );
        let available = self
            .target
            .saturating_sub(self.received)
            .saturating_sub(self.reserved);
        ensure!(available > 0, "cover_target_reached");
        let bytes = requested.min(available);
        ensure!(bytes > 0, "cover_empty_range");
        self.reserved += bytes;
        self.requests += 1;
        self.streams += 1;
        Ok(bytes)
    }
    /// Failed pre-dispatch reservations undo the request; dispatched failures retain it.
    pub fn finish(&mut self, reserved: u64, received: u64, dispatched: bool) {
        self.reserved = self.reserved.saturating_sub(reserved);
        self.streams = self.streams.saturating_sub(1);
        self.received = self.received.saturating_add(received);
        if !dispatched {
            self.requests = self.requests.saturating_sub(1);
        }
    }
    pub fn reserve_padding(&mut self, config: &Config, now: Instant, bytes: u64) -> Result<()> {
        ensure!(!self.exhausted(now), "cover_episode_stopped");
        let p = config
            .padding
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cover_padding_disabled"))?;
        ensure!(
            bytes <= p.max_value_bytes_per_episode.saturating_sub(self.padding),
            "cover_episode_padding_limit"
        );
        self.padding += bytes;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    #[test]
    fn overlapping_calls_do_not_reset_deadline_or_target() {
        let mut c = crate::cover::tests::example_config();
        c.start_delay = toml::from_str("distribution='uniform'\nmin_ms=0\nmax_ms=0").unwrap();
        let now = Instant::now();
        let mut e = Episode::new(&c, now, &mut StdRng::seed_from_u64(4)).unwrap();
        let deadline = e.deadline;
        let target = e.target;
        e.attach();
        let a = e.reserve(&c, now, 10000).unwrap();
        let b = e.reserve(&c, now, 10000).unwrap();
        assert!(e.reserve(&c, now, 1).is_err());
        assert!(a + b <= target);
        e.finish(a, 500, true);
        e.finish(b, 0, false);
        assert_eq!(e.received, 500);
        assert_eq!(e.requests, 1);
        assert_eq!(e.reserved, 0);
        e.detach(now);
        assert!(e.tail_deadline.is_none());
        e.detach(now);
        assert!(e.tail_deadline.is_some());
        e.attach();
        assert_eq!(e.deadline, deadline);
        assert_eq!(e.target, target);
        assert!(e.reserve(&c, deadline, 1).is_err());
        assert_eq!(e.calls, 1);
    }
}
