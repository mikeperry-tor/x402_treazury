//! These fixtures supply only external readiness/rotation observations. Reservation,
//! dispatch, completion, settlement inspection and child close/reopen remain real.
use super::*;
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{Arc, Mutex},
};

// A single owned process consumes EOF and acknowledges its drain. An external
// `cat` adds a shell/descendant lifetime that the application fixture does not
// need and that the supervisor intentionally does not manage by process group.
const EOF_CHILD: &str = "while IFS= read -r line; do :; done\nprintf 'fixture-drained\\n'\n";
const DRAINED: &[u8] = b"fixture-drained\n";

fn assert_drained(application: &session::Application<'_>, closed: &Value) {
    assert_eq!(closed["complete"], true, "{closed}");
    assert_eq!(
        files::read(&application.current.directory.join("serve.stdout")).unwrap(),
        DRAINED,
        "successful cleanup must include the child's EOF acknowledgement"
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    None,
    AfterCase,
    Finish,
    Restart,
    CancelRestart,
    FinalReadiness,
    Settlement,
    BeforeDispatchStop,
    BeforeDispatchFault,
    Dependency,
    Reservation,
}
struct Observations {
    failure: Failure,
    phase_deadline: Option<Instant>,
    run_deadline: Instant,
    finished: Vec<usize>,
    restarted: bool,
    reopened_pid: Option<u32>,
    final_checked: bool,
}
tokio::task_local! { static OBSERVATIONS: RefCell<Observations>; }
pub(super) fn active() -> bool {
    OBSERVATIONS.try_with(|_| ()).is_ok()
}
fn observe<T>(f: impl FnOnce(&mut Observations) -> Result<T>) -> Result<T> {
    OBSERVATIONS.with(|state| f(&mut state.borrow_mut()))
}
pub(super) fn start_round(
    round: usize,
    deadline: Instant,
    child: &mut Process,
    stop: &CancellationToken,
) -> Result<()> {
    observe(|s| {
        if let Some(previous) = s.phase_deadline {
            assert_eq!(deadline, previous);
        }
        s.phase_deadline = Some(deadline);
        if s.failure == Failure::BeforeDispatchStop {
            stop.cancel();
        }
        if s.failure == Failure::BeforeDispatchFault {
            child.fault.cancel();
        }
        assert_eq!(
            s.finished.len(),
            round,
            "prior readiness must precede next round"
        );
        if round > 0 {
            assert!(s.restarted);
        }
        Ok(())
    })
}
pub(super) fn after_case(_round: usize, _case: &str, deadline: Instant) -> Result<()> {
    observe(|s| {
        assert_eq!(Some(deadline), s.phase_deadline);
        ensure!(
            s.failure != Failure::AfterCase,
            "fixture rotation observation failed"
        );
        Ok(())
    })
}
pub(super) fn finish_round(round: usize, child: &mut Process, deadline: Instant) -> Result<()> {
    observe(|s| {
        assert_eq!(Some(deadline), s.phase_deadline);
        assert!(s.restarted);
        assert_eq!(Some(child.pid()), s.reopened_pid);
        ensure!(
            s.failure != Failure::Finish,
            "fixture refill readiness failed"
        );
        s.finished.push(round);
        Ok(())
    })
}
pub(super) fn readiness(
    phase: Option<&Phase>,
    _child: &mut Process,
    deadline: Instant,
) -> Result<()> {
    observe(|s| {
        if let Some(phase) = phase {
            assert_eq!(phase.depends_on, ["prerequisite"]);
            assert!(
                s.phase_deadline.is_none(),
                "prerequisite observation precedes controller"
            );
            ensure!(
                s.failure != Failure::Dependency,
                "fixture managed prerequisite readiness failed"
            );
            assert_eq!(s.failure, Failure::Reservation);
            return Ok(());
        }
        assert_eq!(deadline, s.run_deadline);
        assert_eq!(s.finished, [0, 1]);
        s.final_checked = true;
        ensure!(
            s.failure != Failure::FinalReadiness,
            "fixture final readiness failed"
        );
        Ok(())
    })
}
pub(super) async fn restart(
    registry: &Registry,
    m: &Manifest,
    application: &mut session::Application<'_>,
    wait: restart::Wait<'_>,
) -> Result<()> {
    observe(|s| {
        assert!(!s.restarted, "only round zero restarts");
        assert_eq!(Some(wait.deadline), s.phase_deadline);
        Ok(())
    })?;
    let previous = application.current.binding.clone();
    let old_pid = application.running()?.pid();
    let config = application.config()?;
    let closed = application
        .close(registry, m, &config, "fixture_checkpoint")
        .await?;
    assert_drained(application, &closed);
    assert_eq!(registry.execution_state(&m.run_id, "s0")?, "COMPLETED");
    let failure = observe(|s| Ok(s.failure))?;
    if failure == Failure::CancelRestart {
        wait.stop.cancel();
    }
    ensure!(
        failure != Failure::Restart,
        "fixture restart checkpoint failed after drain"
    );
    ensure!(
        !wait.stop.is_cancelled(),
        "fixture restart cancelled after drain"
    );
    application.reopen(registry, m)?;
    let next = &application.current.binding;
    assert_ne!(next.session, previous.session);
    let mut normalized = next.clone();
    normalized.session = previous.session.clone();
    assert_eq!(
        serde_json::to_value(normalized)?,
        serde_json::to_value(&previous)?
    );
    let new_pid = application.running()?.pid();
    assert_ne!(old_pid, new_pid);
    observe(|s| {
        s.restarted = true;
        s.reopened_pid = Some(new_pid);
        Ok(())
    })?;
    ready(
        wait.clients,
        wait.snapshot,
        application.running()?,
        m.limits.result_bytes,
        wait.deadline,
        wait.stop,
    )
    .await
}

struct Fixture {
    _dir: tempfile::TempDir,
    state: PathBuf,
    directory: PathBuf,
    frozen: PathBuf,
    manifest: Manifest,
    registry: Registry,
    tool: Value,
}
impl Fixture {
    fn new(failure: Failure) -> Self {
        use crate::registry::{Authorization, Pins};
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let store = x402_treazury::rotation::store::Store::create(
            &state,
            &dir.path().join("key"),
            1,
            b"unfunded execution fixture",
        )
        .unwrap();
        let treasury = store.id().to_owned();
        drop(store);
        let binary = dir.path().join("child");
        std::fs::write(&binary, format!("#!/bin/sh\n{EOF_CHILD}")).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut m = crate::tests::manifest();
        m.binary = binary;
        m.treasury_id = treasury.clone();
        m.evidence_dir = dir.path().join("evidence");
        m.limits.api_reservation_usdc = "0.10".into();
        m.limits.new_funding_jobs = 2;
        m.limits.source_exposure_zec = "0.01".into();
        m.limits.run_seconds = 30;
        m.limits.phase_seconds = 20;
        m.limits.call_seconds = 5;
        m.limits.cleanup_seconds = 5;
        m.limits.result_bytes = 10000;
        if failure == Failure::Settlement {
            m.limits.post_batch_wait_ms = 20000;
        }
        m.windows[0].not_after = now().unwrap() as u64 + 120;
        let template = m.cases[0].clone();
        m.cases = ["d0", "s0", "d1", "s1"]
            .into_iter()
            .map(|id| {
                let mut c = template.clone();
                c.id = id.into();
                c
            })
            .collect();
        m.phases[0].cases = m.cases.iter().map(|c| c.id.clone()).collect();
        m.phases[0].scenario = serde_json::from_value(json!({"kind":"lifecycle","refill_slots":2,"restart":"queued_refill","rounds":[
            {"pool":"pool","expected_price_usdc":"0.01","depletion_cases":["d0"],"service_cases":["s0"]},
            {"pool":"pool","expected_price_usdc":"0.01","depletion_cases":["d1"],"service_cases":["s1"]}]})).unwrap();
        if failure == Failure::Reservation {
            let Scenario::Lifecycle { rounds, .. } = &m.phases[0].scenario else {
                unreachable!()
            };
            m.phases[0].scenario = Scenario::Rotation {
                refill_slots: 1,
                rounds: vec![rounds[0].clone()],
            };
            m.phases[0].cases.truncate(2);
            m.cases.truncate(2);
            m.limits.new_funding_jobs = 1;
        }
        if failure == Failure::Dependency {
            let mut prerequisite = m.cases[0].clone();
            prerequisite.id = "prerequisite".into();
            m.cases.push(prerequisite);
            let mut phase = m.phases[0].clone();
            phase.id = "prerequisite".into();
            phase.cases = vec!["prerequisite".into()];
            phase.scenario = Scenario::Smoke {};
            phase.window = "closed".into();
            m.phases[0].depends_on = vec!["prerequisite".into()];
            m.phases.insert(0, phase);
            let mut closed = m.windows[0].clone();
            closed.id = "closed".into();
            closed.not_after = now().unwrap() as u64 - 1;
            m.windows.push(closed);
        }
        let resolved = json!({"sources":{},"resolved_wallets":{"pool":{"mode":"zcash_rotation","max_api_payment_usdc":"0.02"}},"wallet_bindings":{"main":{"api":{"wallet":"pool"}}},"treasury":{"id":treasury},"funding":{"confidentiality":"public","auto_fund":true},"network":{"mode":"direct"}});
        files::create_dir(&m.evidence_dir).unwrap();
        let directory = m.evidence_dir.join(&m.run_id);
        files::create_dir(&directory).unwrap();
        let frozen = directory.join("deployment.toml");
        files::publish(&frozen, b"version = 1\n[servers]\n").unwrap();
        let tool = json!({"name":"api_read","source":"api","description":"fixture", "input_schema":{"type":"object"},"method":"GET","path":"/read"});
        let snapshot = json!({"version":1,"inventory":[{"server":"main","tools":[tool]}]});
        let build = x402_treazury::build_identity::current();
        for (name, data) in [
            ("snapshot.json", serde_json::to_vec(&snapshot).unwrap()),
            (
                "frozen-snapshot.json",
                serde_json::to_vec(&snapshot).unwrap(),
            ),
            ("build.json", serde_json::to_vec(&build).unwrap()),
        ] {
            files::publish(&directory.join(name), &data).unwrap();
        }
        let pinned_files = [
            "snapshot.json",
            "frozen-snapshot.json",
            "build.json",
            "deployment.toml",
        ]
        .into_iter()
        .map(|n| (n.into(), files::hash_file(&directory.join(n)).unwrap()))
        .collect();
        let pins = Pins {
            binary_sha256: files::hash_file(&m.binary).unwrap(),
            cargo_lock_sha256: build.cargo_lock_sha256.clone(),
            source_revision: build.source_revision.clone(),
            source_dirty: build.source_dirty,
            resolved_config: resolved,
            qualification: "catalogs_prepared".into(),
            catalogs: Some(crate::preparation::CatalogPins {
                build,
                directory: directory.clone(),
                files: pinned_files,
            }),
        };
        let auth = Authorization {
            version: 1,
            id: m.registry_authorization.clone(),
            treasury_id: treasury,
            cumulative_api_usdc: "0.10".into(),
            cumulative_source_zec: "0.01".into(),
            cumulative_new_jobs: 2,
        };
        let mut registry = Registry::authorize(&state, &auth, now().unwrap()).unwrap();
        let plan = crate::planner::build(&m, &pins.resolved_config).unwrap();
        registry.prepare(&plan, &pins, now().unwrap()).unwrap();
        registry.begin_execution(&m.run_id, now().unwrap()).unwrap();
        if failure == Failure::Reservation {
            drop(registry);
            let mut changed = auth;
            changed.id = "revised_authority".into();
            registry = Registry::authorize(&state, &changed, now().unwrap()).unwrap();
        }
        Self {
            _dir: dir,
            state,
            directory,
            frozen,
            manifest: m,
            registry,
            tool,
        }
    }
}

async fn endpoint(
    f: &Fixture,
) -> (
    BTreeMap<String, mcp::Client>,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    use axum::{Json, Router, extract::State, routing::post};
    #[derive(Clone)]
    struct HttpState {
        database: PathBuf,
        run: String,
        tool: Value,
        calls: Arc<Mutex<Vec<String>>>,
    }
    async fn handle(State(state): State<HttpState>, Json(request): Json<Value>) -> Json<Value> {
        let result = match request["method"].as_str().unwrap() {
            "initialize" => json!({}),
            "tools/list" => {
                json!({"tools":[{"name":state.tool["name"],"description":state.tool["description"],"inputSchema":state.tool["input_schema"]}]})
            }
            "tools/call" => {
                let id = request["id"].as_str().unwrap();
                let db = rusqlite::Connection::open(&state.database).unwrap();
                let execution: String = db
                    .query_row(
                        "SELECT execution FROM cases WHERE run=?1 AND id=?2",
                        rusqlite::params![state.run, id],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    execution, "DISPATCHING",
                    "durable reservation and dispatch precede HTTP"
                );
                let mut calls = state.calls.lock().unwrap();
                if let Some(previous) = calls.last() {
                    let prior: String = db
                        .query_row(
                            "SELECT execution FROM cases WHERE run=?1 AND id=?2",
                            rusqlite::params![state.run, previous],
                            |r| r.get(0),
                        )
                        .unwrap();
                    assert_eq!(
                        prior, "COMPLETED",
                        "previous accepted call must drain before next request"
                    );
                }
                assert!(!calls.iter().any(|prior| prior == id));
                let session: String = db.query_row("SELECT json_extract(detail,'$.session') FROM events WHERE kind='application_session' ORDER BY seq DESC LIMIT 1", [], |r| r.get(0)).unwrap();
                for (kind, detail) in [
                    (
                        "application_claim",
                        json!({"case":id,"session":session,"mode":"managed","server":"main","source":"api","tool":"api_read","started_micros":1}),
                    ),
                    (
                        "application_finished",
                        json!({"case":id,"session":session,"is_error":false,"finished_micros":2}),
                    ),
                ] {
                    db.execute(
                        "INSERT INTO events(run,kind,detail,at) VALUES(?1,?2,?3,?4)",
                        rusqlite::params![state.run, kind, detail.to_string(), now().unwrap()],
                    )
                    .unwrap();
                }
                calls.push(id.to_owned());
                json!({"isError":false,"content":[{"type":"text","text":"fixture response"}]})
            }
            method => panic!("unexpected method {method}"),
        };
        Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = HttpState {
        database: f.state.join("live-integration/registry.sqlite"),
        run: f.manifest.run_id.clone(),
        tool: f.tool.clone(),
        calls: calls.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/mcp", post(handle)).with_state(state),
        )
        .await
        .unwrap();
    });
    let network = x402_treazury::network::NetworkContext::new(Default::default()).unwrap();
    let clients = BTreeMap::from([(
        "main".into(),
        mcp::Client {
            http: network
                .discovery(&endpoint, Duration::from_secs(5))
                .unwrap(),
            endpoint,
            token: "fixture-token".into(),
        },
    )]);
    (clients, calls, server)
}

