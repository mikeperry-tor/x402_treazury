use crate::manifest::Manifest;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expectation {
    Disabled,
    Empty,
    Nonempty,
}
pub fn validate(m: &Manifest, config: &Value) -> Result<()> {
    ensure!(
        m.pricing_checks.keys().all(|id| config["sources"]
            .as_object()
            .is_some_and(|s| s.contains_key(id))),
        "pricing_checks names an unknown source"
    );
    Ok(())
}
pub fn report(m: &Manifest, config: &Value, r: &Value) -> Result<Value> {
    validate(m, config)?;
    let Some(sources) = config["sources"].as_object() else {
        ensure!(
            m.pricing_checks.is_empty(),
            "pricing source configuration missing"
        );
        return Ok(json!([]));
    };
    let mut sessions = BTreeSet::new();
    let mut starts = BTreeMap::new();
    let mut ends = BTreeMap::new();
    for event in r["runtime_events"]
        .as_array()
        .context("pricing runtime evidence missing")?
    {
        let e = &event["detail"];
        if event["kind"] == "application_session" {
            let session = e["session"]
                .as_str()
                .context("pricing session identity missing")?;
            ensure!(
                sessions.insert(session),
                "duplicate pricing application session"
            );
        }
        if event["kind"] != "application_pricing" {
            continue;
        }
        let session = e["session"]
            .as_str()
            .context("pricing stage session missing")?;
        let source = e["source"]
            .as_str()
            .context("pricing stage source missing")?;
        ensure!(
            sessions.contains(session) && sources.contains_key(source),
            "pricing stage has unknown session/source"
        );
        let key = (session, source);
        let at = e["observed_micros"]
            .as_u64()
            .context("pricing stage time missing")?;
        match e["phase"].as_str() {
            Some("started") => ensure!(
                starts.insert(key, at).is_none(),
                "duplicate pricing stage start"
            ),
            Some("completed" | "failed") => {
                ensure!(
                    starts.get(&key).is_some_and(|start| *start <= at),
                    "pricing stage completion lacks an ordered start"
                );
                ensure!(
                    ends.insert(key, e).is_none(),
                    "duplicate pricing stage completion"
                );
                if e["phase"] == "completed" {
                    let data = serde_json::from_value::<x402_treazury::pricing::Evidence>(
                        e["evidence"].clone(),
                    )?;
                    data.validate()?;
                    let prices:Vec<x402_treazury::pricing::Price>=serde_json::from_value(e["prices"].clone())?;
                    ensure!(x402_treazury::pricing::price_map(&prices)?.len()==data.available_prices,"recorded pricing lines/count disagree");
                    ensure!(
                        sources[source]["settings"]["probe_pricing"].as_bool()
                            == Some(data.enabled),
                        "pricing state differs from pinned source policy"
                    );
                } else {
                    ensure!(
                        matches!(
                            e["failure"].as_str(),
                            Some("http_connect" | "http_timeout" | "http_status" | "other")
                        ),
                        "invalid pricing failure category"
                    );
                }
            }
            _ => anyhow::bail!("unknown pricing stage phase"),
        }
    }
    ensure!(
        sessions
            .len()
            .max(1)
            .checked_mul(sources.len())
            .is_some_and(|n| n <= 10000),
        "pricing report exceeds 10000 source/session rows; split qualification runs, no partial report"
    );
    let mut rows = Vec::new();
    for session in sessions
        .iter()
        .copied()
        .map(Some)
        .chain(if sessions.is_empty() {
            Some(None)
        } else {
            None
        })
    {
        for source in sources.keys() {
            let expected = m.pricing_checks.get(source);
            let key = (session.unwrap_or_default(), source.as_str());
            let end = ends.get(&key);
            let mut stage = if starts.contains_key(&key) {
                "incomplete"
            } else {
                "not_observed"
            };
            let mut assessment = if expected.is_some() {
                "incomplete"
            } else {
                "not_required"
            };
            let mut evidence = Value::Null;
            let mut failure = Value::Null;
            if let Some(end) = end {
                if end["phase"] == "failed" {
                    stage = "failed";
                    failure = end["failure"].clone();
                    if expected.is_some() {
                        assessment = "failed";
                    }
                } else {
                    let data: x402_treazury::pricing::Evidence =
                        serde_json::from_value(end["evidence"].clone())?;
                    stage = if !data.enabled {
                        "disabled"
                    } else {
                        "completed"
                    };
                    if let Some(expected) = expected {
                        let matched = match expected {
                            Expectation::Disabled => !data.enabled,
                            Expectation::Empty => data.enabled && data.available_prices == 0,
                            Expectation::Nonempty => data.enabled && data.available_prices > 0,
                        };
                        assessment = if matched { "passed" } else { "failed" };
                    }
                    evidence = serde_json::to_value(data)?;
                }
            }
            rows.push(json!({"session":session,"source":source,"required":expected.is_some(),"expected":expected,"stage":stage,"assessment":assessment,"evidence":evidence,"failure":failure}));
        }
    }
    Ok(json!(rows))
}
pub fn qualification(r: &Value) -> Result<(bool, bool)> {
    let rows = r["pricing_stages"]
        .as_array()
        .context("pricing stages missing")?;
    let required: Vec<_> = rows.iter().filter(|r| r["required"] == true).collect();
    Ok((
        required
            .iter()
            .all(|r| matches!(r["assessment"].as_str(), Some("passed" | "failed"))),
        required.iter().all(|r| r["assessment"] == "passed"),
    ))
}

