//! Atomic process reservations. Outstanding work never expires out of accounting.
use super::Limits;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    StreamLimit,
    RequestLimit,
    BodyLimit,
    PaddingLimit,
    HistoryLimit,
}
impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::StreamLimit => "cover_stream_limit",
            Self::RequestLimit => "cover_request_limit",
            Self::BodyLimit => "cover_body_limit",
            Self::PaddingLimit => "cover_padding_limit",
            Self::HistoryLimit => "cover_history_limit",
        }
    }
}
#[derive(Clone, Copy)]
struct Entry {
    body: u64,
    padding: u64,
    range: bool,
    dispatched: bool,
    completed: Option<Instant>,
}
struct State {
    next: u64,
    entries: BTreeMap<u64, Entry>,
}
#[derive(Clone)]
pub struct Budget {
    limits: Limits,
    state: Arc<Mutex<State>>,
}
pub struct Reservation {
    budget: Budget,
    id: u64,
    received: Arc<std::sync::atomic::AtomicU64>,
    finished: bool,
}
impl Budget {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            state: Arc::new(Mutex::new(State {
                next: 0,
                entries: BTreeMap::new(),
            })),
        }
    }
    pub fn reserve(
        &self,
        now: Instant,
        body: u64,
        padding: u64,
        range: bool,
    ) -> Result<Reservation, Refusal> {
        let mut state = self.state.lock().expect("cover budget poisoned");
        let window = Duration::from_millis(self.limits.window_ms);
        state.entries.retain(|_, e| {
            e.completed
                .is_none_or(|t| now.saturating_duration_since(t) < window)
        });
        // Bound metadata even for padding-only requests. Refusal never blocks real calls.
        if state.entries.len() >= self.limits.max_requests_per_window.saturating_mul(2) {
            return Err(Refusal::HistoryLimit);
        }
        let mut streams = 0;
        let mut requests = 0;
        let mut reserved_body = 0u64;
        let mut reserved_padding = 0u64;
        for e in state.entries.values() {
            if e.range {
                requests += 1;
                if e.completed.is_none() {
                    streams += 1;
                }
            }
            reserved_body = reserved_body.saturating_add(e.body);
            reserved_padding = reserved_padding.saturating_add(e.padding);
        }
        if range && streams >= self.limits.max_active_streams {
            return Err(Refusal::StreamLimit);
        }
        if range && requests >= self.limits.max_requests_per_window {
            return Err(Refusal::RequestLimit);
        }
        if body
            > self
                .limits
                .max_cover_body_bytes_per_window
                .saturating_sub(reserved_body)
        {
            return Err(Refusal::BodyLimit);
        }
        if padding
            > self
                .limits
                .max_padding_value_bytes_per_window
                .saturating_sub(reserved_padding)
        {
            return Err(Refusal::PaddingLimit);
        }
        let id = state.next;
        state.next = state
            .next
            .checked_add(1)
            .expect("cover reservation sequence exhausted");
        state.entries.insert(
            id,
            Entry {
                body,
                padding,
                range,
                dispatched: false,
                completed: None,
            },
        );
        Ok(Reservation {
            budget: self.clone(),
            id,
            received: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            finished: false,
        })
    }
    #[cfg(test)]
    fn totals(&self) -> (usize, u64, u64) {
        let s = self.state.lock().unwrap();
        (
            s.entries.len(),
            s.entries.values().map(|e| e.body).sum(),
            s.entries.values().map(|e| e.padding).sum(),
        )
    }
}
impl Reservation {
    pub fn counter(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.received.clone()
    }
    pub fn dispatched(&mut self) {
        self.budget
            .state
            .lock()
            .unwrap()
            .entries
            .get_mut(&self.id)
            .unwrap()
            .dispatched = true;
    }
    /// Include every observed chunk, including the chunk that crossed a read bound.
    pub fn received(&mut self, bytes: u64) {
        self.received
            .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn finish(mut self, now: Instant) {
        self.complete(now);
    }
    fn complete(&mut self, now: Instant) {
        if self.finished {
            return;
        }
        self.finished = true;
        let mut state = self.budget.state.lock().expect("cover budget poisoned");
        let e = state
            .entries
            .get_mut(&self.id)
            .expect("outstanding reservation retained");
        if e.dispatched {
            e.body = self.received.load(std::sync::atomic::Ordering::Relaxed);
            e.completed = Some(now);
        } else {
            state.entries.remove(&self.id);
        }
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.complete(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> Limits {
        Limits {
            max_active_streams: 2,
            max_requests_per_window: 4,
            max_cover_body_bytes_per_window: 100,
            max_padding_value_bytes_per_window: 10,
            window_ms: 10,
            ..Default::default()
        }
    }
    #[test]
    fn expiry_never_forgets_outstanding_and_drop_releases_unused_only() {
        let b = Budget::new(limits());
        let t = Instant::now();
        let mut a = b.reserve(t, 80, 2, true).unwrap();
        a.dispatched();
        a.received(30);
        assert!(matches!(
            b.reserve(t + Duration::from_secs(1), 30, 0, true),
            Err(Refusal::BodyLimit)
        ));
        a.finish(t + Duration::from_secs(1));
        assert_eq!(b.totals(), (1, 30, 2));
        let c = b
            .reserve(t + Duration::from_millis(1001), 70, 8, true)
            .unwrap();
        assert!(matches!(
            b.reserve(t + Duration::from_millis(1001), 0, 1, false),
            Err(Refusal::PaddingLimit)
        ));
        drop(c);
        assert_eq!(b.totals(), (1, 30, 2));
        drop(
            b.reserve(t + Duration::from_millis(1010), 100, 10, true)
                .unwrap(),
        );
        assert_eq!(b.totals(), (0, 0, 0));
    }
    #[test]
    fn concurrent_reservations_cannot_overbook() {
        let b = Budget::new(limits());
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let done = Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let b = b.clone();
                let barrier = barrier.clone();
                let done = done.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let r = b.reserve(Instant::now(), 50, 0, true);
                    done.wait();
                    r.is_ok()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .filter(|h| h.thread().id() != std::thread::current().id())
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>(),
            2
        );
    }
    #[test]
    fn canceled_dispatched_requests_and_overruns_remain_charged() {
        let b = Budget::new(limits());
        let t = Instant::now();
        {
            let mut a = b.reserve(t, 10, 0, true).unwrap();
            a.dispatched();
            a.received(120);
        }
        assert_eq!(b.totals(), (1, 120, 0));
        assert!(matches!(b.reserve(t, 1, 0, true), Err(Refusal::BodyLimit)));
    }
}