#[tokio::test]
async fn fixture_child_acknowledges_eof_even_before_first_runtime_yield() {
    let dir = tempfile::tempdir().unwrap();
    for yield_before_close in [false, true] {
        for _ in 0..16 {
            let child = Process::launch(
                Path::new("/bin/sh"),
                &["-c".into(), EOF_CHILD.into()],
                &BTreeMap::new(),
                dir.path(),
                1024,
            )
            .unwrap();
            let mut slot = crate::process::ProcessSlot::new(child);
            if yield_before_close {
                tokio::task::yield_now().await;
            }
            slot.begin_shutdown("fixture_eof", Duration::from_secs(5))
                .unwrap();
            let evidence = slot.finish_shutdown().await.unwrap();
            assert!(evidence.success && !evidence.forced_kill && evidence.valid_output());
            assert_eq!(evidence.stdout.bytes, DRAINED);
        }
    }
}

#[tokio::test]
async fn lifecycle_rounds_use_real_reservations_completion_and_child_restart() {
    run_fixture(Failure::None).await;
}
#[tokio::test]
async fn lifecycle_failures_retain_accepted_work_and_prevent_later_requests() {
    for failure in [
        Failure::AfterCase,
        Failure::Finish,
        Failure::Restart,
        Failure::CancelRestart,
        Failure::FinalReadiness,
        Failure::Settlement,
        Failure::BeforeDispatchStop,
        Failure::BeforeDispatchFault,
        Failure::Dependency,
        Failure::Reservation,
    ] {
        run_fixture(failure).await;
    }
}
async fn run_fixture(failure: Failure) {
    let mut f = Fixture::new(failure);
    let (clients, calls, server) = endpoint(&f).await;
    let environment = BTreeMap::new();
    let expires_at = now().unwrap() + 30;
    let launcher = session::Launcher {
        binary: &f.manifest.binary,
        directory: &f.directory,
        frozen: &f.frozen,
        environment: &environment,
        profile: None,
        managed: true,
        new_funding: false,
        expires_at,
    };
    let mut application = session::Application::start(launcher, &f.registry, &f.manifest).unwrap();
    let stop = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(25);
    let observations = RefCell::new(Observations {
        failure,
        phase_deadline: None,
        run_deadline: deadline,
        finished: Vec::new(),
        restarted: false,
        reopened_pid: None,
        final_checked: false,
    });
    OBSERVATIONS
        .scope(observations, async {
            let result = execute(
                &mut f.registry,
                &f.manifest,
                &clients,
                &mut application,
                deadline,
                &stop,
                &mut None,
            )
            .await;
            assert_eq!(
                result.is_ok(),
                failure == Failure::None,
                "{failure:?}: {result:?}"
            );
            if let Err(error) = result {
                let expected = match failure {
                    Failure::AfterCase => "fixture rotation observation failed",
                    Failure::Finish => "fixture refill readiness failed",
                    Failure::Restart => "fixture restart checkpoint failed after drain",
                    Failure::CancelRestart => "fixture restart cancelled after drain",
                    Failure::FinalReadiness => "fixture final readiness failed",
                    Failure::Settlement => {
                        "post-batch wait cannot fit within remaining phase/run/window deadline"
                    }
                    Failure::BeforeDispatchStop | Failure::BeforeDispatchFault => {
                        "phase deadline/cancellation/child failure prevents dispatch"
                    }
                    Failure::Dependency => "fixture managed prerequisite readiness failed",
                    Failure::Reservation => {
                        "rotation did not occur within reviewed depletion calls"
                    }
                    Failure::None => unreachable!(),
                };
                assert_eq!(error.to_string(), expected);
            }
            let expected_calls = match failure {
                Failure::BeforeDispatchStop
                | Failure::BeforeDispatchFault
                | Failure::Dependency
                | Failure::Reservation => vec![],
                Failure::AfterCase | Failure::Settlement => vec!["d0"],
                Failure::Finish | Failure::Restart | Failure::CancelRestart => vec!["d0", "s0"],
                Failure::None | Failure::FinalReadiness => vec!["d0", "s0", "d1", "s1"],
            };
            assert_eq!(*calls.lock().unwrap(), expected_calls);
            for case in &f.manifest.cases {
                assert_eq!(
                    f.registry
                        .execution_state(&f.manifest.run_id, &case.id)
                        .unwrap(),
                    if expected_calls.contains(&case.id.as_str()) {
                        "COMPLETED"
                    } else {
                        "UNATTEMPTED"
                    }
                );
            }
            let report = f
                .registry
                .report(Some(&f.manifest.run_id), now().unwrap())
                .unwrap();
            assert_eq!(
                report["api_reserved_atomic"],
                (expected_calls.len() * 20000) as u64
            );
            let timings: Vec<_> = report["runtime_events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["kind"] == "mcp_finished")
                .map(|e| e["detail"]["mcp_finished_ms"].as_u64().unwrap())
                .collect();
            assert_eq!(timings.len(), expected_calls.len());
            assert!(
                timings.windows(2).all(|pair| pair[0] <= pair[1]),
                "execution origin must not reset at restart"
            );
            observe(|s| {
                assert_eq!(
                    s.final_checked,
                    matches!(failure, Failure::None | Failure::FinalReadiness)
                );
                Ok(())
            })
            .unwrap();
            let config = application.config().unwrap();
            let closed = application
                .close(&f.registry, &f.manifest, &config, "fixture_done")
                .await
                .unwrap();
            assert_drained(&application, &closed);
            assert!(
                application
                    .summaries()
                    .iter()
                    .all(|s| s["complete"] == true)
            );
        })
        .await;
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn rotation_early_guards_preserve_attempted_state_and_reject_second_baselines() {
    for mode in ["partial", "attempted", "repeated"] {
        let mut f = Fixture::new(Failure::None);
        let environment = BTreeMap::new();
        let launcher = session::Launcher {
            binary: &f.manifest.binary,
            directory: &f.directory,
            frozen: &f.frozen,
            environment: &environment,
            profile: None,
            managed: true,
            new_funding: false,
            expires_at: now().unwrap() + 30,
        };
        let mut application =
            session::Application::start(launcher, &f.registry, &f.manifest).unwrap();
        if mode != "repeated" {
            let ids = if mode == "partial" {
                vec!["d0".into()]
            } else {
                vec!["d0".into(), "s0".into()]
            };
            for id in ids {
                f.registry
                    .reserve_batch(
                        &f.manifest.run_id,
                        &format!("prior_{id}"),
                        &[id],
                        now().unwrap(),
                    )
                    .unwrap();
            }
        } else {
            let request = f
                .registry
                .request_pool_observation(&f.manifest.run_id, "p", "pool", now().unwrap())
                .unwrap();
            let observation = json!({"request":request,"session":application.current.binding.session,"pool":"p","pool_name":"pool","query_started_micros":1,"observed_micros":2,
                "state":{"treasury_id":f.manifest.treasury_id,"pools":[{"id":"p","name":"pool","generation":0,"addresses":[{"id":"a","role":"ACTIVE"},{"id":"b","role":"READY"}]}]},
                "balances":{"block_height":1,"block_hash":format!("0x{}", "11".repeat(32)),"confirmed":{"a":"0","b":"20000"},"unresolved":{}}});
            let estimate =
                x402_treazury::qualification::depletion_estimate(&observation, "10000", 1).unwrap();
            f.registry
                .event(
                    &f.manifest.run_id,
                    "application_pool_observation",
                    &observation,
                    now().unwrap(),
                )
                .unwrap();
            f.registry.event(&f.manifest.run_id,"rotation_started",&json!({"phase":f.manifest.phases[0].id,"round":0,"request":request,"estimate":estimate}),now().unwrap()).unwrap();
        }
        let before = f
            .registry
            .report(Some(&f.manifest.run_id), now().unwrap())
            .unwrap();
        let result = rotation::Controller::start(
            &mut f.registry,
            &f.manifest,
            (&f.manifest.phases[0], 0),
            application.running().unwrap(),
            Instant::now() + Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await;
        match mode {
            "attempted" => assert!(result.unwrap().is_none()),
            "partial" => assert_eq!(
                result.err().unwrap().to_string(),
                "partially dispatched rotation is observation-only; no automatic paid resumption"
            ),
            _ => assert_eq!(
                result.err().unwrap().to_string(),
                "rotation already started; retain observations rather than create another baseline"
            ),
        }
        let after = f
            .registry
            .report(Some(&f.manifest.run_id), now().unwrap())
            .unwrap();
        assert_eq!(before["cases"], after["cases"]);
        assert_eq!(before["runtime_events"], after["runtime_events"]);
        assert_eq!(before["api_reserved_atomic"], after["api_reserved_atomic"]);
        let config = application.config().unwrap();
        assert_eq!(
            application
                .close(&f.registry, &f.manifest, &config, "guards_done")
                .await
                .unwrap()["complete"],
            true
        );
    }
}

#[tokio::test]
async fn outage_preparation_refuses_managed_application_before_any_dispatch() {
    let mut f = Fixture::new(Failure::None);
    let environment = BTreeMap::new();
    let launcher = session::Launcher {
        binary: &f.manifest.binary,
        directory: &f.directory,
        frozen: &f.frozen,
        environment: &environment,
        profile: None,
        managed: true,
        new_funding: false,
        expires_at: now().unwrap() + 30,
    };
    let mut application = session::Application::start(launcher, &f.registry, &f.manifest).unwrap();
    let mut phase = f.manifest.phases[0].clone();
    phase.scenario = Scenario::TorOutage {
        warm_case: "d0".into(),
        cached_case: "s0".into(),
        uncached_case: "d1".into(),
    };
    let before = f
        .registry
        .report(Some(&f.manifest.run_id), now().unwrap())
        .unwrap();
    let error = prepare_outage(
        &mut f.registry,
        &f.manifest,
        (&phase, Instant::now() + Duration::from_secs(5)),
        &BTreeMap::new(),
        &json!({}),
        &mut None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "refusing Tor outage while a managed application is running; use a separate keyless deployment"
    );
    let after = f
        .registry
        .report(Some(&f.manifest.run_id), now().unwrap())
        .unwrap();
    assert_eq!(before["cases"], after["cases"]);
    assert_eq!(before["runtime_events"], after["runtime_events"]);
    assert_eq!(after["api_reserved_atomic"], 0);
    let config = application.config().unwrap();
    assert_eq!(
        application
            .close(&f.registry, &f.manifest, &config, "outage_refused")
            .await
            .unwrap()["complete"],
        true
    );
}
