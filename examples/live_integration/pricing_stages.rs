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
    let indexed = PricingEvents::collect(sources, r)?;
    ensure!(
        indexed
            .sessions
            .len()
            .max(1)
            .checked_mul(sources.len())
            .is_some_and(|n| n <= 10000),
        "pricing report exceeds 10000 source/session rows; split qualification runs, no partial report"
    );
    let mut rows = Vec::new();
    for session in
        indexed
            .sessions
            .iter()
            .copied()
            .map(Some)
            .chain(if indexed.sessions.is_empty() {
                Some(None)
            } else {
                None
            })
    {
        for source in sources.keys() {
            rows.push(indexed.row(session, source, m.pricing_checks.get(source))?);
        }
    }
    Ok(json!(rows))
}

#[derive(Default)]
struct PricingEvents<'a> {
    sessions: BTreeSet<&'a str>,
    starts: BTreeMap<(&'a str, &'a str), u64>,
    ends: BTreeMap<(&'a str, &'a str), &'a Value>,
}
impl<'a> PricingEvents<'a> {
    fn collect(sources: &serde_json::Map<String, Value>, r: &'a Value) -> Result<Self> {
        let mut indexed = Self::default();
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
                    indexed.sessions.insert(session),
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
                indexed.sessions.contains(session) && sources.contains_key(source),
                "pricing stage has unknown session/source"
            );
            let key = (session, source);
            let at = e["observed_micros"]
                .as_u64()
                .context("pricing stage time missing")?;
            match e["phase"].as_str() {
                Some("started") => ensure!(
                    indexed.starts.insert(key, at).is_none(),
                    "duplicate pricing stage start"
                ),
                Some("completed" | "failed") => {
                    ensure!(
                        indexed.starts.get(&key).is_some_and(|start| *start <= at),
                        "pricing stage completion lacks an ordered start"
                    );
                    ensure!(
                        indexed.ends.insert(key, e).is_none(),
                        "duplicate pricing stage completion"
                    );
                    if e["phase"] == "completed" {
                        validate_completion(e, &sources[source])?;
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
        Ok(indexed)
    }
    fn row(
        &self,
        session: Option<&str>,
        source: &str,
        expected: Option<&Expectation>,
    ) -> Result<Value> {
        let key = (session.unwrap_or_default(), source);
        let end = self.ends.get(&key);
        let mut stage = if self.starts.contains_key(&key) {
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
        Ok(
            json!({"session":session,"source":source,"required":expected.is_some(),"expected":expected,"stage":stage,"assessment":assessment,"evidence":evidence,"failure":failure}),
        )
    }
}

fn validate_completion(e: &Value, source: &Value) -> Result<()> {
    let data = serde_json::from_value::<x402_treazury::pricing::Evidence>(e["evidence"].clone())?;
    data.validate()?;
    let prices: Vec<x402_treazury::pricing::Price> = serde_json::from_value(e["prices"].clone())?;
    ensure!(
        x402_treazury::pricing::price_map(&prices)?.len() == data.available_prices,
        "recorded pricing lines/count disagree"
    );
    ensure!(
        source["settings"]["probe_pricing"].as_bool() == Some(data.enabled),
        "pricing state differs from pinned source policy"
    );
    Ok(())
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
    #[test]
    fn report_completion_matrix_preserves_full_rows() {
        for outcome in [Expectation::Disabled, Expectation::Empty, Expectation::Nonempty] {
            let (mut m, mut config, mut r) = fixture();
            match outcome {
                Expectation::Disabled => {
                    config["sources"]["api"]["settings"]["probe_pricing"] = json!(false);
                    r["runtime_events"][2]["detail"]["evidence"] = json!(x402_treazury::pricing::Evidence::default());
                }
                Expectation::Empty => (),
                Expectation::Nonempty => {
                    let e = &mut r["runtime_events"][2]["detail"];
                    e["evidence"]["available_prices"] = json!(1);
                    e["evidence"]["outcomes"] = json!({"discovered":1});
                    e["evidence"]["http_statuses"] = json!({"402":1});
                    e["prices"] = json!([{"method":"GET","path":"/read","line":"Price: $0.01"}]);
                }
            }
            for expected in [None, Some(Expectation::Disabled), Some(Expectation::Empty), Some(Expectation::Nonempty)] {
                m.pricing_checks.clear();
                if let Some(expected) = expected { m.pricing_checks.insert("api".into(), expected); }
                let assessment = match expected {
                    None => "not_required",
                    Some(expected) if json!(expected) == json!(outcome) => "passed",
                    Some(_) => "failed",
                };
                assert_eq!(report(&m, &config, &r).unwrap(), json!([{
                    "session":"s", "source":"api", "required":expected.is_some(), "expected":expected,
                    "stage":if matches!(outcome, Expectation::Disabled) {"disabled"} else {"completed"},
                    "assessment":assessment, "evidence":r["runtime_events"][2]["detail"]["evidence"], "failure":null
                }]));
            }
        }
    }

    #[test]
    fn report_failure_and_unfinished_stages_preserve_requirement_status() {
        for required in [false, true] {
            let (mut m, config, original) = fixture();
            if !required { m.pricing_checks.clear(); }
            for failure in ["http_connect", "http_timeout", "http_status", "other"] {
                let mut r = original.clone();
                let end = &mut r["runtime_events"][2]["detail"];
                end["phase"] = json!("failed");
                end["failure"] = json!(failure);
                end["evidence"] = Value::Null;
                end["prices"] = Value::Null;
                assert_eq!(report(&m, &config, &r).unwrap(), json!([{
                    "session":"s", "source":"api", "required":required,
                    "expected":if required {json!("empty")} else {Value::Null},
                    "stage":"failed", "assessment":if required {"failed"} else {"not_required"},
                    "evidence":null, "failure":failure
                }]));
            }
            for (count, session, stage) in [(0, Value::Null, "not_observed"), (1, json!("s"), "not_observed"), (2, json!("s"), "incomplete")] {
                let mut r = original.clone();
                r["runtime_events"].as_array_mut().unwrap().truncate(count);
                assert_eq!(report(&m, &config, &r).unwrap(), json!([{
                    "session":session, "source":"api", "required":required,
                    "expected":if required {json!("empty")} else {Value::Null},
                    "stage":stage, "assessment":if required {"incomplete"} else {"not_required"},
                    "evidence":null, "failure":null
                }]));
            }
        }
    }

    #[test]
    fn report_rejects_malformed_or_ambiguous_evidence_before_emitting_rows() {
        let (m, config, original) = fixture();
        for index in [0, 1, 2] {
            let mut r = original.clone();
            let duplicate = r["runtime_events"][index].clone();
            r["runtime_events"].as_array_mut().unwrap().push(duplicate);
            assert!(report(&m, &config, &r).is_err(), "duplicate event {index}");
        }
        for (pointer, value) in [
            ("/runtime_events/0/detail/session", Value::Null),
            ("/runtime_events/1/detail/session", Value::Null),
            ("/runtime_events/1/detail/source", Value::Null),
            ("/runtime_events/1/detail/source", json!("unknown")),
            ("/runtime_events/1/detail/observed_micros", json!(-1)),
            ("/runtime_events/1/detail/phase", json!("unknown")),
            ("/runtime_events/2/detail/evidence", Value::Null),
            ("/runtime_events/2/detail/evidence/observed", json!(2)),
            ("/runtime_events/2/detail/prices", json!({})),
            ("/runtime_events/2/detail/prices", json!([{"method":"POST","path":"/read","line":"price"}])),
            ("/runtime_events/2/detail/prices", json!([{"method":"GET","path":"/read","line":"price"}])),
        ] {
            let mut r = original.clone();
            *r.pointer_mut(pointer).unwrap() = value;
            assert!(report(&m, &config, &r).is_err(), "accepted {pointer}");
        }
        let mut r = original.clone();
        r["runtime_events"].as_array_mut().unwrap().remove(1);
        assert!(report(&m, &config, &r).is_err(), "completion without start");
        let mut r = original.clone();
        r["runtime_events"][2]["detail"]["observed_micros"] = json!(1);
        assert_eq!(report(&m, &config, &r).unwrap(), report(&m, &config, &original).unwrap(), "equal timestamps remain ordered");
        let mut r = original.clone();
        let end = &mut r["runtime_events"][2]["detail"];
        end["evidence"]["available_prices"] = json!(1);
        end["evidence"]["outcomes"] = json!({"discovered":1});
        end["evidence"]["http_statuses"] = json!({"402":1});
        let price = json!({"method":"GET","path":"/read","line":"Price: $0.01"});
        end["prices"] = json!([price.clone(), price]);
        assert!(report(&m, &config, &r).is_err(), "duplicate prices cannot collapse to a valid count");
        let mut r = original.clone();
        r["runtime_events"].as_array_mut().unwrap().swap(0, 1);
        assert!(report(&m, &config, &r).is_err(), "stage before session");
        let mut r = original;
        r["runtime_events"][2]["detail"]["phase"] = json!("failed");
        r["runtime_events"][2]["detail"]["failure"] = json!("upstream prose");
        assert!(report(&m, &config, &r).is_err());
        assert!(report(&m, &config, &json!({})).is_err());
        assert!(report(&m, &json!({}), &json!({})).is_err());
        let mut optional = m;
        optional.pricing_checks.clear();
        assert_eq!(report(&optional, &json!({}), &json!({})).unwrap(), json!([]));
    }

    #[test]
    fn report_row_bound_is_complete_and_ordered_across_sessions() {
        let (mut m, _, _) = fixture();
        m.pricing_checks.clear();
        let sources: serde_json::Map<String, Value> = (0..5000).rev()
            .map(|i| (format!("source_{i:04}"), json!({"settings":{"probe_pricing":true}}))).collect();
        let mut config = json!({"sources":sources});
        let events = json!({"runtime_events":[
            {"kind":"application_session","detail":{"session":"z"}},
            {"kind":"ignored","detail":null},
            {"kind":"application_session","detail":{"session":"a"}}
        ]});
        let rows = report(&m, &config, &events).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 10000);
        for (index, row) in rows.as_array().unwrap().iter().enumerate() {
            assert_eq!(row["session"], if index < 5000 {"a"} else {"z"});
            assert_eq!(row["source"], format!("source_{:04}", index % 5000));
        }
        config["sources"].as_object_mut().unwrap().insert("extra".into(), json!({}));
        assert!(report(&m, &config, &events).is_err());
        // With no session yet, the synthetic unobserved rows have the same cap.
        let sources: serde_json::Map<String, Value> = (0..10000)
            .map(|i| (format!("source_{i:05}"), json!({}))).collect();
        let mut config = json!({"sources":sources});
        let no_sessions = json!({"runtime_events":[]});
        assert_eq!(report(&m, &config, &no_sessions).unwrap().as_array().unwrap().len(), 10000);
        config["sources"].as_object_mut().unwrap().insert("extra".into(), json!({}));
        assert!(report(&m, &config, &no_sessions).is_err());
    }

}
