//! Graceful application-only restart at an observed funding checkpoint. Tor is
//! owned by the outer run and is never stopped, restarted or signalled here.
use super::*;
use crate::manifest::{Phase, Restart};
use x402_treazury::qualification::lifecycle::restart::{self, Checkpoint};

pub(super) struct Wait<'a> {
    pub clients: &'a BTreeMap<String, mcp::Client>,
    pub snapshot: &'a Value,
    pub deadline: Instant,
    pub stop: &'a CancellationToken,
}
pub(super) async fn perform(
    registry: &Registry,
    m: &Manifest,
    phase: &Phase,
    controller: &rotation::Controller,
    application: &mut session::Application<'_>,
    wait: Wait<'_>,
) -> Result<()> {
    let Scenario::Lifecycle { restart, .. } = phase.scenario else {
        anyhow::bail!("restart requires lifecycle scenario")
    };
    let checkpoint = match restart {
        Restart::QueuedRefill => Checkpoint::QueuedRefill,
        Restart::SubmittedDeposit => Checkpoint::SubmittedDeposit,
    };
    let (state, pool, job) = controller.restart_target()?;
    let mut progress = Instant::now() - Duration::from_secs(15);
    let observed = loop {
        let snapshot = rotation::snapshot(state).await?;
        if restart::observed(&snapshot, pool, job, checkpoint)? {
            break snapshot;
        }
        let funding = snapshot["funding_jobs"]
            .as_array()
            .context("restart jobs missing")?
            .iter()
            .find(|j| j["id"] == job)
            .context("restart replacement disappeared")?;
        let missed = match checkpoint {
            Checkpoint::QueuedRefill => funding["phase"] != "ALLOCATED",
            Checkpoint::SubmittedDeposit => matches!(
                funding["phase"].as_str(),
                Some("SWAPPING" | "VERIFYING_CREDIT" | "COMPLETE")
            ),
        };
        if missed {
            registry.event(&m.run_id, "restart_checkpoint_unobserved",
                &json!({"phase":phase.id,"round":0,"job":job,"checkpoint":restart,"observed_phase":funding["phase"]}), now()?)?;
            anyhow::bail!(
                "restart checkpoint was not observed before funding advanced; no forced restart or additional paid calls"
            );
        }
        if progress.elapsed() >= Duration::from_secs(15) {
            eprintln!(
                "qualification phase {} awaiting {:?} restart checkpoint; no extra paid calls or funding delays",
                phase.id, restart
            );
            progress = Instant::now();
        }
        rotation::tick(application.running()?, wait.deadline, wait.stop).await?;
    };
    let previous = serde_json::to_value(&application.current.binding)?;
    registry.event(
        &m.run_id,
        "restart_requested",
        &json!({"phase":phase.id,"round":0,
        "pool":pool,"job":job,"checkpoint":restart,"binding":previous,"state":observed}),
        now()?,
    )?;
    eprintln!(
        "qualification phase {} restarting application deliberately: draining for transaction safety; the owned Tor instance remains running",
        phase.id
    );
    let config = application.config()?;
    let closed = application
        .close(registry, m, &config, "lifecycle_restart")
        .await?;
    ensure!(
        closed["complete"] == true,
        "restart stopped after incomplete application cleanup"
    );
    let drained = rotation::snapshot(state).await?;
    restart::continuity(&observed, &drained, &drained, pool, job, checkpoint)?;
    registry.event(
        &m.run_id,
        "restart_drained",
        &json!({"phase":phase.id,"round":0,
        "binding":previous,"closed":closed,"state":drained}),
        now()?,
    )?;
    ensure!(
        !wait.stop.is_cancelled() && Instant::now() < wait.deadline,
        "restart cancelled or deadline reached after safe drain; no new application started"
    );
    application.reopen(registry, m)?;
    let session=application.current.binding.session.clone();
    ready(
        wait.clients,
        wait.snapshot,
        application.running()?,
        (registry,&m.run_id,&session),
        m.limits.result_bytes,
        wait.deadline,
        wait.stop,
    )
    .await?;
    let reopened = rotation::snapshot(state).await?;
    let continuity = restart::continuity(&observed, &drained, &reopened, pool, job, checkpoint)?;
    registry.event(&m.run_id, "restart_completed", &json!({"phase":phase.id,"round":0,
        "previous_binding":previous,"binding":application.current.binding,"state":reopened,"continuity":continuity}), now()?)?;
    Ok(())
}
