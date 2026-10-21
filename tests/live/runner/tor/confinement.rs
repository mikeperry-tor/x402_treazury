//! macOS profile with positive and denied local egress controls.
use crate::{files, process::Process};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
pub fn profile(socks: u16, mcp: &[u16]) -> Result<String> {
    ensure!(
        socks != 0 && mcp.iter().all(|p| *p != 0),
        "confinement ports must be nonzero"
    );
    let mut seen = BTreeSet::from([socks]);
    for port in mcp {
        ensure!(
            seen.insert(*port),
            "confinement SOCKS/MCP ports must be distinct"
        );
    }
    let mut result = format!(
        "(version 1)\n(allow default)\n(deny network*)\n(allow network-outbound (remote tcp \"localhost:{socks}\"))\n"
    );
    for port in mcp {
        result.push_str(&format!("(allow network-bind (local tcp \"localhost:{port}\"))\n(allow network-inbound (local tcp \"localhost:{port}\"))\n"));
    }
    Ok(result)
}
pub async fn qualify(runner: &Path, profile: &Path, evidence: &Path) -> Result<serde_json::Value> {
    let signals = crate::process::StopSignals::install()?;
    qualify_with_cancel(runner, profile, evidence, signals.stop.clone()).await
}
pub async fn qualify_with_cancel(
    runner: &Path,
    profile: &Path,
    evidence: &Path,
    stop: CancellationToken,
) -> Result<serde_json::Value> {
    ensure!(
        cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").is_file(),
        "required macOS confinement is unavailable; refusing proxy-only qualification"
    );
    let probes = x402_treazury::network::QualificationProbes::start().await?;
    let mut args = vec!["egress-probe".into()];
    for (name, address) in &probes.addresses {
        args.push(format!("--{name}"));
        args.push(address.to_string());
    }
    for (label, confinement) in [("positive", None), ("denied", Some(profile))] {
        let process = Process::launch_confined(
            runner,
            &args,
            &BTreeMap::new(),
            evidence,
            65536,
            confinement,
        )?;
        let output = process
            .wait(
                Duration::from_secs(15),
                Duration::from_secs(5),
                stop.clone(),
            )
            .await?;
        files::publish(
            &evidence.join(format!("egress-{label}.stdout")),
            &output.stdout.bytes,
        )?;
        files::publish(
            &evidence.join(format!("egress-{label}.stderr")),
            &output.stderr.bytes,
        )?;
        files::publish(
            &evidence.join(format!("egress-{label}.process.json")),
            &serde_json::to_vec_pretty(&output)?,
        )?;
        ensure!(
            output.success && output.valid_output() && !output.forced_kill && !stop.is_cancelled(),
            "{label} egress probe failed or lost output evidence"
        );
        let values: serde_json::Value = serde_json::from_slice(&output.stdout.bytes)?;
        ensure!(
            values
                .as_object()
                .context("probe result must be object")?
                .len()
                == 4,
            "missing/unexpected confinement probe outcomes"
        );
        for name in probes.addresses.keys() {
            ensure!(
                if label == "positive" {
                    values[name]["reached"] == true && values[name]["denied"] == false
                } else {
                    values[name]["reached"] == false && values[name]["denied"] == true
                },
                "{label} egress probe {name} did not establish the required outcome"
            );
        }
        // Yield until positive TCP accepts are observed; this is a bounded local
        // handshake check, not a proxy-connect failure used as proof of confinement.
        let until = tokio::time::Instant::now() + Duration::from_secs(1);
        while probes.counts().contains(&0) && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        ensure!(
            probes.counts() == [1, 1, 1, 1],
            "forbidden traffic reached local control listeners or positive controls were incomplete"
        );
    }
    let result = serde_json::json!({"status":"qualified","backend":"macos_sandbox_exec","positive_counts":probes.counts(),"denied_tcp_ipv4":true,"denied_tcp_ipv6":true,"denied_udp_ipv4":true,"denied_udp_ipv6":true,"inbound_mcp":"requires successful confined application startup and MCP exchange"});
    files::publish(
        &evidence.join("confinement.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_only_allows_socks_egress_and_declared_inbound_ports() {
        let text = profile(19050, &[18000, 18001]).unwrap();
        assert_eq!(text.matches("network-outbound").count(), 1);
        assert!(text.contains("(deny network*)"));
        assert!(text.contains("(remote tcp \"localhost:19050\")"));
        assert!(text.contains("(local tcp \"localhost:18000\")"));
        assert!(!text.contains("network-outbound (remote tcp \"localhost:18000"));
        assert!(profile(0, &[]).is_err());
        assert!(profile(19050, &[19050]).is_err());
        assert!(profile(19050, &[18000, 18000]).is_err());
    }
}
