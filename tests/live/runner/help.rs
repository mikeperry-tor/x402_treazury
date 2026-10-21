//! Help retrieval/cache evidence, independent of outer MCP success.
use crate::manifest::Manifest;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedCache {
    Fetch,
    Hit,
    Shared,
}

pub fn report(m: &Manifest, r: &Value) -> Result<Value> {
    let cases: BTreeMap<_, _> = m.cases.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut claims = BTreeMap::new();
    let mut closed = BTreeSet::new();
    let mut starts = BTreeMap::new();
    let mut ends = BTreeMap::new();
    for event in r["runtime_events"]
        .as_array()
        .context("help runtime events missing")?
    {
        let e = &event["detail"];
        match event["kind"].as_str() {
            Some("application_claim") => {
                let id = e["case"].as_str().context("help claim identity missing")?;
                ensure!(claims.insert(id, e).is_none(), "duplicate help claim");
            }
            Some("application_finished") => {
                closed.insert(
                    e["case"]
                        .as_str()
                        .context("help completion identity missing")?,
                );
            }
            Some("application_help") => {
                let id = e["case"].as_str().context("help case missing")?;
                ensure!(cases.contains_key(id), "unknown help case");
                let claim = claims.get(id).context("help observation before claim")?;
                ensure!(
                    !closed.contains(id) && claim["session"] == e["session"],
                    "help event outside claimed session/interval"
                );
                let at = e["observed_micros"]
                    .as_u64()
                    .context("help event time missing")?;
                ensure!(
                    claim["started_micros"].as_u64().is_some_and(|t| t <= at),
                    "help observation precedes claim time"
                );
                match e["result"]["status"].as_str() {
                    Some("started") => {
                        ensure!(
                            e["cache"].is_null() && starts.insert(id, e).is_none(),
                            "invalid/duplicate help start"
                        );
                    }
                    Some("succeeded" | "failed") => {
                        let start = starts.get(id).context("help result lacks start")?;
                        ensure!(
                            start["observed_micros"].as_u64().is_some_and(|t| t <= at)
                                && ends.insert(id, e).is_none(),
                            "invalid/duplicate help completion"
                        );
                        ensure!(
                            matches!(e["cache"].as_str(), Some("fetch" | "hit" | "shared")),
                            "invalid help cache outcome"
                        );
                        if e["result"]["status"] == "succeeded" {
                            ensure!(e["result"]["bytes"].as_u64().is_some(), "help size missing");
                        } else {
                            ensure!(
                                e["cache"] == "fetch"
                                    && matches!(
                                        e["result"]["failure_category"].as_str(),
                                        Some(
                                            "http_connect"
                                                | "http_timeout"
                                                | "http_status"
                                                | "other"
                                        )
                                    ),
                                "invalid help failure outcome"
                            );
                            ensure!(
                                e["result"]["http_status"].is_null()
                                    || e["result"]["http_status"]
                                        .as_u64()
                                        .is_some_and(|code| (100..=999).contains(&code)),
                                "invalid help HTTP status code"
                            );
                        }
                    }
                    _ => anyhow::bail!("unknown help stage status"),
                }
            }
            _ => (),
        }
    }
    let valid = r["application_evidence"]["claims"].as_u64() == Some(claims.len() as u64);
    let rows:Vec<_>=m.cases.iter().filter(|c|c.help_cache.is_some() || starts.contains_key(c.id.as_str())).map(|c| {
        let end=ends.get(c.id.as_str());
        let expected=c.help_cache.map(|v|serde_json::to_value(v).expect("enum"));
        let status=match end.filter(|_|valid) {
            None=>"incomplete",
            Some(end) if end["result"]["status"]=="failed" || expected.as_ref().is_some_and(|v|*v!=end["cache"])=>"failed",
            Some(_)=>"passed",
        };
        json!({"case":c.id,"required":expected.is_some(),"expected_cache":expected,"status":status,
            "cache":end.filter(|_|valid).map(|e|e["cache"].clone()),
            "result":end.filter(|_|valid).map(|e|e["result"].clone()),
            "meaning":"Application cache-path observation, not proof of Tor stream isolation or same-channel reuse."})
    }).collect();
    Ok(json!(rows))
}
pub fn qualification(r: &Value, selected: Option<&[String]>) -> Result<(bool, bool)> {
    let rows = r["help_stages"]
        .as_array()
        .context("help stage report missing")?;
    let required: Vec<_> = rows
        .iter()
        .filter(|r| {
            r["required"] == true
                && selected.is_none_or(|ids| ids.iter().any(|id| r["case"] == *id))
        })
        .collect();
    Ok((
        required
            .iter()
            .all(|r| matches!(r["status"].as_str(), Some("passed" | "failed"))),
        required.iter().all(|r| r["status"] == "passed"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Manifest, Value) {
        let mut m = crate::tests::manifest();
        m.cases[0].help_cache = Some(ExpectedCache::Hit);
        let r = json!({"application_evidence":{"claims":1},"runtime_events":[
            {"kind":"application_claim","detail":{"case":"call","session":"s","started_micros":1}},
            {"kind":"application_help","detail":{"case":"call","session":"s","observed_micros":2,"cache":null,"result":{"status":"started"}}},
            {"kind":"application_help","detail":{"case":"call","session":"s","observed_micros":3,"cache":"hit","result":{"status":"succeeded","bytes":100}}}]});
        (m, r)
    }
    #[test]
    fn expectations_gate_success_and_preserve_missing_evidence() {
        let (mut m, mut r) = fixture();
        assert_eq!(report(&m, &r).unwrap()[0]["status"], "passed");
        m.cases[0].help_cache = Some(ExpectedCache::Fetch);
        assert_eq!(report(&m, &r).unwrap()[0]["status"], "failed");
        r["runtime_events"].as_array_mut().unwrap().pop();
        assert_eq!(report(&m, &r).unwrap()[0]["status"], "incomplete");
        r["runtime_events"] = json!([]);
        assert_eq!(report(&m, &r).unwrap()[0]["status"], "incomplete");
        r["help_stages"] = report(&m, &r).unwrap();
        assert_eq!(qualification(&r, None).unwrap(), (false, false));
        assert_eq!(
            qualification(&r, Some(&["different".into()])).unwrap(),
            (true, true)
        );
    }
    #[test]
    fn errors_and_bad_correlations_cannot_qualify_cache_reuse() {
        let (m, r) = fixture();
        for (field, value) in [("session", json!("other")), ("observed_micros", json!(0))] {
            let mut bad = r.clone();
            bad["runtime_events"][2]["detail"][field] = value;
            assert!(report(&m, &bad).is_err());
        }
        let mut bad = r.clone();
        let duplicate = bad["runtime_events"][2].clone();
        bad["runtime_events"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(report(&m, &bad).is_err());
        let mut unknown = r.clone();
        unknown["application_evidence"] = json!({"status":"invalid_or_incomplete"});
        assert_eq!(report(&m, &unknown).unwrap()[0]["status"], "incomplete");
        let mut failed = r;
        failed["runtime_events"][2]["detail"]["cache"] = json!("fetch");
        failed["runtime_events"][2]["detail"]["result"] =
            json!({"status":"failed","failure_category":"http_timeout"});
        assert_eq!(report(&m, &failed).unwrap()[0]["status"], "failed");
    }
}
