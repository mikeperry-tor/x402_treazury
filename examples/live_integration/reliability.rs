//! Repeated identical case observations. No retry, scheduler or provider health inference.
use crate::manifest::{Case, Manifest, Phase, Scenario, Window};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

struct Sample<'a> {
    case: &'a Case,
    phase: &'a Phase,
    window: &'a Window,
    spacing: u64,
    later_day: bool,
}
fn groups(m: &Manifest) -> Result<BTreeMap<String, Vec<Sample<'_>>>> {
    let mut groups = BTreeMap::<String, Vec<Sample<'_>>>::new();
    for phase in &m.phases {
        let Scenario::Reliability {
            min_spacing_seconds,
            later_utc_day,
        } = &phase.scenario
        else {
            continue;
        };
        let window = m
            .windows
            .iter()
            .find(|w| w.id == phase.window)
            .context("reliability window missing")?;
        for id in &phase.cases {
            let case = m
                .cases
                .iter()
                .find(|c| c.id == *id)
                .context("reliability case missing")?;
            let key = serde_json::to_string(&json!([
                case.server,
                case.source,
                case.tool,
                case.arguments,
                case.unsigned,
                case.checks,
                case.help_cache
            ]))?;
            groups.entry(key).or_default().push(Sample {
                case,
                phase,
                window,
                spacing: *min_spacing_seconds,
                later_day: *later_utc_day,
            });
        }
    }
    Ok(groups)
}
pub fn validate(m: &Manifest) -> Result<()> {
    for samples in groups(m)?.values() {
        ensure!(
            samples.len() >= 2,
            "reliability requires at least two identical server/source/tool/arguments/mode/assertion observations"
        );
        ensure!(
            samples.iter().all(|sample| sample.spacing > 0),
            "reliability spacing must be positive"
        );
        let first = samples.first().expect("nonempty group");
        let last = samples.last().expect("nonempty group");
        ensure!(
            last.window
                .not_before
                .saturating_sub(first.window.not_after)
                < m.limits.run_seconds,
            "reliability windows cannot fit the original run deadline; increase run_seconds explicitly"
        );
        for pair in samples.windows(2) {
            let (before, after) = (&pair[0], &pair[1]);
            ensure!(
                after.spacing > 0
                    && before
                        .window
                        .not_after
                        .checked_add(after.spacing)
                        .is_some_and(|start| after.window.not_before >= start),
                "reliability windows overlap or lack declared spacing"
            );
            ensure!(
                !after.later_day
                    || after.window.not_before / 86400 > before.window.not_after / 86400,
                "reliability requires a later UTC day"
            );
        }
    }
    Ok(())
}
fn indexed<'a>(rows: &'a Value, field: &str) -> Result<BTreeMap<&'a str, &'a Value>> {
    let mut result = BTreeMap::new();
    for row in rows
        .as_array()
        .context("reliability evidence array missing")?
    {
        ensure!(
            result
                .insert(
                    row[field]
                        .as_str()
                        .context("reliability evidence identity missing")?,
                    row
                )
                .is_none(),
            "duplicate reliability evidence"
        );
    }
    Ok(result)
}
fn observation(row: &Value, semantic: &Value, debits: &Value, case: &Case) -> &'static str {
    if row["execution"] != "COMPLETED" {
        return "incomplete";
    }
    let settled = match row["settlement"].as_str() {
        Some("NOT_SIGNED" | "EXPIRED_UNUSED") => true,
        Some("USED") => debits["status"] == "validated" && debits["cases"].get(&case.id).is_some(),
        _ => false,
    };
    if !settled {
        return "incomplete";
    }
    if row["semantic"] == "FAILED" || semantic["status"] == "failed" {
        return "failed";
    }
    if row["semantic"] == "PASSED"
        && semantic["status"] == "passed"
        && (case.unsigned || row["settlement"] == "USED")
    {
        "passed"
    } else {
        "incomplete"
    }
}
pub fn report(m: &Manifest, r: &Value) -> Result<Value> {
    validate(m)?;
    let groups = groups(m)?;
    if groups.is_empty() {
        return Ok(json!([]));
    }
    let cases = indexed(&r["cases"], "case")?;
    let semantics = indexed(&r["provider_semantics"], "case")?;
    let mut times = BTreeMap::new();
    for event in r["runtime_events"]
        .as_array()
        .context("reliability runtime events missing")?
        .iter()
        .filter(|e| e["kind"] == "mcp_dispatch_intent")
    {
        let id = event["detail"]["case"]
            .as_str()
            .context("dispatch case missing")?;
        ensure!(cases.contains_key(id), "dispatch names an unknown case");
        ensure!(
            times
                .insert(
                    id,
                    event["at"].as_u64().context("dispatch timestamp invalid")?
                )
                .is_none(),
            "duplicate dispatch intent"
        );
    }
    let mut result = Vec::new();
    for samples in groups.values() {
        let mut rows = Vec::new();
        let mut previous = None;
        let mut invalid = false;
        let mut passed = 0;
        let mut failed = 0;
        let mut incomplete = 0;
        for sample in samples {
            let id = sample.case.id.as_str();
            let row = cases.get(id).context("reliability case evidence missing")?;
            let semantic = semantics
                .get(id)
                .context("reliability semantic evidence missing")?;
            let time = times.get(id).copied();
            let mut category = "observed";
            let status = if row["execution"] == "UNATTEMPTED" && time.is_some() {
                invalid = true;
                category = "unattempted_case_has_dispatch";
                "invalid"
            } else if let Some(at) = time {
                let in_window = at >= sample.window.not_before && at < sample.window.not_after;
                let spaced = previous.is_none_or(|before: u64| {
                    before
                        .checked_add(sample.spacing)
                        .is_some_and(|start| at >= start)
                        && (!sample.later_day || at / 86400 > before / 86400)
                });
                previous = Some(at);
                if !in_window || !spaced {
                    invalid = true;
                    category = "dispatch_outside_reviewed_window_or_spacing";
                    "invalid"
                } else {
                    observation(row, semantic, &r["canonical_api_debits"], sample.case)
                }
            } else {
                category = "dispatch_not_observed";
                "incomplete"
            };
            match status {
                "passed" => passed += 1,
                "failed" => failed += 1,
                _ => incomplete += 1,
            }
            rows.push(json!({"case":id,"phase":sample.phase.id,"window":sample.window.id,"dispatch_at":time,"status":status,"category":category,
                "mcp":row["semantic"],"provider_semantics":semantic["status"],"settlement":row["settlement"]}));
        }
        let status = if invalid {
            "invalid"
        } else if incomplete > 0 {
            "incomplete"
        } else if failed > 0 {
            "failed"
        } else {
            "passed"
        };
        result.push(json!({"status":status,"planned":samples.len(),"passed":passed,"failed":failed,"incomplete":incomplete,"samples":rows,
            "scope":"Repeated identical end-to-end cases using driver dispatch-intent wall-clock timestamps, not packet-arrival timing. Failure is not automatically attributed to the provider or Tor; no future reliability guarantee."}));
    }
    Ok(json!(result))
}
pub fn qualification(r: &Value) -> Result<(bool, bool)> {
    let rows = r["reliability"]
        .as_array()
        .context("reliability report missing")?;
    Ok((
        rows.iter()
            .all(|r| matches!(r["status"].as_str(), Some("passed" | "failed"))),
        rows.iter().all(|r| r["status"] == "passed"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Manifest, Value) {
        let mut m = crate::tests::manifest();
        m.limits.run_seconds = 172800;
        m.cases[0].unsigned = true;
        m.cases[0].reserve_usdc = "0".into();
        m.start.pools.clear();
        m.phases[0].pools.clear();
        m.cases[0].checks = vec![crate::semantics::Check::JsonExists {
            pointer: "/data".into(),
        }];
        let mut second = m.cases[0].clone();
        second.id = "later".into();
        m.cases.push(second);
        m.windows[0].not_before = 10;
        m.windows[0].not_after = 100;
        let mut window = m.windows[0].clone();
        window.id = "day_two".into();
        window.not_before = 86410;
        window.not_after = 86500;
        m.windows.push(window);
        m.phases[0].scenario = Scenario::Reliability {
            min_spacing_seconds: 60,
            later_utc_day: true,
        };
        let mut phase = m.phases[0].clone();
        phase.id = "later_phase".into();
        phase.window = "day_two".into();
        phase.cases = vec!["later".into()];
        m.phases.push(phase);
        let r = json!({"cases":[{"case":"call","execution":"COMPLETED","semantic":"PASSED","settlement":"NOT_SIGNED"},{"case":"later","execution":"COMPLETED","semantic":"PASSED","settlement":"NOT_SIGNED"}],
            "provider_semantics":[{"case":"call","status":"passed"},{"case":"later","status":"passed"}],
            "canonical_api_debits":{"status":"validated","cases":{}},
            "runtime_events":[{"kind":"mcp_dispatch_intent","detail":{"case":"call"},"at":20},{"kind":"mcp_dispatch_intent","detail":{"case":"later"},"at":86420}]});
        (m, r)
    }
    #[test]
    fn separated_successes_qualify_without_claiming_provider_health() {
        let (m, r) = fixture();
        let report = report(&m, &r).unwrap();
        assert_eq!(report[0]["status"], "passed");
        assert_eq!(report[0]["passed"], 2);
    }
    #[test]
    fn failures_missing_windows_and_manual_semantics_remain_visible() {
        let (m, mut r) = fixture();
        r["cases"][0]["semantic"] = json!("FAILED");
        r["provider_semantics"][0]["status"] = json!("failed");
        let v = report(&m, &r).unwrap();
        assert_eq!(v[0]["status"], "failed");
        assert_eq!(v[0]["failed"], 1);
        r["cases"][1]["execution"] = json!("UNATTEMPTED");
        r["runtime_events"].as_array_mut().unwrap().pop();
        let v = report(&m, &r).unwrap();
        assert_eq!(v[0]["status"], "incomplete");
        assert_eq!(v[0]["failed"], 1);
        let (m, mut r) = fixture();
        r["provider_semantics"][1]["status"] = json!("manual_review_required");
        assert_eq!(report(&m, &r).unwrap()[0]["status"], "incomplete");
    }
    #[test]
    fn invalid_timestamps_duplicates_changed_assertions_and_singletons_cannot_qualify() {
        let (m, r) = fixture();
        for at in [19, 100, 86399, 86500] {
            let mut r = r.clone();
            r["runtime_events"][1]["at"] = json!(at);
            assert_eq!(report(&m, &r).unwrap()[0]["status"], "invalid");
        }
        let mut duplicate = r.clone();
        let event = duplicate["runtime_events"][0].clone();
        duplicate["runtime_events"]
            .as_array_mut()
            .unwrap()
            .push(event);
        assert!(report(&m, &duplicate).is_err());
        let mut changed = m.clone();
        changed.cases[1].checks.clear();
        assert!(validate(&changed).is_err());
        let mut single = m;
        single.phases.pop();
        assert!(validate(&single).is_err());
    }
    #[test]
    fn impossible_deadlines_are_refused_instead_of_reset_between_windows() {
        let (mut m, _) = fixture();
        m.limits.run_seconds = 86310;
        assert!(
            validate(&m)
                .unwrap_err()
                .to_string()
                .contains("original run deadline")
        );
        m.limits.run_seconds = 86311;
        validate(&m).unwrap();
        m.limits.run_seconds = 604800;
        m.validate().unwrap();
        m.limits.run_seconds = 604801;
        assert!(m.validate().is_err());
        m.limits.run_seconds = 604800;
        m.limits.call_seconds = 86401;
        assert!(m.validate().is_err());
    }
    #[test]
    fn paid_success_needs_a_canonical_debit_in_each_window() {
        let (mut m, mut r) = fixture();
        for c in &mut m.cases {
            c.unsigned = false;
            c.reserve_usdc = "0.02".into();
        }
        for c in r["cases"].as_array_mut().unwrap() {
            c["settlement"] = json!("USED");
        }
        r["canonical_api_debits"]["cases"] = json!({"call":"1000"});
        assert_eq!(report(&m, &r).unwrap()[0]["status"], "incomplete");
        r["canonical_api_debits"]["cases"]["later"] = json!("1000");
        assert_eq!(report(&m, &r).unwrap()[0]["status"], "passed");
    }
}
