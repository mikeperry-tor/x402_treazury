//! Explicit registry-bound execution; registry reservations precede every tools/call.
use crate::{
    files,
    manifest::{Manifest, Scenario},
    process::{Evidence, Process},
    registry::Registry,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use x402_treazury::{
    deployment::MetaConfig,
    network::{NetworkContext, NetworkPolicy},
};
#[path = "execution_bootstrap.rs"]
pub(crate) mod bootstrap;
#[path = "cover_report.rs"]
mod cover_report;
#[path = "execution_mcp.rs"]
mod mcp;
#[path = "execution_restart.rs"]
mod restart;
#[path = "execution_rotation.rs"]
mod rotation;
#[path = "execution_session.rs"]
mod session;
fn now() -> Result<i64> {
    Ok(i64::try_from(x402_treazury::rotation::base::now()?)?)
}

pub async fn run(state: &Path, run: &str) -> Result<u8> {
    run_confined(state, run, None).await
}
pub async fn run_with_funding(state: &Path, run_id: &str, allow_funding: bool) -> Result<u8> {
    if !allow_funding {
        return run(state, run_id).await;
    }
    run_inner(state, run_id, None, None, true).await
}
pub async fn run_confined(state: &Path, run: &str, profile: Option<&Path>) -> Result<u8> {
    run_inner(state, run, profile, None, false).await
}
pub async fn run_owned(
    state: &Path,
    run: &str,
    profile: &Path,
    runtime: &mut crate::tor::session::Runtime,
) -> Result<u8> {
    run_inner(state, run, Some(profile), Some(runtime), false).await
}
pub async fn run_owned_with_funding(
    state: &Path,
    run: &str,
    profile: &Path,
    runtime: &mut crate::tor::session::Runtime,
    allow: bool,
) -> Result<u8> {
    if !allow {
        return run_owned(state, run, profile, runtime).await;
    }
    run_inner(state, run, Some(profile), Some(runtime), true).await
}

async fn run_inner(
    state: &Path,
    run: &str,
    profile: Option<&Path>,
    mut runtime: Option<&mut crate::tor::session::Runtime>,
    allow_funding: bool,
) -> Result<u8> {
    let mut registry = Registry::open(state, false)?;
    registry.require_unstarted(run)?;
    let manifest = registry.manifest(run)?;
    ensure!(
        (manifest.network.tor_mode == crate::manifest::TorMode::Owned)
            == (profile.is_some() && runtime.is_some()),
        "owned Tor runs require the live supervised confinement context"
    );
    ensure!(
        manifest.limits.new_funding_jobs == 0 || allow_funding,
        "positive funding requires --allow-funding as well as reviewed registry authority"
    );
    let managed = !manifest.start.pools.is_empty();
    let pins = registry.pins(run)?;
    let catalogs = pins
        .catalogs
        .as_ref()
        .context("run requires complete prepare artifacts")?;
    eprintln!("qualification verifying frozen catalog and executable provenance before serving");
    catalogs.verify(&manifest, &pins)?;
    if let Some(profile) = profile {
        ensure!(
            files::hash_file(profile)?
                == *catalogs
                    .files
                    .get("client.sb")
                    .context("pinned confinement profile missing")?,
            "confinement profile differs from preparation"
        );
    }
    ensure!(
        manifest.limits.result_bytes <= files::DOCUMENT_BYTES,
        "runner result_bytes exceeds supported {}-byte evidence limit; lower it explicitly",
        files::DOCUMENT_BYTES
    );
    ensure!(
        manifest.phases.iter().all(|p| matches!(
            p.scenario,
            Scenario::Unsigned {}
                | Scenario::Smoke {}
                | Scenario::ProviderSweep {}
                | Scenario::Reliability { .. }
                | Scenario::Concurrency { .. }
                | Scenario::TorOutage { .. }
                | Scenario::Rotation { .. }
                | Scenario::RefillService { .. }
                | Scenario::Lifecycle { .. }
        )),
        "this runner does not yet implement the requested scenario assertions"
    );
    let inspection = crate::resumption::inspect(
        &manifest,
        &registry.report(Some(run), now()?)?,
        u64::try_from(now()?)?,
    )?;
    if inspection["eligible"]
        .as_array()
        .context("eligible cases missing")?
        .is_empty()
    {
        eprintln!(
            "no untouched cases in an open execution window; no child started; prior work remains observation-only"
        );
        return summarize(&registry, &manifest);
    }
    let end = registry.begin_execution(run, now()?)?;
    let seconds = end
        .checked_sub(now()?)
        .filter(|s| *s > 0)
        .context("run execution deadline expired; no new work allowed")?;
    let deadline = Instant::now() + Duration::from_secs(seconds as u64);
    let frozen = catalogs.directory.join("deployment.toml");
    let config: MetaConfig = toml::from_str(std::str::from_utf8(&files::read(&frozen)?)?)?;
    let new_funding = funding_enabled(&manifest, &config, allow_funding)?;
    let mut environment = BTreeMap::new();
    let mut clients = BTreeMap::new();
    let local = NetworkContext::new(NetworkPolicy::default())?;
    for (name, listener) in &config.servers {
        ensure!(listener.auth, "live qualification requires authenticated listeners");
        ensure!(
            listener.listen.ip().is_loopback() && listener.listen.port() != 0,
            "driver refuses non-loopback/ephemeral listener"
        );
        let token = environment
            .entry(listener.bearer_token_env.clone())
            .or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let endpoint = format!("http://{}/mcp", listener.listen);
        let http = local.discovery(&endpoint, Duration::from_secs(manifest.limits.call_seconds))?;
        clients.insert(
            name.clone(),
            mcp::Client {
                http,
                endpoint,
                token,
            },
        );
    }
    if managed {
        managed_environment(&config, &mut environment, |name| std::env::var(name).ok())?;
    }
    let launcher = session::Launcher {
        binary: &manifest.binary,
        directory: &catalogs.directory,
        frozen: &frozen,
        environment: &environment,
        profile,
        managed,
        new_funding,
        expires_at: end,
    };
    let mut application = session::Application::start(launcher, &registry, &manifest)?;
    let stop = runtime
        .as_ref()
        .map_or_else(CancellationToken::new, |r| r.stop.clone());
    let signals = tokio::spawn(signal_stop(stop.clone()));
    let faults = runtime
        .as_ref()
        .map(|r| r.faults())
        .transpose()?
        .unwrap_or_else(|| [CancellationToken::new(), CancellationToken::new()]);
    let outcome = tokio::select! {
        result = execute(&mut registry, &manifest, &clients, &mut application, deadline, &stop, &mut runtime) => result,
        _ = stop.cancelled() => Err(anyhow::anyhow!("qualification cancelled; accepted calls remain reserved")),
        _ = faults[0].cancelled() => Err(anyhow::anyhow!("owned Tor output evidence failed; stopping dispatch")),
        _ = faults[1].cancelled() => Err(anyhow::anyhow!("owned Tor control observer failed; stopping dispatch")),
    };
    signals.abort();
    eprintln!(
        "qualification stopping: draining the supervised application deliberately; accepted work and durable reservations are preserved"
    );
    application
        .close(
            &registry,
            &manifest,
            &config,
            if outcome.is_ok() {
                "cases_finished"
            } else {
                "execution_failure"
            },
        )
        .await?;
    let sessions = application.summaries();
    let complete = !sessions.is_empty() && sessions.iter().all(|s| s["complete"] == true);
    let cover = cover_report::combine(&sessions);
    let cover_value = cover
        .as_ref()
        .cloned()
        .unwrap_or_else(|_| json!({"status":"invalid_or_incomplete","sessions":sessions}));
    let application_evidence = registry.application_evidence(run);
    if managed && complete && application_evidence.is_ok() {
        registry.observe_settlement(run, true)?;
        registry.capture_accounting(run, &application.current.binding.session, now()?)?;
    }
    let mut report = registry.report(Some(run), now()?)?;
    report["cover_evidence"] = cover_value;
    report["application_evidence"] = match &application_evidence {
        Ok(value) => value.clone(),
        Err(_) => json!({"status":"invalid_or_incomplete"}),
    };
    files::publish(
        &application.current.directory.join("results.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    ensure!(
        complete,
        "child did not finish with complete evidence; see private process record"
    );
    application_evidence?;
    cover_report::require_samples(&cover?)?;
    if let Err(error) = outcome {
        eprintln!("qualification incomplete: {error:#}");
        println!(
            "{}",
            serde_json::to_string_pretty(&registry.report(Some(run), now()?)?)?
        );
        return Ok(3);
    }
    summarize(&registry, &manifest)
}
async fn signal_stop(stop: CancellationToken) {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    stop.cancel();
}
fn summarize(registry: &Registry, m: &Manifest) -> Result<u8> {
    let report = registry.report(Some(&m.run_id), now()?)?;
    registry.application_evidence(&m.run_id)?;
    let outage_qualified = report["runtime_events"]
        .as_array()
        .is_some_and(|events| events.iter().any(|e| e["kind"] == "tor_outage_qualified"));
    if outage_qualified {
        for phase in &m.phases {
            if matches!(phase.scenario, Scenario::TorOutage { .. }) {
                crate::tor::outage::results(registry, m, phase)?;
            }
        }
    }
    let cases = report["cases"].as_array().context("report cases missing")?;
    let skipped = registry.validated_skips(&m.run_id)?;
    let debits = registry.verified_debits(&m.run_id)?;
    let concurrency = report["concurrency"]
        .as_array()
        .context("concurrency report missing")?;
    let concurrency_complete = concurrency
        .iter()
        .all(|c| matches!(c["status"].as_str(), Some("passed" | "failed")));
    let rotation = report["rotation"]
        .as_array()
        .context("rotation report missing")?;
    let rotation_complete = rotation
        .iter()
        .all(|r| matches!(r["status"].as_str(), Some("passed" | "failed")));
    let (provider_complete, provider_passed) = crate::semantics::qualification(&report, None)?;
    let (help_complete, help_passed) = crate::help::qualification(&report, None)?;
    let (pricing_complete, pricing_passed) = crate::pricing_stages::qualification(&report)?;
    let (reliability_complete, reliability_passed) = crate::reliability::qualification(&report)?;
    let complete = pricing_complete
        && help_complete
        && reliability_complete
        && provider_complete
        && rotation_complete
        && concurrency_complete
        && cases.iter().all(|c| {
            if c["case"].as_str().is_some_and(|id| skipped.contains(id)) {
                return true;
            }
            c["execution"] == "COMPLETED"
                && matches!(
                    c["settlement"].as_str(),
                    Some("NOT_SIGNED" | "USED" | "EXPIRED_UNUSED")
                )
                && debit_complete(c, m, &debits)
        });
    let passed = pricing_passed
        && help_passed
        && reliability_passed
        && provider_passed
        && rotation.iter().all(|r| r["status"] == "passed")
        && concurrency.iter().all(|c| c["status"] == "passed")
        && cases.iter().all(|c| {
            if c["case"].as_str().is_some_and(|id| skipped.contains(id)) {
                return true;
            }
            (c["semantic"] == "PASSED"
                || (c["case"]
                    .as_str()
                    .is_some_and(|id| crate::tor::outage::uncached(m, id))
                    && c["semantic"] == "FAILED"
                    && outage_qualified))
                && matches!(
                    c["settlement"].as_str(),
                    Some("NOT_SIGNED" | "USED" | "EXPIRED_UNUSED")
                )
        });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(if !complete {
        3
    } else if !passed {
        2
    } else {
        0
    })
}
pub(crate) fn debit_complete(
    case: &Value,
    manifest: &Manifest,
    debits: &BTreeMap<String, Value>,
) -> bool {
    let Some(id) = case["case"].as_str() else {
        return false;
    };
    let requires_payment = manifest.cases.iter().any(|c| c.id == id && !c.unsigned);
    if case["settlement"] == "USED" || (requires_payment && case["semantic"] == "PASSED") {
        case["settlement"] == "USED" && debits.contains_key(id)
    } else {
        true
    }
}

async fn ready(
    clients: &BTreeMap<String, mcp::Client>,
    snapshot: &Value,
    child: &mut Process,
    pricing: (&Registry, &str, &str),
    limit: usize,
    deadline: Instant,
    stop: &CancellationToken,
) -> Result<()> {
    for (name, client) in clients {
        loop {
            ensure!(
                !child.exited()? && !child.fault.is_cancelled(),
                "supervised child failed before readiness"
            );
            let ready = tokio::select! {
                _ = tokio::time::sleep_until(deadline) => anyhow::bail!("run deadline reached during startup"),
                _ = stop.cancelled() => anyhow::bail!("startup cancelled"),
                result = client.initialize() => result,
            };
            match ready {
                Ok(_) => break,
                Err(e)
                    if e.downcast_ref::<reqwest::Error>()
                        .is_some_and(|e| e.is_connect()) =>
                {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                Err(e) => return Err(e.context("MCP initialization failed")),
            }
        }
        let observations = pricing.0.report(Some(pricing.1), now()?)?;
        let priced =
            crate::pricing_stages::inventory(snapshot, &observations["runtime_events"], pricing.2)?;
        let expected = priced["inventory"]
            .as_array()
            .context("missing inventory")?
            .iter()
            .find(|i| i["server"] == *name)
            .context("missing prepared listener")?;
        tokio::time::timeout_at(deadline, client.check_inventory(expected, limit))
            .await
            .context("inventory deadline exceeded")??;
    }
    Ok(())
}

async fn execute(
    registry: &mut Registry,
    m: &Manifest,
    clients: &BTreeMap<String, mcp::Client>,
    child: &mut session::Application<'_>,
    deadline: Instant,
    stop: &CancellationToken,
    runtime: &mut Option<&mut crate::tor::session::Runtime>,
) -> Result<()> {
    let pins = registry.pins(&m.run_id)?;
    let dir = &pins
        .catalogs
        .as_ref()
        .context("catalog pins missing")?
        .directory;
    let snapshot: Value =
        serde_json::from_slice(&files::read_catalog(&dir.join("snapshot.json"))?)?;
    let session = child.current.binding.session.clone();
    ready(
        clients,
        &snapshot,
        child.running()?,
        (registry, &m.run_id, &session),
        m.limits.result_bytes,
        deadline,
        stop,
    )
    .await?;
    if matches!(m.start.mode, crate::manifest::StartMode::TreasuryOnly) && !m.start.pools.is_empty()
    {
        let state_dir = config_state(&pins.resolved_config)?;
        bootstrap::wait(
            registry,
            m,
            &state_dir,
            child.running()?,
            deadline.min(Instant::now() + Duration::from_secs(m.limits.phase_seconds)),
            stop,
        )
        .await?;
    }
    let execution_origin = Instant::now();
    for phase in &m.phases {
        let window = m
            .windows
            .iter()
            .find(|w| w.id == phase.window)
            .context("phase window missing")?;
        let time = now()? as u64;
        if time < window.not_before {
            eprintln!(
                "phase {} waiting_window; no daemon or automatic scheduling",
                phase.id
            );
            continue;
        }
        if time >= window.not_after {
            eprintln!("phase {} expired_window; no calls dispatched", phase.id);
            continue;
        }
        let phase_deadline = deadline
            .min(Instant::now() + Duration::from_secs(m.limits.phase_seconds))
            .min(Instant::now() + Duration::from_secs(window.not_after - time));
        if !m.start.pools.is_empty() && !phase.depends_on.is_empty() {
            bootstrap::wait_evidence(
                registry,
                m,
                Some(phase),
                child.running()?,
                phase_deadline,
                stop,
            )
            .await?;
        }
        if let Scenario::TorOutage { warm_case, .. } = &phase.scenario {
            ensure!(
                m.start.pools.is_empty(),
                "refusing Tor outage while a managed application is running; use a separate keyless deployment"
            );
            registry
                .dependencies_ready(&m.run_id, &phase.id)
                .context("outage prerequisites failed; refusing planned Tor shutdown")?;
            let warm = registry.response(&m.run_id, warm_case)?;
            ensure!(
                warm["result"]["isError"] != true,
                "warm help failed; refusing outage qualification"
            );
            tokio::time::timeout_at(
                phase_deadline,
                runtime
                    .as_mut()
                    .context("Tor outage requires owned runtime")?
                    .outage(),
            )
            .await
            .context("outage preparation deadline exceeded")??;
            let observations = registry.report(Some(&m.run_id), now()?)?;
            let priced = crate::pricing_stages::inventory(
                &snapshot,
                &observations["runtime_events"],
                &child.current.binding.session,
            )?;
            for (name, client) in clients {
                let expected = priced["inventory"]
                    .as_array()
                    .context("inventory missing")?
                    .iter()
                    .find(|i| i["server"] == *name)
                    .context("listener missing")?;
                tokio::time::timeout_at(
                    phase_deadline,
                    client.check_inventory(expected, m.limits.result_bytes),
                )
                .await
                .context("outage tools/list deadline exceeded")??;
            }
            registry.event(
                &m.run_id,
                "tor_outage_started",
                &json!({"phase":phase.id,"socks_closed":true,"inventory_unchanged":true}),
                now()?,
            )?;
        }
        let round_count = match &phase.scenario {
            Scenario::Lifecycle { rounds, .. } => rounds.len(),
            _ => 1,
        };
        for round_index in 0..round_count {
            let mut rotation = rotation::Controller::start(
                registry,
                m,
                (phase, round_index),
                child.running()?,
                phase_deadline,
                stop,
            )
            .await?;
            let batches = match &phase.scenario {
                Scenario::Concurrency { batches } => batches.clone(),
                Scenario::Lifecycle { rounds, .. } => rounds[round_index]
                    .depletion_cases
                    .iter()
                    .chain(&rounds[round_index].service_cases)
                    .map(|id| vec![id.clone()])
                    .collect(),
                _ => phase.cases.iter().map(|c| vec![c.clone()]).collect(),
            };
            for ids in batches {
                let untouched = ids.iter().all(|id| {
                    registry
                        .execution_state(&m.run_id, id)
                        .is_ok_and(|s| s == "UNATTEMPTED")
                });
                if !untouched {
                    continue;
                } // no replay, even after a reservation/dispatch crash
                ensure!(
                    Instant::now() < phase_deadline
                        && !stop.is_cancelled()
                        && !child.running()?.fault.is_cancelled()
                        && !child.running()?.exited()?,
                    "phase deadline/cancellation/child failure prevents dispatch"
                );
                if let Some(controller) = &rotation {
                    ensure!(ids.len() == 1, "rotation dispatch must be sequential");
                    controller.before_case(&ids[0])?;
                }
                let batch = format!("batch_{}", files::hash(ids.join(":")));
                // Dependency refusal leaves these cases untouched, permitting independent phases.
                if let Err(error) = registry.reserve_batch(&m.run_id, &batch, &ids, now()?) {
                    eprintln!("phase {} cannot dispatch: {error:#}", phase.id);
                    break;
                }
                let mut tasks = tokio::task::JoinSet::new();
                let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(ids.len()));
                for id in &ids {
                    let case = m
                        .cases
                        .iter()
                        .find(|c| &c.id == id)
                        .context("unknown reserved case")?
                        .clone();
                    registry.dispatching(&m.run_id, id)?;
                    registry.event(
                        &m.run_id,
                        "mcp_dispatch_intent",
                        &json!({"case":id,"server":case.server,"source":case.source}),
                        now()?,
                    )?;
                    let client = clients[&case.server].clone();
                    let barrier = barrier.clone();
                    let limit = m.limits.result_bytes;
                    let end = phase_deadline
                        .min(Instant::now() + Duration::from_secs(m.limits.call_seconds));
                    let stop = stop.clone();
                    let fault = child.running()?.fault.clone();
                    tasks.spawn(async move {
                    let mut began = None;
                    let result = tokio::select! {
                        _ = stop.cancelled() => Err(anyhow::anyhow!("case cancelled")),
                        _ = fault.cancelled() => Err(anyhow::anyhow!("child output failure")),
                        result = tokio::time::timeout_at(end, async {
                            barrier.wait().await;
                            began = Some(execution_origin.elapsed().as_millis());
                            client.call(&case.id, &case.tool, Value::Object(case.arguments), limit).await
                        }) => result.context("case deadline exceeded").and_then(|r|r),
                    }; (case.id, result, began, execution_origin.elapsed().as_millis())
                });
                }
                let mut progress = tokio::time::interval(Duration::from_secs(15));
                progress.tick().await;
                while !tasks.is_empty() {
                    let result = tokio::select! {
                        value = tasks.join_next() => value.context("missing MCP task result")?,
                        _ = progress.tick() => {
                            eprintln!("qualification awaiting {} in-flight MCP calls within their deadlines; reservations remain held", tasks.len());
                            continue;
                        }
                    };
                    let (id, result, began, ended) = result
                        .context("MCP driver task failed; reserved cases remain uncertain")?;
                    match result {
                        Ok((bytes, passed)) => {
                            if m.cases.iter().any(|c| c.id == id && c.unsigned) {
                                registry.finish_unsigned(&m.run_id, &id, &bytes, passed)?
                            } else {
                                registry.finish_managed(&m.run_id, &id, Some(&bytes), passed)?
                            }
                        }
                        Err(error) => {
                            eprintln!("case {id} incomplete: {error:#}");
                            if m.cases.iter().any(|c| c.id == id && c.unsigned) {
                                registry.uncertain_unsigned(&m.run_id, &id)?;
                            } else {
                                registry.finish_managed(&m.run_id, &id, None, false)?;
                            }
                            let category = if error
                                .downcast_ref::<reqwest::Error>()
                                .is_some_and(|e| e.is_timeout())
                            {
                                "mcp_transport_timeout"
                            } else if error
                                .downcast_ref::<tokio::time::error::Elapsed>()
                                .is_some()
                            {
                                "case_deadline"
                            } else {
                                "mcp_request_failure"
                            };
                            registry.event(
                            &m.run_id,
                            "mcp_failure",
                            &json!({"case":id,"category":category,"error":format!("{error:#}")}),
                            now()?,
                        )?;
                        }
                    }
                    registry.event(&m.run_id, "mcp_finished", &json!({"case":id,"mcp_started_ms":began,"mcp_finished_ms":ended,"scope":"MCP request interval, not payment/signature interval"}), now()?)?;
                }
                if let Some(controller) = &mut rotation {
                    controller
                        .after_case(registry, m, &ids[0], child.running()?, phase_deadline, stop)
                        .await?;
                }
                if m.limits.post_batch_wait_ms > 0
                    && !matches!(phase.scenario, Scenario::TorOutage { .. })
                {
                    let wait = Duration::from_millis(m.limits.post_batch_wait_ms);
                    ensure!(
                        Instant::now() + wait <= phase_deadline,
                        "post-batch wait cannot fit within remaining phase/run/window deadline"
                    );
                    eprintln!(
                        "qualification settling for {} ms after batch; no new API calls are dispatched",
                        m.limits.post_batch_wait_ms
                    );
                    registry.event(
                        &m.run_id,
                        "post_batch_wait",
                        &json!({"batch":batch,"milliseconds":m.limits.post_batch_wait_ms}),
                        now()?,
                    )?;
                    let fault = child.running()?.fault.clone();
                    tokio::select! {
                        _ = stop.cancelled() => anyhow::bail!("post-batch wait cancelled"),
                        _ = fault.cancelled() => anyhow::bail!("child evidence failed during post-batch wait"),
                        _ = tokio::time::sleep(wait) => {},
                    }
                    registry.event(
                        &m.run_id,
                        "post_batch_wait_completed",
                        &json!({"batch":batch}),
                        now()?,
                    )?;
                }
                if !m.start.pools.is_empty() {
                    registry.observe_settlement(&m.run_id, false)?;
                }
            }
            if let Some(controller) = rotation {
                if round_index == 0 && matches!(phase.scenario, Scenario::Lifecycle { .. }) {
                    restart::perform(
                        registry,
                        m,
                        phase,
                        &controller,
                        child,
                        restart::Wait {
                            clients,
                            snapshot: &snapshot,
                            deadline: phase_deadline,
                            stop,
                        },
                    )
                    .await?;
                }
                controller
                    .finish(registry, m, child.running()?, phase_deadline, stop)
                    .await?;
            }
        }
        if matches!(&phase.scenario, Scenario::TorOutage { .. }) {
            let evidence = crate::tor::outage::results(registry, m, phase)?;
            registry.event(&m.run_id, "tor_outage_qualified", &evidence, now()?)?;
            runtime
                .as_mut()
                .context("owned runtime missing")?
                .outage_completed = true;
        }
    }
    if !m.start.pools.is_empty() {
        bootstrap::wait_evidence(registry, m, None, child.running()?, deadline, stop).await?;
    }
    Ok(())
}
fn save_process(dir: &Path, e: &Evidence) -> Result<()> {
    for (label, output) in [("stdout", &e.stdout), ("stderr", &e.stderr)] {
        files::publish(&dir.join(format!("serve.{label}")), &output.bytes)?;
    }
    let record = json!({"pid":e.pid,"success":e.success,"exit_code":e.exit_code,"reason":e.reason,"forced_kill":e.forced_kill,"valid_output":e.valid_output(),
        "stdout":{"bytes":e.stdout.bytes.len(),"limit_exceeded":e.stdout.limit_exceeded,"read_error":e.stdout.read_error,"pipe_incomplete":e.stdout.pipe_incomplete},
        "stderr":{"bytes":e.stderr.bytes.len(),"limit_exceeded":e.stderr.limit_exceeded,"read_error":e.stderr.read_error,"pipe_incomplete":e.stderr.pipe_incomplete}});
    files::publish(
        &dir.join("serve.process.json"),
        &serde_json::to_vec_pretty(&record)?,
    )
}

// Copy only configured transport credentials/endpoints. Never inherit dotenv,
// wallet keys, proxy variables, or the parent's complete environment.
fn managed_environment(
    config: &MetaConfig,
    environment: &mut BTreeMap<String, String>,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<()> {
    let treasury = config
        .treasury
        .as_ref()
        .context("managed treasury missing")?;
    let funding = config.funding.as_ref().context("managed funding missing")?;
    let mut names = BTreeMap::new();
    for name in [&treasury.indexer_url_env, &treasury.submission_url_env] {
        if !name.is_empty() { names.insert(name.as_str(), true); }
    }
    names
        .entry(funding.base_rpc_url_env.as_str())
        .and_modify(|required| *required |= funding.base_rpc_url_env != "BASE_RPC_URL")
        .or_insert(funding.base_rpc_url_env != "BASE_RPC_URL");
    for name in funding
        .base_rpc_fallback_url_envs
        .iter()
        .flatten()
        .chain(funding.near_api_key_env.iter())
        .chain(funding.near_user_session_env.iter())
    {
        names.insert(name.as_str(), true);
    }
    for (name, required) in names {
        ensure!(
            !environment.contains_key(name),
            "managed transport environment {name} collides with listener credentials"
        );
        match lookup(name) {
            Some(value) => {
                environment.insert(name.into(), value);
            }
            None => ensure!(!required, "missing managed transport environment {name}"),
        }
    }
    Ok(())
}

fn config_state(config: &Value) -> Result<std::path::PathBuf> {
    Ok(config["treasury"]["state_dir"]
        .as_str()
        .context("managed state directory missing")?
        .into())
}

pub(crate) fn funding_enabled(
    manifest: &Manifest,
    config: &MetaConfig,
    allow: bool,
) -> Result<bool> {
    ensure!(
        manifest.limits.new_funding_jobs == 0 || allow,
        "positive funding requires --allow-funding as well as reviewed registry authority"
    );
    let enabled = allow && manifest.limits.new_funding_jobs > 0;
    ensure!(
        !enabled
            || (!manifest.start.pools.is_empty()
                && config.funding.as_ref().is_some_and(|f| f.auto_fund)),
        "funding-enabled execution requires managed pools and auto_fund=true"
    );
    Ok(enabled)
}

#[cfg(test)]
mod environment_tests {
    use super::*;
    #[test]
    fn managed_child_receives_only_explicit_transport_environment() {
        let mut config: MetaConfig =
            toml::from_str(include_str!("../deployments/public-swap-demo.toml")).unwrap();
        // Explicit overrides are independent of the demo's default endpoints.
        let treasury = config.treasury.as_mut().unwrap();
        treasury.indexer_url_env = "TEST_INDEXER_URL".into();
        treasury.submission_url_env = "TEST_SUBMISSION_URL".into();
        let expected = std::collections::BTreeSet::from([
            treasury.indexer_url_env.clone(),
            treasury.submission_url_env.clone(),
            "BASE_RPC_URL".into(),
        ]);
        let funding = config.funding.as_mut().unwrap();
        funding.base_rpc_url_env = "BASE_RPC_URL".into();
        funding.base_rpc_fallback_url_envs = None;
        funding.near_api_key_env = None;
        funding.near_user_session_env = None;
        let mut environment = BTreeMap::new();
        managed_environment(&config, &mut environment, |_| Some("fixture-value".into())).unwrap();
        assert_eq!(
            environment
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        assert!(!environment.contains_key("EVM_PRIVATE_KEY"));
        assert!(managed_environment(&config, &mut BTreeMap::new(), |_| None).is_err());
        let collision = config.treasury.as_ref().unwrap().indexer_url_env.clone();
        assert!(
            managed_environment(
                &config,
                &mut BTreeMap::from([(collision, "token".into())]),
                |_| Some("fixture-value".into())
            )
            .is_err()
        );
        let mut environment = BTreeMap::new();
        managed_environment(&config, &mut environment, |name| {
            (name != "BASE_RPC_URL").then(|| "endpoint".into())
        })
        .unwrap();
        assert!(!environment.contains_key("BASE_RPC_URL"));
    }
}
