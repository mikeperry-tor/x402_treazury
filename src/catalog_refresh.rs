//! Process-local source refresh ownership; never payment or replay authority.
use anyhow::Result;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    refreshing: bool,
    retired: bool,
    prices: Vec<(
        crate::pricing::advertised::AdvertisedPricing,
        Option<Arc<crate::pricing::runtime::CatalogHints>>,
    )>,
}

#[doc(hidden)]
pub struct Binding {
    state: Mutex<State>,
    start: Box<dyn Fn(Arc<Binding>) + Send + Sync>,
}
impl Binding {
    pub(crate) fn new(start: impl Fn(Arc<Self>) + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            start: Box::new(start),
        })
    }
    pub(crate) fn own_prices(&self, tools: &[crate::catalog::ToolSpec]) {
        let mut state = self.state.lock().expect("catalog refresh poisoned");
        state.prices = tools
            .iter()
            .map(|tool| (tool.advertised_pricing.clone(), tool.catalog_hints.clone()))
            .collect();
        if state.retired {
            invalidate_prices(&state);
        }
    }
    pub(crate) fn check(&self) -> Result<()> {
        let state = self.state.lock().expect("catalog refresh poisoned");
        anyhow::ensure!(
            !state.retired,
            "catalog definition has been replaced; reload the tools"
        );
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn refreshing(&self) -> bool {
        self.state
            .lock()
            .expect("catalog refresh poisoned")
            .refreshing
    }
    pub(crate) fn trigger(self: &Arc<Self>) -> bool {
        {
            let mut state = self.state.lock().expect("catalog refresh poisoned");
            if state.refreshing || state.retired {
                return false;
            }
            state.refreshing = true;
        }
        (self.start)(self.clone());
        true
    }
    pub(crate) fn fail(&self) {
        let mut state = self.state.lock().expect("catalog refresh poisoned");
        state.refreshing = false;
    }
    pub(crate) fn retire(&self) {
        let mut state = self.state.lock().expect("catalog refresh poisoned");
        state.refreshing = false;
        state.retired = true;
        invalidate_prices(&state);
    }
    pub(crate) fn observe(
        self: &Arc<Self>,
        facts: Option<crate::payment::ResponseFacts>,
        error: Option<&anyhow::Error>,
    ) {
        let Some(facts) =
            facts.or_else(|| error.and_then(crate::payment::ResponseFacts::from_error))
        else {
            return;
        };
        if !facts.unsigned_target {
            return;
        }
        // A decoded destination belongs to the API. A failed relay envelope does not.
        let status = facts.destination.or_else(|| {
            (!error.is_some_and(|e| crate::discovery_relay::RelayFailure::retained(e).is_some()))
                .then_some(facts.provider)
                .flatten()
        });
        if status.is_some_and(|s| matches!(s.as_u16(), 404 | 405 | 410)) {
            self.trigger();
        }
    }
}

