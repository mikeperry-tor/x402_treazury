//! Validate application acceptance/completion separately from driver observations.
use super::*;
use std::collections::BTreeMap;
impl Registry {
    pub fn application_failure(&self, run: &str, case: &str) -> Result<Option<String>> {
        let (count, category):(i64,Option<String>) = self.db.query_row("SELECT COUNT(*),MIN(json_extract(detail,'$.failure_category')) FROM events WHERE run=?1 AND kind='application_finished' AND json_extract(detail,'$.case')=?2", params![run,case], |r| Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(
            count == 1,
            "expected exactly one application completion for case {case}"
        );
        Ok(category)
    }
    pub fn application_evidence(&self, run: &str) -> Result<Value> {
        let m = self.manifest(run)?;
        let cases: BTreeMap<_, _> = m.cases.iter().map(|c| (c.id.as_str(), c)).collect();
        let mut query=self.db.prepare("SELECT kind,detail FROM events WHERE run=?1 AND kind IN ('application_claim','application_finished') ORDER BY seq LIMIT 20001")?;
        let rows = query
            .query_map([run], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            rows.len() <= 20000,
            "application evidence exceeds 20000-event run limit"
        );
        let mut claims = BTreeMap::new();
        let mut finished = BTreeMap::new();
        for (kind, raw) in rows {
            let event: Value = serde_json::from_str(&raw)?;
            let id = event["case"]
                .as_str()
                .context("application event missing case ID")?
                .to_owned();
            let case = cases
                .get(id.as_str())
                .context("application event names an unreviewed case")?;
            ensure!(
                event["session"].as_str().is_some_and(identifier),
                "application event session missing/invalid"
            );
            if kind == "application_claim" {
                ensure!(
                    case.unsigned
                        && event["mode"] == "unsigned"
                        && event["server"] == case.server
                        && event["source"] == case.source
                        && event["tool"] == case.tool,
                    "application claim differs from reviewed binding"
                );
                ensure!(
                    self.execution_state(run, &id)? != "UNATTEMPTED",
                    "application accepted an unreserved case"
                );
                ensure!(
                    claims.insert(id, event).is_none(),
                    "application accepted a case more than once"
                );
            } else {
                let claim = claims
                    .get(&id)
                    .context("application completed without a claim")?;
                ensure!(
                    claim["session"] == event["session"] && event["is_error"].is_boolean(),
                    "application completion does not match claim"
                );
                ensure!(
                    event["finished_micros"]
                        .as_u64()
                        .zip(claim["started_micros"].as_u64())
                        .is_some_and(|(end, start)| end >= start),
                    "application interval is missing or invalid"
                );
                ensure!(
                    finished.insert(id, event).is_none(),
                    "duplicate application completion"
                );
            }
        }
        for id in cases.keys() {
            let (execution, semantic): (String, String) = self.db.query_row(
                "SELECT execution,semantic FROM cases WHERE run=?1 AND id=?2",
                params![run, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if execution == "COMPLETED" {
                let event = finished
                    .get(*id)
                    .context("completed MCP case lacks application completion evidence")?;
                ensure!(
                    (semantic == "PASSED") == (event["is_error"] == false),
                    "MCP/application result disagreement"
                );
            }
        }
        let pending: Vec<_> = claims
            .keys()
            .filter(|id| !finished.contains_key(*id))
            .collect();
        Ok(
            json!({"claims":claims.len(),"finished":finished.len(),"without_completion":pending,"scope":"keyless application acceptance/completion; no signed payment or Tor isolation qualification"}),
        )
    }
}
