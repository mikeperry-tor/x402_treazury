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
            let owner = Owner {
                runtime: tokio::runtime::Handle::current().id(),
                identity: IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap(),
                origin: "https://api.example.com".into(),
                transport: HttpPolicy::default(),
                public_only: false,
                timeout_ms: 30000,
            };
            let mut config = crate::cover::tests::example_config();
            // No task or network is launched by begin; invoke the real cleanup loop below.
            config.ranges_enabled = false;
            let http = reqwest::Client::new();
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
            .find(|line| line.contains("optional cover episode finished"))
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
