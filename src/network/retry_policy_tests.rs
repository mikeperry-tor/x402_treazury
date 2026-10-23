//! Configured retry counts exercised through real catalog/pricing consumers.
use super::*;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

fn tor_policy(retries: Option<usize>) -> NetworkPolicy {
    NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some("127.0.0.1:9150".parse().unwrap()),
        tor_isolation_retries: retries,
        cover_traffic_enabled: Some(false),
        ..Default::default()
    }
}

#[test]
fn isolation_retry_configuration_defaults_counts_and_representation() {
    for (input, retries, attempts) in [
        ("", None, 3),
        ("0", Some(0), 1),
        ("1", Some(1), 2),
        ("2", Some(2), 3),
        ("7", Some(7), 8),
    ] {
        let field = if input.is_empty() {
            String::new()
        } else {
            format!("tor_isolation_retries={input}\n")
        };
        let policy: NetworkPolicy = toml::from_str(&format!(
            "mode='tor'\nsocks_endpoint='127.0.0.1:9150'\n{field}"
        ))
        .unwrap();
        assert_eq!(policy.tor_isolation_retries, retries);
        assert_eq!(
            policy.inspection()["tor_isolation_retries"],
            retries.unwrap_or(2)
        );
        let context = NetworkContext::new(policy).unwrap();
        assert_eq!(context.max_http_attempts(false), attempts);
        assert_eq!(context.max_http_attempts(true), 1);
        let direct = NetworkContext::new(NetworkPolicy {
            tor_isolation_retries: retries,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(direct.max_http_attempts(false), 1);
    }
    assert!(toml::from_str::<NetworkPolicy>("tor_isolation_retries=-1").is_err());
    assert!(serde_json::from_str::<NetworkPolicy>(r#"{"tor_isolation_retries":-1}"#).is_err());
    let overflow: NetworkPolicy =
        serde_json::from_str(&format!(r#"{{"tor_isolation_retries":{}}}"#, usize::MAX)).unwrap();
    assert!(
        overflow
            .validate()
            .unwrap_err()
            .to_string()
            .contains("overflows total attempt count")
    );
    let largest: NetworkPolicy = serde_json::from_str(&format!(
        r#"{{"tor_isolation_retries":{}}}"#,
        usize::MAX - 1
    ))
    .unwrap();
    largest.validate().unwrap();
    assert_eq!(largest.tor_attempts().unwrap(), usize::MAX);
    assert!(
        serde_json::from_str::<NetworkPolicy>(&format!(
            r#"{{"tor_isolation_retries":{}0}}"#,
            usize::MAX
        ))
        .is_err()
    );
    // TOML integers are signed 64-bit; JSON can express all usize values.
    let toml_max = format!("tor_isolation_retries={}", i64::MAX);
    if usize::BITS == 64 {
        let policy: NetworkPolicy = toml::from_str(&toml_max).unwrap();
        policy.validate().unwrap();
        assert!(
            toml::from_str::<NetworkPolicy>(&format!("tor_isolation_retries={}", usize::MAX))
                .is_err()
        );
    } else {
        assert!(toml::from_str::<NetworkPolicy>(&toml_max).is_err());
        let policy: NetworkPolicy =
            toml::from_str(&format!("tor_isolation_retries={}", usize::MAX)).unwrap();
        assert!(policy.validate().is_err());
    }
}

#[derive(Clone)]
struct Fixture {
    status: StatusCode,
    body: &'static str,
    fail_bodies: usize,
    calls: Arc<AtomicUsize>,
}

async fn respond(State(state): State<Fixture>, headers: HeaderMap) -> Response {
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("x-payment"));
    let attempt = state.calls.fetch_add(1, Ordering::SeqCst);
    if attempt < state.fail_bodies {
        // Yield actual bytes before the body stalls, so this is a body-read
        // failure rather than a response-header failure.
        let body = Body::from_stream(
            futures_util::stream::once(async {
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"{"))
            })
            .chain(futures_util::stream::pending()),
        );
        return (state.status, body).into_response();
    }
    (state.status, state.body).into_response()
}

struct Running {
    network: NetworkContext,
    socks: crate::test_socks::Socks,
    fixture: Fixture,
    server: tokio::task::JoinHandle<()>,
}

impl Running {
    async fn new(
        retries: Option<usize>,
        status: u16,
        body: &'static str,
        fail_bodies: usize,
    ) -> Self {
        let fixture = Fixture {
            status: StatusCode::from_u16(status).unwrap(),
            body,
            fail_bodies,
            calls: Arc::default(),
        };
        let app = axum::Router::new()
            .fallback(respond)
            .with_state(fixture.clone());
        let (address, server) = crate::test_tls::serve(app).await;
        let socks = crate::test_socks::Socks::start(
            BTreeMap::from([("spec.example.com".into(), address)]),
            crate::test_socks::Fault::None,
        )
        .await;
        let mut policy = tor_policy(retries);
        policy.socks_endpoint = Some(socks.address);
        let network = NetworkContext::new(policy)
            .unwrap()
            .with_test_root(crate::test_tls::CA);
        Self {
            network,
            socks,
            fixture,
            server,
        }
    }

    fn assert_attempts(&self, expected: usize) {
        assert_eq!(self.fixture.calls.load(Ordering::SeqCst), expected);
        let records = self.socks.records.lock().unwrap();
        assert_eq!(records.len(), expected);
        let credentials: std::collections::BTreeSet<_> = records
            .iter()
            .map(|record| {
                assert_eq!(record.host, "spec.example.com");
                assert_eq!(record.port, 443);
                assert_eq!(record.address_type, 3, "destination DNS stays remote");
                (record.user.clone(), record.password.clone())
            })
            .collect();
        assert_eq!(credentials.len(), expected, "fresh scope for every attempt");
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn catalog_config() -> crate::catalog::Config {
    crate::catalog::Config {
        spec: "https://spec.example.com/document".into(),
        http_cache_enabled: false,
        read_timeout_seconds: Some(0.1),
        ..Default::default()
    }
}

async fn pricing(fixture: &Running) -> crate::pricing::Discovery {
    let cfg = catalog_config();
    let root = serde_json::json!({"openapi":"3.0.0","paths":{"/price":{"get":{"operationId":"price","responses":{"200":{"description":"complete"}}}}}});
    let tools = crate::catalog::build_tools(&cfg, &root, "fixture").unwrap();
    crate::pricing::PricingCache::default()
        .discover_observed_with(
            &fixture.network,
            &cfg,
            &root,
            &tools,
            "https://spec.example.com",
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn catalog_and_pricing_use_one_configured_status_attempt_sequence() {
    for retries in [Some(0), Some(1), None] {
        for status in [
            400, 403, 408, 429, 500, 502, 503, 504, 401, 404, 410, 501, 505, 520,
        ] {
            let expected = if [400, 403, 408, 429, 500, 502, 503, 504].contains(&status) {
                retries.unwrap_or(2) + 1
            } else {
                1
            };
            let catalog = Running::new(retries, status, "{}", 0).await;
            assert!(
                crate::catalog::load_json_discovery_with(&catalog_config(), &catalog.network)
                    .await
                    .is_err()
            );
            catalog.assert_attempts(expected);
            let probe = Running::new(retries, status, "", 0).await;
            let result = pricing(&probe).await;
            assert_eq!(result.evidence.observed, 1);
            assert_eq!(result.evidence.http_statuses.get(&status), Some(&1));
            assert_eq!(
                result
                    .evidence
                    .outcomes
                    .get(&crate::pricing::Outcome::UnexpectedHttpStatus),
                Some(&1)
            );
            probe.assert_attempts(expected);
        }
    }
}

#[tokio::test]
async fn catalog_body_failures_retry_but_complete_validation_and_pricing_bodies_do_not() {
    for retries in [Some(0), Some(1), None] {
        let allowed = retries.unwrap_or(2) + 1;
        let recovery = Running::new(retries, 200, "{}", 1).await;
        let result =
            crate::catalog::load_json_discovery_with(&catalog_config(), &recovery.network).await;
        assert_eq!(result.is_ok(), allowed > 1);
        recovery.assert_attempts(allowed.min(2));
        let exhausted = Running::new(retries, 200, "{}", usize::MAX).await;
        assert!(
            crate::catalog::load_json_discovery_with(&catalog_config(), &exhausted.network)
                .await
                .is_err()
        );
        exhausted.assert_attempts(allowed);
        let malformed = Running::new(retries, 200, "not JSON", 0).await;
        assert!(
            crate::catalog::load_json_discovery_with(&catalog_config(), &malformed.network)
                .await
                .is_err()
        );
        malformed.assert_attempts(1);
        let probe = Running::new(retries, 200, "", usize::MAX).await;
        let result = pricing(&probe).await;
        assert_eq!(result.evidence.observed, 1);
        assert_eq!(
            result
                .evidence
                .outcomes
                .get(&crate::pricing::Outcome::NoPaymentChallenge),
            Some(&1)
        );
        probe.assert_attempts(1);
    }
}
