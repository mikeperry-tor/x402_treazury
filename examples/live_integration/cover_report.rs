//! Parse exactly one application-owned shutdown summary from complete private output.
use crate::{manifest::Manifest, process::Evidence};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use x402_treazury::{
    cover::metrics::{Metrics, REPORT_PREFIX},
    deployment::MetaConfig,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    connection_affinity: String,
    metrics: Metrics,
}
pub fn collect(config: &MetaConfig, manifest: &Manifest, evidence: &Evidence) -> Result<Value> {
    ensure!(
        evidence.valid_output(),
        "cover evidence requires complete, untruncated child output"
    );
    let reports: Vec<_> = evidence
        .stderr
        .bytes
        .split(|b| *b == b'\n')
        .filter_map(|line| line.strip_prefix(REPORT_PREFIX.as_bytes()))
        .collect();
    if !config.network.cover_enabled() {
        ensure!(
            reports.is_empty(),
            "disabled cover emitted a runtime summary"
        );
        return Ok(json!({"status":"disabled"}));
    }
    ensure!(
        reports.len() == 1,
        "expected exactly one cover shutdown summary; absent/duplicate evidence cannot qualify"
    );
    let envelope: Envelope = serde_json::from_slice(reports[0])?;
    ensure!(
        envelope.version == 1 && envelope.connection_affinity == "pooled_best_effort_unobserved",
        "unsupported cover evidence version/affinity"
    );
    let m = envelope.metrics;
    ensure!(
        m.in_flight_ranges == 0
            && m.peak_in_flight_ranges <= config.network.cover_limits.max_active_streams as u64,
        "cover streams were not cleaned up or exceeded admission limits"
    );
    let total = m
        .range_outcomes
        .values()
        .try_fold(0u64, |a, b| a.checked_add(*b))
        .context("cover outcome count overflow")?;
    ensure!(
        total == m.range_requests
            && m.range_outcomes.get("qualified").copied().unwrap_or(0) == m.qualified_ranges,
        "inconsistent cover request accounting"
    );
    let mut range_requested = false;
    let mut padding_requested = false;
    for case in &manifest.cases {
        if let Some(source) = config.sources.get(&case.source) {
            // Prepared deployments freeze fully resolved source settings, including
            // automatically selected cover resources. Help calls never run cover.
            if source
                .provider
                .get("cover_traffic_enabled")
                .and_then(toml::Value::as_bool)
                == Some(false)
            {
                continue;
            }
            if source.provider.contains_key("help_url")
                && source
                    .provider
                    .get("prefix")
                    .and_then(toml::Value::as_str)
                    .is_some_and(|p| case.tool == format!("{p}_help"))
            {
                continue;
            }
            if let Some(table) = source.provider.get("cover_traffic") {
                let cover: x402_treazury::cover::Config = table.clone().try_into()?;
                range_requested |= cover.ranges_enabled;
                padding_requested |= cover.padding.is_some();
            }
        }
    }
    let observed = (!range_requested || m.qualified_ranges > 0)
        && (!padding_requested || m.padding_requests > 0);
    Ok(
        json!({"status":"observed","requested_modes_observed":observed,"range_requested":range_requested,"padding_requested":padding_requested,"at_least_one_range_qualified":m.qualified_ranges>0,"padding_dispatched":m.padding_requests>0,"metrics":m,"connection_affinity":envelope.connection_affinity,"measurement_layers":{"body":"application cover reader; includes observed overrun chunks","padding":"uncompressed header values dispatched; not TLS bytes"},"wire_padding_qualification":"local fixtures only","privacy_qualification":false}),
    )
}
pub fn require_samples(report: &Value) -> Result<()> {
    ensure!(
        report["status"] == "disabled" || report["requested_modes_observed"] == true,
        "requested cover modes have no successful samples; real-call and refusal evidence retained, cover qualification incomplete"
    );
    Ok(())
}
pub fn combine(sessions: &[Value]) -> Result<Value> {
    ensure!(
        !sessions.is_empty(),
        "cover report lacks closed application sessions"
    );
    let mut seen = std::collections::BTreeSet::new();
    for session in sessions {
        let id = session["session"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("cover session identity missing")?;
        ensure!(
            seen.insert(id) && session["complete"] == true,
            "duplicate or incomplete cover session"
        );
        ensure!(
            session["cover"].is_object(),
            "session cover summary missing"
        );
    }
    if sessions.len() == 1 {
        return Ok(sessions[0]["cover"].clone());
    }
    Ok(
        json!({"status":"multiple_sessions", "requested_modes_observed":sessions.iter().all(|s| require_samples(&s["cover"]).is_ok()),
        "sessions":sessions,"scope":"every application session must have complete output and its own requested cover samples"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requested_cover_cannot_qualify_without_observed_samples() {
        assert!(require_samples(&json!({"status":"disabled"})).is_ok());
        assert!(
            require_samples(&json!({"status":"observed","requested_modes_observed":true})).is_ok()
        );
        assert!(
            require_samples(&json!({"status":"observed","requested_modes_observed":false}))
                .is_err()
        );
        assert!(require_samples(&json!({"status":"observed"})).is_err());
    }
    #[test]
    fn restarted_sessions_each_need_complete_output_and_requested_samples() {
        let a = json!({"session":"a","complete":true,"cover":{"status":"observed","requested_modes_observed":true}});
        let mut b = json!({"session":"b","complete":true,"cover":{"status":"observed","requested_modes_observed":false}});
        assert!(require_samples(&combine(&[a.clone(), b.clone()]).unwrap()).is_err());
        b["cover"]["requested_modes_observed"] = json!(true);
        require_samples(&combine(&[a.clone(), b.clone()]).unwrap()).unwrap();
        assert!(combine(&[a.clone(), a]).is_err());
        b["complete"] = json!(false);
        assert!(combine(&[b]).is_err());
        assert!(combine(&[]).is_err());
    }
    fn manifest() -> Manifest {
        serde_json::from_value(json!({
        "version":1,"run_id":"test_run","treasury_id":"11111111-1111-4111-8111-111111111111",
        "deployment":"deploy.toml","binary":"x402_treazury","evidence_dir":"evidence","registry_authorization":"review_1",
        "start":{"mode":"funded_pools","pools":["pool"]},
        "network":{"tor_mode":"direct","confinement":"none","require_isolation_evidence":false},
        "limits":{"api_reservation_usdc":"0.04","new_funding_jobs":0,"source_exposure_zec":"0","max_in_flight":2,"run_seconds":900,"phase_seconds":600,"call_seconds":300,"cleanup_seconds":900,"result_bytes":1000},
        "catalog":{"execution":"frozen","record_live_discovery":true},
        "windows":[{"id":"now","not_before":1,"not_after":1000}],
        "cases":[{"id":"call","server":"main","source":"api","tool":"api_read","arguments":{},"reserve_usdc":"0.02","reviewed_read_only":true}],
        "phases":[{"id":"smoke","window":"now","cases":["call"],"pools":["pool"],"required":true,"scenario":{"kind":"smoke"}}]
    })).unwrap()
    }
    #[test]
    fn complete_unique_summary_and_accounting_are_required() {
        let mut config: MetaConfig =
            serde_json::from_value(json!({"version":1,"servers":{}})).unwrap();
        config.network.cover_traffic_enabled = Some(true);
        let manifest = manifest();
        let mut evidence = Evidence {
            pid: 1,
            exit_code: Some(0),
            success: true,
            forced_kill: false,
            reason: "completed".into(),
            stdout: Default::default(),
            stderr: Default::default(),
        };
        assert!(collect(&config, &manifest, &evidence).is_err());
        let report = format!(
            "{REPORT_PREFIX}{}\n",
            json!({"version":1,"connection_affinity":"pooled_best_effort_unobserved","metrics":Metrics::default()})
        );
        evidence.stderr.bytes = report.as_bytes().to_vec();
        assert_eq!(
            collect(&config, &manifest, &evidence).unwrap()["at_least_one_range_qualified"],
            false
        );
        evidence.stderr.bytes.extend_from_slice(report.as_bytes());
        assert!(collect(&config, &manifest, &evidence).is_err());
        evidence.stderr.bytes = report.into_bytes();
        evidence.stderr.limit_exceeded = true;
        assert!(collect(&config, &manifest, &evidence).is_err());
        evidence.stderr.limit_exceeded = false;
        let metrics = Metrics {
            range_requests: 1,
            ..Default::default()
        };
        evidence.stderr.bytes = format!("{REPORT_PREFIX}{}\n", json!({"version":1,"connection_affinity":"pooled_best_effort_unobserved","metrics":metrics})).into_bytes();
        assert!(collect(&config, &manifest, &evidence).is_err());
        config.network.cover_traffic_enabled = Some(false);
        assert!(collect(&config, &manifest, &evidence).is_err());
        evidence.stderr.bytes.clear();
        assert_eq!(
            collect(&config, &manifest, &evidence).unwrap()["status"],
            "disabled"
        );
    }
    #[test]
    fn strict_summary_schema_rejects_missing_or_extra_metrics() {
        assert!(serde_json::from_value::<Envelope>(json!({"version":1,"connection_affinity":"pooled_best_effort_unobserved","metrics":{}})).is_err());
        let mut m = serde_json::to_value(Metrics::default()).unwrap();
        m["unverified"] = json!(true);
        assert!(serde_json::from_value::<Metrics>(m).is_err());
    }
}