fn invalidate_prices(state: &State) {
    for (advertised, observed) in &state.prices {
        advertised.invalidate();
        if let Some(observed) = observed {
            observed.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payment::ResponseFacts;
    use reqwest::StatusCode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn refresh_retains_prices_until_retirement_and_late_enrichment_cannot_restore_them() {
        let cfg: crate::catalog::Config = toml::from_str("spec='local.json'\n[pricing]\nsource='catalog'\nformat='payment-info'\nunit='call'\nusd_as_usdc=true\n").unwrap();
        let mut tools = crate::catalog::build_tools(
            &cfg,
            &serde_json::json!({"paths":{
                "/first":{"get":{"x-payment-info":{"price":{"currency":"USD","amount":"0.01"}}}},
                "/second":{"post":{"x-payment-info":{"price":{"currency":"USD","amount":"0.02"}}}}
            }}),
            "fixture",
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let hints = crate::pricing::runtime::fixture(directory.path()).await;
        hints.operation("GET", "/first").learn(
            alloy_primitives::U256::from(42),
            crate::pricing::runtime::received().unwrap(),
        );
        for tool in &mut tools {
            tool.catalog_hints = Some(hints.clone());
        }
        let binding = Binding::new(|_| {});
        binding.own_prices(&tools);
        assert!(
            tools
                .iter()
                .all(|tool| tool.advertised_pricing.amount().is_some())
        );
        binding.trigger();
        assert!(binding.check().is_ok());
        assert!(
            tools
                .iter()
                .all(|tool| tool.advertised_pricing.amount().is_some())
        );
        assert!(hints.operation("GET", "/first").get().is_some());
        binding.fail();
        assert!(binding.check().is_ok());
        assert!(
            tools
                .iter()
                .all(|tool| tool.advertised_pricing.amount().is_some())
        );
        assert!(hints.operation("GET", "/first").get().is_some());
        binding.retire();
        let price = crate::pricing::advertised::NormalizedPrice::parse(
            &crate::pricing::advertised::PricingConfig::legacy(""),
            &serde_json::json!({"price":{"currency":"USD","amount":"9"}}),
        );
        for tool in &tools {
            tool.advertised_pricing
                .replace(price.clone(), "late-document".into(), None);
            assert!(tool.advertised_pricing.amount().is_none());
            assert!(tool.advertised_pricing.external_description().is_none());
            assert!(
                !tool.render_description(None).contains("0.0"),
                "old catalog price must not reappear from canonical text"
            );
        }
        assert!(hints.operation("GET", "/first").get().is_none());
    }

    #[tokio::test]
    async fn cancelled_waiter_keeps_final_observation_owned_and_never_replays() {
        use axum::{Router, routing::get};
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let hits = Arc::new(AtomicUsize::new(0));
        let (address, http) = crate::test_tls::serve(Router::new().route(
            "/gone",
            get({
                let entered = entered.clone();
                let release = release.clone();
                let hits = hits.clone();
                move || {
                    let entered = entered.clone();
                    let release = release.clone();
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        entered.notify_one();
                        release.notified().await;
                        axum::http::StatusCode::GONE
                    }
                }
            }),
        ))
        .await;
        let cfg = crate::catalog::Config {
            prefix: Some("fixture".into()),
            ..Default::default()
        };
        let tool = crate::catalog::build_tools(
            &cfg,
            &serde_json::json!({"paths":{"/gone":{"get":{}}}}),
            "fixture",
        )
        .unwrap()
        .remove(0);
        let client = crate::payment::PaidClient::unsigned();
        let mut bound =
            crate::catalog_state::bind(vec![(tool, client.clone(), format!("https://{address}"))])
                .remove(0);
        let started = Arc::new(tokio::sync::Notify::new());
        let seen = started.clone();
        let binding = Binding::new(move |_| seen.notify_one());
        bound.refresh = Some(binding.clone());
        let waiter = tokio::spawn(async move { bound.invoke_output(&Default::default()).await });
        entered.notified().await;
        waiter.abort();
        let _ = waiter.await;
        release.notify_one();
        started.notified().await;
        client.drain().await;
        assert!(binding.refreshing());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        http.abort();
        let _ = http.await;
    }

    #[test]
    fn unsigned_status_refreshes_once_and_obsolete_results_cannot_refresh_again() {
        for status in [404, 405, 410] {
            let starts = Arc::new(AtomicUsize::new(0));
            let seen = starts.clone();
            let binding = Binding::new(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
            });
            assert!(binding.check().is_ok());
            let facts = Some(ResponseFacts {
                unsigned_target: true,
                ..ResponseFacts::provider(StatusCode::from_u16(status).unwrap())
            });
            binding.observe(facts, None);
            binding.observe(facts, None);
            assert_eq!(starts.load(Ordering::SeqCst), 1);
            assert!(binding.refreshing());
            assert!(binding.check().is_ok());
            binding.retire();
            binding.observe(facts, None);
            assert_eq!(starts.load(Ordering::SeqCst), 1);
            assert!(!binding.refreshing());
            assert!(binding.check().is_err());
        }
    }

    #[test]
    fn only_unsigned_removal_hints_refresh_and_failure_retains_admission() {
        let starts = Arc::new(AtomicUsize::new(0));
        let seen = starts.clone();
        let binding = Binding::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        for status in [200, 304, 400, 402, 403, 422, 429, 500, 503] {
            binding.observe(
                Some(ResponseFacts {
                    unsigned_target: true,
                    ..ResponseFacts::provider(StatusCode::from_u16(status).unwrap())
                }),
                None,
            );
        }
        for status in [404, 405, 410] {
            // Signed or unattributed responses cannot infer catalog drift.
            binding.observe(
                Some(ResponseFacts::provider(
                    StatusCode::from_u16(status).unwrap(),
                )),
                None,
            );
        }
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        binding.observe(
            Some(ResponseFacts {
                unsigned_target: true,
                ..ResponseFacts::provider(StatusCode::NOT_FOUND)
            }),
            None,
        );
        binding.fail();
        assert!(!binding.refreshing());
        assert!(binding.check().is_ok());
        assert_eq!(
            starts.load(Ordering::SeqCst),
            1,
            "failure must not schedule a retry"
        );
        // A subsequent unsigned response may request another refresh.
        binding.observe(
            Some(ResponseFacts {
                unsigned_target: true,
                ..ResponseFacts::provider(StatusCode::GONE)
            }),
            None,
        );
        assert_eq!(starts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn relay_destination_controls_expiry() {
        let starts = Arc::new(AtomicUsize::new(0));
        let seen = starts.clone();
        let binding = Binding::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        binding.observe(
            Some(ResponseFacts {
                unsigned_target: true,
                provider: Some(StatusCode::NOT_FOUND),
                destination: Some(StatusCode::OK),
                ..Default::default()
            }),
            None,
        );
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        binding.observe(
            Some(ResponseFacts {
                unsigned_target: true,
                provider: Some(StatusCode::OK),
                destination: Some(StatusCode::NOT_FOUND),
                ..Default::default()
            }),
            None,
        );
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod financial_tests;
