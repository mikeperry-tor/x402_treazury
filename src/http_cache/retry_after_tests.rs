//! Unrepresentable server delays retain dispatch authority through discovery and relay.
use crate::{catalog, network};
use axum::{
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

#[derive(Clone, Default)]
struct Origin {
    calls: Arc<Mutex<BTreeMap<String, usize>>>,
    changed: Arc<tokio::sync::Notify>,
}
impl Origin {
    fn count(&self, path: &str) -> usize {
        self.calls.lock().unwrap().get(path).copied().unwrap_or(0)
    }
    async fn wait(&self, path: &str, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.count(path) >= count {
                    return;
                }
                changed.await;
            }
        })
        .await
        .expect("origin request did not arrive");
    }
}

#[tokio::test]
async fn unrepresentable_retry_after_is_pending_instead_of_missing() {
    let mut headers = HeaderMap::new();
    headers.insert("retry-after", u64::MAX.to_string().parse().unwrap());
    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let delay = network::parse_retry_after(status, &headers).unwrap();
        let deadline = network::RetryAfter::after(delay);
        assert!(matches!(deadline, network::RetryAfter::BeyondClock));
        let wait = network::sleep_retry_after(Some(deadline));
        tokio::pin!(wait);
        assert!(futures_util::poll!(&mut wait).is_pending());
    }
}

#[tokio::test]
async fn unrepresentable_retry_after_blocks_origin_and_relay_without_blocking_other_work() {
    let observed = Origin::default();
    let app = axum::Router::new().fallback({
        let observed = observed.clone();
        move |uri: Uri, headers: HeaderMap| {
            let observed = observed.clone();
            async move {
                assert!(!headers.contains_key("payment-signature"));
                let ordinal = {
                    let mut calls = observed.calls.lock().unwrap();
                    let count = calls.entry(uri.path().to_owned()).or_default();
                    *count += 1;
                    *count
                };
                observed.changed.notify_waiters();
                if uri.path() == "/ready" {
                    return (StatusCode::OK, "{\"paths\":{}}").into_response();
                }
                if uri.path().ends_with("final") && ordinal < 3 {
                    return StatusCode::FORBIDDEN.into_response();
                }
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [("retry-after", "18446744073709551615")],
                    "unused error body",
                )
                    .into_response()
            }
        }
    });
    let (address, server) = crate::test_tls::serve(app).await;
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    let context = Arc::new(
        network::NetworkContext::new(network::NetworkPolicy {
            mode: network::Mode::Tor,
            socks_endpoint: Some(socks.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let base = "https://api.example.com";
    let id = network::IsolationId::discovery(base).unwrap();
    let selected = context
        .http_participating(&id, base, None, false, Default::default())
        .unwrap();
    let envelope = serde_json::json!({"url":base,"method":"GET","statusCode":200,
        "headers":{},"body":"{\"paths\":{}}","redirectChain":[],"error":null});
    let (relay, fixture, relay_server) =
        crate::discovery_relay::tests::fixture(envelope, false, false).await;

    for path in ["/catalog-first", "/catalog-final"] {
        let expected = if path.ends_with("final") { 3 } else { 1 };
        let cfg = catalog::Config {
            spec: format!("{base}{path}"),
            relay_fallback: Some(relay.clone()),
            ..Default::default()
        };
        let owned_context = context.clone();
        let owner = tokio::spawn(async move {
            // Explicitly retain and resume the final-status continuation;
            // its scheduling authority must survive that handoff.
            match catalog::prepare_json_discovery_with(&cfg, &owned_context)
                .await
                .unwrap()
            {
                catalog::CatalogLoad::Ready(_) => panic!("failed origin unexpectedly succeeded"),
                catalog::CatalogLoad::RelayPending(pending) => {
                    pending.resume().await.unwrap();
                }
            }
        });
        observed.wait(path, expected).await;
        let ready = catalog::Config {
            spec: format!("{base}/ready"),
            ..Default::default()
        };
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            catalog::load_json_discovery_with(&ready, &context),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            result["paths"].is_object(),
            "unrelated discovery must make progress"
        );
        let mut owner = owner;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut owner)
                .await
                .is_err(),
            "valid huge delay must remain pending"
        );
        assert_eq!(observed.count(path), expected, "no later origin attempt");
        assert_eq!(
            fixture.calls.load(Ordering::SeqCst),
            0,
            "no unsigned relay request"
        );
        assert_eq!(
            fixture.signed.load(Ordering::SeqCst),
            0,
            "no payment authority from overflow"
        );
        assert!(Arc::ptr_eq(
            &selected,
            &context
                .http_participating(&id, base, None, false, Default::default())
                .unwrap()
        ));
        owner.abort();
        assert!(owner.await.unwrap_err().is_cancelled());
        assert_eq!(observed.count(path), expected);
    }
    assert!(crate::discovery_relay::tests::is_idle(&relay));
    relay_server.abort();
    server.abort();
}
