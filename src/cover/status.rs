//! Agent-facing evidence is local to a listener/source binding, never an owner total.
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

pub const TOOL_NAME: &str = "treazury_cover_status";
pub const RETAINED: usize = 32;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Scope {
    pub listener: String,
    pub source: String,
}
#[derive(Clone, Serialize)]
pub struct Event {
    pub reason: String,
    pub transport: &'static str,
}
#[derive(Default)]
pub struct History {
    events: VecDeque<Event>,
    evicted: u64,
}
#[derive(Serialize)]
pub struct Report<'a> {
    pub source: &'a str,
    pub events: Vec<Event>,
    pub retained_limit: usize,
    pub evicted: u64,
}
#[derive(Default)]
pub struct Status {
    histories: BTreeMap<Scope, History>,
}
impl Status {
    /// Register only operator-selected bindings. Runtime events cannot allocate scopes.
    pub fn register(&mut self, scope: Scope) {
        self.histories.entry(scope).or_default();
    }
    pub fn record(&mut self, scope: &Scope, reason: &str) {
        if let Some(h) = self.histories.get_mut(scope) {
            if h.events.len() == RETAINED {
                h.events.pop_front();
                h.evicted = h.evicted.saturating_add(1);
                tracing::warn!(
                    code = "cover_status_evicted",
                    limit = RETAINED,
                    "cover status retention exhausted; eviction count is included in status"
                );
            }
            h.events.push_back(Event {
                reason: reason.into(),
                transport: "pooled_best_effort",
            });
        }
    }
    /// The server must pass its selected binding set, never caller-supplied scope names.
    pub fn report<'a>(&self, selected: &'a [Scope]) -> Vec<Report<'a>> {
        selected
            .iter()
            .filter_map(|scope| {
                self.histories.get(scope).map(|h| Report {
                    source: &scope.source,
                    events: h.events.iter().cloned().collect(),
                    retained_limit: RETAINED,
                    evicted: h.evicted,
                })
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_does_not_expose_other_listeners_or_owner_totals() {
        let a = Scope {
            listener: "a".into(),
            source: "api".into(),
        };
        let b = Scope {
            listener: "b".into(),
            source: "private".into(),
        };
        let mut s = Status::default();
        s.register(a.clone());
        s.register(b.clone());
        s.record(&b, "secret-source-event");
        for _ in 0..35 {
            s.record(&a, "cover_request_limit");
        }
        let scopes = [a];
        let reports = s.report(&scopes);
        let json = serde_json::to_string(&reports).unwrap();
        assert!(!json.contains("private") && !json.contains("secret-source-event"));
        assert_eq!(reports[0].evicted, 3);
        assert_eq!(reports[0].events.len(), RETAINED);
        assert!(
            s.report(&[Scope {
                listener: "none".into(),
                source: "api".into()
            }])
            .is_empty()
        );
    }
}
