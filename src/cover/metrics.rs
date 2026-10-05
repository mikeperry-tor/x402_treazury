//! Aggregate operator evidence only; never returned by the agent status tool.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    pub episodes: u64,
    pub range_requests: u64,
    pub cover_body_bytes: u64,
    pub qualified_ranges: u64,
    pub padding_requests: u64,
    pub padding_value_bytes: u64,
    pub peak_in_flight_ranges: u64,
    pub in_flight_ranges: u64,
    pub range_outcomes: BTreeMap<String, u64>,
}
pub type Shared = Arc<Mutex<Metrics>>;
pub struct PaddingReservation {
    inner: super::budget::Reservation,
    metrics: Shared,
    bytes: u64,
    dispatched: bool,
}
impl PaddingReservation {
    pub fn new(inner: super::budget::Reservation, metrics: Shared, bytes: u64) -> Self {
        Self {
            inner,
            metrics,
            bytes,
            dispatched: false,
        }
    }
    pub fn dispatched(&mut self) {
        if !self.dispatched {
            self.dispatched = true;
            self.inner.dispatched();
            let mut m = self.metrics.lock().unwrap();
            m.padding_requests += 1;
            m.padding_value_bytes = m.padding_value_bytes.saturating_add(self.bytes);
        }
    }
}
pub const REPORT_PREFIX: &str = "TREAZURY_COVER_REPORT ";
