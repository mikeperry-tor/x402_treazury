//! Prepared help-cache boundary and expected, explicit connection refusal.
use crate::{
    manifest::{Manifest, Scenario},
    registry::Registry,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

pub fn uncached(m: &Manifest, id: &str) -> bool {
    m.phases.iter().any(
        |p| matches!(&p.scenario, Scenario::TorOutage {uncached_case,..} if uncached_case == id),
    )
}
fn tool<'a>(m: &Manifest, snapshot: &'a Value, id: &str) -> Result<&'a Value> {
    let case = m
        .cases
        .iter()
        .find(|c| c.id == id)
        .context("outage case missing")?;
    let listener = snapshot["inventory"]
        .as_array()
        .context("inventory missing")?
        .iter()
        .find(|s| s["server"] == case.server)
        .context("outage listener missing")?;
    listener["tools"]
        .as_array()
        .context("tools missing")?
        .iter()
        .find(|t| t["source"] == case.source && t["name"] == case.tool)
        .context("outage tool missing")
}
pub fn validate(m: &Manifest, snapshot: &Value) -> Result<()> {
    for phase in &m.phases {
        let Scenario::TorOutage {
            warm_case,
            cached_case,
            uncached_case,
        } = &phase.scenario
        else {
            continue;
        };
        let warm = m
            .cases
            .iter()
            .find(|c| c.id == *warm_case)
            .context("warm case missing")?;
        let cached = m
            .cases
            .iter()
            .find(|c| c.id == *cached_case)
            .context("cached case missing")?;
        ensure!(
            warm.server == cached.server
                && warm.source == cached.source
                && warm.tool == cached.tool
                && warm.arguments == cached.arguments,
            "warm/cached cases must address the same help cache and arguments"
        );
        let warm_url = tool(m, snapshot, warm_case)?["help_url"]
            .as_str()
            .context("warm case must be a help tool")?;
        let fresh_url = tool(m, snapshot, uncached_case)?["help_url"]
            .as_str()
            .context("uncached case must be a help tool")?;
        ensure!(
            warm_url != fresh_url,
            "outage requires a distinct uncached help URL"
        );
        for case in &m.cases {
            if case.id != *uncached_case {
                ensure!(
                    tool(m, snapshot, &case.id)?["help_url"].as_str() != Some(fresh_url),
                    "uncached help URL is already selected by another case"
                );
            }
        }
    }
    Ok(())
}
pub fn results(registry: &Registry, m: &Manifest, phase: &crate::manifest::Phase) -> Result<Value> {
    let Scenario::TorOutage {
        warm_case,
        cached_case,
        uncached_case,
    } = &phase.scenario
    else {
        anyhow::bail!("not an outage phase")
    };
    let warm = registry.response(&m.run_id, warm_case)?;
    let cached = registry.response(&m.run_id, cached_case)?;
    let fresh = registry.response(&m.run_id, uncached_case)?;
    compare(
        &warm,
        &cached,
        &fresh,
        registry
            .application_failure(&m.run_id, uncached_case)?
            .as_deref(),
    )?;
    Ok(
        json!({"phase":phase.id,"cached_case":cached_case,"uncached_case":uncached_case,"cached_help_identical":true,"uncached_http_connect_failure":true,"scope":"stopped owned SOCKS listener; not an in-flight stalled-request timeout"}),
    )
}
fn compare(warm: &Value, cached: &Value, fresh: &Value, failure: Option<&str>) -> Result<()> {
    for result in [warm, cached] {
        ensure!(
            result["result"].is_object()
                && result["result"]["isError"] != true
                && result["result"]["content"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty()),
            "warm/cached help did not succeed"
        );
    }
    ensure!(
        warm["result"] == cached["result"],
        "cached help changed during outage"
    );
    ensure!(
        fresh["result"]["isError"] == true && failure == Some("http_connect"),
        "uncached help lacks application HTTP connection-failure evidence; arbitrary MCP/provider failures do not qualify"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outage_requires_same_help_and_typed_connection_failure() {
        let warm = json!({"id":"warm","result":{"content":[{"type":"text","text":"docs"}]}});
        let mut cached = warm.clone();
        cached["id"] = json!("cached");
        let mut fresh = json!({"result":{"isError":true,"content":[{"type":"text","text":"opaque SOCKS connection failure"}]}});
        compare(&warm, &cached, &fresh, Some("http_connect")).unwrap();
        fresh["result"]["content"][0]["text"] = json!("HTTP 403");
        assert!(compare(&warm, &cached, &fresh, Some("http_status")).is_err());
        assert!(compare(&warm, &cached, &fresh, None).is_err());
        cached["result"]["content"][0]["text"] = json!("different");
        assert!(compare(&warm, &cached, &fresh, Some("http_connect")).is_err());
    }
}
