//! Explicit keyless execution; registry reservations precede every tools/call.
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
#[path = "execution_mcp.rs"]
mod mcp;
fn now() -> Result<i64> {
    Ok(i64::try_from(x402_treazury::rotation::base::now()?)?)
}

pub async fn run(state: &Path, run: &str) -> Result<u8> {
    run_confined(state, run, None).await
}
pub async fn run_confined(state: &Path, run: &str, profile: Option<&Path>) -> Result<u8> {
    run_inner(state, run, profile, None).await
}
pub async fn run_owned(
    state: &Path,
    run: &str,
    profile: &Path,
    runtime: &mut crate::tor::session::Runtime,
) -> Result<u8> {
    run_inner(state, run, Some(profile), Some(runtime)).await
}
async fn run_inner(
    state: &Path,
    run: &str,
    profile: Option<&Path>,
    mut runtime: Option<&mut crate::tor::session::Runtime>,
) -> Result<u8> {
    let mut registry = Registry::open(state, false)?;
    let manifest = registry.manifest(run)?;
    ensure!(
        (manifest.network.tor_mode == crate::manifest::TorMode::Owned)
            == (profile.is_some() && runtime.is_some()),
        "owned Tor runs require the live supervised confinement context"
    );
    let pins = registry.pins(run)?;
    let catalogs = pins
        .catalogs
        .as_ref()
        .context("run requires prepare-catalogs, not configuration-only pins")?;
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
                | Scenario::Concurrency { .. }
                | Scenario::TorOutage { .. }
        )),
        "this runner does not yet implement the requested scenario assertions"
    );
    let current_time = now()? as u64;
    let eligible = manifest.cases.iter().any(|c| {
        registry
            .execution_state(run, &c.id)
            .is_ok_and(|s| s == "UNATTEMPTED")
    });
    if !eligible {
        return summarize(&registry, &manifest);
    }
    if !manifest.phases.iter().any(|p| {
        let w = manifest
            .windows
            .iter()
            .find(|w| w.id == p.window)
            .expect("validated window");
        current_time >= w.not_before && current_time < w.not_after
    }) {
        eprintln!(
            "no active execution window; no child started; waiting/expired cases remain unattempted"
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
    let mut environment = BTreeMap::new();
    let mut clients = BTreeMap::new();
    let local = NetworkContext::new(NetworkPolicy::default())?;
    for (name, listener) in &config.servers {
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
    // Inspecting the pinned executable and hashing it happened before launch; keys,
    // proxy variables and ambient dotenv paths are never inherited by this child.
    let session = catalogs
        .directory
        .join(format!("session-{}", uuid::Uuid::new_v4()));
    files::create_dir(&session)?;
    let binding = registry.application_binding(
        run,
        session
            .file_name()
            .context("session name missing")?
            .to_str()
            .context("session name must be UTF8")?,
        files::hash_file(&frozen)?,
        end,
    )?;
    let binding_file = session.join("binding.json");
    files::publish(&binding_file, &serde_json::to_vec(&binding)?)?;
    registry.event(
        run,
        "application_session",
        &serde_json::to_value(&binding)?,
        now()?,
    )?;
    let mut child = Process::launch_confined(
        &manifest.binary,
        &[
            "--meta-config".into(),
            frozen.to_string_lossy().into_owned(),
            "--qualification-unsigned".into(),
            "--qualification-parent-stdin".into(),
            "--qualification-no-new-funding".into(),
            "--qualification-binding".into(),
            binding_file.to_string_lossy().into_owned(),
        ],
        &environment,
        &catalogs.directory,
        files::DOCUMENT_BYTES,
        profile,
    )?;
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
        result = execute(&mut registry, &manifest, &clients, &mut child, deadline, &stop, &mut runtime) => result,
        _ = stop.cancelled() => Err(anyhow::anyhow!("qualification cancelled; accepted calls remain reserved")),
        _ = faults[0].cancelled() => Err(anyhow::anyhow!("owned Tor output evidence failed; stopping dispatch")),
        _ = faults[1].cancelled() => Err(anyhow::anyhow!("owned Tor control observer failed; stopping dispatch")),
    };
    signals.abort();
    eprintln!(
        "qualification stopping: draining the supervised application deliberately; accepted work and durable reservations are preserved"
    );
    let evidence = child
        .shutdown(
            if outcome.is_ok() {
                "cases_finished"
            } else {
                "execution_failure"
            },
            Duration::from_secs(manifest.limits.cleanup_seconds),
        )
        .await?;
    save_process(&session, &evidence)?;
    registry.event(run, "child_finished", &json!({"success":evidence.success,"forced_kill":evidence.forced_kill,"valid_output":evidence.valid_output()}), now()?)?;
    let application = registry.application_evidence(run);
    let mut report = registry.report(Some(run), now()?)?;
    report["application_evidence"] = match &application {
        Ok(value) => value.clone(),
        Err(_) => json!({"status":"invalid_or_incomplete"}),
    };
    files::publish(
        &session.join("results.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    ensure!(
        evidence.success && !evidence.forced_kill && evidence.valid_output(),
        "child did not finish with complete evidence; see private process record"
    );
    application?;
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
    let complete = cases.iter().all(|c| c["execution"] == "COMPLETED");
    let passed = cases.iter().all(|c| {
        (c["semantic"] == "PASSED"
            || (c["case"]
                .as_str()
                .is_some_and(|id| crate::tor::outage::uncached(m, id))
                && c["semantic"] == "FAILED"
                && outage_qualified))
            && c["settlement"] == "NOT_SIGNED"
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
async fn execute(
    registry: &mut Registry,
    m: &Manifest,
    clients: &BTreeMap<String, mcp::Client>,
    child: &mut Process,
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
    let snapshot: Value = serde_json::from_slice(&files::read(&dir.join("snapshot.json"))?)?;
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
        let expected = snapshot["inventory"]
            .as_array()
            .context("missing inventory")?
            .iter()
            .find(|i| i["server"] == *name)
            .context("missing prepared listener")?;
        tokio::time::timeout_at(
            deadline,
            client.check_inventory(expected, m.limits.result_bytes),
        )
        .await
        .context("inventory deadline exceeded")??;
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
        if let Scenario::TorOutage { warm_case, .. } = &phase.scenario {
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
            for (name, client) in clients {
                let expected = snapshot["inventory"]
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
        let batches = match &phase.scenario {
            Scenario::Concurrency { batches } => batches.clone(),
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
                    && !child.fault.is_cancelled()
                    && !child.exited()?,
                "phase deadline/cancellation/child failure prevents dispatch"
            );
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
                let end =
                    phase_deadline.min(Instant::now() + Duration::from_secs(m.limits.call_seconds));
                let stop = stop.clone();
                let fault = child.fault.clone();
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
                let (id, result, began, ended) =
                    result.context("MCP driver task failed; reserved cases remain uncertain")?;
                match result {
                    Ok((bytes, passed)) => {
                        registry.finish_unsigned(&m.run_id, &id, &bytes, passed)?
                    }
                    Err(error) => {
                        eprintln!("case {id} incomplete: {error:#}");
                        registry.uncertain_unsigned(&m.run_id, &id)?;
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
