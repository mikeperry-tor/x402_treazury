//! Natural single-rotation dispatch. No forced promotion or paid replay.
use super::*;
use crate::manifest::{Phase, RotationRound};
use std::path::PathBuf;
use x402_treazury::qualification::{depletion_estimate, lifecycle::promotion};

pub(super) struct Controller {
    phase: Phase,
    round: RotationRound,
    round_index: usize,
    before: Value,
    pool: String,
    generation: u64,
    job: Option<String>,
    state: PathBuf,
}
pub(super) async fn snapshot(state: &Path) -> Result<Value> {
    let state = state.to_owned();
    let snapshot = tokio::task::spawn_blocking(move || {
        x402_treazury::rotation::store::qualification_state(&state)
    })
    .await??;
    Ok(snapshot["lifecycle"].clone())
}
fn pool<'a>(state: &'a Value, id: &str) -> Result<&'a Value> {
    state["pools"]
        .as_array()
        .context("lifecycle pools missing")?
        .iter()
        .find(|p| p["id"] == id)
        .context("selected lifecycle pool missing")
}
pub(super) async fn tick(
    child: &mut Process,
    deadline: Instant,
    stop: &CancellationToken,
) -> Result<()> {
    ensure!(
        !child.exited()? && !child.fault.is_cancelled(),
        "application failed while observing rotation"
    );
    ensure!(
        Instant::now() < deadline,
        "rotation observation deadline reached; retained evidence is incomplete"
    );
    tokio::select! {
        _ = stop.cancelled() => anyhow::bail!("rotation cancelled; accepted work remains durable"),
        _ = child.fault.cancelled() => anyhow::bail!("rotation application evidence failed"),
        _ = tokio::time::sleep_until(deadline) => anyhow::bail!("rotation observation deadline reached"),
        _ = tokio::time::sleep(Duration::from_millis(500)) => Ok(()),
    }
}
impl Controller {
    pub(super) async fn start(
        registry: &mut Registry,
        m: &Manifest,
        (phase, round_index): (&Phase, usize),
        child: &mut Process,
        deadline: Instant,
        stop: &CancellationToken,
    ) -> Result<Option<Self>> {
        let rounds = match &phase.scenario {
            Scenario::Rotation { rounds, .. }
            | Scenario::RefillService { rounds, .. }
            | Scenario::Lifecycle { rounds, .. } => rounds,
            _ => return Ok(None),
        };
        let round = rounds
            .get(round_index)
            .context("rotation round missing")?
            .clone();
        let round_cases: Vec<_> = round
            .depletion_cases
            .iter()
            .chain(&round.service_cases)
            .collect();
        if round_cases.iter().all(|id| {
            registry
                .execution_state(&m.run_id, id)
                .is_ok_and(|s| s != "UNATTEMPTED")
        }) {
            return Ok(None); // observation/reporting only; never replay a completed phase
        }
        ensure!(
            round_cases.iter().all(|id| registry
                .execution_state(&m.run_id, id)
                .is_ok_and(|s| s == "UNATTEMPTED")),
            "partially dispatched rotation is observation-only; no automatic paid resumption"
        );
        registry.dependencies_ready(&m.run_id, &phase.id)?;
        let report = registry.report(Some(&m.run_id), now()?)?;
        ensure!(
            !report["runtime_events"]
                .as_array()
                .context("runtime events missing")?
                .iter()
                .any(|e| e["kind"] == "rotation_started"
                    && e["detail"]["phase"] == phase.id
                    && e["detail"]
                        .get("round")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        == round_index as u64),
            "rotation already started; retain observations rather than create another baseline"
        );
        let prior = if round_index > 0 {
            let phase_report = report["rotation"]
                .as_array()
                .context("rotation reports missing")?
                .iter()
                .find(|r| r["phase"] == phase.id)
                .context("prior rotation report missing")?;
            let previous = phase_report["rounds"]
                .get(round_index - 1)
                .context("prior rotation round report missing")?;
            ensure!(
                previous["rotation_status"] == "passed",
                "preceding rotation must have verified service and ready refill before next round"
            );
            Some(
                previous["generation"]
                    .as_u64()
                    .context("previous promoted generation missing")?,
            )
        } else {
            None
        };
        let state = config_state(&registry.pins(&m.run_id)?.resolved_config)?;
        let initial = snapshot(&state).await?;
        let pool = initial["pools"]
            .as_array()
            .context("rotation pools missing")?
            .iter()
            .find(|p| p["name"] == round.pool)
            .context("rotation pool missing")?["id"]
            .as_str()
            .context("rotation pool ID missing")?
            .to_owned();
        let request = registry.request_pool_observation(&m.run_id, &pool, &round.pool, now()?)?;
        let mut progress = Instant::now() - Duration::from_secs(15);
        let observation = loop {
            if let Some(observation) = registry.pool_observation(&m.run_id, &request)? {
                break observation;
            }
            if progress.elapsed() >= Duration::from_secs(15) {
                eprintln!(
                    "qualification phase {} awaiting fresh pool balance and unresolved authorization evidence; no depletion calls dispatched",
                    phase.id
                );
                progress = Instant::now();
            }
            tick(child, deadline, stop).await?;
        };
        let selected = observation["state"]["pools"]
            .as_array()
            .context("observed pools missing")?
            .iter()
            .find(|p| p["id"] == pool)
            .context("observed pool missing")?;
        ensure!(
            selected["enabled"] == true && selected["bootstrapped"] == true,
            "rotation requires an enabled bootstrapped pool"
        );
        let price = crate::manifest::usdc(&round.expected_price_usdc)?.to_string();
        let estimate = depletion_estimate(&observation, &price, round.depletion_cases.len())?;
        eprintln!(
            "qualification phase {}: estimated {} depletion calls including promotion trigger, {} atomic USDC at reviewed price; {} explicit calls available",
            phase.id,
            estimate["estimated_calls"],
            estimate["estimated_depletion_fees_atomic"],
            round.depletion_cases.len()
        );
        registry.event(
            &m.run_id,
            "rotation_started",
            &json!({"phase":phase.id,"round":round_index,"request":request,"estimate":estimate}),
            now()?,
        )?;
        let generation = estimate["generation"]
            .as_u64()
            .context("rotation generation missing")?;
        ensure!(
            prior.is_none_or(|expected| expected == generation),
            "pool generation changed between rotation rounds"
        );
        Ok(Some(Self {
            phase: phase.clone(),
            round,
            round_index,
            before: observation["state"].clone(),
            pool,
            generation,
            job: None,
            state,
        }))
    }
    pub(super) fn restart_target(&self) -> Result<(&Path, &str, &str)> {
        Ok((
            &self.state,
            &self.pool,
            self.job
                .as_deref()
                .context("restart requires an observed promotion")?,
        ))
    }
    pub(super) fn before_case(&self, id: &str) -> Result<()> {
        if self.round.service_cases.iter().any(|c| c == id) {
            ensure!(
                self.job.is_some(),
                "reviewed depletion calls exhausted without promotion; no unplanned calls or service dispatch"
            );
        } else {
            ensure!(
                self.round.depletion_cases.iter().any(|c| c == id) && self.job.is_none(),
                "depletion cannot continue after its promotion boundary"
            );
        }
        Ok(())
    }
    pub(super) async fn after_case(
        &mut self,
        registry: &mut Registry,
        m: &Manifest,
        case: &str,
        child: &mut Process,
        deadline: Instant,
        stop: &CancellationToken,
    ) -> Result<()> {
        ensure!(
            registry.execution_state(&m.run_id, case)? == "COMPLETED",
            "uncertain depletion/service call stops further rotation dispatch; observe only"
        );
        let after = snapshot(&self.state).await?;
        let generation = pool(&after, &self.pool)?["generation"]
            .as_u64()
            .context("observed generation missing")?;
        if self.job.is_some() {
            ensure!(
                generation == self.generation + 1,
                "unexpected additional promotion during post-rotation service"
            );
        } else if generation != self.generation {
            let job = promotion(&self.before, &after, &self.pool)?;
            registry.stop_depletion(
                &m.run_id,
                &self.phase.id,
                self.round_index,
                &self.before,
                &after,
                now()?,
            )?;
            self.job = Some(job);
        } else {
            // Resolve each depletion payment before requesting another challenge.
            // After promotion, service must proceed immediately rather than wait
            // for either the triggering receipt or the new funding job.
            bootstrap::wait_evidence(registry, m, None, child, deadline, stop).await?;
        }
        Ok(())
    }
    pub(super) async fn finish(
        &self,
        registry: &Registry,
        m: &Manifest,
        child: &mut Process,
        deadline: Instant,
        stop: &CancellationToken,
    ) -> Result<()> {
        let job = self
            .job
            .as_deref()
            .context("rotation did not occur within reviewed depletion calls")?;
        let mut progress = Instant::now() - Duration::from_secs(15);
        loop {
            let state = snapshot(&self.state).await?;
            if crate::rotation_report::refill_ready(&state, &self.pool, job, self.generation + 1)? {
                registry.event(
                    &m.run_id,
                    "rotation_refill_ready",
                    &json!({"phase":self.phase.id,"round":self.round_index,"job":job,"state":state}),
                    now()?,
                )?;
                return Ok(());
            }
            if progress.elapsed() >= Duration::from_secs(15) {
                eprintln!(
                    "qualification phase {} awaiting confirmed replacement credit; post-promotion service calls have completed; funding insufficiency/failures remain in application warnings",
                    self.phase.id
                );
                progress = Instant::now();
            }
            tick(child, deadline, stop).await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dispatch_switches_from_depletion_to_separate_service_without_extra_calls() {
        let phase = crate::tests::manifest().phases.remove(0);
        let mut controller = Controller {
            phase,
            round: RotationRound {
                pool: "pool".into(),
                expected_price_usdc: "0.01".into(),
                depletion_cases: vec!["deplete".into()],
                service_cases: vec!["service".into()],
            },
            round_index: 0,
            before: json!({}),
            pool: "p".into(),
            generation: 0,
            job: None,
            state: PathBuf::new(),
        };
        controller.before_case("deplete").unwrap();
        assert!(controller.before_case("service").is_err());
        assert!(controller.before_case("unreviewed").is_err());
        controller.job = Some("replacement".into());
        controller.before_case("service").unwrap();
        assert!(controller.before_case("deplete").is_err());
    }
}
