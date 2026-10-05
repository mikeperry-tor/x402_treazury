//! One owned Tor across catalog preparation and supervised application execution.
use super::{audit, confinement, identities, owned::Tor};
use crate::{
    execution, files,
    manifest::{Confinement, Manifest, TorMode},
    preparation,
    registry::Registry,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use x402_treazury::{deployment::MetaConfig, network::NetworkPolicy};
pub struct Runtime {
    tor: Option<Tor>,
    events: Option<Vec<String>>,
    evidence: PathBuf,
    profile: PathBuf,
    socks: std::net::SocketAddr,
    pub outage_completed: bool,
    pub stop: tokio_util::sync::CancellationToken,
}
impl Runtime {
    pub fn faults(&self) -> Result<[tokio_util::sync::CancellationToken; 2]> {
        Ok(self
            .tor
            .as_ref()
            .context("owned Tor is not running")?
            .faults())
    }
    async fn stop(&mut self) -> Result<()> {
        if let Some(tor) = self.tor.take() {
            let events = tor.stop(&self.evidence).await?;
            let path = self.evidence.join("tor-events.log");
            files::publish(
                &self.evidence.join("control-completion.json"),
                &serde_json::to_vec_pretty(&json!({"version":1,"complete":true,
                    "event_count":events.len(),"events_sha256":files::hash_file(&path)?}))?,
            )?;
            self.events = Some(events);
        }
        ensure!(
            self.events.is_some(),
            "owned Tor shutdown evidence is incomplete"
        );
        Ok(())
    }
    pub async fn outage(&mut self) -> Result<()> {
        ensure!(
            self.tor.is_some(),
            "outage must stop this session's running owned Tor exactly once"
        );
        eprintln!(
            "Tor outage qualification: deliberately stopping only the owned Tor; the MCP application stays running"
        );
        self.stop().await?;
        match x402_treazury::network::local_control(self.socks, Duration::from_secs(2)).await {
            Ok(_) => anyhow::bail!("owned SOCKS listener still accepts connections after shutdown"),
            Err(error) => ensure!(
                error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionRefused),
                "SOCKS closure was not an explicit connection refusal: {error:#}"
            ),
        }
        let evidence = self.evidence.join("outage-egress");
        files::create_dir(&evidence)?;
        confinement::qualify_with_cancel(
            &std::env::current_exe()?.canonicalize()?,
            &self.profile,
            &evidence,
            self.stop.clone(),
        )
        .await?;
        Ok(())
    }
}
pub async fn prepare(state: &Path, m: &Manifest) -> Result<u8> {
    supervise(state, m, false, true).await
}
pub async fn run(state: &Path, m: &Manifest, allow: bool) -> Result<u8> {
    supervise(state, m, allow, false).await
}
// Admission is checked before installing signals, opening the registry or launching Tor.
fn qualification_policy(m: &Manifest, allow: bool) -> Result<(bool, bool)> {
    m.validate()?;
    let managed = !m.start.pools.is_empty();
    let outage_required = m
        .phases
        .iter()
        .any(|p| matches!(p.scenario, crate::manifest::Scenario::TorOutage { .. }));
    ensure!(
        m.limits.new_funding_jobs == 0 || allow,
        "positive Tor funding requires --allow-funding"
    );

    ensure!(
        m.network.tor_mode == TorMode::Owned
            && m.network.confinement == Confinement::MacosSandbox
            && m.network.require_isolation_evidence,
        "owned Tor requires macOS confinement and required isolation evidence"
    );
    ensure!(
        managed || m.cases.iter().all(|c| c.unsigned),
        "paid Tor qualification requires managed pools"
    );
    ensure!(
        !managed || !outage_required,
        "managed Tor outage is unsupported; use a separate keyless deployment"
    );
    Ok((managed, outage_required))
}
fn qualification_scope(managed: bool, outage_qualified: bool) -> &'static str {
    if managed {
        "confined managed MCP, canonical debit evidence and Tor credential/destination separation; no claim of complete unlinkability"
    } else if outage_qualified {
        "unsigned discovery identities, confined MCP and stopped-Tor help cache; no payment or wallet isolation claim"
    } else {
        "unsigned discovery identities and confined MCP; no outage, payment or wallet isolation claim"
    }
}
async fn supervise(state: &Path, m: &Manifest, allow: bool, prepare_only: bool) -> Result<u8> {
    // Preparation never allocates or spends, regardless of declared future authority.
    let (managed, outage_required) = qualification_policy(m, allow || prepare_only)?;
    let mut signals = crate::process::StopSignals::install()?;
    let mut registry = Registry::open(state, false)?;
    let prepared = if prepare_only {
        None
    } else {
        registry.require_unstarted(&m.run_id)?;
        let pins = registry.pins(&m.run_id)?;
        pins.catalogs
            .as_ref()
            .context("run requires prepare")?
            .verify(m, &pins)?;
        let (_, current) = preparation::collect(m, state).await?;
        ensure!(
            current.resolved_config == pins.resolved_config,
            "configuration changed; prepare a new run"
        );
        Some(pins)
    };
    let config: MetaConfig = toml::from_str(std::str::from_utf8(&files::read(&m.deployment)?)?)?;
    let policy: NetworkPolicy = config.network;
    let socks = policy.socks_endpoint.context("SOCKS endpoint missing")?;
    let ports: Vec<_> = config.servers.values().map(|s| s.listen.port()).collect();
    let profile_text = confinement::profile(socks.port(), &ports)?;
    files::directory(&m.evidence_dir)
        .context("Tor evidence root must already exist and be owner-only")?;
    let interval = format!(
        "{}-tor-{}",
        m.run_id,
        if prepare_only { "prepare" } else { "run" }
    );
    let evidence = m.evidence_dir.join(&interval);
    files::create_dir(&evidence)?;
    let profile = evidence.join("client.sb");
    files::publish(&profile, profile_text.as_bytes())?;
    let runner = std::env::current_exe()?.canonicalize()?;
    let confine =
        confinement::qualify_with_cancel(&runner, &profile, &evidence, signals.stop.clone())
            .await?;
    let data = state.join("live-integration/tor-state");
    let tor = Tor::start_with_cancel(
        m.network
            .tor_binary
            .as_deref()
            .context("Tor binary missing")?,
        &data,
        &evidence,
        &policy,
        signals.stop.clone(),
    )
    .await?;
    signals.observe_faults(tor.faults());
    let mut runtime = Runtime {
        tor: Some(tor),
        events: None,
        evidence: evidence.clone(),
        profile: profile.clone(),
        socks,
        outage_completed: false,
        stop: signals.stop.clone(),
    };
    let mut expected = None;
    let outcome = async {
        runtime
            .tor
            .as_mut()
            .context("owned Tor missing")?
            .healthy()?;
        if prepare_only {
            let (plan, pins) = preparation::collect_catalogs_confined(
                m,
                state,
                Some(&profile),
                signals.stop.clone(),
            )
            .await?;
            registry.prepare(
                &plan,
                &pins,
                i64::try_from(x402_treazury::rotation::base::now()?)?,
            )?;
            return Ok(0);
        }
        let pins = prepared.context("prepared pins missing")?;
        let interval_scope = super::scope::Sessions::default();
        let directory = &pins
            .catalogs
            .as_ref()
            .context("catalog pins missing")?
            .directory;
        let snapshot: Value =
            serde_json::from_slice(&files::read_catalog(&directory.join("snapshot.json"))?)?;
        let map = if managed {
            identities::managed_map_frozen(
                m,
                &snapshot,
                state,
                |name| std::env::var(name).ok(),
                &[],
            )?
        } else {
            identities::unsigned_map_frozen(m, &snapshot, state)?
        };
        files::publish(
            &evidence.join("identities.json"),
            &serde_json::to_vec_pretty(&map)?,
        )?;
        expected = Some(map);
        drop(registry);
        runtime
            .tor
            .as_mut()
            .context("owned Tor missing")?
            .healthy()?;
        let execution =
            execution::run_owned_with_funding(state, &m.run_id, &profile, &mut runtime, allow)
                .await;
        if managed {
            let registry = Registry::open(state, true)?;
            let report = registry.report(
                Some(&m.run_id),
                i64::try_from(x402_treazury::rotation::base::now()?)?,
            )?;
            let projection = interval_scope.project(m, &report)?;
            let map = identities::managed_map_frozen(
                &projection.manifest,
                &snapshot,
                state,
                |name| std::env::var(name).ok(),
                &projection.payments,
            )?;
            files::publish(
                &evidence.join("session-scope.json"),
                &serde_json::to_vec_pretty(
                    &json!({"version":1,"sessions":projection.sessions,"catalogs_fetched":false}),
                )?,
            )?;
            files::publish(
                &evidence.join("identities-final.json"),
                &serde_json::to_vec_pretty(&map)?,
            )?;
            expected = Some(map);
        }
        execution
    }
    .await;
    let stopped = runtime.stop().await;
    if prepare_only {
        stopped?;
        let code = outcome?;
        ensure!(
            !signals.stop.is_cancelled(),
            "catalog preparation cancelled"
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"version":1,"run":m.run_id,
            "status":"prepared","tor_isolation":"not_qualified","spending_authorized":false}))?
        );
        return Ok(code);
    }
    let isolation = match (expected, stopped) {
        (Some(expected), Ok(())) => audit::verify(
            &expected,
            runtime.events.as_deref().context("Tor events missing")?,
        ),
        (_, Err(error)) => Err(error),
        (None, _) => Err(anyhow::anyhow!(
            "catalog preparation did not produce an identity map"
        )),
    };
    let passed = outcome.as_ref().is_ok_and(|code| *code == 0)
        && isolation.is_ok()
        && (!outage_required || runtime.outage_completed)
        && !signals.stop.is_cancelled();
    let result = json!({"version":1,"run":m.run_id,"tor_interval":interval,"resumed":false,"confinement":confine,
        "mcp_exit_code":outcome.as_ref().ok(),"isolation":isolation.as_ref().ok(),
        "outage":if runtime.outage_completed {"qualified"}else if outage_required {"incomplete"}else{"not_requested"},
        "qualification":if passed {"qualified"}else{"incomplete"},
        "scope":qualification_scope(managed, outage_required && runtime.outage_completed)});
    files::publish(
        &evidence.join("qualification.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    if let Err(error) = outcome {
        eprintln!("Tor qualification execution failed: {error:#}");
    }
    if let Err(error) = isolation {
        eprintln!("Tor isolation evidence failed: {error:#}");
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(if passed { 0 } else { 3 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Scenario;
    fn unsigned() -> Manifest {
        let mut m = crate::tests::manifest();
        m.start.pools.clear();
        m.phases[0].pools.clear();
        m.phases[0].scenario = Scenario::Unsigned {};
        m.cases[0].unsigned = true;
        m.cases[0].reserve_usdc = "0".into();
        m.network.tor_mode = TorMode::Owned;
        m.network.tor_binary = Some("/unused/tor".into());
        m.network.confinement = Confinement::MacosSandbox;
        m.network.require_isolation_evidence = true;
        m
    }
    #[test]
    fn unsigned_owned_tor_admits_without_forcing_outage() {
        let m = unsigned();
        assert_eq!(qualification_policy(&m, false).unwrap(), (false, false));
        let scope = qualification_scope(false, false);
        assert!(scope.contains("no outage, payment or wallet isolation claim"));
        assert!(!scope.contains("stopped-Tor help cache"));
        assert!(qualification_scope(false, true).contains("stopped-Tor help cache"));
    }
    #[test]
    fn optional_outage_does_not_relax_payment_or_confinement_admission() {
        let mut m = unsigned();
        m.cases[0].unsigned = false;
        m.cases[0].reserve_usdc = "0.02".into();
        assert!(qualification_policy(&m, false).is_err());
        let mut m = unsigned();
        m.network.confinement = Confinement::None;
        assert!(qualification_policy(&m, false).is_err());
        let mut m = unsigned();
        m.network.require_isolation_evidence = false;
        assert!(qualification_policy(&m, false).is_err());
        let mut m = unsigned();
        m.start.pools.push("pool".into());
        m.phases[0].scenario = Scenario::TorOutage {
            warm_case: "warm".into(),
            cached_case: "cached".into(),
            uncached_case: "uncached".into(),
        };
        assert!(
            qualification_policy(&m, false)
                .unwrap_err()
                .to_string()
                .contains("separate keyless deployment")
        );
        m.phases[0].scenario = Scenario::Unsigned {};
        m.limits.new_funding_jobs = 1;
        m.limits.source_exposure_zec = "0.01".into();
        assert!(qualification_policy(&m, false).is_err());
        assert_eq!(qualification_policy(&m, true).unwrap(), (true, false));
    }
}
