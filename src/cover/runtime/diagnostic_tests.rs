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
    if crate::test_log_capture::run_in_child(
        "cover::runtime::diagnostic_tests::deadline_cleanup_reports_actual_capability_without_warn_on_available_shortfall",
    ) {
        return;
    }
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
                matches!(registry.owners[&owner].capability, Capability::Unknown)
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
        ] {
            assert!(episode.contains(field), "{logs}");
        }
        if checking {
            assert!(episode.contains("DEBUG"), "{logs}");
            assert!(episode.contains("capability=\"unknown\""), "{logs}");
            assert!(episode.contains("automatic_requalification=true"), "{logs}");
            assert!(!logs.contains("WARN"), "{logs}");
        } else {
            assert!(episode.contains("DEBUG"), "{logs}");
            assert!(episode.contains("capability=\"available\""), "{logs}");
            assert!(
                episode.contains("automatic_requalification=false"),
                "{logs}"
            );
            assert!(!logs.contains("WARN"), "{logs}");
        }
    }
}

#[tokio::test]
async fn capacity_refusal_summaries_warn_without_reclassifying_normal_completion() {
    if crate::test_log_capture::run_in_child(
        "cover::runtime::diagnostic_tests::capacity_refusal_summaries_warn_without_reclassifying_normal_completion",
    ) {
        return;
    }
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
            "cover_task_failed",
            "cover_sampling_exhausted",
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
                diagnostics::episode(&call.session, &engine.limits, code, &capability, "primary");
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
            } else if matches!(code, "cover_task_failed" | "cover_sampling_exhausted") {
                "INFO"
            } else {
                "DEBUG"
            };
            assert!(logs.contains(expected_level), "{code}: {logs}");
            if code == "cover_task_failed" {
                assert!(logs.contains("cover task stopped unexpectedly"), "{logs}");
            } else if code == "cover_sampling_exhausted" {
                assert!(logs.contains("distribution sampling failed"), "{logs}");
            }
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
    for (code, level, message) in [
        (
            "cover_fallback_selected",
            tracing::Level::INFO,
            "An alternate static resource was selected",
        ),
        (
            "cover_http2_unavailable",
            tracing::Level::INFO,
            "This API response does not use HTTP/2",
        ),
        (
            "cover_padding_rejected",
            tracing::Level::INFO,
            "The provider rejected optional request padding",
        ),
        (
            "cover_sampling_exhausted",
            tracing::Level::INFO,
            "Optional cover distribution sampling failed",
        ),
        (
            "cover_header_list_limit",
            tracing::Level::WARN,
            "would exceed the configured header list limit",
        ),
        (
            "cover_padding_limit",
            tracing::Level::WARN,
            "reached the process padding byte limit",
        ),
        (
            "cover_history_limit",
            tracing::Level::WARN,
            "reached the process accounting history limit",
        ),
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
            diagnostics::status(&call.session, &engine.limits, code, level);
        });
        let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert!(logs.contains(message), "{code}: {logs}");
        assert!(logs.contains(level.as_str()), "{code}: {logs}");
        assert!(logs.contains("source=source"), "{code}: {logs}");
        assert!(!logs.contains("https://api.example.com"));
    }
    assert_eq!(engine.metrics().range_requests, 0);
}

#[tokio::test]
async fn scheduler_and_cleanup_preserve_completed_facts_at_local_deadlines() {
    for qualified in [false, true] {
        for cleanup_first in [false, true] {
            for code in ["cover_deadline", "cover_range_ignored", "success"] {
                let engine = Engine::new(Limits::default());
                let identity = IsolationId::treasury("cover-deadline-fixture");
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
                config.ranges_enabled = false;
                let call = engine
                    .begin(
                        owner.clone(),
                        Arc::new(config),
                        Scope {
                            listener: "fixture".into(),
                            source: "source".into(),
                        },
                        transport.client.clone(),
                    )
                    .unwrap();
                let initial = if qualified {
                    Capability::Available {
                        length: 4096,
                        validator: None,
                    }
                } else {
                    Capability::Checking
                };
                engine
                    .registry
                    .lock()
                    .unwrap()
                    .owners
                    .get_mut(&owner)
                    .unwrap()
                    .capability = initial;
                let result = if code == "success" {
                    Ok(range::Representation {
                        length: 4096,
                        validator: None,
                    })
                } else {
                    Err(range::Failure(code))
                };
                let outcome = range::Outcome {
                    result,
                    observation: range::Observation {
                        start: 0,
                        requested_bytes: 1024,
                        http_status: None,
                        http_version: None,
                        declared_body_bytes: None,
                        received_bytes: 0,
                        elapsed_ms: 0,
                        stage: "request",
                    },
                };
                let mut pending = JoinSet::new();
                let handle = pending.spawn(async move { (!qualified, outcome) });
                while !handle.is_finished() {
                    tokio::task::yield_now().await;
                }
                call.session.episode.lock().unwrap().deadline = Instant::now();
                if cleanup_first {
                    // The scheduler deadline wins before this completed range is
                    // consumed. Abort/drain must retain its actual result.
                    engine
                        .drain_ranges(&owner, &call.session, &mut pending)
                        .await;
                } else {
                    let item = pending.join_next().await;
                    let _ = engine.complete_range(&owner, &call.session, item);
                }
                engine
                    .clone()
                    .run(
                        owner.clone(),
                        call.session.clone(),
                        transport.client.clone(),
                    )
                    .await;
                let capability = engine.registry.lock().unwrap().owners[&owner]
                    .capability
                    .clone();
                match code {
                    "cover_deadline" if !qualified => {
                        assert!(matches!(capability, Capability::Unknown))
                    }
                    "cover_deadline" | "success" => {
                        assert!(matches!(capability, Capability::Available { .. }))
                    }
                    _ => assert!(matches!(
                        capability,
                        Capability::Unavailable("cover_range_ignored")
                    )),
                }
                drop(call);
                // A new episode must preserve refusal/success and allow the
                // interrupted unknown check to qualify under the same owner.
                let config = engine.registry.lock().unwrap().owners[&owner]
                    .config
                    .clone();
                let next = engine.begin(
                    owner.clone(),
                    config,
                    Scope {
                        listener: "fixture".into(),
                        source: "source".into(),
                    },
                    transport.client.clone(),
                );
                assert!(next.is_ok());
                assert!(
                    matches!(
                        engine.registry.lock().unwrap().owners[&owner].capability,
                        Capability::Unknown
                    ) == (code == "cover_deadline" && !qualified)
                );
                assert_eq!(engine.metrics().range_requests, 0);
            }
        }
    }
}