/// Rebuild only description text from recorded startup prices. Names, selected
/// routes, schemas and local tools remain exactly as frozen during preparation.
pub fn inventory(snapshot:&Value,events:&Value,session:&str)->Result<Value> {
    let mut result=snapshot.clone();let mut seen=BTreeSet::new();
    for event in events.as_array().context("pricing events missing")? {
        let e=&event["detail"];
        if event["kind"]!="application_pricing" || e["session"]!=session || e["phase"]!="completed" {continue}
        let source=e["source"].as_str().context("pricing source missing")?;
        ensure!(seen.insert(source),"duplicate source pricing completion");
        let data:x402_treazury::pricing::Evidence=serde_json::from_value(e["evidence"].clone())?;data.validate()?;
        let rows:Vec<x402_treazury::pricing::Price>=serde_json::from_value(e["prices"].clone())?;
        let prices=x402_treazury::pricing::price_map(&rows)?;
        ensure!(prices.len()==data.available_prices,"pricing line count differs");
        let record=&snapshot["sources"][source];
        let cfg:x402_treazury::catalog::Config=serde_json::from_value(record["settings"].clone())?;
        ensure!(cfg.probe_pricing==data.enabled,"runtime pricing policy changed");
        if prices.is_empty(){continue}
        let known:BTreeSet<_>=x402_treazury::catalog::operations(&record["document"],cfg.pricing_key.as_deref())?.into_iter()
            .filter(|o|!o["pricing"].is_null()&&o["pricing"]!=json!({})).map(|o|(o["method"].as_str().unwrap_or_default().to_uppercase(),o["path"].as_str().unwrap_or_default().to_owned())).collect();
        let mut eligible=BTreeSet::new();
        for server in snapshot["inventory"].as_array().context("snapshot inventory missing")? {
            for tool in server["tools"].as_array().context("snapshot tools missing")? {
                if tool["source"]==source && tool["method"]=="GET" && tool["help_url"].is_null() {
                    let path=tool["path"].as_str().context("tool route missing")?;
                    let key=("GET".to_owned(),path.to_owned());
                    if !path.contains('{')&&!known.contains(&key){eligible.insert(key);}
                }
            }
        }
        ensure!(prices.keys().all(|key|eligible.contains(key)),"price names an ineligible or unselected route");
        let tools=x402_treazury::catalog::build_tools_with_prices(&cfg,&record["document"],cfg.prefix.as_deref().unwrap_or(source),&prices)?;
        let tools:BTreeMap<_,_>=tools.into_iter().map(|t|(t.name.clone(),t)).collect();
        for server in result["inventory"].as_array_mut().context("snapshot inventory missing")? {
            for tool in server["tools"].as_array_mut().context("snapshot tools missing")? {
                if tool["source"]==source {
                    let generated=tools.get(tool["name"].as_str().context("tool name missing")?).context("priced tool disappeared")?;
                    ensure!(tool["input_schema"]==generated.input_schema && tool["method"]==generated.method && tool["path"]==generated.path,"pricing changed a pinned route/schema");
                    tool["description"]=json!(generated.description);
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Manifest, Value, Value) {
        let mut m = crate::tests::manifest();
        m.pricing_checks.insert("api".into(), Expectation::Empty);
        let config = json!({"sources":{"api":{"settings":{"probe_pricing":true}}}});
        let mut data = x402_treazury::pricing::Evidence {
            enabled: true,
            selected_tools: 1,
            eligible: 1,
            observed: 1,
            ..Default::default()
        };
        data.cache
            .insert(x402_treazury::pricing::CachePath::Initialized, 1);
        data.outcomes
            .insert(x402_treazury::pricing::Outcome::UnexpectedHttpStatus, 1);
        data.http_statuses.insert(429, 1);
        let r = json!({"runtime_events":[{"kind":"application_session","detail":{"session":"s"}},
            {"kind":"application_pricing","detail":{"session":"s","source":"api","phase":"started","observed_micros":1}},
            {"kind":"application_pricing","detail":{"session":"s","source":"api","phase":"completed","observed_micros":2,"evidence":data,"prices":[]}}]});
        (m, config, r)
    }
    #[test]
    fn priced_inventory_rebuilds_descriptions_but_refuses_route_and_schema_drift() {
        let cfg=x402_treazury::catalog::Config{prefix:Some("api".into()),..Default::default()};
        let document=json!({"paths":{"/read":{"get":{"description":"Instructions"}}}});
        let tool=x402_treazury::catalog::build_tools(&cfg,&document,"api").unwrap().pop().unwrap();
        let mut selected=serde_json::to_value(&tool).unwrap();selected["source"]=json!("api");
        let snapshot=json!({"sources":{"api":{"settings":cfg,"document":document}},"inventory":[{"server":"main","tools":[selected]}]});
        let mut data=x402_treazury::pricing::Evidence{enabled:true,selected_tools:1,eligible:1,observed:1,available_prices:1,..Default::default()};
        data.cache.insert(x402_treazury::pricing::CachePath::Initialized,1);data.outcomes.insert(x402_treazury::pricing::Outcome::Discovered,1);data.http_statuses.insert(402,1);
        let mut events=json!([{"kind":"application_pricing","detail":{"source":"api","session":"s","phase":"completed","evidence":data,"prices":[{"method":"GET","path":"/read","line":"Price: $0.014 per call."}]}}]);
        let built=inventory(&snapshot,&events,"s").unwrap();
        assert!(built["inventory"][0]["tools"][0]["description"].as_str().unwrap().contains("$0.014"));
        assert_eq!(built["inventory"][0]["tools"][0]["input_schema"],snapshot["inventory"][0]["tools"][0]["input_schema"]);
        assert_eq!(inventory(&snapshot,&events,"different").unwrap(),snapshot);
        let mut bad=snapshot.clone();bad["inventory"][0]["tools"][0]["input_schema"]=json!({"type":"string"});assert!(inventory(&bad,&events,"s").is_err());
        events[0]["detail"]["prices"][0]["path"]=json!("/unselected");assert!(inventory(&snapshot,&events,"s").is_err());
    }
    #[test]
    fn empty_map_keeps_the_failure_and_never_means_provider_health() {
        let (mut m, config, r) = fixture();
        let rows = report(&m, &config, &r).unwrap();
        assert_eq!(rows[0]["assessment"], "passed");
        assert_eq!(rows[0]["evidence"]["http_statuses"]["429"], 1);
        m.pricing_checks.insert("api".into(), Expectation::Nonempty);
        assert_eq!(report(&m, &config, &r).unwrap()[0]["assessment"], "failed");
        m.pricing_checks
            .insert("missing".into(), Expectation::Empty);
        assert!(validate(&m, &config).is_err());
    }
    #[test]
    fn every_application_session_needs_observation_when_requested() {
        let (m, config, mut r) = fixture();
        r["runtime_events"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"application_session","detail":{"session":"resumed"}}));
        r["pricing_stages"] = report(&m, &config, &r).unwrap();
        assert_eq!(qualification(&r).unwrap(), (false, false));
        r["runtime_events"] = json!([]);
        r["pricing_stages"] = report(&m, &config, &r).unwrap();
        assert_eq!(qualification(&r).unwrap(), (false, false));
    }
    #[test]
    fn changed_counters_order_session_and_policy_are_refused() {
        let (m, config, r) = fixture();
        for (field, value) in [("session", json!("other")), ("observed_micros", json!(0))] {
            let mut bad = r.clone();
            bad["runtime_events"][2]["detail"][field] = value;
            assert!(report(&m, &config, &bad).is_err());
        }
        let mut bad = r.clone();
        bad["runtime_events"][2]["detail"]["evidence"]["observed"] = json!(2);
        assert!(report(&m, &config, &bad).is_err());
        let mut config = config;
        config["sources"]["api"]["settings"]["probe_pricing"] = json!(false);
        assert!(report(&m, &config, &r).is_err());
        let mut bad = r;
        let e = bad["runtime_events"][2].clone();
        bad["runtime_events"].as_array_mut().unwrap().push(e);
        assert!(report(&m, &config, &bad).is_err());
    }
}
