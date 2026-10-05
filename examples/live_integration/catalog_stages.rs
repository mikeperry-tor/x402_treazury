//! Catalog observations from hash-pinned inspection artifacts, never inferred from MCP success.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use x402_treazury::deployment::catalog_evidence::{Observation, State};

pub fn project(snapshot: &Value, sources: &BTreeSet<String>, phase: &str) -> Result<Value> {
    let Some(value) = snapshot.get("catalog_stages") else {
        return Ok(json!({"phase":phase,"status":"unobserved","sources":[]}));
    };
    let rows: Vec<Observation> = serde_json::from_value(value.clone())?;
    if rows.len() > 10000 {
        eprintln!("catalog evidence exceeds 10000 sources; no partial report emitted");
        anyhow::bail!("catalog evidence exceeds 10000 sources; no partial report emitted");
    }
    let mut seen = BTreeSet::new();
    for row in &rows {
        ensure!(sources.contains(&row.source) && seen.insert(row.source.clone()), "unknown or duplicate catalog evidence source");
        ensure!(matches!(row.stage.as_str(), "configuration" | "fetch_parse" | "headers" | "body" | "local_read" | "parse" | "generation"), "invalid catalog stage");
        ensure!(row.http_status.is_none_or(|s| (400..=599).contains(&s) && row.state == State::Failed && row.stage == "headers"), "invalid catalog HTTP failure status");
        ensure!(row.state != State::Completed || row.stage == "generation", "catalog completion before generation");
        ensure!(row.state != State::NotStarted || row.stage == "configuration", "unstarted catalog has stage evidence");
    }
    ensure!(&seen == sources, "missing catalog source observations");
    let complete = rows.iter().all(|row| row.state == State::Completed);
    ensure!(snapshot["preparation_failed"] == true || complete, "successful snapshot has incomplete catalog evidence");
    Ok(json!({"phase":phase,"status":if complete && snapshot["preparation_failed"] != true {"completed"} else {"incomplete"},"sources":rows}))
}

pub fn report(pins: &crate::registry::Pins, m: &crate::manifest::Manifest) -> Result<Value> {
    let Some(catalogs) = &pins.catalogs else { return Ok(json!([])); };
    let snapshot = catalogs.verify_archive(m)?;
    let sources = pins.resolved_config["sources"].as_object().context("missing pinned catalog sources")?.keys().cloned().collect();
    let frozen: Value = serde_json::from_slice(&crate::files::read_catalog(&catalogs.directory.join("frozen-snapshot.json"))?)?;
    Ok(json!([project(&snapshot, &sources, "initial_inspection")?, project(&frozen, &sources, "frozen_reload")?]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stages_require_complete_source_binding_and_never_repair_failure() {
        let sources = BTreeSet::from(["a".into(), "b".into()]);
        let rows = json!([{"source":"a","state":"failed","stage":"headers","http_status":429},{"source":"b","state":"not_started","stage":"configuration","http_status":null}]);
        let mut snapshot = json!({"preparation_failed":true,"catalog_stages":rows});
        assert_eq!(project(&snapshot,&sources,"initial_inspection").unwrap()["status"], "incomplete");
        snapshot["preparation_failed"] = json!(false);
        assert!(project(&snapshot,&sources,"initial_inspection").is_err());
        snapshot["preparation_failed"] = json!(true);
        snapshot["catalog_stages"][1]["source"] = json!("a");
        assert!(project(&snapshot,&sources,"initial_inspection").is_err());
        assert_eq!(project(&json!({}),&sources,"initial_inspection").unwrap()["status"], "unobserved");
    }
}
