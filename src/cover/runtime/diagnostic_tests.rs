//! Actual episode cleanup must distinguish a shortfall from retained refusal.
use super::*;
use crate::network::{HttpPolicy, IsolationId};
use std::io::Write;
use tracing::instrument::WithSubscriber;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn deadline_cleanup_reports_actual_capability_without_warn_on_available_shortfall() {
    for checking in [false, true] {
        let logs = Capture::default();
        let sink = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || sink.clone())
            .finish();
        async {
            let engine = Engine::new(Limits::default());
            let identity = IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
            let transport = crate::network::global()
                .http_transport(
                    &identity,
                    "https://api.example.com",
                    None,
                    false,
                    HttpPolicy::default(),
                    crate::network::SelectionPolicy::Legacy,
                )
                .unwrap();
            let owner = Owner::from_transport(&transport);
            let mut config = crate::cover::tests::example_config();
            // No task or network is launched by begin; invoke the real cleanup loop below.
            config.ranges_enabled = false;
            let http = transport.client.clone();
            let call = engine
                .begin(
                    owner.clone(),
                    Arc::new(config),
                    Scope {
                        listener: "fixture".into(),
                        source: "source".into(),
                    },
                    http.clone(),
                )
                .unwrap();
            engine
                .registry
                .lock()
                .unwrap()
                .owners
                .get_mut(&owner)
                .unwrap()
                .capability = if checking {
                Capability::Checking
            } else {
                Capability::Available {
                    length: 4096,
                    validator: None,
                }
            };
            call.session.episode.lock().unwrap().deadline = Instant::now();
            engine
                .clone()
                .run(owner.clone(), call.session.clone(), http)
                .await;
            let registry = engine.registry.lock().unwrap();
            assert!(if checking {
                matches!(
                    registry.owners[&owner].capability,
                    Capability::Unavailable("cover_qualification_cancelled")
                )
            } else {
                matches!(
                    registry.owners[&owner].capability,
                    Capability::Available { .. }
                )
            });
            assert_eq!(engine.metrics().range_requests, 0);
        }
        .with_subscriber(subscriber)
        .await;
        let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        let episode = logs
            .lines()
            .find(|line| line.contains("sampled_body_bytes="))
            .unwrap();
        for field in [
            "episode_id=",
            "source=source",
            "sampled_body_bytes=",
            "cover_body_bytes=0",
            "code=\"cover_budget_unspent\"",
            "deadline_expired=true",
            "automatic_requalification=false",
        ] {
            assert!(episode.contains(field), "{logs}");
        }
        if checking {
            assert!(
                episode.contains(
                    "HTTP/2 range cover qualification did not finish for provider [source]"
                ),
                "{logs}"
            );
            assert!(episode.contains("further range attempts disabled for this identity/origin until restart or cache eviction"), "{logs}");
            assert!(episode.contains("WARN"), "{logs}");
            assert!(episode.contains("capability=\"unavailable\""), "{logs}");
            assert!(
                episode.contains("retained_failure=\"cover_qualification_cancelled\""),
                "{logs}"
            );
        } else {
            assert!(episode.contains("INFO"), "{logs}");
            assert!(episode.contains("capability=\"available\""), "{logs}");
            assert!(!logs.contains("WARN"), "{logs}");
        }
    }
}

#[tokio::test]
async fn capacity_refusal_summaries_warn_without_reclassifying_normal_completion() {
    let engine = Engine::new(Limits::default());
    let identity = IsolationId::treasury("cover-diagnostic-fixture");
    let transport = crate::network::global()
        .http_transport(
            &identity,
            "https://api.example.com",
            None,
            false,
            HttpPolicy::default(),
            crate::network::SelectionPolicy::Legacy,
        )
        .unwrap();
    let mut config = crate::cover::tests::example_config();
    config.ranges_enabled = false;
    let call = engine
        .begin(
            Owner::from_transport(&transport),
            Arc::new(config),
            Scope {
                listener: "fixture".into(),
                source: "source".into(),
            },
            transport.client.clone(),
        )
        .unwrap();
    for cached in [false, true] {
        let capability = if cached {
            Capability::Unavailable("cover_forbidden")
        } else {
            Capability::Available {
                length: 4096,
                validator: None,
            }
        };
        for code in [
            "cover_stream_limit",
            "cover_request_limit",
            "cover_body_limit",
            "cover_padding_limit",
            "cover_history_limit",
            "cover_episode_limit",
            "cover_budget_completed",
            "cover_budget_unspent",
            "cover_episode_deadline",
            "cover_episode_request_limit",
        ] {
            let logs = Capture::default();
            let sink = logs.clone();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(move || sink.clone())
                .finish();
            tracing::subscriber::with_default(subscriber, || {
                diagnostics::episode(
                    &call.session,
                    &engine.limits,
                    code,
                    &capability,
                    "primary",
                    false,
                );
            });
            let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
            let refusal = matches!(
                code,
                "cover_stream_limit"
                    | "cover_request_limit"
                    | "cover_body_limit"
                    | "cover_padding_limit"
                    | "cover_history_limit"
            );
            let expected_level = if refusal {
                "WARN"
            } else if cached {
                "DEBUG"
            } else {
                "INFO"
            };
            assert!(logs.contains(expected_level), "{code}: {logs}");
            assert!(logs.contains(code));
            for field in [
                "process_stream_limit=6",
                "process_history_limit=128",
                "process_request_limit=64",
            ] {
                assert!(logs.contains(field), "{logs}");
            }
            assert!(!logs.contains("https://api.example.com"));
            assert!(!logs.contains("cover-diagnostic-fixture"));
        }
    }
    assert_eq!(engine.metrics().range_requests, 0);
}
