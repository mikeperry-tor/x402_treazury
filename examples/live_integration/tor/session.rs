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
            self.events = Some(tor.stop(&self.evidence).await?);
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
pub async fn qualify(state: &Path, m: &Manifest) -> Result<u8> {
    ensure!(
        m.network.tor_mode == TorMode::Owned
            && m.network.confinement == Confinement::MacosSandbox
            && m.network.require_isolation_evidence,
        "qualify-tor requires owned Tor, macOS confinement and required isolation evidence"
    );
    ensure!(
        m.cases.iter().all(|c| c.unsigned),
        "Tor qualification currently refuses paid cases"
    );
    ensure!(
        m.phases
            .iter()
            .filter(|p| matches!(p.scenario, crate::manifest::Scenario::TorOutage { .. }))
            .count()
            == 1,
        "Tor qualification requires exactly one explicit final outage phase"
    );
    let mut signals = crate::process::StopSignals::install()?;
    let mut registry = Registry::open(state, false)?;
    let _ = preparation::collect(m, state).await?;
    let config: MetaConfig = toml::from_str(std::str::from_utf8(&files::read(&m.deployment)?)?)?;
    let policy: NetworkPolicy = config.network;
    let socks = policy.socks_endpoint.context("SOCKS endpoint missing")?;
    let ports: Vec<_> = config.servers.values().map(|s| s.listen.port()).collect();
    let profile_text = confinement::profile(socks.port(), &ports)?;
    let evidence = m.evidence_dir.join(format!("{}-tor", m.run_id));
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
        let (plan, pins) =
            preparation::collect_catalogs_confined(m, state, Some(&profile), signals.stop.clone())
                .await?;
        let directory = &pins
            .catalogs
            .as_ref()
            .context("catalog pins missing")?
            .directory;
        let snapshot: Value =
            serde_json::from_slice(&files::read(&directory.join("snapshot.json"))?)?;
        let map = identities::unsigned_map(m, &snapshot, state)?;
        files::publish(
            &evidence.join("identities.json"),
            &serde_json::to_vec_pretty(&map)?,
        )?;
        expected = Some(map);
        registry.prepare(
            &plan,
            &pins,
            i64::try_from(x402_treazury::rotation::base::now()?)?,
        )?;
        drop(registry);
        runtime
            .tor
            .as_mut()
            .context("owned Tor missing")?
            .healthy()?;
        execution::run_owned(state, &m.run_id, &profile, &mut runtime).await
    }
    .await;
    let stopped = runtime.stop().await;
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
        && runtime.outage_completed
        && !signals.stop.is_cancelled();
    let result = json!({"version":1,"run":m.run_id,"confinement":confine,
        "mcp_exit_code":outcome.as_ref().ok(),"isolation":isolation.as_ref().ok(),
        "outage":if runtime.outage_completed {"qualified"}else{"incomplete"},
        "qualification":if passed {"qualified"}else{"incomplete"},
        "scope":"unsigned discovery identities, confined MCP and stopped-Tor help cache; no payment or wallet isolation claim"});
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
