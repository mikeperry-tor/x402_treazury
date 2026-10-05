//! Wait for production bootstrap; never promotes, allocates or changes balances.
use super::*;
pub(super) fn ready(snapshot: &Value, pools: &[String]) -> Result<bool> {
    let actual = snapshot["treasury_status"]["pools"]
        .as_array()
        .context("bootstrap pool snapshot missing")?;
    Ok(pools.iter().all(|name| {
        actual.iter().any(|p| {
            p["name"] == *name
                && p["bootstrapped"] == true
                && p["enabled"] == true
                && p["addresses"]
                    .as_array()
                    .is_some_and(|wallets| wallets.iter().any(|w| w["role"] == "ACTIVE"))
        })
    }))
}
pub(super) async fn wait(
    registry: &mut Registry,
    manifest: &Manifest,
    state: &Path,
    child: &mut Process,
    deadline: Instant,
    stop: &CancellationToken,
) -> Result<()> {
    eprintln!(
        "qualification waiting for treasury funding, swap completion and confirmed active/standby bootstrap; no API calls dispatched yet"
    );
    registry.event(
        &manifest.run_id,
        "bootstrap_wait_started",
        &json!({"pools":manifest.start.pools}),
        now()?,
    )?;
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    let mut progress = Instant::now();
    loop {
        ensure!(
            !child.exited()? && !child.fault.is_cancelled(),
            "application failed during bootstrap"
        );
        tokio::select! {
            _ = stop.cancelled() => anyhow::bail!("bootstrap cancelled; accepted funding work remains durable"),
            _ = tokio::time::sleep_until(deadline) => anyhow::bail!("bootstrap deadline reached; inspect funding warnings/status; no API calls dispatched"),
            _ = ticks.tick() => {},
        }
        let state = state.to_owned();
        let snapshot = tokio::task::spawn_blocking(move || {
            x402_treazury::rotation::store::qualification_state(&state)
        })
        .await??;
        ensure!(
            snapshot["treasury_status"]["treasury_id"] == manifest.treasury_id,
            "bootstrap treasury mismatch"
        );
        if ready(&snapshot, &manifest.start.pools)? {
            registry.event(
                &manifest.run_id,
                "bootstrap_ready",
                &json!({"pools":manifest.start.pools,
                "snapshot":snapshot["treasury_status"]}),
                now()?,
            )?;
            eprintln!(
                "qualification managed pools bootstrapped; proceeding with reviewed API cases"
            );
            return Ok(());
        }
        if progress.elapsed() >= Duration::from_secs(15) {
            let jobs = snapshot["treasury_status"]["funding_jobs"]
                .as_array()
                .context("funding job snapshot missing")?;
            let failures: Vec<_> = jobs
                .iter()
                .filter_map(|j| j["last_error"].as_str())
                .collect();
            eprintln!(
                "qualification still waiting for managed bootstrap; funding_jobs={}, recorded_funding_errors={}; source insufficiency and refill failures remain in application stderr/status",
                jobs.len(),
                failures.len()
            );
            registry.event(
                &manifest.run_id,
                "bootstrap_wait_progress",
                &json!({"funding_jobs":jobs.len(),"recorded_errors":failures.len()}),
                now()?,
            )?;
            progress = Instant::now();
        }
    }
}
/// Keep the child reconciliation tasks alive while successful dispatched cases
/// await canonical evidence. Failed/unattempted prerequisites cannot be repaired
/// by waiting, and no paid case is retried.
pub(super) async fn wait_evidence(
    registry: &mut Registry,
    manifest: &Manifest,
    phase: Option<&crate::manifest::Phase>,
    child: &mut Process,
    deadline: Instant,
    stop: &CancellationToken,
) -> Result<()> {
    let mut progress = Instant::now() - Duration::from_secs(15);
    loop {
        registry.observe_settlement(&manifest.run_id, false)?;
        let report = registry.report(Some(&manifest.run_id), now()?)?;
        let debits = registry.verified_debits(&manifest.run_id)?;
        let waiting = waiting_count(&report, manifest, phase, &debits)?;
        if waiting == 0 {
            return Ok(());
        }
        ensure!(
            !child.exited()? && !child.fault.is_cancelled(),
            "application failed while awaiting payment evidence"
        );
        if progress.elapsed() >= Duration::from_secs(15) {
            eprintln!(
                "qualification awaiting canonical payment evidence for {waiting} cases; no paid calls are retried"
            );
            progress = Instant::now();
        }
        tokio::select! {
            _ = stop.cancelled() => anyhow::bail!("payment observation cancelled; reservations remain held"),
            _ = tokio::time::sleep_until(deadline) => anyhow::bail!("payment observation deadline reached; unresolved debit evidence remains incomplete"),
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
        }
    }
}

pub(crate) fn waiting_count(
    report: &Value,
    manifest: &Manifest,
    phase: Option<&crate::manifest::Phase>,
    debits: &BTreeMap<String, Value>,
) -> Result<usize> {
    let cases = report["cases"].as_array().context("case report missing")?;
    let selected = |case: &&Value| {
        phase.is_none_or(|phase| {
            phase.depends_on.iter().any(|id| {
                manifest
                    .phases
                    .iter()
                    .any(|p| p.id == *id && p.cases.iter().any(|c| case["case"] == *c))
            })
        })
    };
    let mut selected: Vec<_> = cases
        .iter()
        .filter(selected)
        .filter(|c| c["execution"] != "SKIPPED_TARGET_REACHED")
        .collect();
    if phase.is_some()
        && selected
            .iter()
            .any(|c| c["execution"] != "COMPLETED" || c["semantic"] != "PASSED")
    {
        return Ok(0);
    }
    // Independent failed cases must not prevent observing successful peers.
    selected.retain(|c| c["execution"] == "COMPLETED" && c["semantic"] == "PASSED");
    let admitted = |case: &&&Value| {
        report["runtime_events"].as_array().is_some_and(|events| {
            events
                .iter()
                .any(|e| e["kind"] == "application_payment" && e["detail"]["case"] == case["case"])
        })
    };
    let waiting = selected
        .iter()
        .filter(admitted)
        .filter(|c| c["settlement"] == "PENDING" || !super::debit_complete(c, manifest, debits))
        .count();

    Ok(waiting)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_uses_production_bootstrap_and_does_not_demand_an_unspent_active_on_resume() {
        let names = vec!["a".into(), "b".into()];
        let mut snapshot = json!({"treasury_status":{"pools":[
            {"name":"a","enabled":true,"bootstrapped":true,"addresses":[{"role":"ACTIVE","confirmed_balance":"1"}]},
            {"name":"b","enabled":true,"bootstrapped":false,"addresses":[{"role":"ALLOCATED","confirmed_balance":"2000000"}]}
        ]}});
        assert!(!ready(&snapshot, &names).unwrap());
        snapshot["treasury_status"]["pools"][1]["bootstrapped"] = json!(true);
        assert!(!ready(&snapshot, &names).unwrap());
        snapshot["treasury_status"]["pools"][1]["addresses"][0]["role"] = json!("ACTIVE");
        assert!(ready(&snapshot, &names).unwrap());
        snapshot["treasury_status"]["pools"][0]["enabled"] = json!(false);
        assert!(!ready(&snapshot, &names).unwrap());
    }
}
